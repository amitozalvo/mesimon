//! The inactivity wheel uses settled turn time and the ordinary sleep/wake path.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState, StopReason};
use mesimon_core::command::{Command, Response};

const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";
const DONE: &str = r#"{"stop_hook_active":false,"background_tasks":[]}"#;

fn start(h: &Harness, c: &mut TestClient, title: &str) -> (uuid::Uuid, uuid::Uuid) {
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let id = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    // A distinct Claude identity proves wake preserves the hosted conversation.
    let conversation = uuid::Uuid::new_v4();
    let path = h.dir.join(format!("{conversation}.jsonl"));
    std::fs::write(&path, "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n").unwrap();
    hook_send(
        &h.paths.hook_sock(),
        &id.to_string(),
        "SessionStart",
        &serde_json::json!({"session_id": conversation, "transcript_path": path}).to_string(),
    );
    c.await_state(id, "started", |s| matches!(s, SessionState::Idle { .. }));
    (id, conversation)
}

fn hook(h: &Harness, id: uuid::Uuid, event: &str, payload: &str) {
    hook_send(&h.paths.hook_sock(), &id.to_string(), event, payload);
}

fn finish(h: &Harness, c: &mut TestClient, id: uuid::Uuid) {
    hook(h, id, "UserPromptSubmit", "{}");
    c.await_state(id, "running", |s| *s == SessionState::Running);
    hook(h, id, "Stop", DONE);
    c.await_state(id, "finished", |s| {
        *s == SessionState::Idle { stop_reason: StopReason::EndTurn }
    });
}

#[test]
fn inactivity_parks_only_finished_resumable_turns_and_wakes_the_same_conversation() {
    let Some(h) = Harness::boot_with_env(
        "inactivity",
        Some(STUB),
        &[
            ("MESIMON_INACTIVITY_PARK_TICKS", "1"),
            ("MESIMON_INACTIVITY_MINUTE_MS", "3000"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
        ],
    ) else {
        return;
    };
    let mut c = h.client("inactivity");
    let (idle, conversation) = start(&h, &mut c, "finished");
    finish(&h, &mut c, idle);
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(c.board().park_after_minutes, 0);
    assert!(matches!(
        c.board().sessions.iter().find(|s| s.id == idle).unwrap().state,
        SessionState::Idle { stop_reason: StopReason::EndTurn }
    ));

    let mut background_sessions = Vec::new();
    for (kind, reason) in [("subagent", StopReason::Background), ("shell", StopReason::Monitoring)]
    {
        let (id, _) = start(&h, &mut c, kind);
        hook(&h, id, "UserPromptSubmit", "{}");
        hook(
            &h,
            id,
            "Stop",
            &serde_json::json!({
                "stop_hook_active": false,
                "background_tasks": [{"id": "task", "type": kind, "status": "running"}]
            })
            .to_string(),
        );
        c.await_state(id, kind, |s| *s == SessionState::Idle { stop_reason: reason });
        background_sessions.push(id);
    }
    let (attention, _) = start(&h, &mut c, "attention");
    hook(
        &h,
        attention,
        "Notification",
        r#"{"notification_type":"permission_prompt","message":"permission"}"#,
    );
    c.await_state(attention, "attention", |s| matches!(s, SessionState::RequiresAction { .. }));
    let (missing, missing_conversation) = start(&h, &mut c, "missing history");
    finish(&h, &mut c, missing);
    std::fs::remove_file(h.dir.join(format!("{missing_conversation}.jsonl"))).unwrap();
    let (unknown, _) = start(&h, &mut c, "no finished turn");

    // Time spent running never consumes the idle timeout, including a new turn
    // on a record whose previous completion is already older than the timeout.
    hook(&h, idle, "UserPromptSubmit", "{}");
    c.await_state(idle, "running again", |s| *s == SessionState::Running);
    assert!(matches!(c.request(Command::SetParkAfterMinutes { minutes: 1 }), Response::Ok));
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(
        c.board().sessions.iter().find(|s| s.id == idle).unwrap().state,
        SessionState::Running
    );
    hook(&h, idle, "Stop", DONE);
    c.await_state(idle, "settled", |s| {
        *s == SessionState::Idle { stop_reason: StopReason::EndTurn }
    });
    std::thread::sleep(Duration::from_secs(1));
    assert!(matches!(
        c.board().sessions.iter().find(|s| s.id == idle).unwrap().state,
        SessionState::Idle { .. }
    ));
    c.await_state(idle, "automatically sleeping", |s| *s == SessionState::Sleeping);
    let board = c.board();
    for id in background_sessions.into_iter().chain([attention, missing, unknown]) {
        assert!(
            board.sessions.iter().find(|s| s.id == id).unwrap().state.has_pane(),
            "unsafe park: {id}"
        );
    }
    let rec = board.sessions.iter().find(|s| s.id == idle).unwrap();
    assert_eq!(rec.claude_session_id, Some(conversation));
    assert!(h.paths.transcripts_dir().join(format!("{idle}.jsonl")).is_file());
    assert!(matches!(
        c.request(Command::WakeSession { id: idle }),
        Response::Spawned { fresh: false, .. }
    ));
    let board = c.board();
    let rec = board.sessions.iter().find(|s| s.id == idle).unwrap();
    assert_eq!(rec.claude_session_id, Some(conversation));
    assert!(rec.argv.windows(2).any(|a| a == ["--resume", &conversation.to_string()]));
    assert!(matches!(c.request(Command::SetParkAfterMinutes { minutes: 0 }), Response::Ok));
    assert_eq!(c.board().park_after_minutes, 0);
}

/// T-543: a column opts its tickets into the idle park. Nothing sleeps
/// before a column asks, a person attached to the pane holds it awake past
/// the timer, and the park lands the moment they leave — on the column's
/// ticket only, with the column's rule in the feed.
#[test]
fn a_column_parks_its_idle_agents_and_spares_the_rest() {
    let Some(h) = Harness::boot_with_env(
        "autosleep",
        Some(STUB),
        &[
            ("MESIMON_INACTIVITY_PARK_TICKS", "1"),
            // One "minute" is long enough that a check three seconds after
            // the attach is still inside the attached person's window.
            ("MESIMON_INACTIVITY_MINUTE_MS", "10000"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
        ],
    ) else {
        return;
    };
    let mut c = h.client("autosleep");
    let (kept, _) = start(&h, &mut c, "stays awake");
    finish(&h, &mut c, kept);
    let (parked, conversation) = start(&h, &mut c, "sleeps in done");
    finish(&h, &mut c, parked);
    let state = |c: &mut TestClient, id| {
        c.board().sessions.iter().find(|s| s.id == id).unwrap().state.clone()
    };

    // Older than a column minute, and no column has asked: both awake.
    std::thread::sleep(Duration::from_millis(10_500));
    assert_eq!(c.board().park_after_minutes, 0);
    for id in [kept, parked] {
        assert!(state(&mut c, id).has_pane(), "slept with nothing on: {id}");
    }

    // Somebody attaches to the agent about to be parked.
    let sid16 = parked.simple().to_string()[..16].to_string();
    let mut client = tmux(&h.paths.tmux_sock())
        .args(["-C", "attach", "-t", &sid16])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    wait_until(Duration::from_secs(10), "the attached client", || {
        tmux(&h.paths.tmux_sock())
            .args(["list-clients", "-t", &sid16])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    });

    let board = c.board();
    let mut done = board.column("DONE").unwrap().settings.clone();
    done.sleep_after_minutes = 1;
    done.requires_merge = false;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "DONE".into(), settings: done }),
        Response::Ok
    ));
    let ticket = board.sessions.iter().find(|s| s.id == parked).unwrap().ticket;
    assert!(matches!(
        c.request(Command::MoveTicket { id: ticket, column: "DONE".into(), before: None }),
        Response::Ok
    ));
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        state(&mut c, parked).has_pane(),
        "parked under a person attached to it: {:?}",
        state(&mut c, parked)
    );

    client.kill().unwrap();
    let _ = client.wait();
    c.await_state(parked, "parked by its column", |s| *s == SessionState::Sleeping);
    assert!(state(&mut c, kept).has_pane(), "a column that did not ask kept its agent");
    let board = c.board();
    let rec = board.sessions.iter().find(|s| s.id == parked).unwrap();
    assert_eq!(rec.claude_session_id, Some(conversation), "the conversation is kept");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    wait_until(Duration::from_secs(5), "the autosleep feed line", || {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines().any(|l| l.contains("\"autosleep\"") && l.contains("\"automation\""))
        })
    });
}
