//! A person parks a ticket in TODO by hand, then asks its agent again — and
//! the card must follow the work into IN PROGRESS (T-186, dogfood 2026-09-04).
//!
//! The no-undo rule in `movegate` refuses an automatic move that is the exact
//! reverse of a move somebody else just made. The `<<` that parked the card
//! was IN PROGRESS → TODO, so the `Running` edge the next prompt produces asked
//! for TODO → IN PROGRESS inside the window and was refused as ping-pong —
//! automove undoing the human. But the drag and the ask are the same hand,
//! and the ask is the newer intent: a `UserPromptSubmit` now supersedes the
//! person's OWN last move on the ticket. An agent's move stays protected
//! (`mcp_e2e` holds that side).

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";

fn wait_for_column(c: &mut TestClient, ticket: ulid::Ulid, want: &str, what: &str) {
    wait_until(Duration::from_secs(6), what, || c.board().ticket(ticket).unwrap().column == want);
}

#[test]
fn asking_again_after_parking_by_hand_moves_the_card_to_in_progress() {
    // The window, widened past the test's wall clock: the guard is certainly
    // armed at the moment that matters, so a pass means the rule yielded.
    let Some(h) =
        Harness::boot_with_env("askpark", Some(STUB), &[("MESIMON_PINGPONG_MS", "600000")])
    else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("askpark");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "test".into(),
        workspace: None,
    });
    let ticket = c.board().tickets.first().expect("ticket").id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sid_s = sid.to_string();

    // "said hello": the turn runs and ends — TODO → IN PROGRESS → REVIEW.
    hook_send(&hook_sock, &sid_s, "SessionStart", r#"{"source":"startup"}"#);
    hook_send(&hook_sock, &sid_s, "UserPromptSubmit", r#"{"session_id":"x"}"#);
    wait_for_column(&mut c, ticket, "IN PROGRESS", "the first turn");
    hook_send(&hook_sock, &sid_s, "Stop", r#"{"stop_hook_active":false}"#);
    wait_for_column(&mut c, ticket, "REVIEW", "the first turn's end");
    wait_until(Duration::from_secs(6), "idle", || {
        matches!(
            c.board().sessions.iter().find(|s| s.id == sid).unwrap().state,
            SessionState::Idle { .. }
        )
    });

    // `<<` by hand: REVIEW → IN PROGRESS → TODO. The last move on the ticket
    // is now the person's IN PROGRESS → TODO.
    for col in ["IN PROGRESS", "TODO"] {
        assert!(matches!(
            c.request(Command::MoveTicket { id: ticket, column: col.into(), before: None }),
            Response::Ok
        ));
    }
    assert_eq!(c.board().ticket(ticket).unwrap().column, "TODO");

    // Shift+Enter from TODO: the prompt lands and the agent runs. The card
    // must go with it — this is the edge the gate refused before.
    assert!(matches!(
        c.request(Command::PromptSession { ticket, text: "again".into(), queued: false }),
        Response::Ok
    ));
    hook_send(&hook_sock, &sid_s, "UserPromptSubmit", r#"{"session_id":"x"}"#);
    wait_for_column(&mut c, ticket, "IN PROGRESS", "the ask after the park");
}
