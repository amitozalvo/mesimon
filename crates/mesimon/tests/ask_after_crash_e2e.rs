//! Shift+Enter on a ticket whose claude crashed resumes THAT conversation
//! (T-1119). The corpse holds no seat, so the ask used to read the ticket as
//! empty and start a second claude beside it — silently, with the crashed
//! conversation left on the rail unreachable from the card. A crash is an
//! exit the person did not choose; the conversation it leaves is the one
//! they expect to go on, exactly as a parked one does. Real tmux, real
//! `pane-died`, in-process daemon, a stub that dies with status 1.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use mesimon_core::board::{ExitReason, SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

#[test]
fn asking_a_crashed_claude_resumes_its_conversation_and_starts_nothing() {
    // A claude that dies a second in, nonzero: the pane-died road files it
    // as a crash, which the park-on-exit gate deliberately leaves a corpse.
    let Some(h) = Harness::boot_bare("askcrash", Some("#!/bin/sh\nsleep 1\nexit 1\n"), &[]) else {
        return;
    };
    let mut c = h.client("askcrash");
    // Claude's own store is what `--resume` reads: a transcript under the
    // census root makes the record resumable rather than fresh.
    let projects = h.dir.join("claude-home").join("projects").join("msmn");
    std::fs::create_dir_all(&projects).unwrap();

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "crashed".into(),
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
    std::fs::write(
        projects.join(format!("{sid}.jsonl")),
        "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n",
    )
    .unwrap();

    let dead = c.await_state(sid, "exited", |s| matches!(s, SessionState::Exited { .. }));
    assert_eq!(dead, SessionState::Exited { reason: ExitReason::Crashed }, "a crash is a corpse");

    // The board's Shift+Enter with words: the receipt is the SAME record,
    // resumed — not a fresh conversation, not a second session.
    match c.request(Command::PromptSession {
        ticket,
        text: "carry on".into(),
        queued: false,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: Vec::new(),
    }) {
        Response::Spawned { id, fresh } => {
            assert_eq!(id, sid, "the crashed record is the one resumed");
            assert!(!fresh, "its conversation was there to resume");
        }
        other => panic!("the ask did not resume: {other:?}"),
    }
    let board = c.board();
    let agents: Vec<_> =
        board.sessions.iter().filter(|s| s.ticket == ticket && s.kind.is_agent()).collect();
    assert_eq!(agents.len(), 1, "no second claude beside the corpse: {agents:?}");
    assert!(agents[0].state.is_live(), "the record is live again: {:?}", agents[0].state);
}
