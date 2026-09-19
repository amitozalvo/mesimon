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

#[test]
fn explicit_abort_clears_a_held_permission_without_completion_movement() {
    let Some(h) =
        Harness::boot_with_env("state-cancel", Some(STUB), &[("MESIMON_PANE_QUIET_MS", "600000")])
    else {
        return;
    };
    let mut client = h.client("state-cancel");
    let sid = spawn(&mut client, "Cancelled permission");
    let transcript = h.dir.join("cancel.jsonl");
    std::fs::write(&transcript, "").unwrap();
    hook_send(
        &h.paths.hook_sock(),
        &sid.to_string(),
        "SessionStart",
        &serde_json::json!({
            "transcript_path": transcript, "source": "startup"
        })
        .to_string(),
    );
    hook_send(&h.paths.hook_sock(), &sid.to_string(), "UserPromptSubmit", "{}");
    hook_send(
        &h.paths.hook_sock(),
        &sid.to_string(),
        "PermissionRequest",
        r#"{"tool_name":"Bash"}"#,
    );
    wait_until(Duration::from_secs(5), "permission visible", || {
        matches!(
            client.board().sessions.iter().find(|s| s.id == sid).unwrap().state,
            SessionState::RequiresAction { .. }
        )
    });
    std::thread::sleep(Duration::from_secs(3));
    // Capture-derived 2.1.266 spelling, intentionally without interruptedMessageId.
    std::fs::write(&transcript, concat!(r#"{"uuid":"abort","type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#, "\n")).unwrap();
    wait_until(Duration::from_secs(8), "permission cancelled", || {
        let board = client.board();
        let session = board.sessions.iter().find(|s| s.id == sid).unwrap();
        assert_ne!(board.ticket(session.ticket).unwrap().column, "REVIEW");
        session.state == SessionState::Idle { stop_reason: StopReason::Interrupted }
    });
}

#[test]
fn waiting_then_busy_clears_permission_before_the_tool_finishes() {
    let Some(h) = Harness::boot_with_env(
        "state-approved",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let mut client = h.client("state-approved");
    let sid = spawn(&mut client, "Approved long tool");
    let home = h.dir.join("claude-home/sessions");
    std::fs::create_dir_all(&home).unwrap();
    let file = home.join(format!("{}.json", std::process::id()));
    let write_status = |status: &str, stamp: u64| {
        std::fs::write(
            &file,
            serde_json::json!({"pid":std::process::id(),"sessionId":sid,
            "status":status,"statusUpdatedAt":stamp})
            .to_string(),
        )
        .unwrap();
    };
    write_status("waiting", 0);
    hook_send(&h.paths.hook_sock(), &sid.to_string(), "UserPromptSubmit", "{}");
    hook_send(
        &h.paths.hook_sock(),
        &sid.to_string(),
        "PermissionRequest",
        r#"{"tool_name":"Bash"}"#,
    );
    wait_until(Duration::from_secs(5), "permission visible", || {
        matches!(
            client.board().sessions.iter().find(|s| s.id == sid).unwrap().state,
            SessionState::RequiresAction { .. }
        )
    });
    let since =
        client.board().sessions.iter().find(|s| s.id == sid).unwrap().state_changed_at.unwrap();
    write_status("waiting", since);
    std::thread::sleep(Duration::from_secs(3));
    assert!(matches!(
        client.board().sessions.iter().find(|s| s.id == sid).unwrap().state,
        SessionState::RequiresAction { .. }
    ));
    write_status("busy", since + 3000);
    wait_until(Duration::from_secs(8), "approved tool is running before completion", || {
        let board = client.board();
        let session = board.sessions.iter().find(|s| s.id == sid).unwrap();
        assert_ne!(board.ticket(session.ticket).unwrap().column, "REVIEW");
        session.state == SessionState::Running
    });
}

#[test]
fn background_liveness_reclassifies_and_restart_drops_the_registry() {
    if !require_tmux() {
        return;
    }
    let fixture = TestFixture::new("monitor-identity");
    let repo = fixture.dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let stub = fixture.dir.join("claude-stub.sh");
    std::fs::write(&stub, STUB).unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    let paths = fixture.paths(&repo);
    let daemon = fixture.daemon(&repo);
    let mut client = TestClient::connect(&paths.orch_sock());
    client.request(Command::Hello {
        version: mesimon_core::command::PROTOCOL_VERSION,
        client: "monitor".into(),
    });
    let sid = spawn(&mut client, "Monitor and build");
    let send = |event, body| hook_send(&paths.hook_sock(), &sid.to_string(), event, body);
    send("SessionStart", r#"{"source":"startup"}"#);
    send("UserPromptSubmit", "{}");
    send("PostToolUse", r#"{"tool_name":"Monitor","tool_response":{"taskId":"watch"}}"#);
    send(
        "Stop",
        r#"{"background_tasks":[{"id":"watch","type":"shell"},{"id":"build","type":"subagent"}]}"#,
    );
    wait_until(Duration::from_secs(5), "ordinary build still blocks completion", || {
        let board = client.board();
        let rec = board.sessions.iter().find(|s| s.id == sid).unwrap();
        rec.state == SessionState::Idle { stop_reason: StopReason::Background }
    });
    send("Stop", r#"{"background_tasks":[{"id":"watch","type":"shell"}]}"#);
    wait_until(Duration::from_secs(5), "watch-only work releases the working gate", || {
        let board = client.board();
        let rec = board.sessions.iter().find(|s| s.id == sid).unwrap();
        rec.state == SessionState::Idle { stop_reason: StopReason::Monitoring }
            && !mesimon_core::quiet::is_working(rec)
    });
    send(
        "PostToolUse",
        r#"{"agent_id":"parent","tool_name":"Agent","tool_response":{"agentId":"orphan","status":"async_launched"}}"#,
    );
    wait_until(Duration::from_secs(5), "nested agent makes parked work active", || {
        client.board().sessions.iter().find(|s| s.id == sid).unwrap().state
            == SessionState::Idle { stop_reason: StopReason::Background }
    });
    assert!(matches!(client.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
    drop(client);
    let _ = std::fs::remove_file(paths.orch_sock());
    let daemon = fixture.daemon(&repo);
    let mut client = TestClient::connect(&paths.orch_sock());
    client.request(Command::Hello {
        version: mesimon_core::command::PROTOCOL_VERSION,
        client: "monitor-restarted".into(),
    });
    send("UserPromptSubmit", "{}");
    send("Stop", r#"{"background_tasks":[{"id":"watch","type":"shell"}]}"#);
    wait_until(Duration::from_secs(5), "fresh Stop reclassifies shell after restart", || {
        client.board().sessions.iter().find(|s| s.id == sid).unwrap().state
            == SessionState::Idle { stop_reason: StopReason::Monitoring }
    });
    // An ambient artifact watch alone is a finished turn (T-408): Claude
    // Code lists it as a running `monitor` no tool armed.
    send("UserPromptSubmit", "{}");
    send(
        "Stop",
        r#"{"background_tasks":[{"id":"sart","type":"monitor","status":"running","description":"live updates for artifact plan (comments)"}]}"#,
    );
    wait_until(Duration::from_secs(5), "an unarmed monitor is not a park", || {
        client.board().sessions.iter().find(|s| s.id == sid).unwrap().state
            == SessionState::Idle { stop_reason: StopReason::EndTurn }
    });
    assert!(matches!(client.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}
