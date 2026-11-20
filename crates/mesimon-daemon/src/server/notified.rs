//! T-1287: a background task's `<task-notification>` fires Claude Code's
//! `UserPromptSubmit` (T-135), inside a running turn or waking an idle seat.
//! It is the harness's words, never a person's: the readers that stand for a
//! person take nothing from it. Each case below sends the notification, then
//! a person's prompt, so that it fails when a reader counts the first.

use super::*;
use crate::server::crownwake::ToldHand;
use crate::server::queued_person_push_tests::{daemon, prompt_frame, worker_ticket};
use mesimon_core::board::Raised;

const NOTIFICATION: &str = "<task-notification>\n<task-id>bqvffdb91</task-id>\n\
    <status>completed</status>\n<summary>Background command \"cargo ut\" completed \
    (exit code 0)</summary>\n</task-notification>";

fn raised() -> Raised {
    Raised { at: "@0".into(), by: "agent:x".into(), reason: "Ship is yours".into() }
}

/// The fixture's daemon (one Claude pane, an ask queued at it) with its
/// ticket on the board, a hand up, and the seat at `state`.
fn with_seat(tag: &str, state: SessionState, f: impl FnOnce(&mut Daemon, ulid::Ulid, uuid::Uuid)) {
    let dir = std::env::temp_dir()
        .join(format!("msmn-t1287-{tag}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let (mut d, _, _rx) = daemon(&dir);
    let sid = d.board.sessions[0].id;
    let ticket = d.board.sessions[0].ticket;
    let mut t = worker_ticket(ticket, "IN PROGRESS", "a");
    t.raised = Some(raised());
    d.board.tickets.push(t);
    d.board.sessions[0].state = state.clone();
    d.board.sessions[0].pane_key = Some("1:%0".into());
    d.machines.insert(sid, Machine::new(state, 1_000));
    f(&mut d, ticket, sid);
    drop(d);
    let _ = std::fs::remove_dir_all(dir);
}

fn hand_up(d: &Daemon, ticket: ulid::Ulid) -> bool {
    d.board.ticket(ticket).is_some_and(|t| t.raised.is_some())
}

/// The T-1196 and T-1208 shapes: the worker's own `cargo` run ends inside
/// its turn. The queued ask, the hand, the crown's told hand, the merged
/// notice's mark and the dialog count all stand; a person's prompt then
/// takes each as before.
#[test]
fn a_notification_inside_a_turn_is_nobody_s_prompt() {
    with_seat("mid", SessionState::Running, |d, ticket, sid| {
        d.crown_hands.insert(ticket, ToldHand { reason: "Ship is yours".into(), told_ms: 1 });
        d.notice_turns.insert(sid);
        d.on_hook(prompt_frame(sid, Road::Hooks, NOTIFICATION));
        assert_eq!(d.board.session(sid).unwrap().state, SessionState::Running);
        assert_eq!(d.queued.len(), 1, "the queued ask waits for the turn's end");
        assert!(hand_up(d, ticket), "the hand stays up");
        assert!(d.crown_hands.contains_key(&ticket), "the crown's told hand stands");
        assert!(d.notice_turns.contains(&sid), "the notice turn is still the notice's");
        assert_eq!(mesophon::prompts_now(&d.prompts_heard, sid), 0, "no dialog spoken past");
        d.on_hook(prompt_frame(sid, Road::Hooks, "a person's words"));
        assert!(d.queued.is_empty(), "a person talking ahead of the ask drops it");
        assert!(!hand_up(d, ticket), "a person's prompt answers the hand");
        assert!(!d.crown_hands.contains_key(&ticket));
        assert!(!d.notice_turns.contains(&sid), "a person's prompt begins a turn of its own");
        assert_eq!(mesophon::prompts_now(&d.prompts_heard, sid), 1);
    });
}

/// The T-1204 shape: a subagent's report wakes the idle seat. The turn
/// begins (the machine still takes the edge), and it is nobody's answer:
/// T-311's turn-start edge leaves the hand up, and the ask stays queued.
#[test]
fn a_notification_waking_an_idle_seat_begins_a_turn_nobody_asked_for() {
    let idle = SessionState::Idle { stop_reason: StopReason::EndTurn };
    with_seat("idle", idle.clone(), |d, ticket, sid| {
        d.on_hook(prompt_frame(sid, Road::Hooks, NOTIFICATION));
        assert_eq!(d.board.session(sid).unwrap().state, SessionState::Running, "a turn began");
        assert!(hand_up(d, ticket), "a turn the agent's own work began answers no hand");
        assert_eq!(d.queued.len(), 1, "the ask waits for this turn's end");
    });
    with_seat("idle-person", idle, |d, ticket, sid| {
        d.on_hook(prompt_frame(sid, Road::Hooks, "a person's words"));
        assert!(!hand_up(d, ticket), "a person's prompt on an idle seat answers it");
    });
}

/// A person's last move keeps its no-undo guard through a notification:
/// automation may not reverse it until the person asks the agent to work.
#[test]
fn a_notification_keeps_the_person_s_move_guarded() {
    with_seat("move", SessionState::Running, |d, ticket, sid| {
        let now = Instant::now();
        d.moves.record(ticket, "IN PROGRESS", "BACKLOG", &Principal::Local, None, now);
        let auto = Principal::Automation { rule: "automove".into() };
        d.on_hook(prompt_frame(sid, Road::Hooks, NOTIFICATION));
        let held = d.moves.check(ticket, "BACKLOG", "IN PROGRESS", &auto, now);
        assert!(held.is_err(), "the person's drag is still theirs");
        d.on_hook(prompt_frame(sid, Road::Hooks, "a person's words"));
        let freed = d.moves.check(ticket, "BACKLOG", "IN PROGRESS", &auto, now);
        assert!(freed.is_ok(), "their ask supersedes their drag (T-186): {freed:?}");
    });
}

/// The crown's journal (`crown_prompted`): inside a turn a notification
/// leaves the turn's cause; on an idle crown seat it begins a turn with no
/// cause, never a person's (2026-11-13: three acts read `person`).
#[test]
fn a_notification_on_the_crown_s_seat_is_no_cause() {
    let wake = mesimon_core::crownact::Cause::Wake { news: "T-9 delivered".into() };
    with_seat("crown-mid", SessionState::Running, |d, ticket, sid| {
        d.board.crown = Some(ticket);
        d.crown_cause = Some(wake.clone());
        d.on_hook(prompt_frame(sid, Road::Hooks, NOTIFICATION));
        assert_eq!(d.crown_cause, Some(wake.clone()), "the wake's turn keeps its cause");
        d.on_hook(prompt_frame(sid, Road::Hooks, "a person's words"));
        assert_eq!(d.crown_cause, Some(mesimon_core::crownact::Cause::Person));
    });
    let idle = SessionState::Idle { stop_reason: StopReason::EndTurn };
    with_seat("crown-idle", idle, |d, ticket, sid| {
        d.board.crown = Some(ticket);
        d.crown_cause = Some(wake.clone());
        d.on_hook(prompt_frame(sid, Road::Hooks, NOTIFICATION));
        assert_eq!(d.crown_cause, None, "a turn nobody started has no cause");
    });
}

/// The T-1270 shape, kept: the person answers the hand with a `!` line
/// from the board. Its receipt is the box reading empty, no prompt hook,
/// and it lowers the hand as their prompt would; the line's own
/// notification no longer has to.
#[test]
fn a_person_s_shell_line_from_the_board_answers_the_hand() {
    let idle = SessionState::Idle { stop_reason: StopReason::EndTurn };
    with_seat("shell", idle, |d, ticket, sid| {
        d.crown_hands.insert(ticket, ToldHand { reason: "Ship is yours".into(), told_ms: 1 });
        let sid16 = d.board.sessions[0].sid16();
        let t0 = now_ms();
        let mut owed = Owed::pasted(ticket, Ack::PROMPT, t0);
        owed.screen_acked = true;
        d.owed.insert(sid, owed);
        let empty = mesimon_backend_tmux::InputScreen {
            lines: vec!["─".repeat(20), "❯ ".into(), "─".repeat(20)],
            cursor: None,
        };
        d.screens.insert(sid16, (t0 + SHELL_RENDER_MS + 1, empty));
        d.settle_owed(t0 + SHELL_RENDER_MS + 2);
        assert!(!d.owed.contains_key(&sid), "the box read empty: the line went in");
        assert!(!hand_up(d, ticket), "the person's line answers the hand");
        assert!(!d.crown_hands.contains_key(&ticket));
    });
}
