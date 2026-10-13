//! Pull requests read off the forge, end to end (T-739): a stub `gh` on the
//! daemon's PATH answers for github.com, the ticket's PR binds by its own
//! branch, its failing pipeline reaches the snapshot and the gates, `m`
//! says the PR is the merge, and once the stub says merged the ticket reads
//! merged by the forge's word and a column's `Arrives here when: merged`
//! takes it.
//!
//! The stub is the forge: `auth status` passes, `pr list` prints the file
//! the test writes, `api …/check-runs` the merge commit's pipeline. No
//! network, no token, nothing of the real CLI.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::{Path, PathBuf};
use std::time::Duration;

use mesimon_core::board::{Arrival, SessionKind, WorkspaceStrategy};
use mesimon_core::command::{Command, MergeOutcome, PrItem, Response};

/// The stub forge CLI: one shell script answering as `gh` would, from the
/// files the test points it at.
const GH_STUB: &str = r#"#!/bin/sh
case "$1 $2" in
  "auth status") exit 0 ;;
  "pr list") cat "$MSMN_GH_PRS" 2>/dev/null || echo '[]' ;;
  api\ *)
    case "$4" in
      *check-runs*) cat "$MSMN_GH_CHECKS" 2>/dev/null || echo '{"check_runs":[]}' ;;
      *deployments/*/statuses*) echo '[{"state":"success"}]' ;;
      *deployments*) cat "$MSMN_GH_DEPLOYS" 2>/dev/null || echo '[]' ;;
      *) echo '{}' ;;
    esac ;;
  *) echo "stub gh: $*" >&2; exit 1 ;;
esac
"#;

/// A worktree ticket with a spawned agent and one commit on its branch.
fn ready(c: &mut TestClient, title: &str) -> (ulid::Ulid, uuid::Uuid, String, PathBuf) {
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    });
    let id = c.board().tickets.iter().find(|t| t.title == title).unwrap().id;
    assert!(matches!(
        c.request(Command::SetWorkspace { id, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: id,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning
    ));
    let wt = wait_attached(c, id);
    let path = PathBuf::from(wt.path.clone().expect("an attached binding has a path"));
    wait_until(Duration::from_secs(15), "the parked spawn to land", || {
        c.board().sessions.iter().any(|s| s.ticket == id)
    });
    let sid = c.board().sessions.iter().find(|s| s.ticket == id).unwrap().id;
    std::fs::write(path.join(format!("{title}.txt")), format!("{title}\n")).unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", title]);
    (id, sid, wt.branch, path)
}

fn prs_of(c: &mut TestClient) -> Vec<PrItem> {
    match c.request(Command::Snapshot) {
        Response::Board { prs, .. } => prs,
        other => panic!("not a board: {other:?}"),
    }
}

/// One PR as `gh pr list --json` prints it.
fn pr_json(branch: &str, tip: &str, state: &str, merge_oid: Option<&str>, failing: Option<&str>) -> String {
    let rollup = match failing {
        Some(job) => format!(
            r#"[{{"__typename":"CheckRun","name":"{job}","status":"COMPLETED","conclusion":"FAILURE"}},
                {{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}}]"#
        ),
        None => r#"[{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}]"#
            .to_string(),
    };
    let merge = match merge_oid {
        Some(oid) => format!(r#"{{"oid":"{oid}"}}"#),
        None => "null".to_string(),
    };
    format!(
        r#"[{{"number":42,"url":"https://github.com/example/forge/pull/42","state":"{state}","isDraft":false,
          "headRefName":"{branch}","headRefOid":"{tip}","mergeCommit":{merge},"mergeable":"MERGEABLE",
          "reviewDecision":"REVIEW_REQUIRED","updatedAt":"2026-10-13T06:00:00Z","statusCheckRollup":{rollup}}}]"#
    )
}

fn write(path: &Path, body: &str) {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

#[test]
fn a_pull_request_binds_pipelines_gate_and_the_forges_merge_lands() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-stub-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let gh = stub_dir.join("gh");
    std::fs::write(&gh, GH_STUB).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let prs_file = stub_dir.join("prs.json");
    let checks_file = stub_dir.join("checks.json");
    write(&prs_file, "[]");
    write(&checks_file, r#"{"check_runs":[{"name":"release","status":"completed","conclusion":"success"}]}"#);
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_CHECKS", &checks_file.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    // A github.com remote is what names the forge; nothing is ever fetched.
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let mut c = h.client("forge");
    let (id, sid, branch, path) = ready(&mut c, "alpha");
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();

    // The PR opens on the forge, its pipeline red: bound by the ticket's own
    // branch, read with the job's name, and no merge of mesimon's while it
    // stands.
    write(&prs_file, &pr_json(&branch, &tip, "OPEN", None, Some("test-linux")));
    wait_until(Duration::from_secs(20), "the PR to bind", || {
        prs_of(&mut c).iter().any(|p| p.ticket == id && p.number == 42)
    });
    let pr = prs_of(&mut c).into_iter().find(|p| p.ticket == id).unwrap();
    assert_eq!((pr.forge.as_str(), pr.state.as_str()), ("github", "open"));
    assert_eq!(pr.pipeline, "failed");
    assert_eq!(pr.job.as_deref(), Some("test-linux"));
    assert_eq!(pr.review, "required");
    // The first read is a baseline: the feed says nothing of it, and a
    // restart's first read never says every old PR over.
    assert_eq!(feed_count(&h, "pr", id), 0, "a first read says nothing");

    // A column that needs a passed pipeline refuses it, in the pipeline's
    // words; the agent's allowed columns say the same.
    let mut review = c.board().column("REVIEW").unwrap().settings.clone();
    review.requires_pipeline = true;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "REVIEW".into(), settings: review }),
        Response::Ok
    ));
    match c.request(Command::MoveTicket { id, column: "REVIEW".into(), before: None }) {
        Response::Err { message } => assert!(message.contains("pipeline failed: test-linux"), "{message}"),
        other => panic!("REVIEW with a red pipeline: {other:?}"),
    }

    // `m` says the PR is the merge. The quiet gate has first say, so the
    // agent goes first.
    let _ = c.request(Command::KillSession { id: sid });
    wait_until(Duration::from_secs(15), "the merge to answer for the PR", || {
        matches!(
            c.request(Command::MergeTicket { id }),
            Response::Merge { outcome: MergeOutcome::Refused, detail, .. } if detail.contains("PR #42 open")
        )
    });
    match c.request(Command::MergeTicket { id }) {
        Response::Merge { outcome: MergeOutcome::Refused, detail, .. } => {
            assert!(detail.contains("merge it on GitHub"), "{detail}");
        }
        other => panic!("merge with a PR open: {other:?}"),
    }

    // DONE takes a ticket once merged, on the forge's word.
    let mut done = c.board().column("DONE").unwrap().settings.clone();
    done.arrives_on = Arrival::Merged;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "DONE".into(), settings: done }),
        Response::Ok
    ));
    let merge_oid = "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d";
    write(&prs_file, &pr_json(&branch, &tip, "MERGED", Some(merge_oid), None));
    wait_until(Duration::from_secs(20), "the board to read the merge", || {
        worktrees_of(&mut c).iter().any(|w| w.ticket == id && w.merged)
    });
    let after = worktrees_of(&mut c).into_iter().find(|w| w.ticket == id).unwrap();
    assert_eq!(after.merged_in, "main (PR #42)", "landed on the base, by the forge's word");
    assert_eq!(after.merged_oid, merge_oid);
    assert!(!after.needs_rebase);
    wait_until(Duration::from_secs(20), "the ticket to arrive in DONE", || {
        c.board().ticket(id).is_some_and(|t| t.column == "DONE")
    });
    let pr = prs_of(&mut c).into_iter().find(|p| p.ticket == id).unwrap();
    assert_eq!(pr.state, "merged");
    assert_eq!(pr.pipeline, "passed", "the merge commit's pipeline, off check-runs");
    wait_until(Duration::from_secs(10), "the feed to say the merge", || feed_count(&h, "pr", id) >= 1);
    let _ = std::fs::remove_dir_all(&stub_dir);
}
