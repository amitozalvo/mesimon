//! The Esc-interrupt catch, end to end (spike S-E): a user interrupt fires no
//! hook and may write nothing to the transcript, so the pane-activity probe is
//! the only thing standing between the board and "working" forever. Real tmux,
//! in-process daemon, a stub agent that paints for a while and then goes
//! silent — exactly the byte signature of a turn that was interrupted.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{Confidence, SessionKind, SessionState, StopReason};
use mesimon_core::command::{Command, Response};

#[test]
fn interrupted_turn_demotes_to_idle_without_any_hook() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("intr");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();

    // "Working, then interrupted": paint sub-second for ~6 s, then silence.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ntrap 'exit 0' TERM\ni=0\nwhile [ $i -lt 20 ]; do echo tick; i=$((i+1)); sleep 0.3; done\nwhile true; do sleep 1; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
    // Real threshold is 8 s; the e2e can't spend that per verdict.
    fixture.set_env("MESIMON_PANE_QUIET_MS", "1500");

    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "intr".into() }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "intr".into() });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;

    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    // The hooks a real session would fire: started, then a submitted prompt.
    hook_send(&hook_sock, &sid.to_string(), "SessionStart", r#"{"session_id":"x"}"#);
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    let rec = |c: &mut TestClient| {
        board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .expect("session")
            .clone()
    };
    assert_eq!(rec(&mut c).state, SessionState::Running);

    // While the pane paints, quiet never trips: still Running 3 s in.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(rec(&mut c).state, SessionState::Running, "painting pane must hold Running");

    // The paint stops (the "Esc"); no hook will ever fire. The probe must
    // demote within quiet (1.5 s seam) + settle (1.5 s) + cadence slack.
    let deadline = Instant::now() + Duration::from_secs(12);
    let final_rec = loop {
        let r = rec(&mut c);
        if r.state != SessionState::Running {
            break r;
        }
        assert!(Instant::now() < deadline, "never left Running after the pane went quiet");
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(
        final_rec.state,
        SessionState::Idle { stop_reason: StopReason::Interrupted },
        "quiet Running pane must read idle, not working"
    );
    assert_eq!(final_rec.confidence, Confidence::Medium, "byte silence is inference");

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}
