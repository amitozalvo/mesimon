//! Daemon-restart recovery: a restart mid-turn strands our own session at
//! `Unknown{DaemonRestarted}` with no hook due until the next turn boundary
//! (dogfood 2026-08-30: "?" on the card while Claude visibly streams). The
//! transcript tail must re-derive state at Low confidence until hooks
//! re-assert. Real tmux, two in-process daemon generations over one board.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::io::Write;
use std::time::{Duration, Instant};

use mesimon_core::board::{Confidence, SessionKind, SessionState, UnknownReason};
use mesimon_core::command::{Command, Response};

/// Both tests set the same process-global env vars — serialize them.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn restart_recovers_state_from_the_transcript() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("restart-mid");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();

    // Paints forever: the pane must read alive across the restart, and the
    // recovered Running must not trip the quiet probe during the assertions.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do echo tick; sleep 0.3; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let transcript = dir.join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);

    // --- Generation 1: spawn, reach Running via hooks, die mid-turn.
    let repo1 = repo.clone();
    let daemon1 = fixture.daemon(&repo1);
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "restart".into()
        }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "restart".into(),
        workspace: None,
        tier: None,
    });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        &format!(r#"{{"session_id":"x","transcript_path":"{}"}}"#, transcript.display()),
    );
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
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon1.join().unwrap();

    // --- Generation 2: reconcile must be honest, then the tail must recover.
    let _ = std::fs::remove_file(&sock); // gen 1's socket file lingers
    let repo2 = repo.clone();
    let daemon2 = fixture.daemon(&repo2);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "gen-2 socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "restart2".into()
        }),
        Response::Hello { .. }
    ));
    assert_eq!(
        rec(&mut c).state,
        SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
        "a restart must not trust stale activity claims"
    );

    // Let the tail poller mint its at-EOF cursor, then stream one record —
    // the only evidence a mid-turn session emits between hook boundaries.
    std::thread::sleep(Duration::from_secs(3));
    let mut f = std::fs::OpenOptions::new().append(true).open(&transcript).unwrap();
    writeln!(
        f,
        r#"{{"uuid":"u1","type":"assistant","message":{{"content":[{{"type":"text","text":"still working"}}]}}}}"#
    )
    .unwrap();
    drop(f);
    let deadline = Instant::now() + Duration::from_secs(6);
    let final_rec = loop {
        let r = rec(&mut c);
        if r.state == SessionState::Running {
            break r;
        }
        assert!(
            Instant::now() < deadline,
            "transcript evidence never recovered the session (still {:?})",
            r.state
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(final_rec.confidence, Confidence::Low, "tail evidence is Tier-0");

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon2.join().unwrap();
}

/// The nudge-free case: the turn ENDED before the restart, so the transcript
/// never grows again. The mint-time backfill must read how it rested
/// (`turn_duration` trailing) and seed `Idle{EndTurn}` — no prompt required.
#[test]
fn restart_recovers_done_from_a_resting_transcript() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("restart-rest");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();

    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do echo tick; sleep 0.3; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // The finished turn, at rest on disk before the restart.
    let transcript = dir.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"all done\"}]}}\n\
         {\"uuid\":\"u2\",\"type\":\"system\",\"subtype\":\"turn_duration\"}\n\
         {\"type\":\"last-prompt\"}\n",
    )
    .unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);

    // --- Generation 1: spawn, register the transcript, die while Running so
    // reconcile has no choice but Unknown.
    let repo1 = repo.clone();
    let daemon1 = fixture.daemon(&repo1);
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "rest".into()
        }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "rest".into(),
        workspace: None,
        tier: None,
    });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        &format!(r#"{{"session_id":"x","transcript_path":"{}"}}"#, transcript.display()),
    );
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    // "die while Running" is the whole premise, and a hook frame's ingestion
    // races this connection's next request: shutting down between SessionStart
    // and UserPromptSubmit persists idle{unknown}, which is a STICKY claim that
    // survives the restart, so generation 2 would read it instead of waiting
    // for the backfill. Wait for the promotion before killing the daemon.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let st = board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .expect("session")
            .state
            .clone();
        if st == SessionState::Running {
            break;
        }
        assert!(Instant::now() < deadline, "gen-1 never reached Running (still {st:?})");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon1.join().unwrap();

    // --- Generation 2: no hook will ever fire, the transcript never grows —
    // the backfill alone must land Idle{EndTurn} at Low.
    let repo2 = repo.clone();
    let daemon2 = fixture.daemon(&repo2);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "gen-2 socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "rest2".into()
        }),
        Response::Hello { .. }
    ));
    let deadline = Instant::now() + Duration::from_secs(8);
    let final_rec = loop {
        let r = board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .expect("session")
            .clone();
        if !matches!(r.state, SessionState::Unknown { .. }) {
            break r;
        }
        assert!(
            Instant::now() < deadline,
            "backfill never recovered the resting session (still {:?})",
            r.state
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(
        final_rec.state,
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn },
        "a resting turn_duration transcript must read done, not ?"
    );
    assert_eq!(final_rec.confidence, Confidence::Low);

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon2.join().unwrap();
}

/// T-660: after a restart, a turn spent on board tools and thinking writes
/// little to the transcript and, on the mod road, fires no tool hook at all
/// (a registered tool's call has no `PreToolUse`/`PostToolUse`). T-650's
/// lead wore `Unknown` through five board calls until its reply. The call
/// itself reaches the daemon, and is the agent's own word that a turn is
/// live: one `get_ticket`, with the transcript silent, lifts the card.
#[test]
fn restart_recovers_working_from_a_board_tool_call() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("restart-call");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();
    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do echo tick; sleep 0.3; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let transcript = dir.join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();
    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);

    let daemon1 = fixture.daemon(&repo);
    let mut c = TestClient::connect(&sock);
    let _ = c.request(Command::Hello {
        version: mesimon_core::command::PROTOCOL_VERSION,
        client: "restart-call".into(),
    });
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "restart".into(),
        workspace: None,
        tier: None,
    });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        &format!(r#"{{"session_id":"x","transcript_path":"{}"}}"#, transcript.display()),
    );
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
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon1.join().unwrap();

    let _ = std::fs::remove_file(&sock);
    let daemon2 = fixture.daemon(&repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "gen-2 socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    let _ = c.request(Command::Hello {
        version: mesimon_core::command::PROTOCOL_VERSION,
        client: "restart-call2".into(),
    });
    // Past a tail poll or two: a silent transcript says nothing.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        rec(&mut c).state,
        SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
        "nothing but the call may lift it"
    );
    let _ = c.send(mesimon_core::Principal::Agent { session: sid }, Command::AgentGetTicket);
    assert_eq!(rec(&mut c).state, SessionState::Running, "the call is the turn speaking");

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon2.join().unwrap();
}
