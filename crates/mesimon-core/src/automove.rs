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
//! watch-only parking (`Idle{Monitoring}`) is neither. Agent-backed parking
//! (`Idle{Background}`) triggers `on_working`. Low/Stale confidence never
//! moves a ticket — the observe tier may misread a transcript, and a wrong
//! card position is a lie the user has to undo by hand.

use crate::board::{Board, ColumnSettings, Confidence, SessionState, StopReason};

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
        SessionState::Running | SessionState::Idle { stop_reason: StopReason::Background } => {
            column.on_working.as_deref()
        }
        SessionState::Idle { stop_reason: StopReason::EndTurn } => column.on_done.as_deref(),
        _ => return MoveDecision { outcome: "state_does_not_trigger_move", destination: None },
    };
    MoveDecision {
        outcome: if destination.is_some() { "eligible" } else { "no_column_rule" },
        destination,
    }
}

/// The column a ticket's agent delivers in (T-795): where the turn in
/// flight leaves the card when it ends, by the rules above — `on_working`
/// first for a turn not yet begun, then `on_done`. An agent with no turn in
/// flight delivers where the card sits. Who merges a working ticket is
/// read here, not off the column it is worked in: a worker in IN PROGRESS
/// delivers in REVIEW, where the train merges.
pub fn delivers_in<'a>(board: &'a Board, column: &'a str, state: &SessionState) -> &'a str {
    let rule = |at: &'a str, pick: fn(&ColumnSettings) -> &Option<String>| {
        board.column(at).and_then(|c| pick(&c.settings).as_deref()).unwrap_or(at)
    };
    match state {
        // A stop on a limit (T-808) ended its turn: nothing in flight.
        s if s.stopped_on_limit() => column,
        SessionState::Spawning => rule(rule(column, |s| &s.on_working), |s| &s.on_done),
        SessionState::Running
        | SessionState::RequiresAction { .. }
        | SessionState::Idle { stop_reason: StopReason::Background } => {
            rule(column, |s| &s.on_done)
        }
        _ => column,
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

    /// Watch-only work neither claims active work nor fabricates completion.
    #[test]
    fn a_parked_turn_is_not_done() {
        let parked = SessionState::Idle { stop_reason: StopReason::Monitoring };
        for c in ["TODO", "IN PROGRESS", "REVIEW"] {
            assert_eq!(automove(&col(c), &parked, Confidence::High), None);
        }
    }

    #[test]
    fn background_agents_trigger_working_but_monitors_never_do() {
        let work = SessionState::Idle { stop_reason: StopReason::Background };
        assert_eq!(automove(&col("REVIEW"), &work, Confidence::High), Some("IN PROGRESS"));
        assert_eq!(automove(&col("REVIEW"), &work, Confidence::Low), None);
        assert_eq!(automove(&col("IN PROGRESS"), &work, Confidence::High), None);
    }

    #[test]
    fn requires_action_is_not_working() {
        let modal = SessionState::RequiresAction { reason: Reason::StartupModal };
        assert_eq!(automove(&col("TODO"), &modal, Confidence::High), None);
    }

    /// T-808: a stop on a usage limit ended the turn, and its work is not
    /// done: no `on_done`, and no `on_working` either.
    #[test]
    fn a_usage_limit_is_not_done() {
        let quota = SessionState::RequiresAction { reason: Reason::QuotaResume };
        for c in ["TODO", "IN PROGRESS", "REVIEW"] {
            assert_eq!(automove(&col(c), &quota, Confidence::High), None, "{c}");
        }
    }

    /// A turn in flight delivers where its end puts the card (T-795): the
    /// template's worker in IN PROGRESS delivers in REVIEW, one starting in
    /// TODO takes both hops, and a turn already ended stays put.
    #[test]
    fn a_turn_in_flight_delivers_where_its_end_lands() {
        let board = Board::with_default_columns();
        let at = |column, state: SessionState| delivers_in(&board, column, &state).to_string();
        assert_eq!(at("IN PROGRESS", running()), "REVIEW");
        assert_eq!(at("IN PROGRESS", SessionState::Spawning), "REVIEW");
        assert_eq!(at("TODO", SessionState::Spawning), "REVIEW");
        let question = SessionState::RequiresAction { reason: Reason::Question };
        assert_eq!(at("IN PROGRESS", question), "REVIEW");
        let quota = SessionState::RequiresAction { reason: Reason::QuotaResume };
        assert_eq!(at("IN PROGRESS", quota), "IN PROGRESS", "a usage limit ended the turn");
        assert_eq!(at("IN PROGRESS", SessionState::Throttled), "IN PROGRESS");
        let tasks = SessionState::Idle { stop_reason: StopReason::Background };
        assert_eq!(at("IN PROGRESS", tasks), "REVIEW");
        // No turn in flight, or a column with no rule: where it sits.
        assert_eq!(at("IN PROGRESS", done()), "IN PROGRESS");
        assert_eq!(at("IN PROGRESS", SessionState::Sleeping), "IN PROGRESS");
        assert_eq!(at("TODO", running()), "TODO");
        assert_eq!(at("DONE", running()), "DONE");
        assert_eq!(at("GONE", running()), "GONE");
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
