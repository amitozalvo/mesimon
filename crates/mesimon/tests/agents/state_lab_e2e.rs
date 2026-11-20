//! Transport order and continued-turn behavior through a real daemon/socket.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use crate::common;
use common::*;
use mesimon_core::board::{SessionKind, SessionState, StopReason};
use mesimon_core::command::{Command, Response};
use std::io::Write;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::Duration;

const STUB: &str = "#!/bin/sh\nexec sleep 120\n";

fn spawn(client: &mut TestClient, title: &str) -> uuid::Uuid {
    client.open_to_new("IN PROGRESS");
    client.request(Command::CreateTicket {
        column: "IN PROGRESS".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    });
    let ticket = client.board().tickets.iter().find(|t| t.title == title).unwrap().id;
    match client.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
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
    // The frame below is written by hand, so on the mod road (T-574) its
    // twin is too: the mod relays the event the moment it fires.
    if test_road() == "mod" {
        let body = r#"{"source":"startup"}"#;
        hook_send_road(
            &h.paths.hook_sock(),
            &sid.to_string(),
            "SessionStart",
            None,
            None,
            body,
            Some("mod"),
        );
    }
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
    assert_eq!(client.record(sid).state, SessionState::Running);
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
        client.record(sid).state,
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
        matches!(client.record(sid).state, SessionState::RequiresAction { .. })
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
        matches!(client.record(sid).state, SessionState::RequiresAction { .. })
    });
    let since = client.record(sid).state_changed_at.unwrap();
    write_status("waiting", since);
    std::thread::sleep(Duration::from_secs(3));
    assert!(matches!(client.record(sid).state, SessionState::RequiresAction { .. }));
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
        client.record(sid).state == SessionState::Idle { stop_reason: StopReason::Background }
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
        client.record(sid).state == SessionState::Idle { stop_reason: StopReason::Monitoring }
    });
    // An ambient artifact watch alone is a finished turn (T-408): Claude
    // Code lists it as a running `monitor` no tool armed.
    send("UserPromptSubmit", "{}");
    send(
        "Stop",
        r#"{"background_tasks":[{"id":"sart","type":"monitor","status":"running","description":"live updates for artifact plan (comments)"}]}"#,
    );
    wait_until(Duration::from_secs(5), "an unarmed monitor is not a park", || {
        client.record(sid).state == SessionState::Idle { stop_reason: StopReason::EndTurn }
    });
    assert!(matches!(client.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}

/// T-483, the dogfood sequence of 2026-09-28: the lead backgrounds a command
/// and ends its turn, and an internal agent's `SubagentStop` lists the
/// session's tasks four seconds later. The lead parks as working — the
/// keep-awake hold's own predicate says so — and the nested stop leaves it
/// parked. The turn the command's completion starts ends the park.
#[test]
fn a_background_command_parks_the_lead_through_a_nested_stop() {
    let Some(h) = Harness::boot_with_env("bg-command", Some(STUB), &[]) else { return };
    let mut client = h.client("bg-command");
    let sid = spawn(&mut client, "Release in the background");
    let send = |event, body: &str| hook_send(&h.paths.hook_sock(), &sid.to_string(), event, body);
    let mut record = || client.record(sid);
    send("SessionStart", r#"{"source":"startup"}"#);
    send("UserPromptSubmit", "{}");
    send(
        "PostToolUse",
        r#"{"tool_name":"Bash","tool_response":{"stdout":"","stderr":"","backgroundTaskId":"b8km0xm1p"}}"#,
    );
    let row = r#"{"id":"b8km0xm1p","type":"shell","status":"running","description":"release"}"#;
    send("Stop", &format!(r#"{{"background_tasks":[{row}]}}"#));
    wait_until(Duration::from_secs(5), "a background command parks the lead as working", || {
        let rec = record();
        rec.state == SessionState::Idle { stop_reason: StopReason::Background }
            && mesimon_core::quiet::is_mid_turn(&rec)
    });
    send(
        "SubagentStop",
        &format!(r#"{{"agent_id":"a1","agent_type":"x","background_tasks":[{row}]}}"#),
    );
    std::thread::sleep(Duration::from_millis(2 * mesimon_core::attention::SETTLE_MS));
    assert_eq!(
        record().state,
        SessionState::Idle { stop_reason: StopReason::Background },
        "a nested stop took back the lead's command"
    );
    send("UserPromptSubmit", "{}");
    send("Stop", r#"{"background_tasks":[]}"#);
    wait_until(Duration::from_secs(5), "the command's turn ends the park", || {
        record().state == SessionState::Idle { stop_reason: StopReason::EndTurn }
    });
}

/// T-1313, seen on T-1309: the person stops a parked lead's background
/// commands from Claude Code's tasks panel. No turn and no `Stop` come; the
/// mod's `ModTasksKilled` is the only word. One of two killed, the card says
/// one task and the park stands; the last killed, the turn is over.
#[test]
fn a_task_the_person_kills_leaves_the_card_and_the_last_ends_the_park() {
    let Some(h) = Harness::boot_with_env("bg-killed", Some(STUB), &[]) else { return };
    let mut client = h.client("bg-killed");
    let sid = spawn(&mut client, "Two builds in the background");
    let send = |event, body: &str| hook_send(&h.paths.hook_sock(), &sid.to_string(), event, body);
    let mut record = || client.record(sid);
    send("SessionStart", r#"{"source":"startup"}"#);
    send("UserPromptSubmit", "{}");
    for id in ["birttthoq", "bo47y1ii2"] {
        send(
            "PostToolUse",
            &format!(
                r#"{{"tool_name":"Bash","tool_response":{{"stdout":"","backgroundTaskId":"{id}"}}}}"#
            ),
        );
    }
    send(
        "Stop",
        r#"{"background_tasks":[{"id":"birttthoq","type":"shell","status":"running","description":"watch"},{"id":"bo47y1ii2","type":"shell","status":"running","description":"suite"}]}"#,
    );
    wait_until(Duration::from_secs(5), "two commands park the lead", || {
        let rec = record();
        rec.state == SessionState::Idle { stop_reason: StopReason::Background }
            && rec.tasks_running == Some(2)
    });
    send("ModTasksKilled", r#"{"task_ids":["bo47y1ii2"]}"#);
    wait_until(Duration::from_secs(5), "the card counts the one left", || {
        record().tasks_running == Some(1)
    });
    assert_eq!(record().state, SessionState::Idle { stop_reason: StopReason::Background });
    send("ModTasksKilled", r#"{"task_ids":["birttthoq"]}"#);
    wait_until(Duration::from_secs(5), "the last kill ends the park", || {
        let rec = record();
        rec.state == SessionState::Idle { stop_reason: StopReason::EndTurn }
            && rec.tasks_running.is_none()
    });
}
