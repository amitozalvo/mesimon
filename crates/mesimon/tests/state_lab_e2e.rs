//! Transport order and continued-turn behavior through a real daemon/socket.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use mesimon_core::board::{SessionKind, SessionState, StopReason};
use mesimon_core::command::{Command, Response};
use std::io::Write;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::Duration;

const STUB: &str = "#!/bin/sh\nexec sleep 120\n";

fn spawn(client: &mut TestClient, title: &str) -> uuid::Uuid {
    client.request(Command::CreateTicket {
        column: "IN PROGRESS".into(),
        title: title.into(),
        workspace: None,
    });
    let ticket = client.board().tickets.iter().find(|t| t.title == title).unwrap().id;
    match client.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    }
}

#[test]
fn later_hook_connection_cannot_overtake_a_partial_earlier_frame() {
    let Some(h) = Harness::boot_with_env("state-order", Some(STUB), &[]) else { return };
    let mut client = h.client("state-order");
    let sid = spawn(&mut client, "Ordered hooks");
    let mut first = UnixStream::connect(h.paths.hook_sock()).unwrap();
    write!(
        first,
        "{}\n{{\"source\":\"startup\"}}",
        serde_json::json!({"session": sid, "event": "SessionStart"})
    )
    .unwrap();
    // Keep the first reader awaiting EOF. The second reader finishes first.
    hook_send(&h.paths.hook_sock(), &sid.to_string(), "UserPromptSubmit", "{}");
    std::thread::sleep(Duration::from_millis(200));
    first.shutdown(Shutdown::Write).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let board = client.board();
    assert_eq!(
        board.sessions.iter().find(|s| s.id == sid).unwrap().state,
        SessionState::Running,
        "feed: {}",
        std::fs::read_to_string(h.paths.activity_log()).unwrap_or_default()
    );
    // A late SessionStart schedules idle after 1.5s in the old implementation.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        client.board().sessions.iter().find(|s| s.id == sid).unwrap().state,
        SessionState::Running
    );
}

#[test]
fn timed_out_hook_releases_following_events_and_continued_stop_moves() {
    let Some(h) = Harness::boot_with_env("state-timeout", Some(STUB), &[]) else { return };
    let mut client = h.client("state-timeout");
    let sid = spawn(&mut client, "Continued turn");
    let _stalled = UnixStream::connect(h.paths.hook_sock()).unwrap();
    hook_send(&h.paths.hook_sock(), &sid.to_string(), "UserPromptSubmit", "{}");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        client.board().sessions.iter().find(|s| s.id == sid).unwrap().state,
        SessionState::Running,
        "feed: {}",
        std::fs::read_to_string(h.paths.activity_log()).unwrap_or_default()
    );
    hook_send(
        &h.paths.hook_sock(),
        &sid.to_string(),
        "Stop",
        r#"{"stop_hook_active":true,"background_tasks":[]}"#,
    );
    wait_until(Duration::from_secs(5), "continued turn reached REVIEW", || {
        let board = client.board();
        let session = board.sessions.iter().find(|s| s.id == sid).unwrap();
        session.state == SessionState::Idle { stop_reason: StopReason::EndTurn }
            && board.ticket(session.ticket).unwrap().column == "REVIEW"
    });
    wait_until(Duration::from_secs(3), "decision diagnostics flushed", || {
        std::fs::read_to_string(h.paths.activity_log()).is_ok_and(|s| {
            s.contains("state_decision")
                && s.contains("movement_decision")
                && s.contains("settling")
        })
    });
}
