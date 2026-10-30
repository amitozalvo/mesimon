//! Column reclamation keeps the card, branch and exact wake workspace.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use mesimon_core::board::{SessionKind, SessionState, WorkspaceStrategy};
use mesimon_core::command::{Command, Response};
use std::path::{Path, PathBuf};
use std::time::Duration;

const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";

fn branch_exists(repo: &Path, branch: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])
        .output()
        .unwrap()
        .status
        .success()
}

/// A worktree ticket with a spawned (stub) claude and one commit on its branch.
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

/// Kill the ticket's claude and wait for the reaper: the archive gate wants
/// nothing on the ticket holding a pane.
fn quiet(c: &mut TestClient, id: ulid::Ulid, sid: uuid::Uuid) {
    let _ = c.request(Command::KillSession { id: sid });
    wait_until(Duration::from_secs(15), "the ticket to go quiet", || {
        c.board().sessions.iter().filter(|s| s.ticket == id).all(|s| !s.state.has_pane())
    });
}

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
    init_repo(&h.repo, ".gitignore", "target/\n");
    let mut c = h.client("col-wake");
    let (id, sid, branch, path) = ready(&mut c, "landed");
    git(&h.repo, &["merge", "--ff-only", &branch]);
    std::fs::create_dir(path.join("target")).unwrap();
    std::fs::write(path.join("target/build"), "disposable").unwrap();
    wait_until(Duration::from_secs(30), "stub trap armed", || h.dir.join("armed").exists());
    arrive(&mut c, id, "DONE");
    await_refusal(&mut c, id, "waiting for agents");
    assert!(path.join("landed.txt").exists());
    hook_send(&h.paths.hook_sock(), &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    hook_send(&h.paths.hook_sock(), &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
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
    init_repo(&h.repo, "base", "base\n");
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
    init_repo(&h.repo, "base", "base\n");
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
