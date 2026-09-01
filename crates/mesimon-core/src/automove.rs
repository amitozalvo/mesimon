//! Automove: session state transitions drag the ticket along the default
//! column template. v0.1 hardcodes the rules against the D33i template names
//! (like `SLEEP_SAFE_COLUMN` in the daemon); a per-column policy arrives with
//! column configuration.
//!
//! Rules:
//! - "TODO" → "IN PROGRESS" when a session starts working (`Running`).
//! - "IN PROGRESS" → "REVIEW" when a turn completes (`Idle{EndTurn}`).
//! - "REVIEW" → "IN PROGRESS" when a session works again (review feedback sent
//!   back to the agent reopens the work; the next `EndTurn` returns it).
//!
//! Deliberately NOT rules: an interrupt (`Idle{Interrupted}`) is not "done";
//! `RequiresAction` is not "working" (a trust/startup modal fires before any
//! work happens — a mid-turn permission ask was preceded by `Running` anyway);
//! "DONE" is terminal — nothing reanimates it. Low/Stale confidence never
//! moves a ticket — the observe tier may misread a transcript, and a wrong
//! card position is a lie the user has to undo by hand.

use crate::board::{Confidence, SessionState, StopReason};

pub const TODO: &str = "TODO";
pub const IN_PROGRESS: &str = "IN PROGRESS";
pub const REVIEW: &str = "REVIEW";

/// The destination column a session-state transition drags the ticket to,
/// given the column it sits in now. `None` = stay put.
pub fn automove(column: &str, to: &SessionState, confidence: Confidence) -> Option<&'static str> {
    if matches!(confidence, Confidence::Low | Confidence::Stale) {
        return None;
    }
    match to {
        SessionState::Running if column == TODO || column == REVIEW => Some(IN_PROGRESS),
        SessionState::Idle { stop_reason: StopReason::EndTurn } if column == IN_PROGRESS => {
            Some(REVIEW)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Reason, UnknownReason};

    fn running() -> SessionState {
        SessionState::Running
    }
    fn done() -> SessionState {
        SessionState::Idle { stop_reason: StopReason::EndTurn }
    }

    #[test]
    fn todo_moves_to_in_progress_on_running() {
        assert_eq!(automove(TODO, &running(), Confidence::High), Some(IN_PROGRESS));
        assert_eq!(automove(TODO, &running(), Confidence::Medium), Some(IN_PROGRESS));
    }

    #[test]
    fn in_progress_moves_to_review_on_end_turn() {
        assert_eq!(automove(IN_PROGRESS, &done(), Confidence::High), Some(REVIEW));
    }

    #[test]
    fn low_or_stale_confidence_never_moves() {
        assert_eq!(automove(TODO, &running(), Confidence::Low), None);
        assert_eq!(automove(IN_PROGRESS, &done(), Confidence::Stale), None);
    }

    #[test]
    fn interrupt_is_not_done() {
        let interrupted = SessionState::Idle { stop_reason: StopReason::Interrupted };
        assert_eq!(automove(IN_PROGRESS, &interrupted, Confidence::High), None);
        let unknown_stop = SessionState::Idle { stop_reason: StopReason::Unknown };
        assert_eq!(automove(IN_PROGRESS, &unknown_stop, Confidence::High), None);
    }

    /// A turn parked on background work is neither done nor working: the
    /// ticket stays in IN PROGRESS until the task lands and the agent really
    /// finishes. Promoting here would put a card in REVIEW that is still
    /// going to change (dogfood 2026-09-01, T-128).
    #[test]
    fn a_parked_turn_is_not_done() {
        let parked = SessionState::Idle { stop_reason: StopReason::Background };
        assert_eq!(automove(IN_PROGRESS, &parked, Confidence::High), None);
        assert_eq!(automove(TODO, &parked, Confidence::High), None);
        assert_eq!(automove(REVIEW, &parked, Confidence::High), None);
    }

    #[test]
    fn requires_action_is_not_working() {
        let modal = SessionState::RequiresAction { reason: Reason::StartupModal };
        assert_eq!(automove(TODO, &modal, Confidence::High), None);
    }

    #[test]
    fn review_reopens_to_in_progress_on_running() {
        assert_eq!(automove(REVIEW, &running(), Confidence::High), Some(IN_PROGRESS));
    }

    #[test]
    fn done_and_non_template_columns_never_move() {
        assert_eq!(automove("DONE", &running(), Confidence::High), None);
        assert_eq!(automove(TODO, &done(), Confidence::High), None);
        assert_eq!(automove(REVIEW, &done(), Confidence::High), None);
        // Non-template columns (future config) are untouched.
        assert_eq!(automove("Backlog", &running(), Confidence::High), None);
    }

    #[test]
    fn other_states_never_move() {
        for s in [
            SessionState::Spawning,
            SessionState::Sleeping,
            SessionState::Throttled,
            SessionState::Unknown { reason: UnknownReason::NoSignal },
        ] {
            assert_eq!(automove(TODO, &s, Confidence::High), None);
            assert_eq!(automove(IN_PROGRESS, &s, Confidence::High), None);
        }
    }
}
