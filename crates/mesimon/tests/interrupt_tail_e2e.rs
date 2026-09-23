//! The Esc-interrupt catch via the transcript (dogfood 2026-08-30): current
//! Claude Code keeps painting the pane for a minute after an interrupt, so the
//! pane-quiet probe reads "working" long past the Esc — but it DOES write a
//! `user` record saying `[Request interrupted by user…]` at the keypress. A
//! Running session of ours must demote off that record even while the pane
//! never goes quiet. The record here is the tool-use spelling WITHOUT the
//! `interruptedMessageId` flag — the shape Claude Code 2.1.25x writes about
//! half the time and the one that left a card on "working" (2026-09-04); the
//! flagged form is the unit test's. Real tmux, in-process daemon, a stub
//! agent that paints forever.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::io::Write;
use std::time::{Duration, Instant};

use mesimon_core::board::{Confidence, SessionKind, SessionState, StopReason};
use mesimon_core::command::{Command, Response};

#[test]
fn interrupt_record_demotes_running_while_pane_still_paints() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("intrtail");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();

    // Post-interrupt Claude Code: the pane paints forever. Quiet never trips.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do echo tick; sleep 0.3; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
    // Prove the demote comes from the transcript, not byte silence: park the
    // pane-quiet threshold far beyond the test's horizon.
    fixture.set_env("MESIMON_PANE_QUIET_MS", "600000");

    let transcript = dir.join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();

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
            client: "intrtail".into()
        }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "intrtail".into(),
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
    let start_body = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"{}","source":"startup"}}"#,
        transcript.display(),
        repo.display()
    );
    hook_send(&hook_sock, &sid.to_string(), "SessionStart", &start_body);
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    let rec = |c: &mut TestClient| {
        board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .expect("session")
            .clone()
    };
    // A hook frame is fire-and-forget on its own one-shot socket, so its
    // ingestion races this Snapshot on the client connection. Locally the frame
    // always won; a loaded CI runner snapshotted between SessionStart and
    // UserPromptSubmit and read idle{unknown}. Wait for the promotion instead of
    // assuming it has landed — holding Running is what this test is about, and
    // that is asserted below.
    let deadline = Instant::now() + Duration::from_secs(5);
    while rec(&mut c).state != SessionState::Running {
        assert!(Instant::now() < deadline, "UserPromptSubmit never promoted to Running");
        std::thread::sleep(Duration::from_millis(50));
    }

    // Let the tail cursor mint (2 s poll cadence) — a cursor starts at the
    // file's end, so the abort record must land after it exists.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(rec(&mut c).state, SessionState::Running, "painting pane must hold Running");

    // The Esc: no hook, the pane keeps painting, only the transcript speaks.
    let abort = r#"{"uuid":"u1","type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#;
    use std::fs::OpenOptions;
    let mut f = OpenOptions::new().append(true).open(&transcript).unwrap();
    writeln!(f, "{abort}").unwrap();
    drop(f);

    // Poll cadence (2 s) + settle (1.5 s) + slack.
    let deadline = Instant::now() + Duration::from_secs(12);
    let final_rec = loop {
        let r = rec(&mut c);
        if r.state != SessionState::Running {
            break r;
        }
        assert!(Instant::now() < deadline, "never left Running after the interrupt record");
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(
        final_rec.state,
        SessionState::Idle { stop_reason: StopReason::Interrupted },
        "the transcript's interrupt record must demote a still-painting pane"
    );
    assert_eq!(final_rec.confidence, Confidence::Low, "tier-0 evidence stays Low");

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}
