//! A compaction in flight is painted and moves nothing (T-894). The hook
//! frames of a compaction — `PreCompact`, `SessionStart{compact}`,
//! `PostCompact` — reach the daemon through the real hook binary on both
//! roads, and the snapshot's record carries the window (`compacting`) from
//! the first to the last, with the state and the column it began in kept
//! throughout: a person's `/compact` at a finished turn, an automatic one
//! inside a turn, and a vetoed one that fires `PreCompact` alone.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionRecord, SessionState, StopReason};
use mesimon_core::command::{Command, Response};

/// The automoves the feed recorded for `ticket` as made.
fn moves(h: &Harness, ticket: ulid::Ulid) -> usize {
    let id = ticket.to_string();
    std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            l.contains("\"kind\":\"movement_decision\"")
                && l.contains("\"outcome\":\"moved\"")
                && l.contains(&id)
        })
        .count()
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
    let moved = moves(&h, ticket);
    assert_eq!(moved, 2, "the two automoves of the turn");

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
    assert_eq!(moves(&h, ticket), moved, "a manual compaction made no move");

    // An automatic compaction inside a turn: the window opens over
    // `Running`, which the turn keeps, and the turn ends as any turn does.
    hook_send(&hook_sock, &session, "UserPromptSubmit", r#"{"prompt":"more"}"#);
    wait_until(Duration::from_secs(5), "the next turn moves the ticket to IN PROGRESS", || {
        column(&mut c) == "IN PROGRESS"
    });
    let moved = moves(&h, ticket);
    hook_send(&hook_sock, &session, "PreCompact", r#"{"trigger":"auto"}"#);
    wait_until(Duration::from_secs(5), "the window opens mid-turn", || rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, SessionState::Running, "the turn goes on");
    hook_send_with(&hook_sock, &session, "SessionStart", Some("compact"), r#"{"session_id":"x"}"#);
    hook_send(&hook_sock, &session, "PostCompact", r#"{"trigger":"auto"}"#);
    wait_until(Duration::from_secs(5), "the window closes mid-turn", || !rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, SessionState::Running);
    assert_eq!(column(&mut c), "IN PROGRESS");
    assert_eq!(moves(&h, ticket), moved, "an automatic compaction made no move");
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
