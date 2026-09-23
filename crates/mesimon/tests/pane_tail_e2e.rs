//! The ticket page's preview zone, end to end: a shell keeps no transcript,
//! so `Command::PaneTail` is the only way the board can show what it has been
//! doing. Real tmux, in-process daemon, a real shell in a real pane — the
//! test types a command into it and asks the daemon what the pane says.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};

#[test]
fn the_terminal_zone_reads_the_shell_pane() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("tail");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let tmux_sock = paths.tmux_sock();

    // The pane runs `$SHELL` (D29: allowlisted, never inherited wholesale).
    // Pin it so the prompt and the echo are the same on every machine.
    fixture.set_env("SHELL", "/bin/sh");
    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));

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
            client: "tail".into()
        }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "tail".into(),
        workspace: None,
        tier: None,
    });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;

    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Bash,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sid16 = sid.simple().to_string()[..16].to_string();

    // Type a command into the pane the way a person would. The tty echoes it,
    // so the capture holds the command AND what it printed — which is exactly
    // what the zone is for.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let sent = tmux(&tmux_sock)
            .args(["send-keys", "-t", &sid16, "echo mesimon-probe-42", "Enter"])
            .output();
        if sent.map(|o| o.status.success()).unwrap_or(false) {
            break;
        }
        assert!(Instant::now() < deadline, "pane never accepted keys");
        std::thread::sleep(Duration::from_millis(200));
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let lines = loop {
        let lines = match c.request(Command::PaneTail { session: sid, lines: 20 }) {
            Response::PaneTail { lines } => lines,
            other => panic!("pane tail failed: {other:?}"),
        };
        // The result line: the echo's output, alone on its row. Waiting for
        // that (not for the command) is what makes the assertion below about
        // a command that RAN, not one that was merely typed.
        if lines.iter().any(|l| l.trim() == "mesimon-probe-42") {
            break lines;
        }
        assert!(Instant::now() < deadline, "the shell never echoed: {lines:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(
        lines.iter().any(|l| l.contains("echo mesimon-probe-42")),
        "the command itself must be in the tail: {lines:?}"
    );
    // Oldest first: the command is typed before its output is printed, and
    // the zone draws the vector in order.
    let cmd = lines.iter().position(|l| l.contains("echo mesimon-probe-42")).unwrap();
    let out = lines.iter().position(|l| l.trim() == "mesimon-probe-42").unwrap();
    assert!(cmd < out, "the tail must read top-down, oldest first: {lines:?}");

    // A session with no pane has nothing to show, and says so rather than
    // answering with an empty screen.
    let _ = c.request(Command::KillSession { id: sid });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match c.request(Command::PaneTail { session: sid, lines: 20 }) {
            Response::Err { message } => {
                assert!(message.contains("no pane"), "wrong refusal: {message}");
                break;
            }
            other => {
                assert!(Instant::now() < deadline, "a dead session kept answering: {other:?}");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    assert!(matches!(
        c.request(Command::PaneTail { session: uuid::Uuid::nil(), lines: 20 }),
        Response::Err { .. }
    ));

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}
