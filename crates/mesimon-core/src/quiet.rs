//! Which tickets are WORKING — the one predicate behind "the checkout is
//! quiet" (a queued ask goes out) and "the board is quiet" (the merge train
//! moves). Pure: the daemon wraps it with its in-flight pastes, and it is
//! the daemon's answer that decides a delivery. The TUI reads `is_working`
//! for one HINT — whether a press that would START or WAKE a session stops to
//! ask first (T-294) — never for delivery: it cannot see the in-flight pastes
//! or the grace band, and where the two disagree the cost is a field that
//! opened where a spawn would have gone. The words a waiting card shows are
//! still the snapshot's `waits_on`.
//!
//! Working means a turn is in progress or about to be: `Spawning`,
//! `Running`, `RequiresAction` (a turn waiting on the user is still that
//! turn), `Idle{Background}` (parked on a task, the turn resumes on its own),
//! or an owed Enter (`pending_submit`). Everything else is quiet — `EndTurn`
//! said its piece, `Interrupted` was the user's own Esc, `Unknown` is a
//! Claude session we lost track of and must not wait for, and a parked or dead
//! record has no turn at all. A SHELL never counts: the daemon pins a Bash
//! session at `Running` for the life of its pane (D15), so there `Running`
//! is liveness, not activity — the same line `glyphs::is_working` draws.
//! Codex additionally holds its checkout while observation is missing, even
//! when its last display state said finished. Unknown Codex state cannot
//! authorize automatic checkout operations; only fresh observation or
//! explicit parking/termination can release the hold.

use std::collections::HashSet;

use crate::board::{Board, SessionKind, SessionRecord, SessionState, StopReason};

/// An agent whose turn is in progress, owed, or cannot be proved quiet.
pub fn is_working(s: &SessionRecord) -> bool {
    s.kind.is_agent()
        && (s.pending_submit
            || (s.kind == SessionKind::Codex
                && (s.codex_stopping || s.state.has_pane())
                && (s.codex_stopping
                    || s.observation_hold
                    || matches!(s.state, SessionState::Unknown { .. })))
            || matches!(
                s.state,
                SessionState::Spawning
                    | SessionState::Running
                    | SessionState::RequiresAction { .. }
                    | SessionState::Idle { stop_reason: StopReason::Background }
            ))
}

/// Tickets with a working claude — deduped, in session order — plus every
/// ticket in `inflight` (a paste mesimon made whose `UserPromptSubmit` has
/// not landed: the turn is coming, the hook just has not said so). `cwd`
/// narrows it to one checkout by string equality: every shared-checkout
/// session carries the same resolved repo root, and a worktree's sessions
/// carry its own path.
pub fn working_tickets(
    board: &Board,
    inflight: &HashSet<ulid::Ulid>,
    cwd: Option<&str>,
) -> Vec<ulid::Ulid> {
    let mut out: Vec<ulid::Ulid> = Vec::new();
    let mut push = |t: ulid::Ulid| {
        if !out.contains(&t) {
            out.push(t);
        }
    };
    for s in &board.sessions {
        if cwd.is_some_and(|c| c != s.cwd) {
            continue;
        }
        if is_working(s) {
            push(s.ticket);
        }
    }
    // A set has no order; the snapshot's `waits_on` must not shuffle.
    let mut flying: Vec<ulid::Ulid> = inflight.iter().copied().collect();
    flying.sort();
    for t in flying {
        let same_checkout =
            cwd.is_none_or(|c| board.sessions.iter().any(|s| s.ticket == t && s.cwd == c));
        if same_checkout {
            push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Confidence, Provenance, Reason, StopReason};

    fn session(
        ticket: ulid::Ulid,
        kind: SessionKind,
        cwd: &str,
        state: SessionState,
    ) -> SessionRecord {
        SessionRecord {
            id: uuid::Uuid::new_v4(),
            kind,
            ticket,
            argv: vec![],
            cwd: cwd.into(),
            state,
            waiting_since: None,
            state_changed_at: None,
            transcript_path: None,
            detail: None,
            title: None,
            confidence: Confidence::High,
            provenance: Provenance::Spawned,
            claude_session_id: None,
            codex_thread_id: None,
            codex_generation: None,
            codex_observed_seq: 0,
            codex_pending_seq: None,
            codex_turn_id: None,
            agent_preview_path: None,
            agent_plan_key: None,
            observation_hold: kind == SessionKind::Codex,
            pending_submit: false,
            pending_prefill: false,
            codex_submit_sent: false,
            codex_stopping: false,
            codex_plan_dialog_seen: false,
            codex_plan_dismissed_turn: None,
            idle_teammates: vec![],
            monitor_task_ids: vec![],
            plan_note: None,
            ticket_read: false,
        }
    }

    fn board_with(sessions: Vec<SessionRecord>) -> Board {
        Board { sessions, ..Board::default() }
    }

    #[test]
    fn a_turn_in_progress_or_owed_is_working_and_a_finished_one_is_not() {
        let t = ulid::Ulid::new();
        let working = [
            SessionState::Spawning,
            SessionState::Running,
            SessionState::RequiresAction { reason: Reason::Permission },
            SessionState::Idle { stop_reason: StopReason::Background },
        ];
        for st in working {
            assert!(is_working(&session(t, SessionKind::Claude, "/r", st.clone())), "{st:?}");
        }
        let quiet = [
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::Idle { stop_reason: StopReason::Interrupted },
            SessionState::Idle { stop_reason: StopReason::Unknown },
            SessionState::Sleeping,
            SessionState::Throttled,
            SessionState::unknown(),
        ];
        for st in quiet {
            assert!(!is_working(&session(t, SessionKind::Claude, "/r", st.clone())), "{st:?}");
        }
        let mut owed = session(
            t,
            SessionKind::Claude,
            "/r",
            SessionState::Idle { stop_reason: StopReason::Unknown },
        );
        owed.pending_submit = true;
        assert!(is_working(&owed), "an owed Enter is a turn about to start");
    }

    #[test]
    fn a_shell_never_works() {
        let t = ulid::Ulid::new();
        assert!(!is_working(&session(t, SessionKind::Bash, "/r", SessionState::Running)));
    }

    #[test]
    fn codex_observation_loss_holds_a_finished_checkout_until_reconciled() {
        let mut rec = session(
            ulid::Ulid::new(),
            SessionKind::Codex,
            "/r",
            SessionState::Idle { stop_reason: StopReason::EndTurn },
        );
        assert!(is_working(&rec), "a stale completion cannot release a checkout");
        rec.observation_hold = false;
        assert!(!is_working(&rec), "fresh completion can release it");
        rec.state = SessionState::Idle { stop_reason: StopReason::Unknown };
        assert!(!is_working(&rec), "a reconciled empty input box has no active turn");
        for state in [
            SessionState::unknown(),
            SessionState::Running,
            SessionState::RequiresAction { reason: Reason::Permission },
        ] {
            rec.state = state;
            assert!(is_working(&rec), "{:?}", rec.state);
        }
        rec.observation_hold = true;
        for state in [
            SessionState::Sleeping,
            SessionState::Exited { reason: crate::board::ExitReason::Killed },
        ] {
            rec.state = state;
            assert!(!is_working(&rec), "confirmed stopped sessions release the hold");
            rec.codex_stopping = true;
            assert!(is_working(&rec), "cleanup still owns the checkout even after parking");
            rec.codex_stopping = false;
        }
    }

    #[test]
    fn the_checkout_filter_is_the_cwd_string_and_inflight_rides_along() {
        let a = ulid::Ulid::new();
        let b = ulid::Ulid::new();
        let c = ulid::Ulid::new();
        let board = board_with(vec![
            session(a, SessionKind::Claude, "/repo", SessionState::Running),
            session(b, SessionKind::Claude, "/wt/b", SessionState::Running),
            session(
                c,
                SessionKind::Claude,
                "/repo",
                SessionState::Idle { stop_reason: StopReason::EndTurn },
            ),
            session(a, SessionKind::Bash, "/repo", SessionState::Running),
        ]);
        let none = HashSet::new();
        assert_eq!(working_tickets(&board, &none, None), vec![a, b]);
        assert_eq!(working_tickets(&board, &none, Some("/repo")), vec![a]);
        assert_eq!(working_tickets(&board, &none, Some("/wt/b")), vec![b]);
        assert!(working_tickets(&board, &none, Some("/elsewhere")).is_empty());
        let inflight: HashSet<ulid::Ulid> = [c].into_iter().collect();
        assert_eq!(working_tickets(&board, &inflight, Some("/repo")), vec![a, c]);
        assert_eq!(
            working_tickets(&board, &inflight, Some("/wt/b")),
            vec![b],
            "c's paste is in another checkout"
        );
        assert_eq!(working_tickets(&board, &inflight, None), vec![a, b, c]);
    }
}
