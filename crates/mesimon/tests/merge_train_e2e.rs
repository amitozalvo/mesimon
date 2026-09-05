//! The merge train (2026-09-04), end to end: two worktree tickets whose
//! agents have finished sit in REVIEW; the armed train fast-forwards the
//! first, tells its agent, asks the second to rebase — once — and merges it
//! once it has; and the train stops the moment the board that armed it
//! closes its connection.
//!
//! The stub agent appends every line it reads to one file beside itself.
//! Every sentence the train pastes names the branch, so one file says which
//! agent heard what, and `read` returning proves the separate Enter (T-5).

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionState, WorkspaceStrategy};
use mesimon_core::command::{AutomationStatus, Command, Pending, Response, WorktreeItem};

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Proc::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn git_ok(repo: &Path, args: &[&str]) -> bool {
    Proc::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn worktrees_of(c: &mut TestClient) -> Vec<WorktreeItem> {
    match c.request(Command::Snapshot) {
        Response::Board { worktrees, .. } => worktrees,
        other => panic!("not a board: {other:?}"),
    }
}

fn automation_of(c: &mut TestClient) -> AutomationStatus {
    match c.request(Command::Snapshot) {
        Response::Board { automation, .. } => automation,
        other => panic!("not a board: {other:?}"),
    }
}

fn wait_attached(c: &mut TestClient, ticket: ulid::Ulid) -> WorktreeItem {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let item = worktrees_of(c).into_iter().find(|w| w.ticket == ticket);
        if let Some(w) = &item {
            if w.status == "attached" {
                return w.clone();
            }
        }
        assert!(Instant::now() < deadline, "binding never attached; last: {item:?}");
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// A worktree ticket with a spawned agent and one commit on its branch.
fn ready(c: &mut TestClient, title: &str) -> (ulid::Ulid, uuid::Uuid, String, PathBuf) {
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: title.into() });
    let id = c.board().tickets.iter().find(|t| t.title == title).unwrap().id;
    assert!(matches!(
        c.request(Command::SetWorkspace { id, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: id,
            kind: SessionKind::Claude,
            submit_prompt: false
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

/// A repository under the harness's bare directory, one commit on `main`.
fn init_repo(repo: &Path) {
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.email", "e2e@t"]);
    git(repo, &["config", "user.name", "e2e"]);
    std::fs::write(repo.join("a.txt"), "hello\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "init"]);
}

fn pending_of(c: &mut TestClient, ticket: ulid::Ulid) -> Vec<Pending> {
    match c.request(Command::Snapshot) {
        Response::Board { pending, .. } => {
            pending.into_iter().filter(|p| p.ticket == ticket).collect()
        }
        other => panic!("not a board: {other:?}"),
    }
}

/// `t` on the card (T-227): a REVIEW ticket the armed train would merge is
/// taken off it — the snapshot lists nothing owed, the flag is in the ticket
/// file (a restart cannot re-arm it), and main does not move — and put back,
/// after which the train finishes the job.
#[test]
fn a_ticket_taken_off_the_train_is_left_alone_until_put_back() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let Some(h) = Harness::boot_with_env(
        "train-manual",
        Some(STUB),
        &[("MESIMON_WT_REFRESH_TICKS", "4"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let feed = || std::fs::read_to_string(h.paths.activity_log()).unwrap_or_default();
    let repo = h.repo.clone();
    init_repo(&repo);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("train-manual");
    let (a, sa, branch_a, _) = ready(&mut c, "alpha");
    std::thread::sleep(Duration::from_millis(500));
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(Duration::from_secs(5), "A in REVIEW", || {
        c.board().ticket(a).unwrap().column == "REVIEW"
    });
    // Off the train BEFORE it is armed: the person's choice comes first.
    assert!(matches!(c.request(Command::SetManualMerge { id: a, on: true }), Response::Ok));
    assert!(c.board().ticket(a).unwrap().manual_merge);
    let ticket_file = h.paths.board_dir.join("board/tickets/T-1/ticket.toml");
    let on_disk = std::fs::read_to_string(&ticket_file).unwrap();
    assert!(on_disk.contains("manual_merge = true"), "persisted:\n{on_disk}");
    assert!(
        on_disk.starts_with(&format!("schema_version = {}", mesimon_daemon::store::TICKET_SCHEMA)),
        "{on_disk}"
    );
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    assert!(automation_of(&mut c).merge_train);
    // Nothing is owed and nothing moves, through several passes.
    std::thread::sleep(Duration::from_millis(3000));
    assert!(pending_of(&mut c, a).is_empty(), "a manual ticket is owed nothing");
    assert!(
        !git_ok(&repo, &["merge-base", "--is-ancestor", &branch_a, "main"]),
        "merged while off the train"
    );
    assert!(!feed().contains("merge_train_merged"));
    assert!(feed().contains("set_manual_merge"), "the gesture is in the feed:\n{}", feed());
    // Setting what is already set writes nothing.
    assert!(matches!(c.request(Command::SetManualMerge { id: a, on: true }), Response::Ok));
    // Back on: the next pass merges it.
    assert!(matches!(c.request(Command::SetManualMerge { id: a, on: false }), Response::Ok));
    assert!(!c.board().ticket(a).unwrap().manual_merge);
    assert!(!std::fs::read_to_string(&ticket_file).unwrap().contains("manual_merge"));
    wait_until(Duration::from_secs(15), "A to be merged once back on the train", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_a, "main"])
    });
    let _ = c.request(Command::Shutdown);
}

#[test]
fn the_train_merges_asks_to_rebase_once_and_stops_with_its_board() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    // The flags (and the train) on a 1 s cadence; the quiet probe kept out.
    let Some(h) = Harness::boot_with_env(
        "train",
        Some(STUB),
        &[("MESIMON_WT_REFRESH_TICKS", "4"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let got = h.dir.join("got.txt");
    let text = || std::fs::read_to_string(&got).unwrap_or_default();
    let feed = || std::fs::read_to_string(h.paths.activity_log()).unwrap_or_default();
    let repo = h.repo.clone();
    // The harness boots on a bare directory; the train needs a repository.
    init_repo(&repo);
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("train");

    let (a, sa, branch_a, _wt_a) = ready(&mut c, "alpha");
    let (b, sb, branch_b, wt_b) = ready(&mut c, "beta");
    // Both stubs reading before anything is pasted.
    wait_until(Duration::from_secs(15), "two panes", || {
        tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count() >= 2)
            .unwrap_or(false)
    });
    std::thread::sleep(Duration::from_millis(500));
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    // A turn each: automove lands both in REVIEW, the later one on top —
    // so B first, then A, and A is the first candidate.
    for sid in [sb, sa] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }
    wait_until(Duration::from_secs(5), "both in REVIEW", || {
        let board = c.board();
        [a, b].iter().all(|t| board.ticket(*t).unwrap().column == "REVIEW")
    });
    // Nothing moves before the train is armed.
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!git_ok(&repo, &["merge-base", "--is-ancestor", &branch_a, "main"]));
    assert!(!feed().contains("merge_train"));

    // Arm: this connection owns the train.
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: true }),
        Response::Ok
    ));
    assert!(automation_of(&mut c).merge_train);
    // A merges first, and its agent is told.
    wait_until(Duration::from_secs(15), "A to be merged", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_a, "main"])
    });
    // The feed flushes on the tick, a beat behind git.
    wait_until(Duration::from_secs(5), "the feed to say so", || {
        feed().contains("merge_train_merged")
    });
    wait_until(Duration::from_secs(10), "A's agent to hear it", || {
        text().contains(&format!("Your branch {branch_a} has been merged into main"))
    });
    // The notice started a turn on A: the train waits for it. Ack and end it.
    start(&mut c, sa);
    stop(&mut c, sa);
    // B fell behind: asked to rebase — once.
    wait_until(Duration::from_secs(15), "B to be asked to rebase", || {
        text().contains(&format!("Rebase your current branch {branch_b} onto main"))
    });
    let asks = || {
        text()
            .lines()
            .filter(|l| l.contains(&format!("Rebase your current branch {branch_b}")))
            .count()
    };
    assert_eq!(asks(), 1);
    assert!(automation_of(&mut c)
        .train_asked
        .iter()
        .any(|x| x.ticket == b && x.current && x.by == "train"));
    // The agent takes the turn, rebases, ends it; the same tip is not asked again.
    start(&mut c, sb);
    git(&wt_b, &["rebase", "-q", "main"]);
    stop(&mut c, sb);
    wait_until(Duration::from_secs(15), "B to be merged after its rebase", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_b, "main"])
    });
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(asks(), 1, "asked once per base tip");

    // The safeguard: a third ready ticket, and the arming board goes away.
    let (cid, sc, branch_c, _) = ready(&mut c, "gamma");
    wait_until(Duration::from_secs(15), "three panes", || {
        tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count() >= 3)
            .unwrap_or(false)
    });
    std::thread::sleep(Duration::from_millis(500));
    // Disarm by closing the connection BEFORE C finishes its turn, so the
    // train never sees a quiet board while armed.
    drop(c);
    let mut c2 = h.client("train-2");
    start(&mut c2, sc);
    stop(&mut c2, sc);
    wait_until(Duration::from_secs(5), "C in REVIEW", || {
        c2.board().ticket(cid).unwrap().column == "REVIEW"
    });
    wait_until(Duration::from_secs(5), "the disarm to be recorded", || {
        feed().contains("merge_train_disarmed")
    });
    assert!(!automation_of(&mut c2).merge_train, "a closed board is a stopped train");
    std::thread::sleep(Duration::from_millis(3000));
    assert!(
        !git_ok(&repo, &["merge-base", "--is-ancestor", &branch_c, "main"]),
        "nothing merged unarmed"
    );
    // Re-armed from the new board, it finishes the job.
    assert!(matches!(
        c2.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(15), "C to be merged once re-armed", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_c, "main"])
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !text().contains(&format!("Your branch {branch_c}")),
        "silent after a merge: {}",
        text()
    );
    let _ = c2.request(Command::Shutdown);
}
