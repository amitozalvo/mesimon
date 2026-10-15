//! Pull requests read off the forge, end to end (T-739): a stub `gh` on the
//! daemon's PATH answers for github.com, the ticket's PR binds by its own
//! branch, its failing pipeline reaches the snapshot and the gates, `m`
//! says the PR is the merge, and once the stub says merged the ticket reads
//! merged by the forge's word and a column's `Arrives here when: merged`
//! takes it.
//!
//! The crown hears the forge's edges (T-744): a pipeline that passes, a job
//! that waits on a person, a conflict and a review on one pass, each a
//! `forge` wake in the edge's words.
//!
//! The PRs survive a restart (T-752): the first pass lists as it probes,
//! `forge.json` brings the set back, the first snapshot after a restart
//! carries it while the forge has not answered yet, and the first live pass
//! says only what moved while the daemon was down.
//!
//! One workflow per board (T-810): every test here sets Merge into: pull
//! requests but one, which shows a board that merges into main calling no
//! `gh` at all while its train merges a turn that pushed. A pull-requests
//! board's train lands no branch and `m` asks for a PR.
//!
//! The train asks once per conflict (T-770): a PR the forge flips to
//! conflicting is asked about once, in words naming it; its flip back asks
//! nothing; a second conflict at the same local base is asked about again;
//! and a restart asks nothing it already asked.
//!
//! A ticket arrives as its least advanced PR (T-768): with a PR on each of
//! two repositories, a `deployed` column waits for the second one.
//!
//! An older PR keeps its own (T-771): once a newer PR of the same ticket
//! merges and deploys, the first one's row still reads its merge commit's
//! pipeline and where it was deployed.
//!
//! An open PR holds DONE (T-774): a merged ticket already in DONE whose
//! follow-up PR opens on another branch reads unmerged again where it is,
//! a person's move of another such ticket into DONE is refused in the merge
//! refusal's words, and once the PRs merge DONE admits it.
//!
//! A turn's end's wake waits for that turn's forge read (T-773): a worker
//! that pushed and opened a PR wakes the crown once, after the read, the
//! line saying the opening and the crown's `get_ticket` listing the PR.
//!
//! The pipeline gate judges the branch's tip (T-780): a commit after the
//! PR's pipeline passed holds the turn's end out of REVIEW as not read,
//! and the forge's pass at the new head is the arrival; a merge git read
//! first is named by the forge's word once it says the PR merged.
//!
//! A pipeline in flight is read on the busy cadence (T-781): a merged PR
//! whose merge commit's pipeline runs is read again within
//! `MESIMON_FORGE_BUSY_MS`, and once it passes the minute's cadence is back.
//!
//! A lost sign-in is asked again on the backoff (T-798): a restart with
//! `auth status` failing names the host on the advisory row, and once it
//! answers a ticket opened meanwhile binds.
//!
//! A closed PR stays the ticket's (T-797): it holds the train and `m` as a
//! merged one does, and a PR opened here and closed, or a `pr create` on a
//! checkout ticket no forge read covers, holds no pipeline column.
//!
//! A ticket's PRs are the ones its worktree wrote (T-796): a ticket cut at
//! a fast-forward merged MR's head binds nothing of it, and the MR's own
//! ticket keeps it after the base took the merge.
//!
//! A deploy of a later commit deploys every merge it carries (T-801): with
//! the history here, a ticket's two merged MRs read deployed by ancestry,
//! the crown hears each, the ticket arrives once both are there, and a
//! rollback takes the environment off their rows and leaves the card.
//!
//! The stub is the forge: `auth status` passes, `pr list` prints the file
//! the test writes, `api …/check-runs` and `api …/status` the merge
//! commit's pipeline, by sha where the test wrote one, and `api
//! …/deployments` what an environment ran (T-801). No network, no token,
//! nothing of the real CLI.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::{Path, PathBuf};
use std::time::Duration;

use mesimon_core::board::{Arrival, MergeInto, SessionKind, SessionState, WorkspaceStrategy};
use mesimon_core::command::{Command, MergeOutcome, PrItem, Response};
use mesimon_core::Principal;

/// The stub forge CLI: one shell script answering as `gh` would, from the
/// files the test points it at.
const GH_STUB: &str = r#"#!/bin/sh
[ -n "$MSMN_GH_LOG" ] && echo "$*" >> "$MSMN_GH_LOG"
case "$1 $2" in
  "auth status") exit 0 ;;
  "pr list")
    [ -f "$MSMN_GH_SLOW" ] && sleep "$(cat "$MSMN_GH_SLOW")"
    [ -f "$MSMN_GH_FAIL" ] && { echo "HTTP 502: Bad Gateway" >&2; exit 1; }
    cat "$MSMN_GH_PRS" 2>/dev/null || echo '[]' ;;
  api\ *)
    case "$4" in
      *check-runs*)
        sha=$(echo "$4" | sed -n 's|.*/commits/\([^/]*\)/check-runs.*|\1|p')
        if [ -f "$MSMN_GH_CHECKS.$sha" ]; then cat "$MSMN_GH_CHECKS.$sha"
        else cat "$MSMN_GH_CHECKS" 2>/dev/null || echo '{"check_runs":[]}'; fi ;;
      */commits/*/status) cat "$MSMN_GH_STATUS" 2>/dev/null || echo '{"state":"pending","statuses":[]}' ;;
      *pulls/*/commits*)
        n=$(echo "$4" | sed -n 's|.*/pulls/\([0-9]*\)/commits.*|\1|p')
        cat "$MSMN_GH_COMMITS.$n" 2>/dev/null || echo '[]' ;;
      *deployments/*/statuses*) echo '[{"state":"success"}]' ;;
      *deployments*) cat "$MSMN_GH_DEPLOYS" 2>/dev/null || echo '[]' ;;
      *) echo '{}' ;;
    esac ;;
  *) echo "stub gh: $*" >&2; exit 1 ;;
esac
"#;

/// A worktree ticket with a spawned agent and one commit on its branch.
fn ready(c: &mut TestClient, title: &str) -> (ulid::Ulid, uuid::Uuid, String, PathBuf) {
    let (id, sid, branch, path) = cut(c, title);
    std::fs::write(path.join(format!("{title}.txt")), format!("{title}\n")).unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", title]);
    (id, sid, branch, path)
}

/// A worktree ticket with a spawned agent, its branch as it was cut.
fn cut(c: &mut TestClient, title: &str) -> (ulid::Ulid, uuid::Uuid, String, PathBuf) {
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
    (id, sid, wt.branch, path)
}

/// The board's branches land through pull requests (T-810): every test
/// here but the one about main reads the forge.
fn through_prs(c: &mut TestClient) {
    assert!(matches!(
        c.request(Command::SetMergeInto { into: MergeInto::PullRequests }),
        Response::Ok
    ));
}

fn prs_of(c: &mut TestClient) -> Vec<PrItem> {
    match c.request(Command::Snapshot) {
        Response::Board { prs, .. } => prs,
        other => panic!("not a board: {other:?}"),
    }
}

/// The snapshot's held automoves (T-749), by ticket.
fn held_of(c: &mut TestClient) -> Vec<(ulid::Ulid, String)> {
    match c.request(Command::Snapshot) {
        Response::Board { automation, .. } => {
            automation.held.into_iter().map(|h| (h.ticket, h.words)).collect()
        }
        other => panic!("not a board: {other:?}"),
    }
}

/// One PR as `gh pr list --json` prints it.
fn pr_json(
    branch: &str,
    tip: &str,
    state: &str,
    merge_oid: Option<&str>,
    failing: Option<&str>,
) -> String {
    let rollup = match failing {
        Some(job) => format!(
            r#"[{{"__typename":"CheckRun","name":"{job}","status":"COMPLETED","conclusion":"FAILURE"}},
                {{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}}]"#
        ),
        None => r#"[{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}]"#
            .to_string(),
    };
    let (merge, merged_at) = match merge_oid {
        Some(oid) => (format!(r#"{{"oid":"{oid}"}}"#), format!(r#""{}""#, iso_now())),
        None => ("null".to_string(), "null".to_string()),
    };
    format!(
        r#"[{{"number":42,"url":"https://github.com/example/forge/pull/42","state":"{state}","isDraft":false,
          "headRefName":"{branch}","headRefOid":"{tip}","mergeCommit":{merge},"mergedAt":{merged_at},"mergeable":"MERGEABLE",
          "reviewDecision":"REVIEW_REQUIRED","updatedAt":"2026-10-13T06:00:00Z","statusCheckRollup":{rollup}}}]"#
    )
}

/// Now, as the forges stamp a time: a merge this test makes is a young one.
fn iso_now() -> String {
    let secs =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let t = mesimon_core::usage::utc_of(secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.year + 1900,
        t.mon + 1,
        t.mday,
        t.hour,
        t.min,
        t.sec
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
    let status_file = stub_dir.join("status.json");
    write(&prs_file, "[]");
    write(
        &checks_file,
        r#"{"check_runs":[{"name":"release","status":"in_progress","conclusion":null}]}"#,
    );
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
            ("MSMN_GH_STATUS", &status_file.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    // A github.com remote is what names the forge; nothing is ever fetched.
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let mut c = h.client("forge");
    through_prs(&mut c);
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
        Response::Err { message } => {
            assert!(message.contains("pipeline failed: test-linux"), "{message}")
        }
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

    // DONE takes a ticket once merged, on the forge's word; REVIEW, before
    // it, once a pipeline passes.
    let mut done = c.board().column("DONE").unwrap().settings.clone();
    done.arrives_on = Arrival::Merged;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "DONE".into(), settings: done }),
        Response::Ok
    ));
    let mut review = c.board().column("REVIEW").unwrap().settings.clone();
    review.arrives_on = Arrival::PipelinePasses;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "REVIEW".into(), settings: review }),
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
    assert_eq!(pr.pipeline, "running", "the merge commit's pipeline, off check-runs");
    wait_until(Duration::from_secs(10), "the feed to say the merge", || {
        feed_count(&h, "pr", id) >= 1
    });

    // The merge commit's check runs pass and its commit status, a Jenkins
    // job's, fails: the worse of the two (T-800).
    write(
        &status_file,
        r#"{"state":"failure","statuses":[{"context":"jenkins","state":"failure"}]}"#,
    );
    write(
        &checks_file,
        r#"{"check_runs":[{"name":"release","status":"completed","conclusion":"success"}]}"#,
    );
    wait_until(Duration::from_secs(20), "the commit status to fail the merge commit", || {
        prs_of(&mut c).iter().any(|p| {
            p.ticket == id && p.pipeline == "failed" && p.job.as_deref() == Some("jenkins")
        })
    });
    // Retried green: a failure hours young is read again (T-800). The merge
    // commit's pipeline passing is its deploy run finishing, not the
    // ticket's pipeline passing: REVIEW does not pull it back (T-767).
    write(
        &status_file,
        r#"{"state":"success","statuses":[{"context":"jenkins","state":"success"}]}"#,
    );
    wait_until(Duration::from_secs(20), "the merge commit's pipeline to pass", || {
        prs_of(&mut c).iter().any(|p| p.ticket == id && p.pipeline == "passed")
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(c.board().ticket(id).unwrap().column, "DONE");
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// Entry needs a passed pipeline only where there is one to pass (T-750): a
/// ticket that opened no PR enters at its turn end, as a ticket with no
/// branch passes the merge rule; one whose transcript printed a `gh pr
/// create` URL the forge has not read yet is held, judged on that same
/// turn's lines, until its pipeline reads passed.
#[test]
fn a_ticket_with_no_pull_request_enters_and_one_opened_here_waits() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-nopr-{}", std::process::id()));
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
    write(&prs_file, "[]");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge_nopr",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_nopr");
    through_prs(&mut c);
    let mut review = c.board().column("REVIEW").unwrap().settings.clone();
    review.requires_pipeline = true;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "REVIEW".into(), settings: review }),
        Response::Ok
    ));
    let turn = |c: &mut TestClient, sid: uuid::Uuid, transcript: &Path| {
        hook_send(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            &format!(r#"{{"session_id":"x","transcript_path":"{}"}}"#, transcript.display()),
        );
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };

    // The PR ticket: its turn ran `gh pr create`, whose result printed the
    // URL, and the forge has not bound it when the turn ends.
    let (pr_id, pr_sid, branch, path) = ready(&mut c, "opened");
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    let opened = h.dir.join("opened.jsonl");
    std::fs::write(
        &opened,
        concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"gh pr create --fill"}}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"https://github.com/example/forge/pull/42\n"}]}}"#,
            "\n",
        ),
    )
    .unwrap();
    turn(&mut c, pr_sid, &opened);

    // The ticket with no PR, on the checkout: nothing to pass.
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "plain".into(),
        workspace: None,
        tier: None,
    });
    let plain = c.board().tickets.iter().find(|t| t.title == "plain").unwrap().id;
    let plain_sid = match c.request(Command::SpawnSession {
        ticket: plain,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    let quiet = h.dir.join("plain.jsonl");
    std::fs::write(&quiet, "{\"type\":\"user\",\"message\":{\"content\":\"go\"}}\n").unwrap();
    turn(&mut c, plain_sid, &quiet);
    wait_until(Duration::from_secs(10), "the ticket with no PR to enter REVIEW", || {
        c.board().ticket(plain).is_some_and(|t| t.column == "REVIEW")
    });

    // The PR ticket is held at its turn end, in the gate's words.
    let feed = h.paths.state_dir.join("activity.jsonl");
    wait_until(Duration::from_secs(10), "the held automove's feed line", || {
        std::fs::read_to_string(&feed).unwrap_or_default().lines().any(|l| {
            l.contains("\"kind\":\"movement_decision\"")
                && l.contains(&pr_id.to_string())
                && l.contains("refused: pipeline not read — before REVIEW")
        })
    });
    assert_ne!(c.board().ticket(pr_id).unwrap().column, "REVIEW");
    // And on the page and in `get_ticket` (T-749), in the same words.
    let words = "pipeline not read — before REVIEW";
    assert_eq!(held_of(&mut c), [(pr_id, words.to_string())]);
    match c.send(Principal::Agent { session: pr_sid }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            assert_eq!(ticket.automove.held.as_deref(), Some(words))
        }
        other => panic!("get_ticket: {other:?}"),
    }
    match c.request(Command::MoveTicket { id: pr_id, column: "REVIEW".into(), before: None }) {
        Response::Err { message } => assert!(message.contains("pipeline not read"), "{message}"),
        other => panic!("REVIEW before the forge read the PR: {other:?}"),
    }
    // A person's refusal is the dialog's to say: the automation's stays.
    assert_eq!(held_of(&mut c), [(pr_id, words.to_string())]);

    // The forge binds it, its pipeline green: the rule passes, the held
    // line goes with the card still where it was, and the move goes through.
    write(&prs_file, &pr_json(&branch, &tip, "OPEN", None, None));
    wait_until(Duration::from_secs(20), "the PR to bind, passed", || {
        prs_of(&mut c).iter().any(|p| p.ticket == pr_id && p.pipeline == "passed")
    });
    assert_eq!(held_of(&mut c), []);
    assert!(matches!(
        c.request(Command::MoveTicket { id: pr_id, column: "REVIEW".into(), before: None }),
        Response::Ok
    ));

    // T-797: a PR this ticket's turn opened, read closed unmerged, has no
    // pipeline to pass: its URL is one the forge read, and the turn's end
    // enters REVIEW.
    let (gone, gone_sid, gone_branch, gone_path) = ready(&mut c, "dropped");
    let gone_tip = git(&gone_path, &["rev-parse", "HEAD"]).trim().to_string();
    write(
        &prs_file,
        &format!(
            "[{},{}]",
            numbered_pr(42, &branch, &tip, false),
            closed_pr(44, &gone_branch, &gone_tip)
        ),
    );
    wait_until(Duration::from_secs(20), "the closed PR to bind", || {
        prs_of(&mut c).iter().any(|p| p.ticket == gone && p.number == 44 && p.state == "closed")
    });
    let dropped = h.dir.join("dropped.jsonl");
    std::fs::write(
        &dropped,
        concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_4","name":"Bash","input":{"command":"gh pr create --fill"}}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_4","content":"https://github.com/example/forge/pull/44\n"}]}}"#,
            "\n",
        ),
    )
    .unwrap();
    turn(&mut c, gone_sid, &dropped);
    wait_until(Duration::from_secs(10), "the closed PR's ticket to enter REVIEW", || {
        c.board().ticket(gone).is_some_and(|t| t.column == "REVIEW")
    });

    // And a checkout ticket whose turn ran `pr create`: no forge read asks
    // about a ticket with no branch, so it waits for no read (T-750's pass).
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "aside".into(),
        workspace: None,
        tier: None,
    });
    let aside = c.board().tickets.iter().find(|t| t.title == "aside").unwrap().id;
    let aside_sid = match c.request(Command::SpawnSession {
        ticket: aside,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    let created = h.dir.join("aside.jsonl");
    std::fs::write(
        &created,
        concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_5","name":"Bash","input":{"command":"gh pr create --fill"}}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_5","content":"https://github.com/example/forge/pull/45\n"}]}}"#,
            "\n",
        ),
    )
    .unwrap();
    turn(&mut c, aside_sid, &created);
    wait_until(Duration::from_secs(10), "the checkout ticket to enter REVIEW", || {
        c.board().ticket(aside).is_some_and(|t| t.column == "REVIEW")
    });
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// The crown's stub: each line it reads is appended to `got.txt` beside it.
const RECORDING_STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                              printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

/// An open PR as `gh pr list --json` prints it, with the checks, the
/// mergeability and the review decision the test names.
fn open_pr_json(branch: &str, tip: &str, checks: &str, mergeable: &str, review: &str) -> String {
    format!(
        r#"[{{"number":42,"url":"https://github.com/example/forge/pull/42","state":"OPEN","isDraft":false,
          "headRefName":"{branch}","headRefOid":"{tip}","mergeCommit":null,"mergeable":"{mergeable}",
          "reviewDecision":"{review}","updatedAt":"2026-10-13T06:00:00Z","statusCheckRollup":{checks}}}]"#
    )
}

/// The crown hears its worker's PR (T-744): the pipeline passing, a job
/// waiting on a person, and a conflict with a review on one pass — one
/// `forge` wake each, the second pass's two edges in one line.
#[test]
fn the_crown_hears_the_forges_edges() {
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-crown-{}", std::process::id()));
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
    write(&prs_file, "[]");
    // The merge commits' pipeline, running until the first merge has woken
    // the crown.
    let checks_file = stub_dir.join("checks.json");
    write(&checks_file, r#"{"check_runs":[{"name":"release","status":"in_progress"}]}"#);
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge_crown",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
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
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_crown");
    through_prs(&mut c);
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let turn = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let create = |c: &mut TestClient, title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };

    // The crown, idle, and a worker it started on a worktree.
    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "forge worker");
    let kw = c.board().ticket(w).unwrap().short_key.clone();
    let sa = match c.request(Command::SpawnSession {
        ticket: a,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(Duration::from_millis(500));
    turn(&mut c, sa);
    let seen = match c
        .send(Principal::Agent { session: sa }, Command::AgentReadTicket { key: kw.clone() })
    {
        Response::AgentTicket { ticket } => ticket.seen,
        other => panic!("read: {other:?}"),
    };
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let wt = wait_attached(&mut c, w);
    let path = PathBuf::from(wt.path.clone().expect("a path"));
    wait_until(Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::thread::sleep(Duration::from_millis(500));
    std::fs::write(path.join("work.txt"), "work\n").unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", "work"]);
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    let worker = format!("{kw} \"forge worker\"");
    turn(&mut c, ws);
    wait_until(Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&format!("{worker} delivered")).len() == 1
    });
    turn(&mut c, sa);

    // The PR opens with its pipeline running: the baseline, no wake.
    let running =
        r#"[{"__typename":"CheckRun","name":"test","status":"IN_PROGRESS","conclusion":null}]"#;
    let passed =
        r#"[{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS"}]"#;
    let waiting = r#"[{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS"},
                      {"__typename":"CheckRun","name":"deploy:production","status":"WAITING","conclusion":null}]"#;
    write(&prs_file, &open_pr_json(&wt.branch, &tip, running, "MERGEABLE", "REVIEW_REQUIRED"));
    wait_until(Duration::from_secs(20), "the PR to bind", || {
        prs_of(&mut c).iter().any(|p| p.ticket == w && p.number == 42)
    });
    // The card names the check in progress (T-794); the agent's read keeps
    // its job for a wait or a failure.
    let row = prs_of(&mut c).into_iter().find(|p| p.ticket == w && p.number == 42).unwrap();
    assert_eq!((row.pipeline.as_str(), row.job.as_deref()), ("running", Some("test")));
    match c.send(Principal::Agent { session: ws }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            assert_eq!(ticket.prs[0].pipeline, "running", "{:?}", ticket.prs);
            assert_eq!(ticket.prs[0].job, None, "{:?}", ticket.prs);
        }
        other => panic!("get_ticket: {other:?}"),
    }
    let forge_wakes = || {
        std::fs::read_to_string(&feed_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| {
                l.contains("\"kind\":\"crown_wake\"")
                    && l.contains(&format!("\"worker\":\"{w}\""))
                    && l.contains("\"cause\":\"forge\"")
            })
            .count()
    };
    assert_eq!(forge_wakes(), 0, "an opening is the worker's turn end's to say");
    // The edge's words close the bracket; a column the worker's turn moved
    // may stand before them.
    let news = format!("{worker} has news from the forge (");
    let heard = |lines: &[String], words: &str| lines.iter().any(|l| l.contains(words));

    // ---- the pipeline passes ----------------------------------------------
    write(&prs_file, &open_pr_json(&wt.branch, &tip, passed, "MERGEABLE", "REVIEW_REQUIRED"));
    wait_until(Duration::from_secs(20), "the passed wake", || {
        heard(&lines_with(&news), "pipeline passed #42)")
    });
    wait_until(Duration::from_secs(5), "the forge wake's feed line", || forge_wakes() == 1);
    turn(&mut c, sa);

    // ---- a job waits on a person ------------------------------------------
    write(&prs_file, &open_pr_json(&wt.branch, &tip, waiting, "MERGEABLE", "REVIEW_REQUIRED"));
    wait_until(Duration::from_secs(20), "the waiting wake", || {
        heard(&lines_with(&news), "waiting on deploy:production #42)")
    });
    wait_until(Duration::from_secs(5), "the forge wake's feed line", || forge_wakes() == 2);
    turn(&mut c, sa);

    // ---- a conflict and an approval on one pass: one line -----------------
    write(&prs_file, &open_pr_json(&wt.branch, &tip, waiting, "CONFLICTING", "APPROVED"));
    wait_until(Duration::from_secs(20), "the conflict's and the review's wake", || {
        heard(&lines_with(&news), "conflict #42, review approved #42)")
    });
    wait_until(Duration::from_secs(5), "the forge wake's feed line", || forge_wakes() == 3);
    turn(&mut c, sa);

    // A pass that sees nothing new says nothing.
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(forge_wakes(), 3, "{}", std::fs::read_to_string(&got).unwrap());

    // ---- the PR merges: the flags' merged wake alone (T-757) ---------------
    // A commit made in the worktree and stepped back from first: in HEAD's
    // reflog, so a later PR at it binds, and not on the branch.
    std::fs::write(path.join("greeting.txt"), "hi\n").unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", "greeting"]);
    let fix = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    git(&path, &["reset", "-q", "--hard", "HEAD~1"]);
    // An open PR carries no verdict: a PR bound with one says it (T-769).
    let gh_pr = |number: u64, branch: &str, head: &str, state: &str, merge: &str, at: &str| {
        let (checks, review) =
            if state == "OPEN" { (running, "REVIEW_REQUIRED") } else { (passed, "APPROVED") };
        format!(
            r#"{{"number":{number},"url":"https://github.com/example/forge/pull/{number}","state":"{state}",
              "isDraft":false,"headRefName":"{branch}","headRefOid":"{head}","mergeCommit":{merge},
              "mergeable":"MERGEABLE","reviewDecision":"{review}","updatedAt":"{at}",
              "statusCheckRollup":{checks}}}"#
        )
    };
    let first = gh_pr(
        42,
        &wt.branch,
        &tip,
        "MERGED",
        &format!(r#"{{"oid":"{tip}"}}"#),
        "2026-10-13T06:00:00Z",
    );
    write(&prs_file, &format!("[{first}]"));
    let merged_wakes = || {
        std::fs::read_to_string(&feed_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| {
                l.contains("\"kind\":\"crown_wake\"")
                    && l.contains(&format!("\"worker\":\"{w}\""))
                    && l.contains("\"cause\":\"merged\"")
            })
            .count()
    };
    wait_until(Duration::from_secs(20), "the merge's wake", || merged_wakes() == 1);
    turn(&mut c, sa);
    // The base has it too, so the flags keep reading merged by ancestry once
    // a newer PR stands in front of this one.
    git(&h.repo, &["merge", "--ff-only", "-q", &wt.branch]);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(forge_wakes(), 3, "the first PR's merge is the flags' to say");
    // The merge commit's pipeline passing is no `pipeline passed` (T-767):
    // only an open PR's is.
    write(
        &checks_file,
        r#"{"check_runs":[{"name":"release","status":"completed","conclusion":"success"}]}"#,
    );
    wait_until(Duration::from_secs(20), "the merge commit's pipeline to pass", || {
        prs_of(&mut c).iter().any(|p| p.ticket == w && p.number == 42 && p.pipeline == "passed")
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(forge_wakes(), 3, "{}", std::fs::read_to_string(&got).unwrap());
    assert_eq!(lines_with("pipeline passed").len(), 1, "the open PR's alone");

    // ---- a later PR merges: the forge says it -----------------------------
    let open = gh_pr(43, "fix/greeting", &fix, "OPEN", "null", "2026-10-13T07:00:00Z");
    write(&prs_file, &format!("[{open},{first}]"));
    wait_until(Duration::from_secs(20), "the second PR to bind", || {
        prs_of(&mut c).iter().any(|p| p.ticket == w && p.number == 43)
    });
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(
        forge_wakes(),
        3,
        "an opening is not news: {}",
        std::fs::read_to_string(&got).unwrap()
    );
    let second =
        gh_pr(43, "fix/greeting", &fix, "MERGED", r#"{"oid":"feedf00d"}"#, "2026-10-13T07:05:00Z");
    write(&prs_file, &format!("[{second},{first}]"));
    wait_until(Duration::from_secs(20), "the later merge's wake", || {
        heard(&lines_with(&news), "merged #43)")
    });
    wait_until(Duration::from_secs(5), "the forge wake's feed line", || forge_wakes() == 4);
    turn(&mut c, sa);
    std::thread::sleep(Duration::from_millis(1500));
    let all = std::fs::read_to_string(&got).unwrap();
    assert!(!all.contains("merged #42"), "{all}");
    assert!(!all.contains("pipeline passed #43"), "a merged PR's pipeline: {all}");
    assert_eq!(merged_wakes(), 1, "{all}");
    assert_eq!(forge_wakes(), 4, "{all}");
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// The board's PRs survive a daemon restart (T-752). The cadence is a
/// minute, so the PR binding at all proves the first pass lists as it
/// probes; across the restart the stub holds every call three seconds, so
/// the snapshot read at once is `forge.json`'s; the PR merged while the
/// daemon was down is the one feed line the first live pass adds.
#[test]
fn the_prs_survive_a_restart_and_the_first_pass_lists() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-keep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let gh = stub_dir.join("gh");
    let hold = stub_dir.join("hold");
    std::fs::write(
        &gh,
        GH_STUB.replacen("case", "[ -f \"$(dirname \"$0\")/hold\" ] && sleep 3\ncase", 1),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let prs_file = stub_dir.join("prs.json");
    write(&prs_file, "[]");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge-keep",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "60000"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let mut c = h.client("forge-keep");
    through_prs(&mut c);
    // The PR is on the forge before the ticket's first pass: bound by the
    // branch mesimon will cut.
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "keep".into(),
        workspace: None,
        tier: None,
    });
    let id = c.board().tickets.iter().find(|t| t.title == "keep").unwrap().id;
    let key = c.board().ticket(id).unwrap().short_key.clone();
    let branch = mesimon_core::workspace::branch_name(&key, "keep");
    write(&prs_file, &pr_json(&branch, "0000", "OPEN", None, Some("test-linux")));
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
    assert_eq!(wait_attached(&mut c, id).branch, branch);
    wait_until(Duration::from_secs(20), "the first pass to bind the PR", || {
        prs_of(&mut c).iter().any(|p| p.ticket == id && p.number == 42)
    });
    let kept = h.paths.state_dir.join("forge.json");
    wait_until(Duration::from_secs(10), "forge.json to hold the PR", || {
        std::fs::read_to_string(&kept).is_ok_and(|t| t.contains("pull/42"))
    });
    assert_eq!(feed_count(&h, "pr", id), 0, "a first read says nothing");

    // Down, the PR merges on the forge, and the forge goes slow.
    drop(c);
    h.restart_after(|| {
        std::fs::write(&hold, "").unwrap();
        write(&prs_file, &pr_json(&branch, "0000", "MERGED", Some("feedf00d"), None));
    });
    let mut c = h.client("forge-keep-2");
    let (prs, forge) = match c.request(Command::Snapshot) {
        Response::Board { prs, forge, .. } => (prs, forge),
        other => panic!("not a board: {other:?}"),
    };
    assert_eq!(forge.sampled_at_ms, 0, "the forge has not answered yet: {forge:?}");
    let pr = prs
        .iter()
        .find(|p| p.ticket == id && p.number == 42)
        .expect("the PR on the first snapshot");
    assert_eq!(
        (pr.state.as_str(), pr.pipeline.as_str()),
        ("open", "failed"),
        "as the last read left it"
    );
    // A loaded pipeline is not a gate's answer: only a live read is.
    let mut review = c.board().column("REVIEW").unwrap().settings.clone();
    review.requires_pipeline = true;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "REVIEW".into(), settings: review }),
        Response::Ok
    ));
    match c.request(Command::MoveTicket { id, column: "REVIEW".into(), before: None }) {
        Response::Err { message } => assert!(message.contains("pipeline not read"), "{message}"),
        other => panic!("REVIEW before a live read: {other:?}"),
    }

    // The first live pass says what moved while down, and only that.
    let _ = std::fs::remove_file(&hold);
    wait_until(Duration::from_secs(30), "the first live pass to read the merge", || {
        prs_of(&mut c).iter().any(|p| p.ticket == id && p.state == "merged")
    });
    wait_until(Duration::from_secs(10), "the feed to say the merge", || {
        feed_count(&h, "pr", id) >= 1
    });
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(feed_count(&h, "pr", id), 1, "the merge, and no PR said over");
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// A failed sign-in is a lost host (T-798): a daemon that starts before the
/// network finds `gh auth status` failing on a host whose PRs it holds. The
/// advisory row names the host, the cards keep their PRs, and the probe is
/// asked again on the backoff, not six hours on: once the sign-in answers,
/// a ticket opened meanwhile binds and the row clears.
#[test]
fn a_lost_sign_in_is_said_and_asked_again_on_the_backoff() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let name = "forge_lost";
    let (stub_dir, path_var) = gh_stub(name);
    let gh = stub_dir.join("gh");
    let offline = stub_dir.join("offline");
    std::fs::write(
        &gh,
        GH_STUB.replacen(
            "\"auth status\") exit 0 ;;",
            "\"auth status\") [ -f \"$(dirname \"$0\")/offline\" ] && \
             { echo 'error connecting to api.github.com' >&2; exit 1; }; exit 0 ;;",
            1,
        ),
    )
    .unwrap();
    let prs_file = stub_dir.join("prs.json");
    let log = stub_dir.join("calls.log");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        name,
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_FORGE_BACKOFF_MS", "3000"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_LOG", &log.display().to_string()),
        ],
    ) else {
        return;
    };
    let auths = || {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("auth status"))
            .count()
    };
    let notice = |c: &mut TestClient| match c.request(Command::Snapshot) {
        Response::Board { notices, .. } => {
            notices.into_iter().find(|n| n.kind == "forge_unreachable").map(|n| n.text)
        }
        other => panic!("not a board: {other:?}"),
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let mut c = h.client(name);
    through_prs(&mut c);
    let (keep, _, keep_branch, _) = ready(&mut c, "keep");
    write(&prs_file, &format!("[{}]", numbered_pr(42, &keep_branch, "0000", false)));
    wait_until(Duration::from_secs(20), "the PR to bind", || {
        prs_of(&mut c).iter().any(|p| p.ticket == keep && p.number == 42)
    });
    let kept = h.paths.state_dir.join("forge.json");
    wait_until(Duration::from_secs(10), "forge.json to hold the PR", || {
        std::fs::read_to_string(&kept).is_ok_and(|t| t.contains("pull/42"))
    });

    // The daemon comes back before the network: the sign-in check fails.
    drop(c);
    h.restart_after(|| std::fs::write(&offline, "").unwrap());
    let before = auths();
    let mut c = h.client("forge_lost-2");
    wait_until(Duration::from_secs(20), "the advisory row to name the host", || {
        notice(&mut c).is_some_and(|t| t.contains("github.com sign-in: error connecting"))
    });
    assert!(auths() > before, "the probe was asked");
    assert!(
        prs_of(&mut c).iter().any(|p| p.ticket == keep && p.number == 42),
        "the card keeps its PR"
    );

    // A ticket opened meanwhile, its PR on the forge: unseen while offline.
    let (fresh, _, fresh_branch, path) = ready(&mut c, "fresh");
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    write(
        &prs_file,
        &format!(
            "[{},{}]",
            numbered_pr(42, &keep_branch, "0000", false),
            numbered_pr(43, &fresh_branch, &tip, false)
        ),
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !prs_of(&mut c).iter().any(|p| p.ticket == fresh),
        "no host's list while its sign-in is lost"
    );

    // The network is back: the next probe, on the backoff, finds the CLI.
    let _ = std::fs::remove_file(&offline);
    wait_until(Duration::from_secs(20), "the ticket opened meanwhile to bind", || {
        prs_of(&mut c).iter().any(|p| p.ticket == fresh && p.number == 43)
    });
    assert_eq!(notice(&mut c), None, "the advisory row clears");
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// The stub `glab`: `auth status` passes, `api … user` names the account,
/// `mr list` prints the file the test writes, and `api …/pipelines?sha=X`
/// prints `<pipelines file>.X` where one exists, else no pipeline — so the
/// merge commit's sha has a pipeline and the squash commit's has none,
/// which is the lab's shape (T-754). A pipeline's jobs (T-772) are
/// `<pipelines file>.jobs`. One MR's read (T-784) is `<MSMN_GLAB_MR>.<iid>`
/// where the test wrote one, else an answer that reads as nothing. An
/// environment's deployments (T-801) are `<MSMN_GLAB_DEPLOYS>.<environment>`,
/// else none.
const GLAB_STUB: &str = r#"#!/bin/sh
[ -n "$MSMN_GLAB_LOG" ] && echo "$*" >> "$MSMN_GLAB_LOG"
case "$1 $2" in
  "auth status") exit 0 ;;
  "mr list") cat "$MSMN_GLAB_MRS" 2>/dev/null || echo '[]' ;;
  "api --hostname")
    case "$4" in
      user) echo '{"username":"me"}' ;;
      *deployments?environment=*)
        env="${4#*environment=}"; env="${env%%&*}"
        cat "$MSMN_GLAB_DEPLOYS.$env" 2>/dev/null || echo '[]' ;;
      *pipelines?sha=*)
        sha="${4#*sha=}"; sha="${sha%%&*}"
        if [ -f "$MSMN_GLAB_PIPELINES.$sha" ]; then cat "$MSMN_GLAB_PIPELINES.$sha"; else echo '[]'; fi ;;
      */jobs*) cat "$MSMN_GLAB_PIPELINES.jobs" 2>/dev/null || echo '[]' ;;
      */merge_requests/*) cat "$MSMN_GLAB_MR.${4##*/}" 2>/dev/null || echo '{}' ;;
      *) echo '[]' ;;
    esac ;;
  *) echo "stub glab: $*" >&2; exit 1 ;;
esac
"#;

/// One merged MR as `glab mr list --output json` prints it under GitLab's
/// default merge method: a squash lands as a merge commit whose second
/// parent is the squash commit, so the MR carries both shas.
fn merged_mr_json(branch: &str, tip: &str, merge_sha: &str, squash_sha: &str) -> String {
    format!(
        r#"[{{"iid":1,"web_url":"https://gitlab.example.com/lab/forge-lab-api/-/merge_requests/1","state":"merged",
          "source_branch":"{branch}","sha":"{tip}","merge_commit_sha":"{merge_sha}","squash_commit_sha":"{squash_sha}",
          "draft":false,"has_conflicts":false,"detailed_merge_status":"not_open","updated_at":"2026-10-13T12:00:00Z"}}]"#
    )
}

/// A workspace whose root is a plain folder (T-754, the forge lab's shape):
/// two nested repositories and no repository at the root, so the root has
/// no default branch. A leg with commits reads ahead all the same, an MR
/// merged at that tip folds the leg to `main (MR !1)` with the merge
/// commit — not the squash commit — as what landed, its pipeline read off
/// that commit, and the crown hears the merge. Its manual job is named by
/// GitLab's jobs read (T-772), and `get_ticket` says where it landed and
/// that nobody merges it now.
#[test]
fn a_folder_workspaces_leg_reads_ahead_and_a_gitlab_squash_lands_as_its_merge_commit() {
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let glab = stub_dir.join("glab");
    std::fs::write(&glab, GLAB_STUB).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mrs_file = stub_dir.join("mrs.json");
    let pipelines = stub_dir.join("pipelines");
    write(&mrs_file, "[]");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge_folder",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GLAB_MRS", &mrs_file.display().to_string()),
            ("MSMN_GLAB_PIPELINES", &pipelines.display().to_string()),
        ],
    ) else {
        return;
    };
    // The root stays a folder: no `init_repo` on it.
    let root = h.repo.clone();
    init_repo(&root.join("api"), "server.ts", "one\n");
    init_repo(&root.join("web"), "page.tsx", "hello\n");
    git(
        &root.join("api"),
        &["remote", "add", "origin", "https://gitlab.example.com/lab/forge-lab-api.git"],
    );
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_folder");
    through_prs(&mut c);
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let turn = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let create = |c: &mut TestClient, title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };

    // The crown, and a worker it starts on a worktree of the workspace.
    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "lab worker");
    let kw = c.board().ticket(w).unwrap().short_key.clone();
    let sa = match c.request(Command::SpawnSession {
        ticket: a,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(Duration::from_millis(500));
    turn(&mut c, sa);
    let seen = match c
        .send(Principal::Agent { session: sa }, Command::AgentReadTicket { key: kw.clone() })
    {
        Response::AgentTicket { ticket } => ticket.seen,
        other => panic!("read: {other:?}"),
    };
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let wt = wait_attached(&mut c, w);
    let names: Vec<&str> = wt.repos.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["api", "web"], "a folder root has no root leg");
    let container = PathBuf::from(wt.path.clone().expect("a path"));
    wait_until(Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::thread::sleep(Duration::from_millis(500));

    // ---- three commits in api: the leg reads ahead 3 -----------------------
    let api = container.join("api");
    for n in 1..=3 {
        std::fs::write(api.join(format!("work{n}.ts")), format!("work {n}\n")).unwrap();
        git(&api, &["add", "."]);
        git(&api, &["commit", "-qm", &format!("work {n}")]);
    }
    let tip = git(&api, &["rev-parse", "HEAD"]).trim().to_string();
    wait_until(Duration::from_secs(20), "the api leg to read ahead 3", || {
        wt_of(&mut c, w).is_some_and(|w| {
            w.ahead == 3
                && w.repos.iter().any(|r| r.name == "api" && r.ahead == 3 && !r.merged)
                && w.repos.iter().any(|r| r.name == "web" && r.ahead == 0)
        })
    });
    let worker = format!("{kw} \"lab worker\"");
    turn(&mut c, ws);
    wait_until(Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&format!("{worker} delivered")).len() == 1
    });
    turn(&mut c, sa);

    // ---- the MR merges on the forge, squash under "merge commit" ----------
    let merge_sha = "25559fabaea42e44173d2d161dd893f949a2701b";
    let squash_sha = "65f00df496d207427db655a37f264463a75d2f84";
    write(&pipelines.with_extension(merge_sha), r#"[{"id":44,"status":"manual"}]"#);
    write(
        &pipelines.with_extension("jobs"),
        r#"[{"name":"build","status":"success"},{"name":"deploy:production","status":"manual"}]"#,
    );
    write(&mrs_file, &merged_mr_json(&wt.branch, &tip, merge_sha, squash_sha));
    wait_until(Duration::from_secs(20), "the leg to read merged by the forge", || {
        wt_of(&mut c, w)
            .is_some_and(|w| w.merged && w.repos.iter().any(|r| r.name == "api" && r.merged))
    });
    let after = wt_of(&mut c, w).unwrap();
    assert_eq!(after.merged_in, "main (MR !1)", "landed on the leg's base, by the forge's word");
    assert_eq!(after.merged_oid, merge_sha, "what is on main is the merge commit, not the squash");
    assert!(!after.needs_rebase, "{after:?}");
    assert!(after.repos.iter().any(|r| r.name == "web" && !r.merged && r.ahead == 0), "{after:?}");
    // The manual job by its name (T-772), on the board and to the crown.
    wait_until(Duration::from_secs(20), "the waiting job's name", || {
        prs_of(&mut c).iter().any(|p| {
            p.ticket == w
                && p.pipeline == "waiting"
                && p.job.as_deref() == Some("deploy:production")
        })
    });
    wait_until(Duration::from_secs(20), "the crown to hear the job's name", || {
        !lines_with("waiting on deploy:production !1").is_empty()
    });
    assert!(lines_with("a manual job").is_empty(), "never the bare wait once named");
    // The crown's read of the worker: where it landed, and no merger.
    let view = match c
        .send(Principal::Agent { session: sa }, Command::AgentReadTicket { key: kw.clone() })
    {
        Response::AgentTicket { ticket } => ticket,
        other => panic!("read: {other:?}"),
    };
    assert_eq!(view.merge_state.as_deref(), Some("merged"));
    assert_eq!(view.merged_in.as_deref(), Some("main (MR !1)"));
    let merge = view.merge.expect("a branch says who merges it");
    assert_eq!((merge.by.as_str(), merge.why.as_str()), ("nobody", "already merged"));
    let names: Vec<(&str, Option<&str>)> =
        view.repos.iter().map(|r| (r.name.as_str(), r.merged_in.as_deref())).collect();
    assert_eq!(names, [("api", Some("main (MR !1)")), ("web", None)]);
    write(&pipelines.with_extension(merge_sha), r#"[{"id":44,"status":"success"}]"#);
    wait_until(Duration::from_secs(20), "the merge commit's pipeline", || {
        prs_of(&mut c).iter().any(|p| p.ticket == w && p.number == 1 && p.pipeline == "passed")
    });
    let pr = prs_of(&mut c).into_iter().find(|p| p.ticket == w).unwrap();
    assert_eq!((pr.forge.as_str(), pr.state.as_str()), ("gitlab", "merged"));
    assert_eq!(pr.merged_oid, merge_sha);

    // ---- the crown hears the merge --------------------------------------
    wait_until(Duration::from_secs(20), "the merged wake", || {
        !lines_with(&format!("{worker} merged (")).is_empty()
    });
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// `GLAB_STUB` with `mr list` answered per repository: the file the test
/// writes for the `--repo` URL's last segment (`<mrs file>.forge-lab-web`).
const GLAB_REPO_STUB: &str = r#"#!/bin/sh
case "$1 $2" in
  "auth status") exit 0 ;;
  "mr list") cat "$MSMN_GLAB_MRS.${4##*/}" 2>/dev/null || echo '[]' ;;
  "api --hostname")
    case "$4" in
      user) echo '{"username":"me"}' ;;
      *pipelines?sha=*)
        sha="${4#*sha=}"; sha="${sha%%&*}"
        if [ -f "$MSMN_GLAB_PIPELINES.$sha" ]; then cat "$MSMN_GLAB_PIPELINES.$sha"; else echo '[]'; fi ;;
      */jobs*) cat "$MSMN_GLAB_PIPELINES.jobs" 2>/dev/null || echo '[]' ;;
      *) echo '[]' ;;
    esac ;;
  *) echo "stub glab: $*" >&2; exit 1 ;;
esac
"#;

/// One MR of `repo` as `glab mr list --output json` prints it: open, or
/// merged at `merge_sha`.
fn mr_json(repo: &str, iid: u64, branch: &str, tip: &str, merge_sha: Option<&str>) -> String {
    let (state, merged, status) = match merge_sha {
        Some(m) => ("merged", m, "not_open"),
        None => ("opened", "", "mergeable"),
    };
    format!(
        r#"[{{"iid":{iid},"web_url":"https://gitlab.example.com/lab/{repo}/-/merge_requests/{iid}","state":"{state}",
          "source_branch":"{branch}","sha":"{tip}","merge_commit_sha":"{merged}","squash_commit_sha":"",
          "draft":false,"has_conflicts":false,"detailed_merge_status":"{status}","updated_at":"2026-10-14T17:00:00Z"}}]"#
    )
}

/// T-782, lab run 2: a ticket of two repositories, an MR on each. Its
/// first delivery names each leg ahead. web's !2 merges while api's !8 is
/// open: no leg flip reads the ticket merged, so the forge says `merged
/// !2`, and its merge commit's wait (`waiting on deploy:production !2`)
/// though the set's own pipeline is !8's. api's !8 then merges: the flags'
/// `merged` wake, and no second `merged !2`.
#[test]
fn a_two_repo_tickets_first_merge_and_its_wait_are_said() {
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-two-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let glab = stub_dir.join("glab");
    std::fs::write(&glab, GLAB_REPO_STUB).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mrs = stub_dir.join("mrs");
    let pipelines = stub_dir.join("pipelines");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge_two",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GLAB_MRS", &mrs.display().to_string()),
            ("MSMN_GLAB_PIPELINES", &pipelines.display().to_string()),
        ],
    ) else {
        return;
    };
    let root = h.repo.clone();
    for (dir, file, repo) in
        [("api", "server.ts", "forge-lab-api"), ("web", "page.tsx", "forge-lab-web")]
    {
        init_repo(&root.join(dir), file, "one\n");
        let url = format!("https://gitlab.example.com/lab/{repo}.git");
        git(&root.join(dir), &["remote", "add", "origin", &url]);
    }
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_two");
    through_prs(&mut c);
    let got = h.dir.join("got.txt");
    let all = || std::fs::read_to_string(&got).unwrap_or_default();
    let lines_with = |needle: &str| -> Vec<String> {
        all().lines().filter(|l| l.contains(needle)).map(str::to_string).collect()
    };
    let turn = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let create = |c: &mut TestClient, title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };

    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "two repos");
    let kw = c.board().ticket(w).unwrap().short_key.clone();
    let sa = match c.request(Command::SpawnSession {
        ticket: a,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(Duration::from_millis(500));
    turn(&mut c, sa);
    let seen = match c
        .send(Principal::Agent { session: sa }, Command::AgentReadTicket { key: kw.clone() })
    {
        Response::AgentTicket { ticket } => ticket.seen,
        other => panic!("read: {other:?}"),
    };
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let wt = wait_attached(&mut c, w);
    let container = PathBuf::from(wt.path.clone().expect("a path"));
    wait_until(Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::thread::sleep(Duration::from_millis(500));

    // ---- one commit in each leg, an MR open on each -------------------------
    let mut tips = Vec::new();
    for dir in ["api", "web"] {
        let leg = container.join(dir);
        std::fs::write(leg.join("work.txt"), "work\n").unwrap();
        git(&leg, &["add", "."]);
        git(&leg, &["commit", "-qm", "work"]);
        tips.push(git(&leg, &["rev-parse", "HEAD"]).trim().to_string());
    }
    let (api_tip, web_tip) = (tips[0].clone(), tips[1].clone());
    write(
        &mrs.with_extension("forge-lab-api"),
        &mr_json("forge-lab-api", 8, &wt.branch, &api_tip, None),
    );
    write(
        &mrs.with_extension("forge-lab-web"),
        &mr_json("forge-lab-web", 2, &wt.branch, &web_tip, None),
    );
    wait_until(Duration::from_secs(20), "both legs ahead and both MRs read open", || {
        wt_of(&mut c, w).is_some_and(|w| w.repos.iter().all(|r| r.ahead == 1))
            && prs_of(&mut c).iter().filter(|p| p.ticket == w && p.state == "open").count() == 2
    });
    let worker = format!("{kw} \"two repos\"");
    turn(&mut c, ws);
    wait_until(Duration::from_secs(15), "the delivery's wake", || {
        lines_with(&format!("{worker} delivered")).len() == 1
    });
    let line = lines_with(&format!("{worker} delivered")).remove(0);
    assert!(line.contains("merge_state ahead api 1, web 1"), "each leg ahead by name: {line}");
    turn(&mut c, sa);

    // ---- web's !2 merges while api's !8 is open -----------------------------
    let web_merge = "2222222222222222222222222222222222222222";
    write(&pipelines.with_extension(web_merge), r#"[{"id":45,"status":"manual"}]"#);
    write(
        &pipelines.with_extension("jobs"),
        r#"[{"name":"build","status":"success"},{"name":"deploy:production","status":"manual"}]"#,
    );
    write(
        &mrs.with_extension("forge-lab-web"),
        &mr_json("forge-lab-web", 2, &wt.branch, &web_tip, Some(web_merge)),
    );
    wait_until(Duration::from_secs(20), "the crown to hear merged !2", || {
        !lines_with("merged !2").is_empty()
    });
    wait_until(Duration::from_secs(20), "the crown to hear !2's waiting job", || {
        !lines_with("waiting on deploy:production !2").is_empty()
    });
    let view = match c
        .send(Principal::Agent { session: sa }, Command::AgentReadTicket { key: kw.clone() })
    {
        Response::AgentTicket { ticket } => ticket,
        other => panic!("read: {other:?}"),
    };
    assert_ne!(view.merge_state.as_deref(), Some("merged"), "api's leg is not merged");
    assert!(lines_with(&format!("{worker} merged (")).is_empty(), "no flags flip: {}", all());
    turn(&mut c, sa);

    // ---- api's !8 merges: the ticket's merged wake, !2 not said again -------
    let api_merge = "8888888888888888888888888888888888888888";
    write(
        &mrs.with_extension("forge-lab-api"),
        &mr_json("forge-lab-api", 8, &wt.branch, &api_tip, Some(api_merge)),
    );
    wait_until(Duration::from_secs(20), "the merged wake", || {
        !lines_with(&format!("{worker} merged (")).is_empty()
    });
    std::thread::sleep(Duration::from_millis(1500));
    let said = all();
    assert_eq!(said.matches("merged !2").count(), 1, "{said}");
    assert!(!said.contains("merged !8"), "the flags' wake carries !8: {said}");
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// A board that merges into main never meets the forge (T-810): with `gh`
/// signed in and the repository on github.com, the train merges a ticket
/// whose turn pushed, and the stub is never called — no probe, no list, no
/// read at the turn's end — and `forge.json` is never written. Turning
/// pull requests on is what starts the reads.
#[test]
fn a_board_that_merges_into_main_never_calls_the_forge() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let name = "forge_main";
    let (stub_dir, path_var) = gh_stub(name);
    let prs_file = stub_dir.join("prs.json");
    let log = stub_dir.join("calls.log");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        name,
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_LOG", &log.display().to_string()),
        ],
    ) else {
        return;
    };
    let calls = || std::fs::read_to_string(&log).unwrap_or_default();
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client(name);
    assert_eq!(c.board().merge_into, MergeInto::Main, "every board merges into main");
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    let (id, sid, branch, path) = ready(&mut c, "alpha");
    // The turn pushes, as a `git push` does: on a board reading the forge
    // its end would be owed a read.
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    git(&path, &["update-ref", &format!("refs/remotes/origin/{branch}"), &tip]);
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(30), "the train to merge on the turn's end", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch, "main"])
    });
    wait_until(Duration::from_secs(10), "the train's merge in the feed", || {
        feed_count(&h, "merge_train_merged", id) >= 1
    });
    // Several forge minutes pass at this cadence.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(calls(), "", "no gh call on a board that merges into main");
    assert!(prs_of(&mut c).is_empty());
    assert!(!h.paths.forge_file().exists(), "forge.json is neither read nor written");

    // Pull requests: the forge is read from the next tick.
    assert!(matches!(
        c.request(Command::SetMergeInto { into: MergeInto::PullRequests }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(20), "the first forge pass", || calls().contains("pr list"));
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

fn git_ok(repo: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// One ask per conflict (T-770), the lab's D1 on a stub: the local base
/// never moves (the forge merges), so only the forge's word says a conflict
/// is new, and only `train.json` says one was asked about after a restart.
#[test]
fn a_pr_conflict_is_asked_about_once_and_again_only_when_it_returns() {
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-conflict-{}", std::process::id()));
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
    write(&prs_file, "[]");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge-conflict",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let got = h.dir.join("got.txt");
    let turn = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let arm = |c: &mut TestClient| {
        assert!(matches!(
            c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
            Response::Ok
        ));
    };
    // Several forge passes, each one a train pass (T-761).
    let passes = || std::thread::sleep(Duration::from_secs(3));
    let mut c = h.client("forge-conflict");
    through_prs(&mut c);
    arm(&mut c);
    let (id, sid, branch, path) = ready(&mut c, "alpha");
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    let pr = |mergeable: &str| {
        open_pr_json(
            &branch,
            &tip,
            r#"[{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}]"#,
            mergeable,
            "REVIEW_REQUIRED",
        )
    };
    let asks = || feed_count(&h, "merge_train_rebase_asked", id);
    write(&prs_file, &pr("MERGEABLE"));
    wait_until(Duration::from_secs(20), "the PR to bind", || {
        prs_of(&mut c).iter().any(|p| p.ticket == id && p.number == 42)
    });
    turn(&mut c, sid);
    passes();
    assert_eq!(asks(), 0, "a clean PR is the forge's merge, and nothing to ask");
    assert!(!git_ok(&repo, &["merge-base", "--is-ancestor", &branch, "main"]), "never merged here");

    // The forge reports a conflict: one ask, naming the PR.
    write(&prs_file, &pr("CONFLICTING"));
    wait_until(Duration::from_secs(20), "the rebase ask", || asks() >= 1);
    wait_until(Duration::from_secs(10), "the words in the pane", || {
        std::fs::read_to_string(&got).is_ok_and(|t| t.contains("so PR #42 updates"))
    });
    // The rebase turn runs and ends with the forge still saying conflict:
    // the read before the push. Not asked again.
    turn(&mut c, sid);
    passes();
    assert_eq!(asks(), 1, "one conflict, one ask");

    // The conflict clears: nothing to ask.
    write(&prs_file, &pr("MERGEABLE"));
    passes();
    assert_eq!(asks(), 1, "a flip back asks nothing");

    // A second conflict at the same local base tip (the forge merged
    // something else; the checkout's main never moved) is a new one.
    write(&prs_file, &pr("CONFLICTING"));
    wait_until(Duration::from_secs(20), "the second conflict's ask", || asks() >= 2);
    turn(&mut c, sid);
    let kept = h.paths.state_dir.join("train.json");
    wait_until(Duration::from_secs(10), "train.json to keep the settled ask", || {
        std::fs::read_to_string(&kept)
            .is_ok_and(|t| t.contains("settled") && t.contains("conflict"))
    });

    // A restart, the forge unchanged: the ask is remembered, not repeated.
    drop(c);
    h.restart_after(|| {});
    let mut c = h.client("forge-conflict-2");
    arm(&mut c);
    // A person's turn re-derives the seat as idle; the forge reads after it.
    turn(&mut c, sid);
    passes();
    assert_eq!(asks(), 2, "a restart asks nothing it asked");
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// The stub `gh` for two repositories (T-768): `pr list` and the
/// deployments answer per repository, from `<file>.<repo name>`.
const GH_TWO_STUB: &str = r#"#!/bin/sh
case "$1 $2" in
  "auth status") exit 0 ;;
  "pr list") cat "$MSMN_GH_PRS.${4##*/}" 2>/dev/null || echo '[]' ;;
  api\ *)
    repo=$(echo "$4" | cut -d/ -f3)
    case "$4" in
      *check-runs*) echo '{"check_runs":[]}' ;;
      */commits/*/status) echo '{"state":"pending","statuses":[]}' ;;
      *deployments/*/statuses*) echo '[{"state":"success"}]' ;;
      *deployments*) cat "$MSMN_GH_DEPLOYS.$repo" 2>/dev/null || echo '[]' ;;
      *) echo '{}' ;;
    esac ;;
  *) echo "stub gh: $*" >&2; exit 1 ;;
esac
"#;

/// A ticket's arrival is its least advanced PR (T-768): on a folder
/// workspace with a PR in each of two repositories, a column that arrives
/// on `deployed: staging` waits while one PR is open, and while one merged
/// commit is not on staging, and takes the ticket once both are.
#[test]
fn a_deployed_arrival_waits_for_every_repositorys_pr() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-two-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let gh = stub_dir.join("gh");
    std::fs::write(&gh, GH_TWO_STUB).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let prs = stub_dir.join("prs");
    let deploys = stub_dir.join("deploys");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge_two",
        Some(STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs.display().to_string()),
            ("MSMN_GH_DEPLOYS", &deploys.display().to_string()),
        ],
    ) else {
        return;
    };
    // The root stays a folder; each nested repository has its own remote.
    let root = h.repo.clone();
    for name in ["api", "web"] {
        init_repo(&root.join(name), "a.txt", "one\n");
        let url = format!("https://github.com/example/{name}.git");
        git(&root.join(name), &["remote", "add", "origin", &url]);
    }
    let mut c = h.client("forge_two");
    through_prs(&mut c);
    let id = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "two repos".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
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
    let wt = wait_attached(&mut c, id);
    let names: Vec<&str> = wt.repos.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["api", "web"]);
    let container = PathBuf::from(wt.path.clone().expect("a path"));
    let tip = |name: &str| git(&container.join(name), &["rev-parse", "HEAD"]).trim().to_string();
    let (api_tip, web_tip) = (tip("api"), tip("web"));

    let mut review = c.board().column("REVIEW").unwrap().settings.clone();
    review.arrives_on = Arrival::Deployed;
    review.arrives_env = Some("staging".into());
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "REVIEW".into(), settings: review }),
        Response::Ok
    ));
    let pr = |repo: &str, number: u64, head: &str, merge: Option<&str>| {
        let (state, merge) = match merge {
            Some(oid) => ("MERGED", format!(r#"{{"oid":"{oid}"}}"#)),
            None => ("OPEN", "null".to_string()),
        };
        format!(
            r#"[{{"number":{number},"url":"https://github.com/example/{repo}/pull/{number}","state":"{state}",
              "isDraft":false,"headRefName":"{}","headRefOid":"{head}","mergeCommit":{merge},
              "mergeable":"MERGEABLE","reviewDecision":"APPROVED","updatedAt":"2026-10-13T06:00:00Z",
              "statusCheckRollup":[]}}]"#,
            wt.branch
        )
    };
    let staging = |repo: &str, sha: &str| {
        write(
            &deploys.with_extension(repo),
            &format!(r#"[{{"id":7,"sha":"{sha}","environment":"staging"}}]"#),
        );
    };
    let deployed_of = |c: &mut TestClient, number: u64| {
        prs_of(c)
            .into_iter()
            .find(|p| p.ticket == id && p.number == number)
            .is_some_and(|p| p.deployed.iter().any(|d| d == "staging"))
    };
    let column = |c: &mut TestClient| c.board().ticket(id).unwrap().column.clone();
    let (web_merge, api_merge) =
        ("1111111111111111111111111111111111111111", "3333333333333333333333333333333333333333");

    // ---- both open: the baseline -------------------------------------------
    write(&prs.with_extension("web"), &pr("web", 1, &web_tip, None));
    write(&prs.with_extension("api"), &pr("api", 3, &api_tip, None));
    wait_until(Duration::from_secs(20), "both PRs to bind", || {
        let got = prs_of(&mut c);
        [1, 3].iter().all(|n| got.iter().any(|p| p.ticket == id && p.number == *n))
    });

    // ---- web merged and on staging, api open: no arrival --------------------
    staging("web", web_merge);
    write(&prs.with_extension("web"), &pr("web", 1, &web_tip, Some(web_merge)));
    wait_until(Duration::from_secs(20), "web's merge commit on staging", || deployed_of(&mut c, 1));
    std::thread::sleep(Duration::from_millis(1500));
    assert_ne!(column(&mut c), "REVIEW", "api's PR is open");

    // ---- api merged, its merge commit not on staging: still none -----------
    write(&prs.with_extension("api"), &pr("api", 3, &api_tip, Some(api_merge)));
    wait_until(Duration::from_secs(20), "api's merge", || {
        prs_of(&mut c).iter().any(|p| p.ticket == id && p.number == 3 && p.state == "merged")
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_ne!(column(&mut c), "REVIEW", "api's merge commit is not on staging");

    // ---- api on staging too: the ticket arrives ----------------------------
    staging("api", api_merge);
    wait_until(Duration::from_secs(20), "the ticket to arrive in REVIEW", || {
        column(&mut c) == "REVIEW"
    });
    assert!(deployed_of(&mut c, 3));
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// An older PR keeps its pipeline and deployments (T-771): PR #42 merges,
/// its merge commit passes and reaches staging; then #43 of the same ticket
/// merges and reaches staging too, and #42's row still reads passed and
/// deployed. The list's own rollup for #42 says running, so a row that read
/// only the list would show it. Neither merge commit is in this checkout,
/// so git cannot say staging's newer commit lacks #42's, and its own
/// deployment stands (T-801).
#[test]
fn an_older_pr_keeps_its_pipeline_and_deployments_after_a_newer_one_merges() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-older-{}", std::process::id()));
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
    let deploys_file = stub_dir.join("deploys.json");
    write(&prs_file, "[]");
    write(&checks_file, r#"{"check_runs":[{"name":"release","status":"in_progress"}]}"#);
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge_older",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_CHECKS", &checks_file.display().to_string()),
            ("MSMN_GH_DEPLOYS", &deploys_file.display().to_string()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let mut c = h.client("forge_older");
    through_prs(&mut c);
    let (id, _sid, branch, path) = ready(&mut c, "alpha");
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    // A second commit in HEAD's reflog and off the branch: #43's head.
    std::fs::write(path.join("fix.txt"), "fix\n").unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", "fix"]);
    let fix = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    git(&path, &["reset", "-q", "--hard", "HEAD~1"]);

    // A column waits for staging, so deployments are read.
    let mut done = c.board().column("DONE").unwrap().settings.clone();
    done.arrives_on = Arrival::Deployed;
    done.arrives_env = Some("staging".into());
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "DONE".into(), settings: done }),
        Response::Ok
    ));

    let running =
        r#"[{"__typename":"CheckRun","name":"test","status":"IN_PROGRESS","conclusion":null}]"#;
    let gh_pr = |number: u64, branch: &str, head: &str, merge: &str, at: &str| {
        format!(
            r#"{{"number":{number},"url":"https://github.com/example/forge/pull/{number}","state":"MERGED",
              "isDraft":false,"headRefName":"{branch}","headRefOid":"{head}","mergeCommit":{{"oid":"{merge}"}},
              "mergeable":"UNKNOWN","reviewDecision":"APPROVED","updatedAt":"{at}",
              "statusCheckRollup":{running}}}"#
        )
    };
    let (m1, m2) =
        ("1111111111111111111111111111111111111111", "2222222222222222222222222222222222222222");
    let passed =
        r#"{"check_runs":[{"name":"release","status":"completed","conclusion":"success"}]}"#;
    // What staging ran, newest first.
    let staging = |shas: &[&str]| {
        let each: Vec<String> = shas
            .iter()
            .enumerate()
            .map(|(i, sha)| format!(r#"{{"id":{},"sha":"{sha}","environment":"staging"}}"#, 9 - i))
            .collect();
        format!("[{}]", each.join(","))
    };
    write(&stub_dir.join(format!("checks.json.{m1}")), passed);
    write(&deploys_file, &staging(&[m1]));
    let first = gh_pr(42, &branch, &tip, m1, "2026-10-13T06:00:00Z");
    write(&prs_file, &format!("[{first}]"));
    let row = |c: &mut TestClient, number: u64| {
        prs_of(c).into_iter().find(|p| p.ticket == id && p.number == number)
    };
    wait_until(Duration::from_secs(20), "#42 to read passed and deployed", || {
        row(&mut c, 42).is_some_and(|p| p.pipeline == "passed" && p.deployed == ["staging"])
    });

    // #43 merges; its merge commit runs, then reaches staging.
    let second = gh_pr(43, "fix/alpha", &fix, m2, "2026-10-13T07:00:00Z");
    write(&prs_file, &format!("[{second},{first}]"));
    wait_until(Duration::from_secs(20), "#43 to bind", || {
        row(&mut c, 43).is_some_and(|p| p.pipeline == "running")
    });
    write(&deploys_file, &staging(&[m2, m1]));
    wait_until(Duration::from_secs(20), "#43 to reach staging", || {
        row(&mut c, 43).is_some_and(|p| p.deployed == ["staging"])
    });
    std::thread::sleep(Duration::from_millis(1500));
    let older = row(&mut c, 42).expect("#42 still bound");
    assert_eq!(older.pipeline, "passed", "{older:?}");
    assert_eq!(older.deployed, ["staging"], "{older:?}");
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// A pipeline in flight is read every `MESIMON_FORGE_BUSY_MS` (T-781): a
/// merged PR's merge commit running its deploy is listed again and again
/// inside one minute's cadence, and once it passes the reads go back to the
/// minute.
#[test]
fn a_running_pipeline_is_read_on_the_busy_cadence_and_a_settled_one_on_the_minute() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let (stub_dir, path_var) = gh_stub("forge-busy");
    let prs_file = stub_dir.join("prs.json");
    let checks_file = stub_dir.join("checks.json");
    let log = stub_dir.join("gh.log");
    write(&prs_file, "[]");
    write(&checks_file, r#"{"check_runs":[{"name":"deploy","status":"in_progress"}]}"#);
    let Some(h) = Harness::boot_with_env(
        "forge_busy",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "6000"),
            ("MESIMON_FORGE_BUSY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_CHECKS", &checks_file.display().to_string()),
            ("MSMN_GH_LOG", &log.display().to_string()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let mut c = h.client("forge_busy");
    through_prs(&mut c);
    let (id, _sid, branch, path) = ready(&mut c, "alpha");
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    let merge_oid = "3333333333333333333333333333333333333333";
    write(&prs_file, &pr_json(&branch, &tip, "MERGED", Some(merge_oid), None));
    let pipeline = |c: &mut TestClient| {
        prs_of(c).into_iter().find(|p| p.ticket == id).map(|p| p.pipeline).unwrap_or_default()
    };
    wait_until(Duration::from_secs(20), "the merged PR to read running", || {
        pipeline(&mut c) == "running"
    });
    let lists = |window: Duration| {
        let _ = std::fs::remove_file(&log);
        std::thread::sleep(window);
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("pr list"))
            .count()
    };
    // Running: read every busy step, many times inside one minute's cadence.
    let busy = lists(Duration::from_secs(4));
    assert!(busy >= 2, "a running pipeline is listed {busy} times in 4 s");

    // Passed: settled, and the minute's cadence (6 s here) is back.
    write(
        &checks_file,
        r#"{"check_runs":[{"name":"deploy","status":"completed","conclusion":"success"}]}"#,
    );
    wait_until(Duration::from_secs(20), "the pipeline to read passed", || {
        pipeline(&mut c) == "passed"
    });
    let idle = lists(Duration::from_secs(4));
    assert!(idle <= 1, "a settled pipeline is listed {idle} times in 4 s");
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// A stub `gh` in a fresh directory, and the PATH that puts it first.
fn gh_stub(name: &str) -> (PathBuf, String) {
    let stub_dir = std::env::temp_dir().join(format!("msmn-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let gh = stub_dir.join("gh");
    std::fs::write(&gh, GH_STUB).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    (stub_dir, path_var)
}

/// One turn of a session, by its hooks, ending idle.
fn turn(c: &mut TestClient, hook_sock: &Path, sid: uuid::Uuid) {
    hook_send(hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    hook_send(hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
}

/// A crown, idle, and a worker it started on a worktree with one commit,
/// its delivery heard: the crown's session, the worker's ticket and
/// session, the worktree's branch and path, its tip, and the worker as the
/// crown's line names it.
struct Crowned {
    crown: uuid::Uuid,
    worker: ulid::Ulid,
    ws: uuid::Uuid,
    branch: String,
    path: PathBuf,
    tip: String,
    named: String,
}

fn crowned(h: &Harness, c: &mut TestClient) -> Crowned {
    let hook_sock = h.paths.hook_sock();
    let got = h.dir.join("got.txt");
    let create = |c: &mut TestClient, title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let a = create(c, "coordinate");
    let w = create(c, "forge worker");
    let kw = c.board().ticket(w).unwrap().short_key.clone();
    let sa = match c.request(Command::SpawnSession {
        ticket: a,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(Duration::from_millis(500));
    turn(c, &hook_sock, sa);
    let seen = match c
        .send(Principal::Agent { session: sa }, Command::AgentReadTicket { key: kw.clone() })
    {
        Response::AgentTicket { ticket } => ticket.seen,
        other => panic!("read: {other:?}"),
    };
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let wt = wait_attached(c, w);
    let path = PathBuf::from(wt.path.clone().expect("a path"));
    wait_until(Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::thread::sleep(Duration::from_millis(500));
    std::fs::write(path.join("work.txt"), "work\n").unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", "work"]);
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    let named = format!("{kw} \"forge worker\"");
    turn(c, &hook_sock, ws);
    wait_until(Duration::from_secs(10), "the delivery's wake", || {
        std::fs::read_to_string(&got).is_ok_and(|t| t.contains(&format!("{named} delivered")))
    });
    turn(c, &hook_sock, sa);
    Crowned { crown: sa, worker: w, ws, branch: wt.branch, path, tip, named }
}

/// The feed's crown wakes about `worker` for `cause`.
fn wakes(h: &Harness, worker: ulid::Ulid, cause: &str) -> usize {
    std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            l.contains("\"kind\":\"crown_wake\"")
                && l.contains(&format!("\"worker\":\"{worker}\""))
                && l.contains(&format!("\"cause\":\"{cause}\""))
        })
        .count()
}

/// T-769, findings 4 and 3: a PR first read with its pipeline passed says
/// `pipeline passed` once, and a pass that reads its merge and its merge
/// commit on staging says both in one line, the merge first, with no
/// second `merged` wake from the flags after it.
#[test]
fn a_freshly_bound_prs_state_is_news_and_its_merge_comes_before_its_deployment() {
    let (stub_dir, path_var) = gh_stub("forge-fresh");
    let prs_file = stub_dir.join("prs.json");
    let deploys = stub_dir.join("deploys.json");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        "forge_fresh",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_DEPLOYS", &deploys.display().to_string()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_fresh");
    through_prs(&mut c);
    // A column waits on staging, so the pass asks for deployments.
    let mut done = c.board().column("DONE").unwrap().settings.clone();
    done.arrives_on = Arrival::Deployed;
    done.arrives_env = Some("staging".into());
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "DONE".into(), settings: done }),
        Response::Ok
    ));
    let k = crowned(&h, &mut c);
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    // The ticket's set has been read empty: the PR binds on a later read.
    std::thread::sleep(Duration::from_millis(1500));

    // ---- bound already passed: said once ---------------------------------
    let passed =
        r#"[{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS"}]"#;
    write(&prs_file, &open_pr_json(&k.branch, &k.tip, passed, "MERGEABLE", "REVIEW_REQUIRED"));
    wait_until(Duration::from_secs(20), "the passed wake", || {
        lines_with(&format!("{} has news from the forge (", k.named))
            .iter()
            .any(|l| l.contains("pipeline passed #42)"))
    });
    turn(&mut c, &hook_sock, k.crown);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(wakes(&h, k.worker, "forge"), 1, "{}", std::fs::read_to_string(&got).unwrap());
    assert_eq!(lines_with("pipeline passed #42").len(), 1);

    // ---- merged and on staging on one pass: one line, merged first --------
    write(&deploys, &format!(r#"[{{"id":7,"sha":"{}","environment":"staging"}}]"#, k.tip));
    write(&prs_file, &pr_json(&k.branch, &k.tip, "MERGED", Some(&k.tip), None));
    wait_until(Duration::from_secs(20), "the merge's line", || {
        !lines_with("deployed to staging #42").is_empty()
    });
    let line = lines_with("deployed to staging #42").remove(0);
    assert!(line.contains(&format!("{} merged (", k.named)), "{line}");
    assert!(line.contains("merged #42, deployed to staging #42)"), "{line}");
    turn(&mut c, &hook_sock, k.crown);
    // The flags' wake on the tick found the merge told.
    std::thread::sleep(Duration::from_millis(2000));
    let all = std::fs::read_to_string(&got).unwrap();
    assert_eq!(wakes(&h, k.worker, "merged"), 1, "{all}");
    assert_eq!(all.matches(" merged (").count(), 1, "{all}");
    let _ = (k.ws, &k.path);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-801, forge lab run 4: production's manual deploy job ran on a commit
/// later than either of a ticket's two merge commits, as batched deploys
/// do, with main's history here as a fetch brings it. Production first
/// runs a commit that carries !13 alone: !13 reads deployed and the crown
/// hears it, and the ticket stays, !15 not being there. Then it runs !14's
/// merge commit, which carries both: !15 is heard and the ticket moves
/// into PROD. A rollback takes production off both rows and leaves the
/// card in PROD.
#[test]
fn a_later_deploy_carrying_every_merged_mr_moves_the_ticket_to_prod() {
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-batch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let glab = stub_dir.join("glab");
    std::fs::write(&glab, GLAB_STUB).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mrs = stub_dir.join("mrs.json");
    let pipelines = stub_dir.join("pipelines");
    let deploys = stub_dir.join("deploys");
    write(&mrs, "[]");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge_batch",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GLAB_MRS", &mrs.display().to_string()),
            ("MSMN_GLAB_PIPELINES", &pipelines.display().to_string()),
            ("MSMN_GLAB_DEPLOYS", &deploys.display().to_string()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://gitlab.example.com/lab/forge-lab-api.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_batch");
    through_prs(&mut c);
    // A PROD column a ticket arrives in once production runs its merges.
    assert!(matches!(
        c.request(Command::AddColumn { name: "PROD".into(), after: Some("REVIEW".into()) }),
        Response::Ok
    ));
    let mut prod = c.board().column("PROD").unwrap().settings.clone();
    prod.arrives_on = Arrival::Deployed;
    prod.arrives_env = Some("production".into());
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "PROD".into(), settings: prod }),
        Response::Ok
    ));
    let k = crowned(&h, &mut c);
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let column = |c: &mut TestClient| c.board().ticket(k.worker).unwrap().column.clone();
    let deployed = |c: &mut TestClient, number: u64| {
        prs_of(c)
            .into_iter()
            .find(|p| p.ticket == k.worker && p.number == number)
            .map(|p| p.deployed)
            .unwrap_or_default()
    };

    // ---- main's history, as a fetch brings it: the lab's order ------------
    let base = git(&h.repo, &["rev-parse", "HEAD"]).trim().to_string();
    let tree = git(&h.repo, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    let mut chain: Vec<String> = Vec::new();
    for msg in ["Merge !13", "conflict push", "Merge !15", "Merge !14"] {
        let parent = chain.last().cloned().unwrap_or_else(|| base.clone());
        chain.push(git(&h.repo, &["commit-tree", &tree, "-p", &parent, "-m", msg]).trim().into());
    }
    let (m13, push, m15, m14) = (&chain[0], &chain[1], &chain[2], &chain[3]);
    git(&h.repo, &["update-ref", "refs/remotes/origin/main", m14]);

    // ---- both MRs merged off the ticket's branch, neither deployed ---------
    let row = |iid: u64, merge: &str| {
        let one = mr_json("forge-lab-api", iid, &k.branch, &k.tip, Some(merge));
        one.trim().trim_start_matches('[').trim_end_matches(']').to_string()
    };
    write(&mrs, &format!("[{},{}]", row(15, m15), row(13, m13)));
    wait_until(Duration::from_secs(20), "both MRs to read merged", || {
        prs_of(&mut c).iter().filter(|p| p.ticket == k.worker && p.state == "merged").count() == 2
    });
    wait_until(Duration::from_secs(20), "the merged wake", || {
        !lines_with(&format!("{} merged (", k.named)).is_empty()
    });
    turn(&mut c, &hook_sock, k.crown);
    let runs = |sha: &str, at: &str| {
        write(
            &deploys.with_extension("production"),
            &format!(r#"[{{"id":1,"sha":"{sha}","status":"success","finished_at":"{at}"}}]"#),
        );
    };

    // ---- production runs a commit that carries !13 alone -------------------
    runs(push, "2026-10-15T09:40:00Z");
    wait_until(Duration::from_secs(20), "!13 on production by ancestry", || {
        deployed(&mut c, 13) == ["production"]
    });
    wait_until(Duration::from_secs(20), "the crown to hear !13's deploy", || {
        !lines_with("deployed to production !13").is_empty()
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert!(deployed(&mut c, 15).is_empty(), "the deployed commit is older than !15's merge");
    assert!(lines_with("deployed to production !15").is_empty());
    assert_ne!(column(&mut c), "PROD", "!15 is not on production");
    turn(&mut c, &hook_sock, k.crown);

    // ---- production runs !14's merge commit, which carries both ------------
    runs(m14, "2026-10-15T10:02:16Z");
    wait_until(Duration::from_secs(20), "the ticket to arrive in PROD", || {
        column(&mut c) == "PROD"
    });
    assert_eq!(deployed(&mut c, 15), ["production"]);
    assert_eq!(deployed(&mut c, 13), ["production"]);
    wait_until(Duration::from_secs(20), "the crown to hear !15's deploy", || {
        !lines_with("deployed to production !15").is_empty()
    });
    assert_eq!(lines_with("deployed to production !13").len(), 1, "!13 is said once");
    turn(&mut c, &hook_sock, k.crown);

    // ---- a rollback: off both rows, and the card stays ----------------------
    runs(&base, "2026-10-15T11:00:00Z");
    wait_until(Duration::from_secs(20), "production off both rows", || {
        deployed(&mut c, 13).is_empty() && deployed(&mut c, 15).is_empty()
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(column(&mut c), "PROD", "nothing moves a card back");
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-785 (forge lab run 3's D1, follow-up to T-769's finding 6): a
/// conflict the crown heard before a restart says its clear after it, once,
/// whether it clears after the first read or while the daemon is down. And
/// lab run 3's A: a wake held for the crown's turn says the column the card
/// is in when it goes out, and the next wake runs from there.
#[test]
fn a_conflict_said_before_a_restart_says_its_clear_after_it() {
    let (stub_dir, path_var) = gh_stub("forge-clear");
    let prs_file = stub_dir.join("prs.json");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        "forge_clear",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_clear");
    through_prs(&mut c);
    let k = crowned(&h, &mut c);
    let got = h.dir.join("got.txt");
    let said =
        |words: &str| std::fs::read_to_string(&got).unwrap_or_default().matches(words).count();
    let line_with = |words: &str| {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .rfind(|l| l.contains(words))
            .map(str::to_string)
            .unwrap_or_default()
    };
    let running =
        r#"[{"__typename":"CheckRun","name":"test","status":"IN_PROGRESS","conclusion":null}]"#;
    let pr =
        |mergeable: &str| open_pr_json(&k.branch, &k.tip, running, mergeable, "REVIEW_REQUIRED");
    let kept = h.paths.state_dir.join("forge.json");
    let conflict_kept = || {
        std::fs::read_to_string(&kept)
            .is_ok_and(|t| t.contains("\"conflict\": true") && t.contains("\"conflicts\""))
    };
    let sampled = |c: &mut TestClient| match c.request(Command::Snapshot) {
        Response::Board { forge, .. } => forge.sampled_at_ms,
        other => panic!("not a board: {other:?}"),
    };
    write(&prs_file, &pr("MERGEABLE"));
    wait_until(Duration::from_secs(20), "the PR to bind", || {
        prs_of(&mut c).iter().any(|p| p.ticket == k.worker && p.number == 42)
    });
    write(&prs_file, &pr("CONFLICTING"));
    wait_until(Duration::from_secs(20), "the conflict's wake", || said("conflict #42)") == 1);
    turn(&mut c, &hook_sock, k.crown);
    wait_until(Duration::from_secs(10), "forge.json to hold the conflict said", conflict_kept);

    // D1: a restart with the conflict standing. The first read after it
    // says nothing; the clear after that is said.
    drop(c);
    h.restart_after(|| {});
    let mut c = h.client("forge_clear-2");
    turn(&mut c, &hook_sock, k.crown);
    wait_until(Duration::from_secs(20), "the first live read", || sampled(&mut c) > 0);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(wakes(&h, k.worker, "forge"), 1, "{}", std::fs::read_to_string(&got).unwrap());

    // A: the clear is heard while the crown's turn runs, and the card is
    // moved before the turn ends. The line says where the card is now.
    hook_send(&hook_sock, &k.crown.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(k.crown, "running", |s| *s == SessionState::Running);
    write(&prs_file, &pr("MERGEABLE"));
    wait_until(Duration::from_secs(20), "the clear's wake, held", || {
        wakes(&h, k.worker, "forge") == 2
    });
    assert_eq!(said("conflict cleared"), 0, "held for the crown's turn");
    assert!(matches!(
        c.request(Command::MoveTicket { id: k.worker, column: "TODO".into(), before: None }),
        Response::Ok
    ));
    hook_send(&hook_sock, &k.crown.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(10), "the clear's line", || said("conflict cleared #42)") == 1);
    let line = line_with("conflict cleared #42)");
    assert!(line.contains("column TODO, conflict cleared #42)"), "{line}");
    turn(&mut c, &hook_sock, k.crown);

    // Said again, and cleared while the daemon is down: the first read
    // after the restart says the clear. The column was told: not again.
    write(&prs_file, &pr("CONFLICTING"));
    wait_until(Duration::from_secs(20), "the second conflict's wake", || {
        said("conflict #42)") == 2
    });
    let line = line_with("conflict #42)");
    assert!(!line.contains("column"), "{line}");
    turn(&mut c, &hook_sock, k.crown);
    wait_until(Duration::from_secs(10), "forge.json to hold it", conflict_kept);
    drop(c);
    h.restart_after(|| write(&prs_file, &pr("MERGEABLE")));
    let mut c = h.client("forge_clear-3");
    turn(&mut c, &hook_sock, k.crown);
    // Owed across the restart, the line says so last.
    wait_until(Duration::from_secs(20), "the clear's wake after the restart", || {
        said("conflict cleared #42, after a restart)") == 1
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(said("conflict cleared"), 2, "{}", std::fs::read_to_string(&got).unwrap());
    let _ = (k.ws, &k.path);
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-769, finding 5: the train's rebase turn is a turn's end like any
/// other (T-761): the forge is read seconds after it, not on the minute,
/// and the conflict the rebase fixed clears on the board.
#[test]
fn a_train_rebase_turn_is_read_at_its_end() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let (stub_dir, path_var) = gh_stub("forge-rebase-read");
    let prs_file = stub_dir.join("prs.json");
    let log = stub_dir.join("calls.log");
    write(&prs_file, "[]");
    // The minute is ten: after the first pass, any read is a turn's.
    let Some(h) = Harness::boot_with_env(
        "forge_rebase_read",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "600000"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_LOG", &log.display().to_string()),
        ],
    ) else {
        return;
    };
    let lists = || {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("pr list"))
            .count()
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_rebase_read");
    through_prs(&mut c);
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    let (id, sid, branch, path) = ready(&mut c, "alpha");
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    let pr = |mergeable: &str| {
        open_pr_json(
            &branch,
            &tip,
            r#"[{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}]"#,
            mergeable,
            "REVIEW_REQUIRED",
        )
    };
    let conflict = |c: &mut TestClient| {
        prs_of(c).iter().find(|p| p.ticket == id && p.number == 42).map(|p| p.conflict)
    };
    let push = |path: &Path| {
        let head = git(path, &["rev-parse", "HEAD"]).trim().to_string();
        git(path, &["update-ref", &format!("refs/remotes/origin/{branch}"), &head]);
    };
    // The worker's turn pushes, and the forge, read at its end, says the PR
    // conflicts: the train asks the rebase.
    write(&prs_file, &pr("CONFLICTING"));
    wait_until(Duration::from_secs(20), "the first pass", || lists() >= 1);
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    push(&path);
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(30), "the conflict read at the turn's end", || {
        conflict(&mut c) == Some(true)
    });
    wait_until(Duration::from_secs(20), "the train's rebase ask", || {
        feed_count(&h, "merge_train_rebase_asked", id) >= 1
    });

    // The rebase turn the train asked for: a new tip, pushed, and the forge
    // then reads clean. Read at its end, within the floor, not the minute.
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"rebase"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    std::fs::write(path.join("rebased.txt"), "rebased\n").unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", "rebased"]);
    push(&path);
    write(&prs_file, &pr("MERGEABLE"));
    let before = lists();
    let stopped = std::time::Instant::now();
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(30), "the conflict to clear", || {
        conflict(&mut c) == Some(false)
    });
    assert!(lists() > before, "a read after the rebase turn");
    // The floor of ten seconds per repository from the read before; the
    // minute is ten.
    let took = stopped.elapsed();
    assert!(took < Duration::from_secs(20), "cleared {took:?} after the turn's end");
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// A PR of number `n` as `gh pr list --json` prints it, open or merged at
/// its head.
fn numbered_pr(n: u64, branch: &str, tip: &str, merged: bool) -> String {
    let (state, merge) =
        if merged { ("MERGED", format!(r#"{{"oid":"{tip}"}}"#)) } else { ("OPEN", "null".into()) };
    format!(
        r#"{{"number":{n},"url":"https://github.com/example/forge/pull/{n}","state":"{state}","isDraft":false,
          "headRefName":"{branch}","headRefOid":"{tip}","mergeCommit":{merge},"mergeable":"MERGEABLE",
          "reviewDecision":"REVIEW_REQUIRED","updatedAt":"2026-10-14T06:00:00Z",
          "statusCheckRollup":[{{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}}]}}"#
    )
}

/// PR `n` closed unmerged on the forge, from `branch` at `tip` (T-797).
fn closed_pr(n: u64, branch: &str, tip: &str) -> String {
    numbered_pr(n, branch, tip, false).replace(r#""state":"OPEN""#, r#""state":"CLOSED""#)
}

/// A follow-up commit on a side branch in a ticket's worktree, its HEAD's
/// reflog the fact that binds its PR to the ticket; the ticket's own branch
/// stays where it merged.
fn follow_up(path: &Path, branch: &str) -> String {
    git(path, &["checkout", "-qb", branch]);
    std::fs::write(path.join(format!("{branch}.txt").replace('/', "-")), "more\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-qm", branch]);
    git(path, &["rev-parse", "HEAD"]).trim().to_string()
}

#[test]
fn an_open_pr_holds_done_and_a_ticket_there_reads_unmerged() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let (stub_dir, path_var) = gh_stub("forge-done");
    let prs_file = stub_dir.join("prs.json");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        "forge-done",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let mut c = h.client("forge-done");
    through_prs(&mut c);
    let (a, a_sid, a_branch, a_path) = ready(&mut c, "alpha");
    git(&repo, &["merge", "-q", "--ff-only", &a_branch]);
    let (b, _, b_branch, b_path) = ready(&mut c, "beta");
    git(&repo, &["merge", "-q", "--ff-only", &b_branch]);
    wait_until(Duration::from_secs(20), "both branches to read merged", || {
        let wts = worktrees_of(&mut c);
        [a, b].iter().all(|t| wts.iter().any(|w| w.ticket == *t && w.merged))
    });
    assert!(matches!(
        c.request(Command::MoveTicket { id: a, column: "DONE".into(), before: None }),
        Response::Ok
    ));

    // Follow-up PRs open on side branches of both tickets.
    let a_tip = follow_up(&a_path, "fix/greeting");
    let b_tip = follow_up(&b_path, "fix/beta");
    let open = |merged: bool| {
        format!(
            "[{},{}]",
            numbered_pr(2, "fix/greeting", &a_tip, merged),
            numbered_pr(3, "fix/beta", &b_tip, merged)
        )
    };
    write(&prs_file, &open(false));
    wait_until(Duration::from_secs(20), "both PRs to bind", || {
        let prs = prs_of(&mut c);
        prs.iter().any(|p| p.ticket == a && p.number == 2)
            && prs.iter().any(|p| p.ticket == b && p.number == 3)
    });

    // In DONE already: not evicted, but the card reads unmerged again and
    // `get_ticket` says why.
    let wa = wt_of(&mut c, a).unwrap();
    assert!(!wa.merged, "an open PR reads unmerged on a landed branch");
    assert_eq!(c.board().ticket(a).unwrap().column, "DONE");
    match c.send(Principal::Agent { session: a_sid }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            assert_eq!(ticket.merge_state.as_deref(), Some("open PR #2"));
            assert!(ticket.merged_in.is_none());
        }
        other => panic!("get_ticket: {other:?}"),
    }

    // A person's move into DONE is refused in the merge refusal's words.
    match c.request(Command::MoveTicket { id: b, column: "DONE".into(), before: None }) {
        Response::Err { message } => {
            assert_eq!(message, "PR #3 open — merge it on GitHub before DONE")
        }
        other => panic!("DONE with a PR open: {other:?}"),
    }
    assert_ne!(c.board().ticket(b).unwrap().column, "DONE");

    // The forge merges them: merged again, and DONE admits the ticket.
    write(&prs_file, &open(true));
    wait_until(Duration::from_secs(20), "the PRs to read merged", || {
        prs_of(&mut c).iter().filter(|p| p.state == "merged").count() == 2
    });
    assert!(wt_of(&mut c, a).unwrap().merged);
    assert!(matches!(
        c.request(Command::MoveTicket { id: b, column: "DONE".into(), before: None }),
        Response::Ok
    ));
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-773, lab finding 1: a worker's turn that pushed and opened #42 wakes
/// the crown once, after the read that turn owed: the line says `opened
/// #42`, and the crown's `get_ticket` at that wake lists the PR. Every `pr
/// list` takes two seconds from the turn on, so a wake sent on the turn's
/// look alone would arrive before any read could have seen the PR.
#[test]
fn a_turn_end_wake_waits_for_the_turns_forge_read() {
    let (stub_dir, path_var) = gh_stub("forge-wait");
    let prs_file = stub_dir.join("prs.json");
    let slow = stub_dir.join("slow");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        "forge_wait",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_SLOW", &slow.display().to_string()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_wait");
    through_prs(&mut c);
    let k = crowned(&h, &mut c);
    let key = c.board().ticket(k.worker).unwrap().short_key.clone();
    let got = h.dir.join("got.txt");
    let finished = format!("{} finished its turn (", k.named);
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    // The ticket's set has been read empty.
    std::thread::sleep(Duration::from_millis(1500));

    // The worker's turn pushes its branch and opens #42.
    write(&slow, "2");
    git(&k.path, &["update-ref", &format!("refs/remotes/origin/{}", k.branch), &k.tip]);
    write(&prs_file, &open_pr_json(&k.branch, &k.tip, "[]", "MERGEABLE", "REVIEW_REQUIRED"));
    let ended = std::time::Instant::now();
    turn(&mut c, &hook_sock, k.ws);
    wait_until(Duration::from_secs(30), "the turn's wake", || !lines_with(&finished).is_empty());
    let waited = ended.elapsed();
    let prs = match c
        .send(Principal::Agent { session: k.crown }, Command::AgentReadTicket { key: key.clone() })
    {
        Response::AgentTicket { ticket } => ticket.prs,
        other => panic!("read: {other:?}"),
    };
    let line = lines_with(&finished).remove(0);
    assert!(prs.iter().any(|p| p.label == "#42"), "get_ticket at the wake: {prs:?}; {line}");
    assert!(line.contains("opened #42"), "{line}");
    assert!(waited >= Duration::from_secs(2), "woken {waited:?} after the turn, before a read");
    assert!(waited < Duration::from_secs(16), "the wait is bounded: {waited:?}");

    // One wake for that turn: the read's opening said nothing of its own.
    turn(&mut c, &hook_sock, k.crown);
    std::thread::sleep(Duration::from_millis(3000));
    let all = std::fs::read_to_string(&got).unwrap();
    assert_eq!(wakes(&h, k.worker, "finished"), 1, "{all}");
    assert_eq!(wakes(&h, k.worker, "forge"), 0, "{all}");
    assert_eq!(all.matches("opened #42").count(), 1, "{all}");
    let _ = std::fs::remove_file(&slow);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-782, lab finding 4: the minute's read binds #42 while the turn that
/// pushed and opened it is still running, so the look after the turn owes
/// no read and the wake goes at once. The opening rides that wake all the
/// same, once.
#[test]
fn an_opening_read_mid_turn_rides_the_turns_wake() {
    let (stub_dir, path_var) = gh_stub("forge-midturn");
    let prs_file = stub_dir.join("prs.json");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        "forge_midturn",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_midturn");
    through_prs(&mut c);
    let k = crowned(&h, &mut c);
    let got = h.dir.join("got.txt");
    let finished = format!("{} finished its turn (", k.named);
    let all = || std::fs::read_to_string(&got).unwrap_or_default();
    std::thread::sleep(Duration::from_millis(1500));

    // The turn pushes and opens #42; a read binds it before the turn ends.
    hook_send(&hook_sock, &k.ws.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(k.ws, "running", |s| *s == SessionState::Running);
    git(&k.path, &["update-ref", &format!("refs/remotes/origin/{}", k.branch), &k.tip]);
    write(&prs_file, &open_pr_json(&k.branch, &k.tip, "[]", "MERGEABLE", "REVIEW_REQUIRED"));
    wait_until(Duration::from_secs(15), "a read mid-turn to bind #42", || {
        prs_of(&mut c).iter().any(|p| p.ticket == k.worker && p.number == 42)
    });
    std::thread::sleep(Duration::from_millis(600));
    hook_send(&hook_sock, &k.ws.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(k.ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(Duration::from_secs(20), "the turn's wake", || all().contains(&finished));
    let line = all().lines().find(|l| l.contains(&finished)).unwrap().to_string();
    assert!(line.contains("opened #42"), "{line}");
    turn(&mut c, &hook_sock, k.crown);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(all().matches("opened #42").count(), 1, "{}", all());
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-778: a ticket whose PR merged on the forge at another head, its branch
/// still reading ahead of a local base the forge never moved, is the
/// forge's. The armed train lands none of it — the lab's train had, and its
/// local main and `origin/main` diverged 4 and 4 — `get_ticket` says the
/// forge merges it, and `m` refuses in the forge's words. On a board that
/// merges through pull requests (T-810) a ticket beside it with no PR yet
/// is the forge's as well: no train merge, and `m` asks for a PR.
#[test]
fn a_ticket_whose_work_went_through_a_pr_is_never_the_trains() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let (stub_dir, path_var) = gh_stub("forge-carried");
    let prs_file = stub_dir.join("prs.json");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        "forge-carried",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge-carried");
    through_prs(&mut c);
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    let main_before = git(&repo, &["rev-parse", "main"]).trim().to_string();
    let (id, sid, branch, _path) = ready(&mut c, "alpha");
    // T-797: beside it, a ticket whose PR the team closed unmerged, at the
    // branch's tip.
    let (gamma, gsid, g_branch, g_path) = ready(&mut c, "gamma");
    let g_tip = git(&g_path, &["rev-parse", "HEAD"]).trim().to_string();
    // The PR merged at a head the branch moved past, as merge commits on the
    // forge: nothing folds it merged here.
    let off_tip = "c0ffee42".repeat(5);
    write(
        &prs_file,
        &format!(
            "[{},{}]",
            numbered_pr(42, &branch, &off_tip, true).trim(),
            closed_pr(43, &g_branch, &g_tip).trim()
        ),
    );
    wait_until(Duration::from_secs(20), "the merged and the closed PR to bind", || {
        let prs = prs_of(&mut c);
        prs.iter().any(|p| p.ticket == id && p.number == 42 && p.state == "merged")
            && prs.iter().any(|p| p.ticket == gamma && p.number == 43 && p.state == "closed")
    });
    turn(&mut c, &hook_sock, sid);
    turn(&mut c, &hook_sock, gsid);
    wait_until(Duration::from_secs(10), "the turns' ends to land both in REVIEW", || {
        let b = c.board();
        [id, gamma].iter().all(|t| b.ticket(*t).is_some_and(|t| t.column == "REVIEW"))
    });
    // Several forge and train passes.
    std::thread::sleep(Duration::from_secs(4));
    for t in [id, gamma] {
        let w = worktrees_of(&mut c).into_iter().find(|w| w.ticket == t).unwrap();
        assert!(!w.merged && w.ahead > 0, "the branch reads ahead of the base: {w:?}");
        assert_eq!(
            feed_count(&h, "merge_train_merged", t),
            0,
            "the forge's merge, not the train's"
        );
        assert_eq!(feed_count(&h, "merge_train_rebase_asked", t), 0);
    }
    assert_eq!(git(&repo, &["rev-parse", "main"]).trim(), main_before, "local main untouched");
    // Off the train's list, not a merge it tries and is refused.
    let pending = match c.request(Command::Snapshot) {
        Response::Board { pending, .. } => pending,
        other => panic!("not a board: {other:?}"),
    };
    assert!(!pending.iter().any(|p| p.ticket == id || p.ticket == gamma), "{pending:?}");

    // The agent reads who merges it, and `m` is refused in the forge's words.
    let view = match c.send(Principal::Agent { session: sid }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => ticket,
        other => panic!("get_ticket: {other:?}"),
    };
    let merge = view.merge.expect("a branch says who merges it");
    assert_eq!(
        (merge.by.as_str(), merge.why.as_str()),
        ("forge", "its work goes through pull requests")
    );
    match c.request(Command::MergeTicket { id }) {
        Response::Merge { outcome: MergeOutcome::Refused, detail, .. } => {
            assert_eq!(detail, "its pull requests carry the merge — merge on GitHub");
        }
        other => panic!("m on a ticket its PRs carried: {other:?}"),
    }
    assert_eq!(git(&repo, &["rev-parse", "main"]).trim(), main_before);
    // The closed one (T-797): the forge's too, and its reopening the road
    // back.
    let view = match c.send(Principal::Agent { session: gsid }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => ticket,
        other => panic!("get_ticket: {other:?}"),
    };
    let merge = view.merge.expect("a branch says who merges it");
    assert_eq!(
        (merge.by.as_str(), merge.why.as_str()),
        ("forge", "its pull requests were closed unmerged on the forge")
    );
    match c.request(Command::MergeTicket { id: gamma }) {
        Response::Merge { outcome: MergeOutcome::Refused, detail, .. } => {
            assert_eq!(detail, "PR #43 closed unmerged — reopen it on GitHub");
        }
        other => panic!("m on a ticket whose PR was closed: {other:?}"),
    }
    assert_eq!(git(&repo, &["rev-parse", "main"]).trim(), main_before);

    // T-810: a ticket with no PR yet is the forge's on this board too: the
    // train lands nothing, `get_ticket` says the forge, and `m` asks for a
    // PR.
    let (beta, bsid, _, _) = ready(&mut c, "beta");
    turn(&mut c, &hook_sock, bsid);
    wait_until(Duration::from_secs(10), "beta's turn's end to land it in REVIEW", || {
        c.board().ticket(beta).is_some_and(|t| t.column == "REVIEW")
    });
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(feed_count(&h, "merge_train_merged", beta), 0, "the train merges nothing");
    let view = match c.send(Principal::Agent { session: bsid }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => ticket,
        other => panic!("get_ticket: {other:?}"),
    };
    let merge = view.merge.expect("a branch says who merges it");
    assert_eq!(
        (merge.by.as_str(), merge.why.as_str()),
        ("forge", "this board merges through pull requests")
    );
    match c.request(Command::MergeTicket { id: beta }) {
        Response::Merge { outcome: MergeOutcome::Refused, detail, .. } => assert_eq!(
            detail,
            "this board merges through pull requests — open a PR from its branch and merge it on \
             GitHub"
        ),
        other => panic!("m with no PR on a pull-requests board: {other:?}"),
    }
    assert_eq!(git(&repo, &["rev-parse", "main"]).trim(), main_before, "local main untouched");
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-780: MR !5's pipeline passed, the worker made a commit, and the turn
/// ended before the forge read it. The pass is the commit before's: REVIEW
/// refuses the turn's end as not read and `get_ticket` does not offer it.
/// The forge then reads the new head passed, and REVIEW's `Arrives here
/// when: pipeline passes` takes the ticket. A merge the git road reads
/// first is named by the PR once the forge says it merged.
#[test]
fn the_pipeline_gate_judges_the_branchs_tip() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let (stub_dir, path_var) = gh_stub("forge-tip");
    let prs_file = stub_dir.join("prs.json");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        "forge-tip",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge-tip");
    through_prs(&mut c);
    let mut review = c.board().column("REVIEW").unwrap().settings.clone();
    review.requires_pipeline = true;
    review.arrives_on = Arrival::PipelinePasses;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "REVIEW".into(), settings: review }),
        Response::Ok
    ));
    let (id, sid, branch, path) = ready(&mut c, "alpha");
    let first = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    write(&prs_file, &pr_json(&branch, &first, "OPEN", None, None));
    wait_until(Duration::from_secs(20), "the PR to bind, passed", || {
        prs_of(&mut c).iter().any(|p| p.ticket == id && p.pipeline == "passed")
    });
    hook_send(&hook_sock, &sid.to_string(), "SessionStart", r#"{"session_id":"x"}"#);
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    wait_until(Duration::from_secs(10), "the turn to take the ticket to IN PROGRESS", || {
        c.board().ticket(id).is_some_and(|t| t.column == "IN PROGRESS")
    });

    // The turn commits; the forge still says the head before it, passed.
    std::fs::write(path.join("FAIL"), "fail\n").unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", "fail"]);
    let second = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    let feed = h.paths.state_dir.join("activity.jsonl");
    wait_until(Duration::from_secs(10), "the turn's end to be held as not read", || {
        std::fs::read_to_string(&feed).unwrap_or_default().lines().any(|l| {
            l.contains("\"kind\":\"movement_decision\"")
                && l.contains(&id.to_string())
                && l.contains("refused: pipeline not read — before REVIEW")
        })
    });
    assert_eq!(c.board().ticket(id).unwrap().column, "IN PROGRESS");
    match c.send(Principal::Agent { session: sid }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            assert!(!ticket.allowed_columns.iter().any(|c| c == "REVIEW"), "{ticket:?}")
        }
        other => panic!("get_ticket: {other:?}"),
    }

    // The forge reads the new head, passed: a new commit's pass, and the
    // arrival.
    write(&prs_file, &pr_json(&branch, &second, "OPEN", None, None));
    wait_until(Duration::from_secs(20), "REVIEW to take the ticket on the new head's pass", || {
        c.board().ticket(id).is_some_and(|t| t.column == "REVIEW")
    });

    // Git reads the merge first, the forge listing nothing (an open PR
    // holds the branch unmerged, T-774); its merged PR then names it.
    write(&prs_file, "[]");
    git(&repo, &["merge", "-q", "--ff-only", &branch]);
    wait_until(Duration::from_secs(20), "the git road to read the merge", || {
        worktrees_of(&mut c).iter().any(|w| w.ticket == id && w.merged)
    });
    let by_git = worktrees_of(&mut c).into_iter().find(|w| w.ticket == id).unwrap();
    assert!(!by_git.merged_in.contains("PR"), "{}", by_git.merged_in);
    write(&prs_file, &pr_json(&branch, &second, "MERGED", Some(&second), None));
    wait_until(Duration::from_secs(20), "the merge to read in the PR's words", || {
        worktrees_of(&mut c).iter().any(|w| w.ticket == id && w.merged_in == "main (PR #42)")
    });
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-784, lab run 3: MR !9's pipeline failed, the worker pushed a fix, and
/// GitLab's list kept serving the failed head for minutes while the MR's
/// own read named the new head, passed. The board reads the MR whole where
/// the list's head is not the branch's tip, so REVIEW's `Arrives here
/// when: pipeline passes` takes the ticket with the list still stale.
#[test]
fn an_open_mrs_head_and_pipeline_are_read_from_the_mr_itself() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-mr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let glab = stub_dir.join("glab");
    std::fs::write(&glab, GLAB_STUB).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mrs = stub_dir.join("mrs.json");
    let pipelines = stub_dir.join("pipelines");
    let one = stub_dir.join("mr");
    write(&mrs, "[]");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge-mr",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GLAB_MRS", &mrs.display().to_string()),
            ("MSMN_GLAB_PIPELINES", &pipelines.display().to_string()),
            ("MSMN_GLAB_MR", &one.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://gitlab.example.com/lab/api.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge-mr");
    through_prs(&mut c);
    let mut review = c.board().column("REVIEW").unwrap().settings.clone();
    review.requires_pipeline = true;
    review.arrives_on = Arrival::PipelinePasses;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "REVIEW".into(), settings: review }),
        Response::Ok
    ));
    let (id, sid, branch, path) = ready(&mut c, "alpha");
    let failed = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    write(
        &PathBuf::from(format!("{}.{failed}", pipelines.display())),
        r#"[{"id":1,"status":"failed"}]"#,
    );
    write(&mrs, &mr_json("api", 9, &branch, &failed, None));
    wait_until(Duration::from_secs(20), "!9 to bind, failed", || {
        prs_of(&mut c).iter().any(|p| p.ticket == id && p.pipeline.starts_with("failed"))
    });
    hook_send(&hook_sock, &sid.to_string(), "SessionStart", r#"{"session_id":"x"}"#);
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    wait_until(Duration::from_secs(10), "the turn to take the ticket to IN PROGRESS", || {
        c.board().ticket(id).is_some_and(|t| t.column == "IN PROGRESS")
    });

    // The fix: the list keeps the failed head; the MR's own read has the
    // new one, its head pipeline passed.
    std::fs::write(path.join("fix"), "fix\n").unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", "fix"]);
    let fixed = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    let read = mr_json("api", 9, &branch, &fixed, None);
    let read = read.trim().trim_start_matches('[').trim_end_matches(']').trim_end_matches('}');
    write(
        &PathBuf::from(format!("{}.9", one.display())),
        &format!(r#"{read},"head_pipeline":{{"id":2,"sha":"{fixed}","status":"success"}}}}"#),
    );
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(Duration::from_secs(20), "REVIEW to take the ticket on the MR's own read", || {
        c.board().ticket(id).is_some_and(|t| t.column == "REVIEW")
    });
    assert!(std::fs::read_to_string(&mrs).unwrap().contains(&failed), "the list never caught up");
    let pr = prs_of(&mut c).into_iter().find(|p| p.ticket == id).unwrap();
    assert_eq!(pr.pipeline, "passed", "{pr:?}");
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-787, lab run 3: !11 merged on GitLab, and a minute later the list
/// still served it `opened` with its pipeline passed, so REVIEW's `Arrives
/// here when: pipeline passes` took the ticket back from PROD. Every MR the
/// list calls open is read by itself: the read's `merged` wins on that very
/// pass, DONE's `merged` takes the ticket, and REVIEW never does.
#[test]
fn an_open_mrs_state_is_read_from_the_mr_itself() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let glab = stub_dir.join("glab");
    std::fs::write(&glab, GLAB_STUB).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mrs = stub_dir.join("mrs.json");
    let pipelines = stub_dir.join("pipelines");
    let one = stub_dir.join("mr");
    write(&mrs, "[]");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge-state",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GLAB_MRS", &mrs.display().to_string()),
            ("MSMN_GLAB_PIPELINES", &pipelines.display().to_string()),
            ("MSMN_GLAB_MR", &one.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://gitlab.example.com/lab/api.git"]);
    let mut c = h.client("forge-state");
    through_prs(&mut c);
    let mut review = c.board().column("REVIEW").unwrap().settings.clone();
    review.arrives_on = Arrival::PipelinePasses;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "REVIEW".into(), settings: review }),
        Response::Ok
    ));
    let mut done = c.board().column("DONE").unwrap().settings.clone();
    done.arrives_on = Arrival::Merged;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "DONE".into(), settings: done }),
        Response::Ok
    ));
    let (id, _sid, branch, path) = ready(&mut c, "alpha");
    let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
    let at_tip = PathBuf::from(format!("{}.{tip}", pipelines.display()));
    write(&at_tip, r#"[{"id":1,"status":"running"}]"#);
    write(&mrs, &mr_json("api", 11, &branch, &tip, None));
    wait_until(Duration::from_secs(20), "!11 to bind, running", || {
        prs_of(&mut c).iter().any(|p| p.ticket == id && p.pipeline == "running")
    });
    let from = c.board().ticket(id).unwrap().column.clone();
    assert_ne!(from, "REVIEW");

    // The merge: the MR's own read says merged; the list keeps `opened`,
    // and its head's pipeline passes, which is what the list-only board
    // read as a pass to arrive on.
    let merge_oid = "7a8b9c0d1e2f30415263748596a7b8c9d0e1f203";
    let read = mr_json("api", 11, &branch, &tip, Some(merge_oid));
    write(
        &PathBuf::from(format!("{}.11", one.display())),
        read.trim().trim_start_matches('[').trim_end_matches(']'),
    );
    write(&at_tip, r#"[{"id":1,"status":"success"}]"#);
    let mut seen: Vec<String> = Vec::new();
    wait_until(Duration::from_secs(20), "DONE to take the ticket on the MR's own read", || {
        let column = c.board().ticket(id).unwrap().column.clone();
        if seen.last() != Some(&column) {
            seen.push(column.clone());
        }
        column == "DONE"
    });
    assert!(!seen.iter().any(|c| c == "REVIEW"), "never pulled to REVIEW: {seen:?}");
    assert!(std::fs::read_to_string(&mrs).unwrap().contains("opened"), "the list never caught up");
    let pr = prs_of(&mut c).into_iter().find(|p| p.ticket == id).unwrap();
    assert_eq!(pr.state, "merged", "{pr:?}");
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// One merged PR as `gh pr list --json` prints it, squashed as `squash`.
fn squashed_pr(n: u64, branch: &str, head: &str, squash: &str) -> String {
    format!(
        r#"{{"number":{n},"url":"https://github.com/example/forge/pull/{n}","state":"MERGED","isDraft":false,
          "headRefName":"{branch}","headRefOid":"{head}","mergeCommit":{{"oid":"{squash}"}},"mergeable":"UNKNOWN",
          "reviewDecision":"APPROVED","updatedAt":"2026-10-15T06:00:00Z",
          "statusCheckRollup":[{{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}}]}}"#
    )
}

/// T-799: a reviewer committed a suggestion on GitHub, so #43's head is one
/// past alpha's tip, and the PR was squash-merged. Nothing was fetched: git
/// cannot say the tip landed, and the head is not the tip. The PR's commits
/// carry the tip, so the leg lands: the card reads merged in the PR's words,
/// `m` says so and DONE admits the ticket. Beta's #44 merged from its own
/// branch too, but its head does not carry beta's tip (the branch was
/// rewritten after the push): nothing lands, and DONE refuses. Each PR's
/// commits are asked once, not on every pass.
#[test]
fn a_merged_pr_whose_head_carries_the_tip_lands_and_one_that_does_not_holds_done() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let (stub_dir, path_var) = gh_stub("forge-past");
    let prs_file = stub_dir.join("prs.json");
    let commits = stub_dir.join("commits");
    let log = stub_dir.join("gh.log");
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        "forge-past",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_COMMITS", &commits.display().to_string()),
            ("MSMN_GH_LOG", &log.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let mut c = h.client("forge-past");
    through_prs(&mut c);
    let (a, a_sid, a_branch, a_path) = ready(&mut c, "alpha");
    let (b, b_sid, b_branch, b_path) = ready(&mut c, "beta");
    let a_tip = git(&a_path, &["rev-parse", "HEAD"]).trim().to_string();
    let b_tip = git(&b_path, &["rev-parse", "HEAD"]).trim().to_string();
    // The suggestions' commits live on the forge alone.
    let (a_head, a_squash) = ("5".repeat(40), "a".repeat(40));
    let (b_head, b_squash) = ("6".repeat(40), "b".repeat(40));
    write(&commits.with_extension("43"), &format!(r#"[{{"sha":"{a_tip}"}},{{"sha":"{a_head}"}}]"#));
    let rewritten = "7".repeat(40);
    assert_ne!(rewritten, b_tip);
    write(
        &commits.with_extension("44"),
        &format!(r#"[{{"sha":"{rewritten}"}},{{"sha":"{b_head}"}}]"#),
    );
    write(
        &prs_file,
        &format!(
            "[{},{}]",
            squashed_pr(43, &a_branch, &a_head, &a_squash),
            squashed_pr(44, &b_branch, &b_head, &b_squash)
        ),
    );

    // Alpha: landed by the forge's word, though its head is past the tip.
    wait_until(Duration::from_secs(20), "alpha's leg to land by #43", || {
        wt_of(&mut c, a).is_some_and(|w| w.merged)
    });
    let wa = wt_of(&mut c, a).unwrap();
    assert_eq!(wa.merged_in, "main (PR #43)", "landed on the base, in the PR's words");
    assert_eq!(wa.merged_oid, a_squash);
    assert!(!wa.needs_rebase);
    let _ = c.request(Command::KillSession { id: a_sid });
    wait_until(Duration::from_secs(15), "`m` to say alpha is merged", || {
        matches!(
            c.request(Command::MergeTicket { id: a }),
            Response::Merge { outcome: MergeOutcome::AlreadyMerged, detail, .. }
                if detail.contains("main (PR #43)")
        )
    });
    assert!(matches!(
        c.request(Command::MoveTicket { id: a, column: "DONE".into(), before: None }),
        Response::Ok
    ));

    // Beta: #44 is merged and read, and its head does not carry the tip.
    wait_until(Duration::from_secs(20), "beta's PR to read merged", || {
        prs_of(&mut c).iter().any(|p| p.ticket == b && p.number == 44 && p.state == "merged")
    });
    let asked = |n: u64| {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(&format!("pulls/{n}/commits")))
            .count()
    };
    wait_until(Duration::from_secs(20), "beta's PR's commits to be asked", || asked(44) >= 1);
    // Several passes on: each PR's commits were asked once.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!((asked(43), asked(44)), (1, 1), "an answer is kept, not asked every pass");
    assert!(!wt_of(&mut c, b).unwrap().merged, "a head that does not carry the tip lands nothing");
    match c.request(Command::MoveTicket { id: b, column: "DONE".into(), before: None }) {
        Response::Err { message } => {
            assert_eq!(message, "worktree unmerged — merge before DONE")
        }
        other => panic!("DONE with the tip not landed: {other:?}"),
    }
    let _ = c.request(Command::KillSession { id: b_sid });
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// One MR merged by GitLab's fast-forward method, as `glab mr list` prints
/// it: no merge commit and no squash, so its head is what landed.
fn ff_merged_mr_json(iid: u64, branch: &str, head: &str) -> String {
    format!(
        r#"[{{"iid":{iid},"web_url":"https://gitlab.example.com/lab/api/-/merge_requests/{iid}","state":"merged",
          "source_branch":"{branch}","sha":"{head}","merge_commit_sha":null,"squash_commit_sha":null,
          "draft":false,"has_conflicts":false,"detailed_merge_status":"not_open","updated_at":"2026-10-15T08:00:00Z"}}]"#
    )
}

/// How many `mr list` calls the stub has logged: one per pass here.
fn lists(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("mr list"))
        .count()
}

/// T-796: under GitLab's fast-forward merge method a merged MR's head is
/// the base's new tip, and a ticket cut after the merge starts there
/// (T-779). Its worktree's HEAD reflog holds that head as the commit it was
/// cut at, which is no work of its own: the MR binds the ticket whose
/// worktree wrote the commit, not the one cut at it. The new ticket reads
/// unmerged with no PR, and once it has a commit of its own `m` asks for an
/// MR of its own (T-810).
/// The MR's own ticket, bound by its reflog alone (the MR is from
/// `fix/alpha`), keeps it after the base took the merge.
#[test]
fn a_ticket_cut_at_a_fast_forward_merge_binds_nothing_of_it() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let stub_dir = std::env::temp_dir().join(format!("msmn-forge-cut-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stub_dir);
    std::fs::create_dir_all(&stub_dir).unwrap();
    let glab = stub_dir.join("glab");
    std::fs::write(&glab, GLAB_STUB).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mrs = stub_dir.join("mrs.json");
    let log = stub_dir.join("glab.log");
    write(&mrs, "[]");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    let Some(h) = Harness::boot_with_env(
        "forge-cut",
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GLAB_MRS", &mrs.display().to_string()),
            ("MSMN_GLAB_LOG", &log.display().to_string()),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    git(&repo, &["remote", "add", "origin", "https://gitlab.example.com/lab/api.git"]);
    let mut c = h.client("forge-cut");
    through_prs(&mut c);

    // alpha's commit went up as `fix/alpha` and !1 merged it fast-forward;
    // a fetch took `origin/main` there and local main stayed behind.
    let (alpha, _, _, alpha_path) = ready(&mut c, "alpha");
    let head = git(&alpha_path, &["rev-parse", "HEAD"]).trim().to_string();
    git(&repo, &["update-ref", "refs/remotes/origin/main", &head]);
    write(&mrs, &ff_merged_mr_json(1, "fix/alpha", &head));
    wait_until(Duration::from_secs(20), "!1 to bind alpha by its reflog", || {
        prs_of(&mut c).iter().any(|p| p.ticket == alpha && p.number == 1)
    });

    // beta is cut after the merge, at the fetched upstream: !1's head.
    let (beta, beta_sid, _, beta_path) = cut(&mut c, "beta");
    assert_eq!(git(&beta_path, &["rev-parse", "HEAD"]).trim(), head, "cut at the merge");
    let since = lists(&log);
    wait_until(Duration::from_secs(20), "two passes after beta's cut", || lists(&log) >= since + 2);
    let beta_prs: Vec<PrItem> = prs_of(&mut c).into_iter().filter(|p| p.ticket == beta).collect();
    assert!(beta_prs.is_empty(), "the cut is no work of beta's: {beta_prs:?}");
    let row = wt_of(&mut c, beta).unwrap();
    assert!(!row.merged, "beta reads unmerged: {row:?}");
    match c.send(Principal::Agent { session: beta_sid }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            assert!(ticket.prs.is_empty(), "{:?}", ticket.prs);
            assert_ne!(ticket.merge_state.as_deref(), Some("merged"));
            let merge = ticket.merge.expect("a branch says who merges it");
            // The board's forge (T-810), not !1's merge.
            assert_eq!(merge.why, "this board merges through pull requests", "{merge:?}");
        }
        other => panic!("get_ticket: {other:?}"),
    }
    // alpha keeps its own MR with the base past it, landed in its words.
    assert!(prs_of(&mut c).iter().any(|p| p.ticket == alpha && p.number == 1));
    let row = wt_of(&mut c, alpha).unwrap();
    assert!(row.merged && row.merged_in.contains("MR !1"), "alpha landed as !1: {row:?}");

    // beta's first commit: no PR carries its merge, and `m` asks for one
    // (T-810), not the !1 that never was beta's.
    std::fs::write(beta_path.join("beta.txt"), "beta\n").unwrap();
    git(&beta_path, &["add", "."]);
    git(&beta_path, &["commit", "-qm", "beta"]);
    let since = lists(&log);
    wait_until(Duration::from_secs(20), "two passes after beta's commit", || {
        lists(&log) >= since + 2
    });
    assert!(!prs_of(&mut c).iter().any(|p| p.ticket == beta), "beta's commit binds nothing");
    let _ = c.request(Command::KillSession { id: beta_sid });
    wait_until(Duration::from_secs(15), "beta to go quiet", || {
        c.board().sessions.iter().filter(|s| s.ticket == beta).all(|s| !s.state.has_pane())
    });
    let main_before = git(&repo, &["rev-parse", "main"]).trim().to_string();
    let mut last = String::new();
    wait_until(Duration::from_secs(20), "`m` to ask beta for an MR", || {
        match c.request(Command::MergeTicket { id: beta }) {
            Response::Merge { outcome: MergeOutcome::Refused, detail, .. } => {
                last = detail;
                last.contains("open an MR from its branch and merge it on GitLab")
            }
            other => panic!("merge: {other:?} (last refusal: {last})"),
        }
    });
    assert_eq!(git(&repo, &["rev-parse", "main"]).trim(), main_before, "nothing lands here");
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-810's backfill: a worker's session ran `gh pr create` from another
/// branch in a transcript the cost ledger passed before it looked for one,
/// so its URL is in no `pr_urls` and PR #42, listed from the restart on,
/// binds to nothing by itself. The board already read the worker's set
/// empty. At the restart, a board that merges
/// through pull requests reads that transcript once, off the writer, and
/// the PR binds as a baseline: no feed line, no crown wake, and TODO's
/// `Moves in when: PR opened` takes nothing.
#[test]
fn a_pr_opened_before_the_ledger_looked_binds_quietly_after_the_backfill() {
    let (stub_dir, path_var) = gh_stub("forge-backfill");
    let prs_file = stub_dir.join("prs.json");
    let passed =
        r#"[{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS"}]"#;
    write(&prs_file, "[]");
    let Some(h) = Harness::boot_with_env(
        "forge_backfill",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("forge_backfill");
    through_prs(&mut c);
    let mut todo = c.board().column("TODO").unwrap().settings.clone();
    todo.arrives_on = Arrival::PrOpened;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "TODO".into(), settings: todo }),
        Response::Ok
    ));
    let k = crowned(&h, &mut c);

    // The worker's turn printed the PR's URL.
    let transcript = h.dir.join("old.jsonl");
    std::fs::write(
        &transcript,
        concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"gh pr create --fill --head fix/old"}}]}}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"https://github.com/example/forge/pull/42\n"}]}}"#,
            "\n",
        ),
    )
    .unwrap();
    hook_send(
        &hook_sock,
        &k.ws.to_string(),
        "SessionStart",
        &format!(r#"{{"session_id":"x","transcript_path":"{}"}}"#, transcript.display()),
    );
    // A turn's end is what has the ledger read; the crown takes its wake.
    turn(&mut c, &hook_sock, k.ws);
    std::thread::sleep(Duration::from_millis(1500));
    turn(&mut c, &hook_sock, k.crown);
    let costs = h.paths.costs_file();
    wait_until(Duration::from_secs(20), "the ledger to read the transcript", || {
        std::fs::read_to_string(&costs).is_ok_and(|t| t.contains("pull/42"))
    });
    // The set is read with the PR unbound, and kept in forge.json.
    wait_until(Duration::from_secs(20), "the worker's set read without it", || {
        std::fs::read_to_string(h.paths.forge_file())
            .is_ok_and(|t| t.contains(&k.worker.to_string()))
    });
    assert!(!prs_of(&mut c).iter().any(|p| p.ticket == k.worker));
    let column = c.board().ticket(k.worker).unwrap().column.clone();
    let woke = wakes(&h, k.worker, "forge");

    // As a build before T-739 left it: the cursor past the line, unlooked.
    // The PR is on the forge from `fix/old`, at a head the worktree never
    // wrote: only its URL binds it.
    drop(c);
    h.restart_after(|| {
        write(
            &prs_file,
            &open_pr_json("fix/old", &"c0ffee42".repeat(5), passed, "MERGEABLE", "REVIEW_REQUIRED"),
        );
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&costs).unwrap()).unwrap();
        let t = &mut v["tickets"][k.worker.to_string()];
        t.as_object_mut().unwrap().remove("pr_urls");
        for (_, cursor) in t["conversations"].as_object_mut().unwrap() {
            cursor.as_object_mut().unwrap().remove("backfilled");
        }
        std::fs::write(&costs, v.to_string()).unwrap();
    });
    let mut c = h.client("forge_backfill-2");
    wait_until(Duration::from_secs(20), "the backfilled PR to bind", || {
        prs_of(&mut c).iter().any(|p| p.ticket == k.worker && p.number == 42)
    });
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(wakes(&h, k.worker, "forge"), woke, "no crown wake");
    assert_eq!(c.board().ticket(k.worker).unwrap().column, column, "no card move");
    let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
    assert!(
        !feed.lines().any(|l| l.contains("\"cmd\":\"pr\"") && l.contains("#42")),
        "no feed line: {feed}"
    );
    let ledger = std::fs::read_to_string(&costs).unwrap();
    assert!(ledger.contains("pull/42") && ledger.contains("\"backfilled\":true"), "{ledger}");
    let _ = c.request(Command::Shutdown);
    let _ = (&k.path, &k.branch, &k.tip, &k.named);
    let _ = std::fs::remove_dir_all(&stub_dir);
}

/// T-810, addendum 2: on a board that merges through pull requests a host
/// whose CLI never signed in is trouble from the first probe, never a board
/// of no forge: the advisory row names the host and the fix, and the
/// snapshot's `forge.trouble` carries the same line.
#[test]
fn a_cli_never_signed_in_is_trouble_with_its_fix() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let name = "forge_unsigned";
    let (stub_dir, path_var) = gh_stub(name);
    std::fs::write(
        stub_dir.join("gh"),
        GH_STUB.replacen(
            "\"auth status\") exit 0 ;;",
            "\"auth status\") echo 'You are not logged into any GitHub hosts.' >&2; exit 1 ;;",
            1,
        ),
    )
    .unwrap();
    let Some(h) = Harness::boot_with_env(
        name,
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "500"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    git(&h.repo, &["remote", "add", "origin", "https://github.com/example/forge.git"]);
    let mut c = h.client(name);
    let row = |c: &mut TestClient| match c.request(Command::Snapshot) {
        Response::Board { notices, forge, .. } => (
            notices.into_iter().find(|n| n.kind == "forge_unreachable").map(|n| n.text),
            forge.trouble,
        ),
        other => panic!("not a board: {other:?}"),
    };
    let _ = ready(&mut c, "alpha");
    // Into main: nothing is read, and nothing is said.
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(row(&mut c), (None, None), "main is silent");
    through_prs(&mut c);
    let fix = "github.com sign-in: You are not logged into any GitHub hosts. (gh auth login \
               --hostname github.com)";
    wait_until(Duration::from_secs(20), "the advisory row to name the host and fix", || {
        row(&mut c).0.is_some_and(|t| t == format!("pull requests not read — {fix}"))
    });
    assert_eq!(row(&mut c).1.as_deref(), Some(fix), "the snapshot carries it");
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
}
