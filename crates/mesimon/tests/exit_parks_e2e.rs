//! Leaving a Claude session parks it (2026-09-01). Ctrl+C-out, `/exit` and
//! Ctrl+D end the process, never the conversation — so the record lands in
//! `Sleeping`, exactly where `x` had put it, and `x` brings it back. Real
//! tmux, real pane-died, in-process daemon, stub agents.
//!
//! The three gates are the test: a Claude session with a resumable
//! conversation parks, one without it stays a corpse (parking it would mint
//! a sleeper that can never wake), and a shell is never parked at all — its
//! pane IS its record and there is nothing to resume.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{ExitReason, SessionKind, SessionState, WorkspaceStrategy};
use mesimon_core::command::{Command, Response};

#[test]
fn leaving_claude_parks_the_session() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("exitpark");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    let projects = dir.join("claude-home").join("projects").join("msmn");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&projects).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();

    // The user's own exit, as tmux sees it: the process leaves with status 0
    // and `pane-died` is the frame that reaches the daemon. Nothing here
    // sends a SessionEnd — this is deliberately the road a Ctrl+C takes when
    // the hook loses the race, and it must park on its own.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\nsleep 1\nexit 0\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
    fixture.set_env("MESIMON_CLAUDE_HOME", dir.join("claude-home"));

    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "exitpark".into()
        }),
        Response::Hello { .. }
    ));

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "left it".into(),
        workspace: None,
    });
    // A ticket holds one claude (2026-09-02), so the case with no conversation
    // needs a ticket of its own.
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "lost it".into(),
        workspace: None,
    });
    let find = |c: &mut TestClient, title: &str| {
        c.board().tickets.iter().find(|t| t.title == title).expect("ticket").id
    };
    let ticket = find(&mut c, "left it");
    let other = find(&mut c, "lost it");

    let spawn = |c: &mut TestClient, ticket, kind| match c.request(Command::SpawnSession {
        ticket,
        kind,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };

    // ---- 1. a conversation to come back to → the exit is a park ----------
    let kept = spawn(&mut c, ticket, SessionKind::Claude);
    // Claude's own store is what `--resume` reads, so that file existing is
    // the whole difference between a session that can wake and one that
    // cannot. Write one for this session and not for the next.
    std::fs::write(
        projects.join(format!("{kept}.jsonl")),
        "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n",
    )
    .unwrap();

    // ---- 2. no conversation → the exit is an exit ------------------------
    let lost = spawn(&mut c, other, SessionKind::Claude);

    // ---- 3. a shell is its pane; there is nothing to resume --------------
    let shell = spawn(&mut c, ticket, SessionKind::Bash);

    let parked = c.await_state(kept, "sleeping", |s| !matches!(s, SessionState::Spawning));
    assert_eq!(parked, SessionState::Sleeping, "a resumable claude exit parks");

    let corpse = c.await_state(lost, "exited", |s| {
        matches!(s, SessionState::Exited { .. } | SessionState::Sleeping)
    });
    assert_eq!(
        corpse,
        SessionState::Exited { reason: ExitReason::UserQuit },
        "no transcript means no wake — parking it would strand the record"
    );

    // ...and it is not a dead end. `resume` on a record with no conversation
    // used to refuse forever ("no transcript to resume"), which left the row
    // offering `enter resume` and nothing behind it. Starting fresh loses
    // nothing, because there was nothing.
    match c.request(Command::ResumeSession { id: lost, confirm: false }) {
        Response::Spawned { fresh, .. } => assert!(fresh, "a resume with no transcript is fresh"),
        other => panic!("resume refused: {other:?}"),
    }
    let hosting = c
        .board()
        .sessions
        .iter()
        .find(|s| s.id == lost)
        .and_then(|s| s.claude_session_id)
        .expect("the record points at the conversation it now hosts");
    assert_ne!(hosting, lost, "a newly minted id, never the one that had no transcript");

    // The shell never travels this road, so drive it by hand: the same
    // clean-exit signal, and the kind gate is the only thing refusing it.
    hook_send_with(&hook_sock, &shell.to_string(), "SessionEnd", Some("prompt_input_exit"), "{}");
    let dead_shell = c.await_state(shell, "exited", |s| !matches!(s, SessionState::Running));
    assert_eq!(
        dead_shell,
        SessionState::Exited { reason: ExitReason::UserQuit },
        "a shell's pane is its record; a parked one would wake into a new shell"
    );

    // ---- a parked record does not lock the workspace (T-309) -------------
    //
    // The lock is what the choice would RELOCATE: a worktree, and an agent
    // standing in a directory the field names. `ticket` now holds two records
    // with no pane between them — a sleeping claude and a dead shell — and
    // neither is a checkout, so the choice is open again. It was any record
    // at all until 2026-09-07, which on a real board is a lock nothing can
    // open: a ticket that talked to an agent once could never be moved to a
    // worktree.
    assert!(
        matches!(
            c.request(Command::SetWorkspace {
                id: ticket,
                workspace: Some(WorkspaceStrategy::Worktree)
            }),
            Response::Ok
        ),
        "a parked claude and a dead shell lock nothing"
    );
    assert_eq!(
        c.board().ticket(ticket).and_then(|t| t.workspace),
        Some(WorkspaceStrategy::Worktree)
    );
    // Put it back before the wake: the point is the gate, not a worktree.
    assert!(matches!(
        c.request(Command::SetWorkspace { id: ticket, workspace: None }),
        Response::Ok
    ));
    // …and the record that was just resumed has a pane, so it still locks.
    assert!(
        matches!(
            c.request(Command::SetWorkspace {
                id: other,
                workspace: Some(WorkspaceStrategy::Worktree)
            }),
            Response::Err { .. }
        ),
        "a running agent's directory is where it is"
    );

    // ---- and `x` brings the parked one back ------------------------------
    match c.request(Command::WakeSession { id: kept }) {
        Response::Spawned { .. } => {}
        other => panic!("wake refused: {other:?}"),
    }
    let woken = c.board().sessions.iter().find(|s| s.id == kept).unwrap().state.clone();
    assert_eq!(woken, SessionState::Spawning, "wake resumes the conversation it parked");

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
}
