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
//! The train reads the forge at a turn's end (T-761): a turn that pushed
//! nothing merges on the look after it with no read at all, and one that
//! pushed is read at once rather than on the minute.
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

use mesimon_core::board::{Arrival, SessionKind, SessionState, WorkspaceStrategy};
use mesimon_core::command::{Command, MergeOutcome, PrItem, Response};
use mesimon_core::Principal;

/// The stub forge CLI: one shell script answering as `gh` would, from the
/// files the test points it at.
const GH_STUB: &str = r#"#!/bin/sh
[ -n "$MSMN_GH_LOG" ] && echo "$*" >> "$MSMN_GH_LOG"
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

    // The merge commit's pipeline passing is its deploy run finishing, not
    // the ticket's pipeline passing: REVIEW does not pull it back (T-767).
    write(
        &checks_file,
        r#"{"check_runs":[{"name":"release","status":"completed","conclusion":"success"}]}"#,
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
    let gh_pr = |number: u64, branch: &str, head: &str, state: &str, merge: &str, at: &str| {
        let checks = if state == "OPEN" { running } else { passed };
        format!(
            r#"{{"number":{number},"url":"https://github.com/example/forge/pull/{number}","state":"{state}",
              "isDraft":false,"headRefName":"{branch}","headRefOid":"{head}","mergeCommit":{merge},
              "mergeable":"MERGEABLE","reviewDecision":"APPROVED","updatedAt":"{at}",
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

/// The stub `glab`: `auth status` passes, `api … user` names the account,
/// `mr list` prints the file the test writes, and `api …/pipelines?sha=X`
/// prints `<pipelines file>.X` where one exists, else no pipeline — so the
/// merge commit's sha has a pipeline and the squash commit's has none,
/// which is the lab's shape (T-754).
const GLAB_STUB: &str = r#"#!/bin/sh
case "$1 $2" in
  "auth status") exit 0 ;;
  "mr list") cat "$MSMN_GLAB_MRS" 2>/dev/null || echo '[]' ;;
  "api --hostname")
    case "$4" in
      user) echo '{"username":"me"}' ;;
      *pipelines?sha=*)
        sha="${4#*sha=}"; sha="${sha%%&*}"
        if [ -f "$MSMN_GLAB_PIPELINES.$sha" ]; then cat "$MSMN_GLAB_PIPELINES.$sha"; else echo '[]'; fi ;;
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
/// that commit, and the crown hears the merge.
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
    write(&pipelines.with_extension(merge_sha), r#"[{"status":"success"}]"#);
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

/// One train ticket's turn ends with the forge's minute far away (T-761):
/// the train merges it on the turn's own look. `push` moves the branch's
/// remote-tracking ref in the turn, as a `git push` does. The calls `gh pr
/// list` made after the first read, and how long the merge took.
fn turn_end_merges(name: &str, push: bool) -> Option<(usize, Duration)> {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
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
    let prs_file = stub_dir.join("prs.json");
    let log = stub_dir.join("calls.log");
    write(&prs_file, "[]");
    let path_var = format!(
        "{}:{}",
        stub_dir.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())
    );
    // The minute is ten: after the first pass, any read is the turn's.
    let h = Harness::boot_with_env(
        name,
        Some(STUB),
        &[
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_FORGE_EVERY_MS", "600000"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("PATH", &path_var),
            ("MSMN_GH_PRS", &prs_file.display().to_string()),
            ("MSMN_GH_LOG", &log.display().to_string()),
        ],
    )?;
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
    let mut c = h.client(name);
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    let (id, sid, branch, path) = ready(&mut c, "alpha");
    // The first pass reads the ticket, before its turn.
    wait_until(Duration::from_secs(20), "the first forge pass to list", || lists() >= 1);
    std::thread::sleep(Duration::from_millis(500));
    let first = lists();
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    if push {
        let tip = git(&path, &["rev-parse", "HEAD"]).trim().to_string();
        git(&path, &["update-ref", &format!("refs/remotes/origin/{branch}"), &tip]);
    }
    let stopped = std::time::Instant::now();
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(30), "the train to merge on the turn's end", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch, "main"])
    });
    let took = stopped.elapsed();
    assert!(c.board().ticket(id).is_some());
    let reads = lists() - first;
    let _ = c.request(Command::Shutdown);
    let _ = std::fs::remove_dir_all(&stub_dir);
    Some((reads, took))
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

#[test]
fn a_turn_that_pushed_nothing_merges_on_its_look_with_no_read() {
    let Some((reads, took)) = turn_end_merges("forge_nopush", false) else { return };
    assert_eq!(reads, 0, "nothing pushed, nothing opened: no forge read");
    // The look is a few git forks; the minute is ten.
    assert!(took < Duration::from_secs(10), "merged {took:?} after the turn's end");
}

#[test]
fn a_turn_that_pushed_is_read_at_its_end_not_on_the_minute() {
    let Some((reads, took)) = turn_end_merges("forge_pushed", true) else { return };
    assert_eq!(reads, 1, "the turn's own read, of its repository alone");
    // Under the floor of one read per repository per ten seconds from the
    // first pass; the minute is ten.
    assert!(took < Duration::from_secs(20), "merged {took:?} after the turn's end");
}
