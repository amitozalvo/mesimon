//! The header sleep offer's action (Z, 2026-08-30 rescope): sleeps exactly
//! the sleep-safe set — idle sessions on DONE tickets — and never touches a
//! ticket still in play. Real tmux, in-process daemon, stub agents.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

#[test]
fn z_sleeps_only_the_done_column() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("offer");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();

    let stub = dir.join("claude-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
    fixture.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");

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
            client: "offer".into()
        }),
        Response::Hello { .. }
    ));

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "hot".into(),
        workspace: None,
    });
    let _ = c.request(Command::CreateTicket {
        column: "DONE".into(),
        title: "cold".into(),
        workspace: None,
    });
    let board = board_of(c.request(Command::Snapshot));
    let hot = board.tickets.iter().find(|t| t.column == "TODO").unwrap().id;
    let cold = board.tickets.iter().find(|t| t.column == "DONE").unwrap().id;

    // One idle claude session on each ticket, driven idle by real hooks.
    let mut sids = Vec::new();
    for (ticket, name) in [(hot, "hot"), (cold, "cold")] {
        let sid = match c.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Claude,
            submit_prompt: false,
        }) {
            Response::Spawned { id, .. } => id,
            other => panic!("spawn failed: {other:?}"),
        };
        // A transcript with one user + one assistant record keeps the sleep
        // path's B-A22 cheap check quiet.
        let transcript = dir.join(format!("{name}.jsonl"));
        std::fs::write(
            &transcript,
            "{\"uuid\":\"u0\",\"type\":\"user\",\"message\":{}}\n\
             {\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        )
        .unwrap();
        hook_send(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            &format!(r#"{{"session_id":"x","transcript_path":"{}"}}"#, transcript.display()),
        );
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
        hook_send(
            &hook_sock,
            &sid.to_string(),
            "Stop",
            r#"{"stop_hook_active":false,"background_tasks":[]}"#,
        );
        sids.push((sid, ticket));
    }
    // Stop leaves settle (1.5 s) before Idle commits.
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let board = board_of(c.request(Command::Snapshot));
        let idle =
            board.sessions.iter().filter(|s| matches!(s.state, SessionState::Idle { .. })).count();
        if idle == 2 {
            break;
        }
        assert!(Instant::now() < deadline, "sessions never settled idle");
        std::thread::sleep(Duration::from_millis(200));
    }

    // The offer's action: exactly the DONE session sleeps.
    match c.request(Command::ReclaimAll) {
        Response::Reclaimed { slept, skipped } => {
            assert_eq!((slept, skipped), (1, 0), "Z must take only the done column");
        }
        other => panic!("reclaim failed: {other:?}"),
    }
    let board = board_of(c.request(Command::Snapshot));
    for (sid, ticket) in sids {
        let rec = board.sessions.iter().find(|s| s.id == sid).expect("session");
        if ticket == cold {
            assert_eq!(rec.state, SessionState::Sleeping, "done session must sleep");
        } else {
            assert!(
                matches!(rec.state, SessionState::Idle { .. }),
                "in-play ticket's session must stay awake, got {:?}",
                rec.state
            );
        }
    }

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}
