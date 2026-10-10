//! A `!` command in Claude's composer (T-707), end to end: bash mode fires no
//! hook, keeps `#{pane_current_command}` at the agent's own name and writes
//! the transcript only when the command ends, so the daemon reads the command
//! off the pane's child processes. An idle Claude running one shows the
//! command as its foreground on the snapshot (the card spins on it); a shell
//! as old as the session — an MCP server's `sh -c`, a background task — is
//! not a command the person typed; and the foreground is never persisted.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

/// The stub agent: it spawns one shell child at launch, as Claude spawns its
/// MCP servers, marks `ready` beside itself (the test sends no hook before
/// that shell exists, or under load it is born after the idle spell began),
/// and then watches for two files beside itself — `go` spawns
/// a second shell child running `sleep`, the way bash mode runs a command
/// (a shell, then the command under it; `; true` keeps `sh -c` from exec'ing
/// the command in the shell's place), and `stop` ends that shell. The
/// sleeps are short so an orphaned one outlives no fixture by much.
const STUB: &str = "#!/bin/sh\n\
    d=$(dirname \"$0\")\n\
    sh -c 'sleep 120; true' &\n\
    : > \"$d/ready\"\n\
    pid=\n\
    trap 'exit 0' TERM\n\
    while true; do\n\
      if [ -e \"$d/go\" ] && [ -z \"$pid\" ]; then sh -c 'sleep 60; true' & pid=$!; fi\n\
      if [ -e \"$d/stop\" ] && [ -n \"$pid\" ]; then rm -f \"$d/go\"; kill $pid; pid=; fi\n\
      sleep 0.2\n\
    done\n";

#[test]
fn a_bash_mode_command_under_an_idle_claude_is_its_foreground() {
    let Some(h) = Harness::boot("bashmode", Some(STUB)) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("bashmode");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "run it".into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.first().expect("ticket").id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sid_s = sid.to_string();
    let foreground = |c: &mut TestClient| {
        board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .expect("record")
            .foreground
            .clone()
    };

    let stub_dir = h.stub.as_ref().unwrap().parent().unwrap().to_path_buf();
    wait_until(Duration::from_secs(30), "the stub to spawn its launch-time shell", || {
        stub_dir.join("ready").exists()
    });

    // A turn runs and ends: the seat is idle, with the launch-time shell
    // child (the "MCP server") older than the idle spell.
    hook_send(&hook_sock, &sid_s, "SessionStart", r#"{"source":"startup"}"#);
    hook_send(&hook_sock, &sid_s, "UserPromptSubmit", r#"{"session_id":"x"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &sid_s, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));

    // Older than the spell is not a command: three samples (the pane facts
    // refresh every 2 s) and no foreground. The wait also ages the spell
    // past the second the next shell is born in.
    std::thread::sleep(Duration::from_secs(6));
    assert_eq!(foreground(&mut c), None, "a shell as old as the session is not a `!` command");
    assert_eq!(
        c.board().sessions.iter().find(|s| s.id == sid).unwrap().state,
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn }
    );

    // The person types `! sleep 600`: a shell younger than the spell, with
    // the command under it — the foreground names the command, on the
    // snapshot, while the state stays idle (no hook fired, no turn began).
    std::fs::write(stub_dir.join("go"), "").unwrap();
    wait_until(Duration::from_secs(10), "the `!` command to be read as the foreground", || {
        foreground(&mut c).as_deref() == Some("sleep")
    });
    let rec =
        board_of(c.request(Command::Snapshot)).sessions.into_iter().find(|s| s.id == sid).unwrap();
    assert!(matches!(rec.state, SessionState::Idle { .. }), "{:?}", rec.state);
    // The ticket's agent reads `working` to an agent asking about it.
    let agent = match c.request(Command::Snapshot) {
        Response::Board { board, .. } => board.sessions.into_iter().find(|s| s.id == sid).unwrap(),
        other => panic!("snapshot: {other:?}"),
    };
    assert_eq!(agent.state_word(), "working");

    // Never on disk: a foreground is a fact about a live pane.
    let persisted = std::fs::read_to_string(h.paths.sessions_file()).unwrap();
    assert!(!persisted.contains("foreground"), "{persisted}");

    // The command ends and the foreground clears; the seat is the idle it was.
    std::fs::write(stub_dir.join("stop"), "").unwrap();
    wait_until(Duration::from_secs(10), "the foreground to clear", || foreground(&mut c).is_none());
    assert!(matches!(
        c.board().sessions.iter().find(|s| s.id == sid).unwrap().state,
        SessionState::Idle { .. }
    ));
}
