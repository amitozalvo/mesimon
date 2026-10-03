//! T-614 e2e: the checkout's `.mesimon-worktree-init.sh` runs in each new
//! worktree before the ticket's agent starts, with the checkout and the
//! worktree named, and nothing of how it went holds the agent back: a
//! failing script marks the ticket page and the agent still starts, a
//! script past the timeout is killed and reads timed out, the feed keeps
//! each run's word, and `get_ticket` tells the agent the script exists and
//! how its last run went.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, WorkspaceStrategy};
use mesimon_core::command::{Command, Response};
use mesimon_core::workspace::INIT_SCRIPT;
use mesimon_core::Principal;

/// Claude as a stub: says whether the init script's marker was there when
/// it started (the order the feature promises), then idles.
const STUB: &str = "#!/bin/sh\n\
    if [ -f \"$PWD/init-marker\" ]; then echo seen > \"$PWD/stub-saw\"; \
    else echo unseen > \"$PWD/stub-saw\"; fi\n\
    trap 'exit 0' TERM\nwhile true; do sleep 1; done\n";

fn worktree_ticket(c: &mut TestClient, title: &str) -> ulid::Ulid {
    let id = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: Some(WorkspaceStrategy::Worktree),
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: id,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning
    ));
    id
}

/// The ticket's session id once the parked spawn replayed.
fn session_of(c: &mut TestClient, ticket: ulid::Ulid) -> uuid::Uuid {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(s) = c.board().sessions.iter().find(|s| s.ticket == ticket) {
            return s.id;
        }
        assert!(Instant::now() < deadline, "parked spawn never replayed");
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[test]
fn the_init_script_runs_before_the_agent_and_never_holds_it_back() {
    let Some(h) =
        Harness::boot_with_env("wtinit", Some(STUB), &[("MESIMON_WORKTREE_INIT_MS", "1500")])
    else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let mut c = h.client("wtinit");
    let script = h.repo.join(INIT_SCRIPT);

    // ---- a script that works: uncommitted, not executable ----------------
    // Read from the checkout and run under sh, in the worktree, with both
    // paths and the ticket named; what it prints goes to the journal.
    std::fs::write(
        &script,
        "printf '%s\\n%s\\n%s\\n%s\\n' \"$MESIMON_CHECKOUT\" \"$MESIMON_WORKTREE\" \"$PWD\" \
         \"$MESIMON_TICKET\" > init-marker\necho seeded the cache\n",
    )
    .unwrap();
    let t1 = worktree_ticket(&mut c, "warm start");
    let wt = wait_attached(&mut c, t1);
    assert_eq!(wt.detail, None, "a run that went well says nothing on the page");
    let path = std::path::PathBuf::from(wt.path.clone().unwrap());
    let marker = std::fs::read_to_string(path.join("init-marker")).unwrap();
    let lines: Vec<&str> = marker.lines().collect();
    assert_eq!(
        lines,
        [
            h.paths.repo_root.display().to_string().as_str(),
            path.display().to_string().as_str(),
            path.display().to_string().as_str(),
            "T-1",
        ],
        "the script sees the checkout, the worktree (its cwd) and the ticket"
    );
    // The agent started after the script, and in the same worktree.
    let s1 = session_of(&mut c, t1);
    wait_until(Duration::from_secs(10), "the stub to start", || path.join("stub-saw").exists());
    assert_eq!(std::fs::read_to_string(path.join("stub-saw")).unwrap().trim(), "seen");
    // The feed flushes on the writer's own cadence: waited for, not read.
    wait_until(Duration::from_secs(5), "the feed line", || {
        feed_count(&h, "worktree_init", t1) == 1
    });
    let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
    assert!(feed.contains("\"outcome\":\"ok ∙ 0 s\""), "{feed}");
    let journal = std::fs::read_to_string(h.paths.state_dir.join("daemon.log")).unwrap();
    assert!(journal.contains("init T-1: ok ∙ 0 s"), "{journal}");
    assert!(journal.contains("T-1 | seeded the cache"), "the output is in the journal: {journal}");
    // The agent reads the script on its own ticket: present, and how its
    // last run went.
    match c.send(Principal::Agent { session: s1 }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            let init = ticket.worktree_init.expect("worktree_init on every read");
            assert_eq!(init.script, INIT_SCRIPT);
            assert!(init.present);
            assert_eq!(init.last.as_deref(), Some("ok ∙ 0 s"));
            assert!(init.about.contains("before its agent starts"), "{}", init.about);
        }
        other => panic!("get_ticket: {other:?}"),
    }

    // ---- a script that fails: the page says so, the agent still starts --
    std::fs::write(&script, "#!/bin/sh\necho no cache here >&2\nexit 2\n").unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let t2 = worktree_ticket(&mut c, "cold start");
    let wt = wait_attached(&mut c, t2);
    assert_eq!(wt.detail.as_deref(), Some("init failed ∙ exit 2"));
    let path2 = std::path::PathBuf::from(wt.path.clone().unwrap());
    let s2 = session_of(&mut c, t2);
    wait_until(Duration::from_secs(10), "the second stub", || path2.join("stub-saw").exists());
    assert_eq!(std::fs::read_to_string(path2.join("stub-saw")).unwrap().trim(), "unseen");
    wait_until(Duration::from_secs(5), "the feed line", || {
        feed_count(&h, "worktree_init", t2) == 1
    });
    let journal = std::fs::read_to_string(h.paths.state_dir.join("daemon.log")).unwrap();
    assert!(journal.contains("init T-2: failed ∙ exit 2"), "{journal}");
    assert!(journal.contains("T-2 | no cache here"), "stderr is in the journal too: {journal}");
    match c.send(Principal::Agent { session: s2 }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            let init = ticket.worktree_init.unwrap();
            assert_eq!(init.last.as_deref(), Some("failed ∙ exit 2 ∙ 0 s"));
        }
        other => panic!("get_ticket: {other:?}"),
    }
    // The first ticket's own run is untouched by the second's.
    assert_eq!(wt_of(&mut c, t1).unwrap().detail, None);

    // ---- a script past the timeout: killed, and said -------------------
    std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
    let t3 = worktree_ticket(&mut c, "slow start");
    // While it runs the page says so; the seam is 1.5 s, so a poll sees it.
    let mut seen_running = false;
    let deadline = Instant::now() + Duration::from_secs(15);
    let wt = loop {
        let item = wt_of(&mut c, t3);
        if let Some(w) = &item {
            if w.status == "attached" {
                break w.clone();
            }
            if w.detail.as_deref() == Some("init script running") {
                seen_running = true;
            }
        }
        assert!(Instant::now() < deadline, "never attached: {item:?}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(seen_running, "the page never said the script was running");
    assert_eq!(wt.detail.as_deref(), Some("init timed out"));
    let _ = session_of(&mut c, t3);
    wait_until(Duration::from_secs(5), "the feed line", || {
        feed_count(&h, "worktree_init", t3) == 1
    });
    let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
    assert!(feed.contains("\"outcome\":\"timed out ∙ 2 s\""), "{feed}");

    // ---- no script: nothing runs, and the agent is told there is none --
    std::fs::remove_file(&script).unwrap();
    let t4 = worktree_ticket(&mut c, "plain start");
    let wt = wait_attached(&mut c, t4);
    assert_eq!(wt.detail, None);
    let s4 = session_of(&mut c, t4);
    assert_eq!(feed_count(&h, "worktree_init", t4), 0);
    match c.send(Principal::Agent { session: s4 }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            let init = ticket.worktree_init.unwrap();
            assert!(!init.present);
            assert_eq!(init.last, None);
        }
        other => panic!("get_ticket: {other:?}"),
    }
}
