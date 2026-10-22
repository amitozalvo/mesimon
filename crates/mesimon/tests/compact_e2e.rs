//! A compaction in flight is painted and moves nothing (T-894). The hook
//! frames of a compaction — `PreCompact`, `SessionStart{compact}`,
//! `PostCompact` — reach the daemon through the real hook binary on both
//! roads, and the snapshot's record carries the window (`compacting`) from
//! the first to the last, with the state and the column it began in kept
//! throughout: a person's `/compact` at a finished turn, a quiet one with a
//! daemon restart inside it, an automatic one inside a turn, and a vetoed
//! one that fires `PreCompact` alone.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionRecord, SessionState, StopReason};
use mesimon_core::command::{Command, Response};

/// The automoves the feed recorded for `ticket` as made, each as its row.
/// The feed is flushed on the daemon's tick, a beat after the snapshot
/// shows the move, so a count is waited for (`moves_reach`) and a "no move"
/// is read after a tick has passed (`settled_moves`).
fn moves(h: &Harness, ticket: ulid::Ulid) -> Vec<String> {
    let id = ticket.to_string();
    std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            l.contains("\"kind\":\"movement_decision\"")
                && l.contains("\"outcome\":\"moved\"")
                && l.contains(&id)
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn a_compaction_is_painted_and_moves_nothing() {
    let stub = claude_stub("#!/bin/sh\nexec sleep 120\n");
    let Some(h) = Harness::boot("compact", Some(&stub)) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("compact");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "squeeze".into(),
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
    let session = sid.to_string();
    let rec = |c: &mut TestClient| -> SessionRecord {
        c.board().sessions.iter().find(|s| s.id == sid).cloned().expect("record")
    };
    let column = |c: &mut TestClient| c.board().ticket(ticket).unwrap().column.clone();
    let done = SessionState::Idle { stop_reason: StopReason::EndTurn };

    // A turn, taken and finished: the template's rules carry the ticket to
    // IN PROGRESS and back to REVIEW, which is where a compaction finds it.
    hook_send_with(
        &hook_sock,
        &session,
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t-compact.jsonl","cwd":"/tmp"}"#,
    );
    hook_send(&hook_sock, &session, "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(5), "the turn moves the ticket to IN PROGRESS", || {
        column(&mut c) == "IN PROGRESS"
    });
    hook_send(&hook_sock, &session, "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(5), "the turn's end moves the ticket to REVIEW", || {
        column(&mut c) == "REVIEW"
    });
    assert_eq!(rec(&mut c).state, done);
    let moves_reach = |n: usize| {
        wait_until(Duration::from_secs(5), "the feed records the moves", || {
            moves(&h, ticket).len() == n
        });
        moves(&h, ticket)
    };
    let settled_moves = || {
        std::thread::sleep(Duration::from_millis(600));
        moves(&h, ticket)
    };
    let moved = moves_reach(2);

    // A person's `/compact` at a finished turn: the window opens on the
    // record with the state and the column untouched — before T-894 the
    // seat went `Running`, the ticket to IN PROGRESS and back, and the
    // crown heard a turn end.
    hook_send(&hook_sock, &session, "PreCompact", r#"{"trigger":"manual"}"#);
    wait_until(Duration::from_secs(5), "the compaction window opens", || rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, done, "a compaction moves no state");
    assert_eq!(column(&mut c), "REVIEW", "a compaction moves no ticket");
    // Its duplicate and its own SessionStart keep the window open and move
    // nothing either.
    hook_send(&hook_sock, &session, "PreCompact", r#"{"trigger":"manual"}"#);
    hook_send_with(&hook_sock, &session, "SessionStart", Some("compact"), r#"{"session_id":"x"}"#);
    std::thread::sleep(Duration::from_millis(400));
    let r = rec(&mut c);
    assert!(r.compacting, "still compacting after its SessionStart");
    assert_eq!(r.state, done);
    assert_eq!(column(&mut c), "REVIEW");
    hook_send(&hook_sock, &session, "PostCompact", r#"{"trigger":"manual"}"#);
    wait_until(Duration::from_secs(5), "the compaction window closes", || !rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, done, "the finished turn is where it was");
    assert_eq!(column(&mut c), "REVIEW");
    assert_eq!(settled_moves(), moved, "a manual compaction made no move");

    // A daemon restarted inside a quiet compaction (the `U` chip) comes
    // back with no window and a record it cannot read: the compaction's
    // own `SessionStart` reopens the window, its two helper `SubagentStop`s
    // hold inside it (before the fix they read the restart's Low record as
    // a turn and moved the ticket), and its end closes it. No turn, no move.
    hook_send(&hook_sock, &session, "PreCompact", r#"{"trigger":"auto"}"#);
    wait_until(Duration::from_secs(5), "the quiet window opens", || rec(&mut c).compacting);
    drop(c);
    h.restart();
    let mut c = h.client("compact-after");
    assert!(!rec(&mut c).compacting, "a restart's machine starts with no window");
    hook_send_with(&hook_sock, &session, "SessionStart", Some("compact"), r#"{"session_id":"x"}"#);
    wait_until(Duration::from_secs(5), "the compaction's start reopens the window", || {
        rec(&mut c).compacting
    });
    hook_send(&hook_sock, &session, "SubagentStop", r#"{"agent_type":""}"#);
    hook_send(&hook_sock, &session, "SubagentStop", r#"{"agent_type":""}"#);
    std::thread::sleep(Duration::from_millis(400));
    let r = rec(&mut c);
    assert_ne!(r.state, SessionState::Running, "a helper's stop is no turn");
    assert!(r.compacting, "still compacting through the helper's stops");
    assert_eq!(column(&mut c), "REVIEW");
    hook_send(&hook_sock, &session, "PostCompact", r#"{"trigger":"auto"}"#);
    wait_until(Duration::from_secs(5), "the window closes after the restart", || {
        !rec(&mut c).compacting
    });
    assert_ne!(rec(&mut c).state, SessionState::Running, "a compaction's end is no turn");
    assert_eq!(column(&mut c), "REVIEW");
    assert_eq!(settled_moves(), moved, "a compaction across a restart made no move");

    // An automatic compaction inside a turn: the window opens over
    // `Running`, which the turn keeps, and the turn ends as any turn does.
    hook_send(&hook_sock, &session, "UserPromptSubmit", r#"{"prompt":"more"}"#);
    wait_until(Duration::from_secs(5), "the next turn moves the ticket to IN PROGRESS", || {
        column(&mut c) == "IN PROGRESS"
    });
    let moved = moves_reach(3);
    hook_send(&hook_sock, &session, "PreCompact", r#"{"trigger":"auto"}"#);
    wait_until(Duration::from_secs(5), "the window opens mid-turn", || rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, SessionState::Running, "the turn goes on");
    hook_send_with(&hook_sock, &session, "SessionStart", Some("compact"), r#"{"session_id":"x"}"#);
    hook_send(&hook_sock, &session, "PostCompact", r#"{"trigger":"auto"}"#);
    wait_until(Duration::from_secs(5), "the window closes mid-turn", || !rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, SessionState::Running);
    assert_eq!(column(&mut c), "IN PROGRESS");
    assert_eq!(settled_moves(), moved, "an automatic compaction made no move");
    hook_send(&hook_sock, &session, "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(5), "the turn's end moves the ticket to REVIEW", || {
        column(&mut c) == "REVIEW"
    });

    // A vetoed compaction fires `PreCompact` alone (nothing to compact):
    // the next prompt closes the window, and the turn it begins is a turn.
    hook_send(&hook_sock, &session, "PreCompact", r#"{"trigger":"manual"}"#);
    wait_until(Duration::from_secs(5), "the window opens on a veto", || rec(&mut c).compacting);
    hook_send(&hook_sock, &session, "UserPromptSubmit", r#"{"prompt":"on"}"#);
    wait_until(Duration::from_secs(5), "a prompt closes the window", || {
        let r = rec(&mut c);
        !r.compacting && r.state == SessionState::Running
    });
    assert_eq!(column(&mut c), "IN PROGRESS");
}
