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
use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState, WorkspaceStrategy};
use mesimon_core::command::{AutomationStatus, Command, PendingAction, Response};

fn git_ok(repo: &Path, args: &[&str]) -> bool {
    Proc::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn automation_of(c: &mut TestClient) -> AutomationStatus {
    match c.request(Command::Snapshot) {
        Response::Board { automation, .. } => automation,
        other => panic!("not a board: {other:?}"),
    }
}

/// A worktree ticket with a spawned agent and one commit on its branch.
fn ready(c: &mut TestClient, title: &str) -> (ulid::Ulid, uuid::Uuid, String, PathBuf) {
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
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
    init_repo(&repo, "a.txt", "hello\n");
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
    assert!(pending_of(&mut c, Some(a)).is_empty(), "a manual ticket is owed nothing");
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

/// T-351: the gate is `train_busy`, not the whole board. A worktree agent
/// grinding away on ITS OWN ticket cannot be touched by another ticket's
/// ff-merge — that merge writes the root checkout or nothing at all — so it
/// must not hold one up. Before this the train sat still while any agent
/// anywhere was mid-turn, and a busy board never merged anything.
#[test]
fn a_grinding_worktree_does_not_hold_another_tickets_merge() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let Some(h) = Harness::boot_with_env(
        "train-busy",
        Some(STUB),
        &[("MESIMON_WT_REFRESH_TICKS", "4"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("train-busy");
    let (a, sa, branch_a, _wt_a) = ready(&mut c, "alpha");
    let (b, sb, _branch_b, _wt_b) = ready(&mut c, "beta");
    std::thread::sleep(Duration::from_millis(500));

    // A finishes its turn and lands in REVIEW: a merge candidate.
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "A running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "A idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(Duration::from_secs(5), "A in REVIEW", || {
        c.board().ticket(a).unwrap().column == "REVIEW"
    });

    // B is left MID-TURN in its own worktree — the bystander that used to
    // stop the whole train. No Stop hook: it grinds for the rest of the test.
    hook_send(&hook_sock, &sb.to_string(), "UserPromptSubmit", r#"{"prompt":"grind"}"#);
    c.await_state(sb, "B running", |s| *s == SessionState::Running);
    assert_eq!(c.board().ticket(b).unwrap().column, "IN PROGRESS");

    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    // The card does not claim to be waiting on B: nothing holds this merge.
    wait_until(Duration::from_secs(10), "A to be owed a merge", || {
        pending_of(&mut c, Some(a)).iter().any(|p| p.action == PendingAction::Merge)
    });
    let owed = pending_of(&mut c, Some(a));
    let merge = owed.iter().find(|p| p.action == PendingAction::Merge).unwrap();
    assert!(merge.waits_on.is_empty(), "a worktree bystander is not a wait: {merge:?}");

    // And it merges, with B still running.
    wait_until(Duration::from_secs(15), "A to merge past a grinding B", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_a, "main"])
    });
    assert_eq!(
        c.board().sessions.iter().find(|s| s.id == sb).map(|s| s.state.clone()),
        Some(SessionState::Running),
        "B was never waited for, and never disturbed"
    );
    let _ = c.request(Command::Shutdown);
}

/// A stub that appends every byte it reads to `got.txt` beside itself and
/// repaints while idle: session state decides whether the agent is working,
/// not the time since its terminal last repainted.
const LOGGING_STUB: &str = r#"#!/usr/bin/env python3
import os, select, sys, tty
from pathlib import Path
tty.setcbreak(sys.stdin.fileno())
got = Path(__file__).with_name('got.txt')
while True:
    print('.', end='', flush=True)
    if select.select([sys.stdin], [], [], 0.1)[0]:
        data = os.read(sys.stdin.fileno(), 4096)
        if not data:
            break
        with got.open('ab') as output:
            output.write(data)
"#;

#[test]
fn the_train_merges_asks_to_rebase_once_and_stops_with_its_board() {
    // The flags (and the train) on a 1 s cadence; the quiet probe kept out.
    let Some(h) = Harness::boot_with_env(
        "train",
        Some(LOGGING_STUB),
        &[("MESIMON_WT_REFRESH_TICKS", "4"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let got = h.dir.join("got.txt");
    let text = || std::fs::read_to_string(&got).unwrap_or_default();
    let feed = || std::fs::read_to_string(h.paths.activity_log()).unwrap_or_default();
    let repo = h.repo.clone();
    // The harness boots on a bare directory; the train needs a repository.
    init_repo(&repo, "a.txt", "hello\n");
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

/// One rebase ask outstanding per base tip (T-435). Three REVIEW tickets
/// behind one hand merge were asked 31 s apart: the hold for a mid-rebase
/// ticket read `needs_rebase`, which the git step clears twenty seconds into
/// a turn whose words end "run the tests … before we merge", so the pass
/// fell through and asked the next ticket onto the same tip — and every
/// merge then re-asked the rest. Here B is asked, lands its rebase and keeps
/// its turn: C is not asked and its owed row says it waits on B. B's turn
/// ends, B merges, and only then is C asked — once, onto the tip B moved.
#[test]
fn a_ticket_in_its_rebase_turn_holds_the_next_ask_until_it_merges() {
    let Some(h) = Harness::boot_with_env(
        "train-turn",
        Some(LOGGING_STUB),
        &[("MESIMON_WT_REFRESH_TICKS", "4"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let got = h.dir.join("got.txt");
    let text = || std::fs::read_to_string(&got).unwrap_or_default();
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("train-turn");

    let (a, sa, branch_a, _wt_a) = ready(&mut c, "alpha");
    let (b, sb, branch_b, wt_b) = ready(&mut c, "beta");
    let (cid, sc, branch_c, wt_c) = ready(&mut c, "gamma");
    wait_until(Duration::from_secs(15), "three panes", || {
        tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count() >= 3)
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
    let asks = |branch: &str| {
        text()
            .lines()
            .filter(|l| l.contains(&format!("Rebase your current branch {branch}")))
            .count()
    };
    // C, B, then A: automove parks each on top, so REVIEW reads A, B, C.
    for sid in [sc, sb, sa] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }
    wait_until(Duration::from_secs(5), "all three in REVIEW", || {
        let board = c.board();
        [a, b, cid].iter().all(|t| board.ticket(*t).unwrap().column == "REVIEW")
    });

    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    // A merges; B and C fall behind; B, first in board order, is asked.
    wait_until(Duration::from_secs(15), "A to be merged", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_a, "main"])
    });
    wait_until(Duration::from_secs(15), "B to be asked to rebase", || asks(&branch_b) == 1);
    assert_eq!(asks(&branch_c), 0, "C is behind B in board order");

    // B takes the turn and lands the git step at once — the tests run on.
    start(&mut c, sb);
    git(&wt_b, &["rebase", "-q", "main"]);
    // Three flag refreshes: the old hold read `needs_rebase`, which is now
    // false for B, and the pass asked C onto the same tip within one.
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(asks(&branch_c), 0, "C asked while B's rebase turn runs: {}", text());
    let owed = pending_of(&mut c, Some(cid));
    let rebase = owed.iter().find(|p| p.action == PendingAction::Rebase).unwrap();
    let key_b = c.board().ticket(b).unwrap().short_key.clone();
    assert_eq!(rebase.waits_on, vec![key_b], "C's row names B: {rebase:?}");

    // B's turn ends: B merges, main moves, and C is asked onto the new tip.
    stop(&mut c, sb);
    wait_until(Duration::from_secs(15), "B to be merged after its rebase", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_b, "main"])
    });
    wait_until(Duration::from_secs(15), "C to be asked to rebase", || asks(&branch_c) == 1);
    start(&mut c, sc);
    git(&wt_c, &["rebase", "-q", "main"]);
    stop(&mut c, sc);
    wait_until(Duration::from_secs(15), "C to be merged after its rebase", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_c, "main"])
    });
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!((asks(&branch_b), asks(&branch_c)), (1, 1), "one ask each: {}", text());
    let _ = c.request(Command::Shutdown);
}

/// A merge the CHECKOUT refuses (T-289). An untracked `alpha.txt` sits where
/// the fast-forward would write one, so git refuses it — and before this the
/// card went on saying `auto-merge ∙ next` for as long as the tree stayed
/// dirty, with nothing anywhere saying why. Now the reason rides the pending
/// row and a standing notice spells it out; and taking the file away is
/// enough to unstick it, which the refusal's `(branch tip, base tip)` key
/// never was — a stash moves neither.
#[test]
fn a_merge_the_checkout_refuses_says_why_and_retries_once_it_is_clean() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    let Some(h) = Harness::boot_with_env(
        "train-blocked",
        Some(STUB),
        &[("MESIMON_WT_REFRESH_TICKS", "4"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let feed = || std::fs::read_to_string(h.paths.activity_log()).unwrap_or_default();
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("train-blocked");
    // B needs a rebase; A is already ready to merge onto the newer base.
    // Rebasing B before A lands would only make B need another rebase.
    let (b, sb, branch_b, wt_b) = ready(&mut c, "beta");
    std::fs::write(repo.join("base.txt"), "base advanced\n").unwrap();
    git(&repo, &["add", "base.txt"]);
    git(&repo, &["commit", "-qm", "advance base"]);
    let (a, sa, branch_a, _) = ready(&mut c, "alpha");
    // In the way: the ff would create this file, and git will not overwrite
    // one it does not know about.
    std::fs::write(repo.join("alpha.txt"), "not mine\n").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    for sid in [sa, sb] {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    }
    wait_until(Duration::from_secs(5), "A in REVIEW", || {
        c.board().ticket(a).unwrap().column == "REVIEW"
    });
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(15), "the refusal to be recorded", || {
        feed().contains("merge_train_refused:merge")
    });
    // Cover both the pass that refuses A and later passes that remember
    // that refusal. Neither may fall through to asking B to rebase.
    std::thread::sleep(Duration::from_millis(2500));
    assert!(automation_of(&mut c).train_asked.is_empty(), "rebased past a blocked merge");
    assert!(!feed().contains("merge_train_rebase_asked"));
    let rebase = pending_of(&mut c, Some(b));
    let rebase =
        rebase.iter().find(|p| p.action == PendingAction::Rebase).expect("B is owed a rebase");
    assert_eq!(rebase.waits_on, vec![c.board().ticket(a).unwrap().short_key.clone()]);
    // The row carries the reason, and the board says it in a sentence.
    let owed = pending_of(&mut c, Some(a));
    let merge = owed
        .iter()
        .find(|p| p.action == PendingAction::Merge)
        .unwrap_or_else(|| panic!("a merge row: {owed:?}"));
    let why = merge.text.clone().expect("the refusal travels with the row");
    assert!(why.contains("uncommitted changes"), "{why}");
    let notices = match c.request(Command::Snapshot) {
        Response::Board { notices, .. } => notices,
        other => panic!("not a board: {other:?}"),
    };
    let n = notices
        .iter()
        .find(|n| n.kind == "merge_train_blocked")
        .unwrap_or_else(|| panic!("a standing notice: {notices:?}"));
    assert!(
        n.text.contains(&c.board().ticket(a).unwrap().short_key) && n.text.contains(&why),
        "{}",
        n.text
    );
    assert!(!git_ok(&repo, &["merge-base", "--is-ancestor", &branch_a, "main"]));
    // Out of the way. Neither tip moves — only the checkout's own status —
    // and that is what lets the train try again.
    std::fs::remove_file(repo.join("alpha.txt")).unwrap();
    wait_until(Duration::from_secs(25), "A to merge once the checkout is clean", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_a, "main"])
    });
    wait_until(Duration::from_secs(5), "the notice to go with it", || {
        !pending_of(&mut c, Some(a)).iter().any(|p| p.action == PendingAction::Merge)
    });
    wait_until(Duration::from_secs(15), "B to be asked after A merges", || {
        automation_of(&mut c).train_asked.iter().any(|ask| ask.ticket == b && ask.current)
    });
    hook_send(&hook_sock, &sb.to_string(), "UserPromptSubmit", r#"{"prompt":"rebase"}"#);
    c.await_state(sb, "running", |s| *s == SessionState::Running);
    git(&wt_b, &["rebase", "-q", "main"]);
    hook_send(&hook_sock, &sb.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sb, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(Duration::from_secs(15), "B to merge after one rebase onto A", || {
        git_ok(&repo, &["merge-base", "--is-ancestor", &branch_b, "main"])
    });
    wait_until(Duration::from_secs(5), "both merges in the feed", || {
        feed().lines().filter(|line| line.contains("merge_train_merged")).count() == 2
    });
    let actions: Vec<_> = feed()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|v| v.get("cmd").and_then(|cmd| cmd.as_str()).map(String::from))
        .filter(|cmd| matches!(cmd.as_str(), "merge_train_merged" | "merge_train_rebase_asked"))
        .collect();
    assert_eq!(actions, ["merge_train_merged", "merge_train_rebase_asked", "merge_train_merged"]);
    let _ = c.request(Command::Shutdown);
}
