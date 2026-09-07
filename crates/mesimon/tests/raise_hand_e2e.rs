//! `raise_hand` (T-107) end to end: an agent asks for a person on its own
//! ticket through the real shim, the mark lands on the ticket FILE and in the
//! board's `!N`, the words come back scrubbed and capped, and the three roads
//! down — the person's `LowerHand`, a prompt reaching the agent, and a turn
//! that starts with no prompt at all (T-311) — each put it out.
//!
//! The point of the file is the LIFETIME. Raising is one call; what makes the
//! mark worth having is that the `Stop` which ends the turn moments later
//! leaves it standing, and that is a thing only a live daemon can be asked.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;
use serde_json::json;

mod common;
use common::*;

#[test]
fn a_raised_hand_outlives_the_turn_and_is_lowered_by_the_person_or_the_next_turn() {
    const STUB: &str = "#!/bin/sh\nwhile IFS= read -r line; do :; done\n";
    // The archive at the end needs a parked session, and the sleep floor is
    // a minute of wall clock this test is not going to spend.
    let Some(h) =
        Harness::boot_with_env("raisehand", Some(STUB), &[("MESIMON_SLEEP_MIN_AGE_MS", "0")])
    else {
        return;
    };
    let sock = h.paths.orch_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("raisehand");

    let ticket = match c.request(Command::CreateTicket {
        column: "IN PROGRESS".into(),
        title: "wire up login".into(),
        workspace: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let key = c.board().ticket(ticket).unwrap().short_key.clone();
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };

    // ---- the tool, through the real shim ---------------------------------
    let mut shim = Shim::start(&sock, sid);
    shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    shim.notify("notifications/initialized");
    assert_eq!(c.board().needs_you_count(), 0, "nothing needs anybody yet");

    let r = shim.call_ok("raise_hand", json!({ "reason": "Auth0, or the session cookie?" }));
    assert_eq!(r["reason"], "Auth0, or the session cookie?");
    let board = c.board();
    assert_eq!(board.needs_you_count(), 1, "the header's count sees it");
    let t = board.ticket(ticket).unwrap();
    assert!(t.hand_raised());
    let raised = t.raised.as_ref().unwrap();
    assert_eq!(raised.reason, "Auth0, or the session cookie?");
    assert_eq!(raised.by, format!("agent:{sid}"), "the asker is named the way a note's author is");

    // On the ticket's own file, so a restart still knows somebody is waiting.
    let ticket_file = h.paths.board_dir.join("board/tickets").join(&key).join("ticket.toml");
    let file = std::fs::read_to_string(&ticket_file).unwrap();
    assert!(file.contains("[raised]"), "{file}");
    assert!(file.contains("Auth0"), "{file}");

    // The activity feed says who asked (it flushes on the 250 ms wheel), and
    // never what they asked: the words are the ticket's, not the log's.
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    wait_until(Duration::from_secs(5), "the feed to name the ask", || {
        std::fs::read_to_string(&feed_path).unwrap_or_default().contains("raise_hand")
    });
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(feed.contains("\"actor\":\"agent\""), "{feed}");
    assert!(!feed.contains("Auth0"), "never the words: {feed}");

    // ---- the turn ends, and the mark does not --------------------------
    // This is the whole reason it is not a session state: `Stop` lands
    // moments after the call on every real use.
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(6), "the session goes idle", || {
        c.board().sessions.iter().any(|s| {
            s.id == sid && matches!(s.state, mesimon_core::board::SessionState::Idle { .. })
        })
    });
    assert!(c.board().ticket(ticket).unwrap().hand_raised(), "a finished turn is not an answer");

    // ---- the words are the board's, not the agent's ----------------------
    // Sanitized and capped at the boundary, and the receipt says what was
    // kept — a control character never reaches a card row.
    let long = "x".repeat(400);
    let r = shim.call_ok("raise_hand", json!({ "reason": format!("a\tb\u{7}c {long}") }));
    let kept = r["reason"].as_str().unwrap();
    assert!(kept.len() <= mesimon_core::board::RAISE_REASON_MAX_BYTES, "{}", kept.len());
    assert!(!kept.contains('\u{7}'), "{kept}");
    assert_eq!(c.board().needs_you_count(), 1, "raising again replaces, never stacks");
    // An empty reason is refused where the model can read it.
    let msg = shim.call_err("raise_hand", json!({ "reason": "   " }));
    assert!(msg.contains("reason"), "{msg}");

    // ---- the person lowers it ---------------------------------------------
    assert!(matches!(c.request(Command::LowerHand { id: ticket }), Response::Ok));
    assert!(!c.board().ticket(ticket).unwrap().hand_raised());
    assert_eq!(c.board().needs_you_count(), 0);
    // And doing it twice is a no-op, so the TUI may send it on every
    // departure from a ticket page.
    assert!(matches!(c.request(Command::LowerHand { id: ticket }), Response::Ok));
    let file = std::fs::read_to_string(&ticket_file).unwrap();
    assert!(!file.contains("[raised]"), "{file}");

    // ---- and so does answering it ----------------------------------------
    shim.call_ok("raise_hand", json!({ "reason": "still stuck" }));
    assert!(c.board().ticket(ticket).unwrap().hand_raised());
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    wait_until(Duration::from_secs(6), "the prompt lowers the hand", || {
        !c.board().ticket(ticket).unwrap().hand_raised()
    });
    assert_eq!(c.board().needs_you_count(), 0);

    // ---- and so does a turn that starts without one ----------------------
    // The `!` bash road (T-311): a bash command typed in Claude Code puts its
    // output into the conversation and the model takes a turn on it, firing
    // no `UserPromptSubmit` whatever. That is how a person answers "run
    // `gcloud auth login`, then tell me", so the turn beginning has to be an
    // answer even when no prompt hook says so.
    shim.call_ok("raise_hand", json!({ "reason": "run `gcloud auth login`" }));
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(6), "the turn to end", || {
        c.board().sessions.iter().any(|s| {
            s.id == sid
                && matches!(
                    s.state,
                    mesimon_core::board::SessionState::Idle {
                        stop_reason: mesimon_core::board::StopReason::EndTurn
                    }
                )
        })
    });
    assert!(c.board().ticket(ticket).unwrap().hand_raised(), "the Stop is still not an answer");
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "PostToolUse",
        r#"{"tool_name":"Read","tool_response":{}}"#,
    );
    wait_until(Duration::from_secs(6), "the new turn lowers the hand", || {
        !c.board().ticket(ticket).unwrap().hand_raised()
    });
    assert_eq!(c.board().needs_you_count(), 0);
    assert!(
        c.board()
            .sessions
            .iter()
            .any(|s| s.id == sid && matches!(s.state, mesimon_core::board::SessionState::Running)),
        "and the card is working again (T-228)"
    );

    // ---- the never-tier holds ---------------------------------------------
    // An agent may raise a hand and may not take one down: being answered is
    // not something the asker declares.
    shim.call_ok("raise_hand", json!({ "reason": "one more" }));
    match c.send(Principal::Agent { session: sid }, Command::LowerHand { id: ticket }) {
        Response::Err { message } => assert!(message.contains("agent"), "{message}"),
        other => panic!("an agent may not lower its own hand: {other:?}"),
    }
    assert!(c.board().ticket(ticket).unwrap().hand_raised());

    // ---- an archived ticket has no card to light --------------------------
    // Park the session first: an archive is refused while a pane is held,
    // and only an idle claude sleeps.
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(6), "the turn to end", || {
        c.board().sessions.iter().any(|s| {
            s.id == sid && matches!(s.state, mesimon_core::board::SessionState::Idle { .. })
        })
    });
    assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
    wait_until(Duration::from_secs(6), "the session parks", || {
        c.board()
            .sessions
            .iter()
            .any(|s| s.id == sid && matches!(s.state, mesimon_core::board::SessionState::Sleeping))
    });
    assert!(matches!(c.request(Command::ArchiveTicket { id: ticket }), Response::Ok));
    assert_eq!(c.board().needs_you_count(), 0, "an archived ticket needs nobody");
    match c.send(
        Principal::Agent { session: sid },
        Command::AgentRaiseHand { reason: "hello?".into() },
    ) {
        Response::Err { message } => assert!(message.contains("archived"), "{message}"),
        other => panic!("an archived ticket refuses the ask: {other:?}"),
    }
}
