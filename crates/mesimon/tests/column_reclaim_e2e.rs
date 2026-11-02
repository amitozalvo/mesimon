//! Column reclamation keeps the card, branch and exact wake workspace.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use mesimon_core::board::SessionState;
use mesimon_core::command::{Command, Response};
use std::time::Duration;

const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";

fn arrive(c: &mut TestClient, id: ulid::Ulid, column: &str) {
    let response = c.request(Command::MoveTicket { id, column: column.into(), before: None });
    assert!(matches!(response, Response::Ok), "{response:?}");
}

fn await_refusal(c: &mut TestClient, id: ulid::Ulid, reason: &str) {
    wait_until(Duration::from_secs(15), reason, || {
        wt_of(c, id)
            .is_some_and(|w| w.status == "attached" && w.detail.is_some_and(|d| d.contains(reason)))
    });
}

#[test]
fn arrival_waits_for_a_parked_live_pane_then_wake_recuts_the_same_branch() {
    // The first process ignores TERM, proving Sleeping alone never frees its cwd.
    let stub =
        "#!/bin/sh\ntrap '' TERM\n: > \"$(dirname \"$0\")/armed\"\nwhile true; do sleep 1; done\n";
    let Some(h) = Harness::boot("col-wake", Some(stub)) else { return };
    // The daemon is already running: its atomic .mesimon writes are runtime
    // state, not fixture source files to stage while they are being replaced.
    init_repo(&h.repo, ".gitignore", "target/\n.mesimon/\n");
    let mut c = h.client("col-wake");
    let (id, sid, branch, path) = ready(&mut c, "landed");
    git(&h.repo, &["merge", "--ff-only", &branch]);
    std::fs::create_dir(path.join("target")).unwrap();
    std::fs::write(path.join("target/build"), "disposable").unwrap();
    wait_until(Duration::from_secs(30), "stub trap armed", || h.dir.join("armed").exists());
    arrive(&mut c, id, "DONE");
    await_refusal(&mut c, id, "waiting for agents");
    assert!(path.join("landed.txt").exists());
    h.one_turn(&mut c, sid);
    // Hooks can automove out of DONE only if explicitly configured; its template stays.
    assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
    c.await_state(sid, "sleeping", |s| *s == SessionState::Sleeping);
    assert!(path.exists(), "sleep's live pane still owns the cwd until the reaper");
    wait_until(Duration::from_secs(20), "evicted after reaping", || {
        wt_of(&mut c, id).is_some_and(|w| w.status == "evicted")
    });
    assert!(!path.exists());
    assert!(branch_exists(&h.repo, &branch));
    let card = c.board().ticket(id).unwrap().clone();
    assert_eq!(card.column, "DONE");
    assert!(!card.is_archived());
    assert!(matches!(c.request(Command::WakeSession { id: sid }), Response::Provisioning));
    wait_until(Duration::from_secs(20), "wake after recut", || {
        c.board().sessions.iter().any(|s| s.id == sid && s.state.has_pane())
    });
    let wt = wt_of(&mut c, id).unwrap();
    assert_eq!(wt.status, "attached");
    assert_eq!(wt.branch, branch);
    assert_eq!(wt.path.as_deref(), path.to_str());
    assert!(path.join("landed.txt").exists());
    assert!(!path.join("target").exists());
    assert_eq!(c.board().session(sid).unwrap().cwd, path.to_str().unwrap());
}

#[test]
fn unmerged_dirty_and_untracked_work_stays_and_the_card_says_why() {
    let Some(h) = Harness::boot("col-keep", Some(STUB)) else { return };
    init_repo(&h.repo, ".gitignore", "target/\n.mesimon/\n");
    let mut c = h.client("col-keep");
    // A custom destination exercises reclamation independently of the DONE gate.
    let mut settings = c.board().column("REVIEW").unwrap().settings.clone();
    settings.free_worktree = true;
    settings.train = mesimon_core::board::TrainReach::Off;
    settings.on_merged = None;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "REVIEW".into(), settings }),
        Response::Ok
    ));
    let (id, sid, branch, path) = ready(&mut c, "keep");
    quiet(&mut c, id, sid);
    arrive(&mut c, id, "REVIEW");
    await_refusal(&mut c, id, "branch is not merged");
    assert!(path.join("keep.txt").exists());
    git(&h.repo, &["merge", "--ff-only", &branch]);
    std::fs::create_dir_all(path.join("stray/nested")).unwrap();
    std::fs::write(path.join("stray/nested/mine"), "precious").unwrap();
    await_refusal(&mut c, id, "untracked files");
    assert_eq!(std::fs::read_to_string(path.join("stray/nested/mine")).unwrap(), "precious");
    std::fs::remove_dir_all(path.join("stray")).unwrap();
    std::fs::write(path.join("keep.txt"), "uncommitted").unwrap();
    // New arrival resets the retry clock and reads the modified-file leg.
    arrive(&mut c, id, "TODO");
    arrive(&mut c, id, "REVIEW");
    await_refusal(&mut c, id, "uncommitted changes");
    assert_eq!(std::fs::read_to_string(path.join("keep.txt")).unwrap(), "uncommitted");
    assert!(branch_exists(&h.repo, &branch));
}

#[test]
fn a_live_pane_without_a_ticket_record_also_holds_the_directory() {
    let Some(h) = Harness::boot("col-pane", Some(STUB)) else { return };
    init_repo(&h.repo, ".gitignore", "target/\n.mesimon/\n");
    let mut c = h.client("col-pane");
    let (id, sid, branch, path) = ready(&mut c, "terminal");
    quiet(&mut c, id, sid);
    git(&h.repo, &["merge", "--ff-only", &branch]);
    assert!(tmux(&h.paths.tmux_sock())
        .args([
            "new-session",
            "-d",
            "-s",
            "eviction-holder",
            "-c",
            path.to_str().unwrap(),
            "sleep 60"
        ])
        .status()
        .unwrap()
        .success());
    arrive(&mut c, id, "DONE");
    await_refusal(&mut c, id, "a live pane holds");
    assert!(path.join("terminal.txt").is_file());
    assert!(tmux(&h.paths.tmux_sock())
        .args(["kill-session", "-t", "eviction-holder"])
        .status()
        .unwrap()
        .success());
    wait_until(Duration::from_secs(20), "reclaimed after the independent pane closed", || {
        wt_of(&mut c, id).is_some_and(|w| w.status == "evicted")
    });
    assert!(!path.exists() && branch_exists(&h.repo, &branch));
}
