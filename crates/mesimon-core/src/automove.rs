//! Automove: session state transitions drag the ticket along the board, by
//! the rules ITS COLUMN carries (T-117). Before that the rules were three
//! column-name literals here — TODO/REVIEW → IN PROGRESS on `Running`,
//! IN PROGRESS → REVIEW on `Idle{EndTurn}` — and renaming a column silently
//! switched them off. Now `ColumnSettings::on_working` and `on_done` ARE the
//! rules, `board::template_settings` seeds the same three onto the template
//! columns, and no column name is compared to a literal anywhere.
//!
//! Deliberately NOT rules: an interrupt (`Idle{Interrupted}`) is not "done";
//! `RequiresAction` is not "working" (a trust/startup modal fires before any
//! work happens — a mid-turn permission ask was preceded by `Running` anyway);
//! a parked turn (`Idle{Background}`) is neither. Low/Stale confidence never
//! moves a ticket — the observe tier may misread a transcript, and a wrong
//! card position is a lie the user has to undo by hand.

use crate::board::{ColumnSettings, Confidence, SessionState, StopReason};

/// The destination column a session-state transition drags the ticket to,
/// read off the settings of the column it sits in now. `None` = stay put.
pub fn automove<'a>(
    column: &'a ColumnSettings,
    to: &SessionState,
    confidence: Confidence,
) -> Option<&'a str> {
    explain(column, to, confidence).destination
}

#[derive(Debug, serde::Serialize)]
pub struct MoveDecision<'a> {
    pub outcome: &'static str,
    pub destination: Option<&'a str>,
}

/// The same rule produces the decision and the explanation, including no move.
pub fn explain<'a>(
    column: &'a ColumnSettings,
    to: &SessionState,
    confidence: Confidence,
) -> MoveDecision<'a> {
    if matches!(confidence, Confidence::Low | Confidence::Stale) {
        return MoveDecision { outcome: "insufficient_confidence", destination: None };
    }
    let destination = match to {
        SessionState::Running => column.on_working.as_deref(),
        SessionState::Idle { stop_reason: StopReason::EndTurn } => column.on_done.as_deref(),
        _ => return MoveDecision { outcome: "state_does_not_trigger_move", destination: None },
    };
    MoveDecision {
        outcome: if destination.is_some() { "eligible" } else { "no_column_rule" },
        destination,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{template_settings, Reason, UnknownReason};

    fn running() -> SessionState {
        SessionState::Running
    }
    fn done() -> SessionState {
        SessionState::Idle { stop_reason: StopReason::EndTurn }
    }
    fn col(name: &str) -> ColumnSettings {
        template_settings(name).expect("a template column")
    }

    /// The template's three rules, exactly as they were when they were
    /// literals here.
    #[test]
    fn the_template_rules_survive_on_the_columns() {
        assert_eq!(automove(&col("TODO"), &running(), Confidence::High), Some("IN PROGRESS"));
        assert_eq!(automove(&col("TODO"), &running(), Confidence::Medium), Some("IN PROGRESS"));
        assert_eq!(automove(&col("IN PROGRESS"), &done(), Confidence::High), Some("REVIEW"));
        assert_eq!(automove(&col("REVIEW"), &running(), Confidence::High), Some("IN PROGRESS"));
        // The edges the template does not wire.
        assert_eq!(automove(&col("DONE"), &running(), Confidence::High), None);
        assert_eq!(automove(&col("DONE"), &done(), Confidence::High), None);
        assert_eq!(automove(&col("TODO"), &done(), Confidence::High), None);
        assert_eq!(automove(&col("REVIEW"), &done(), Confidence::High), None);
    }

    #[test]
    fn a_column_with_no_rules_never_moves() {
        let plain = ColumnSettings::default();
        assert_eq!(automove(&plain, &running(), Confidence::High), None);
        assert_eq!(automove(&plain, &done(), Confidence::High), None);
    }

    #[test]
    fn any_column_can_carry_a_rule() {
        let s = ColumnSettings { on_done: Some("QA".into()), ..Default::default() };
        assert_eq!(automove(&s, &done(), Confidence::High), Some("QA"));
        assert_eq!(automove(&s, &running(), Confidence::High), None);
    }

    #[test]
    fn low_or_stale_confidence_never_moves() {
        assert_eq!(automove(&col("TODO"), &running(), Confidence::Low), None);
        assert_eq!(automove(&col("IN PROGRESS"), &done(), Confidence::Stale), None);
    }

    #[test]
    fn interrupt_is_not_done() {
        let interrupted = SessionState::Idle { stop_reason: StopReason::Interrupted };
        assert_eq!(automove(&col("IN PROGRESS"), &interrupted, Confidence::High), None);
        let unknown_stop = SessionState::Idle { stop_reason: StopReason::Unknown };
        assert_eq!(automove(&col("IN PROGRESS"), &unknown_stop, Confidence::High), None);
    }

    /// A turn parked on background work is neither done nor working: the
    /// ticket stays in IN PROGRESS until the task lands and the agent really
    /// finishes. Promoting here would put a card in REVIEW that is still
    /// going to change (dogfood 2026-09-01, T-128).
    #[test]
    fn a_parked_turn_is_not_done() {
        let parked = SessionState::Idle { stop_reason: StopReason::Background };
        for c in ["TODO", "IN PROGRESS", "REVIEW"] {
            assert_eq!(automove(&col(c), &parked, Confidence::High), None);
        }
    }

    #[test]
    fn requires_action_is_not_working() {
        let modal = SessionState::RequiresAction { reason: Reason::StartupModal };
        assert_eq!(automove(&col("TODO"), &modal, Confidence::High), None);
    }

    #[test]
    fn other_states_never_move() {
        for s in [
            SessionState::Spawning,
            SessionState::Sleeping,
            SessionState::Throttled,
            SessionState::Unknown { reason: UnknownReason::NoSignal },
        ] {
            assert_eq!(automove(&col("TODO"), &s, Confidence::High), None);
            assert_eq!(automove(&col("IN PROGRESS"), &s, Confidence::High), None);
        }
    }
}
