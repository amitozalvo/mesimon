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
//!
//! `is_mid_turn` is the one named narrowing: working minus the two clauses
//! that are about a checkout and not about the machine — `RequiresAction`
//! and a record with no pane — for the question "is the machine doing
//! anything" as against "is this turn over" (T-288's keep-awake hold). It is
//! written in terms of `is_working`, so a fifth working state — or another
//! agent kind — joins both answers at once.

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

/// A turn actually IN PROGRESS: `is_working` minus the two clauses that
/// own a CHECKOUT without keeping the MACHINE busy. A permission prompt is
/// stopped on the user, not on the machine, so nothing is lost by letting
/// the machine idle underneath it — which is the difference that matters to
/// T-288's keep-awake hold, and to nothing else so far. And a record with no
/// pane has no turn: a Codex record that is `Sleeping` or `Exited` with
/// `codex_stopping` still owns its checkout until its server confirms
/// cleanup (that is `is_working`'s answer, and the seat's), but a runtime
/// that crashed without ever confirming would hold a laptop awake forever
/// (T-357: a dismissed record on a deleted ticket did exactly that). Written
/// in terms of `is_working` on purpose: the two can then never disagree about
/// what a turn is, only about whether this one is waiting for you or has no
/// process left to wait for. (`glyphs::is_working` is narrower again —
/// `Running` alone — because a spinner may only turn for something moving.)
///
/// It inherits the Codex observation hold above, and should: a session with
/// a pane that cannot be proved quiet errs AWAKE here, which is the direction
/// a keep-awake hold exists to protect.
pub fn is_mid_turn(s: &SessionRecord) -> bool {
    is_working(s) && !matches!(s.state, SessionState::RequiresAction { .. }) && s.state.has_pane()
}

/// The pure half of the daemon's sleep gate — kind × state, nothing the
/// board cannot see. `Err` is the refusal's clause, and the daemon's
/// `sleep_eligible` starts here before it adds what only it knows (the bulk
/// sweep's age floor, the Codex observation hold, a shell's live children).
/// The TUI's `snooze_blocked` asks the same function at the first `z`, so
/// the chord never arms for an Enter the daemon would refuse and never
/// refuses in words the daemon would not — a new clause (a pin, a floor)
/// lands in both at once (T-249).
pub fn sleep_eligible(kind: SessionKind, state: &SessionState) -> Result<(), &'static str> {
    match (kind, state) {
        (SessionKind::Claude | SessionKind::Codex, SessionState::Idle { .. }) => Ok(()),
        (SessionKind::Claude | SessionKind::Codex, _) => Err("only idle sessions sleep"),
        // Bash has no hook surface: Running IS its only live state, so the
        // manual path accepts it — the daemon then checks for live children.
        (SessionKind::Bash, SessionState::Running) => Ok(()),
        (SessionKind::Bash, _) => Err("no live shell to sleep"),
    }
}

/// The sentence a snooze refuses in, over a session `sleep_eligible` (or
/// the daemon's fuller gate) would not sleep: `{who} still awake — {why}`.
/// One place, so the TUI's pre-judgement and the daemon's answer are the
/// same words.
pub fn still_awake(kind: SessionKind, why: &str) -> String {
    let who = if kind.is_agent() { crate::keymap::AGENT_WORD } else { "shell" };
    format!("{who} still awake — {why}")
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

    /// The kind × state clause the daemon's gate and the TUI's `z` share:
    /// an agent sleeps only idle, a shell only running, and the refusal
    /// sentence names who is awake in the board's own word.
    #[test]
    fn sleep_eligible_is_kind_times_state_and_names_who() {
        let idle = SessionState::Idle { stop_reason: StopReason::EndTurn };
        for kind in [SessionKind::Claude, SessionKind::Codex] {
            assert_eq!(sleep_eligible(kind, &idle), Ok(()));
            assert_eq!(
                sleep_eligible(kind, &SessionState::Running),
                Err("only idle sessions sleep")
            );
            assert_eq!(
                sleep_eligible(kind, &SessionState::Spawning),
                Err("only idle sessions sleep")
            );
        }
        assert_eq!(sleep_eligible(SessionKind::Bash, &SessionState::Running), Ok(()));
        assert_eq!(sleep_eligible(SessionKind::Bash, &idle), Err("no live shell to sleep"));
        assert_eq!(
            still_awake(SessionKind::Claude, "only idle sessions sleep"),
            format!("{} still awake — only idle sessions sleep", crate::keymap::AGENT_WORD)
        );
        assert_eq!(
            still_awake(SessionKind::Bash, "bash has live children (vim)"),
            "shell still awake — bash has live children (vim)"
        );
    }

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
            started_by: None,
            foreground: None,
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
            background_tasks: Default::default(),
            plan_note: None,
            ticket_read: false,
        }
    }

    fn board_with(sessions: Vec<SessionRecord>) -> Board {
        Board { sessions, ..Board::default() }
    }

    #[test]
    fn monitoring_releases_working_and_keep_awake_holds() {
        let mut s = session(
            ulid::Ulid::new(),
            SessionKind::Claude,
            "/tmp",
            SessionState::Idle { stop_reason: StopReason::Monitoring },
        );
        assert!(!is_working(&s));
        assert!(!is_mid_turn(&s));
        s.state = SessionState::Idle { stop_reason: StopReason::Background };
        assert!(is_working(&s));
        assert!(is_mid_turn(&s));
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
    fn only_the_wait_on_a_person_is_not_mid_turn() {
        let t = ulid::Ulid::new();
        let mid = [
            SessionState::Spawning,
            SessionState::Running,
            SessionState::Idle { stop_reason: StopReason::Background },
        ];
        for st in mid {
            let s = session(t, SessionKind::Claude, "/r", st.clone());
            assert!(is_mid_turn(&s), "{st:?}");
        }
        let waiting = session(
            t,
            SessionKind::Claude,
            "/r",
            SessionState::RequiresAction { reason: Reason::Permission },
        );
        assert!(is_working(&waiting), "it is still that turn");
        assert!(!is_mid_turn(&waiting), "but it is waiting on a person, not on the machine");
        let mut owed = session(
            t,
            SessionKind::Claude,
            "/r",
            SessionState::Idle { stop_reason: StopReason::Unknown },
        );
        owed.pending_submit = true;
        assert!(is_mid_turn(&owed), "an owed Enter is a turn about to start");
        for st in [
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::Sleeping,
            SessionState::unknown(),
        ] {
            assert!(!is_mid_turn(&session(t, SessionKind::Claude, "/r", st.clone())), "{st:?}");
        }
        assert!(
            !is_mid_turn(&session(t, SessionKind::Bash, "/r", SessionState::Running)),
            "a shell is liveness, not activity — here as everywhere"
        );
    }

    /// The narrowing is written over `is_working`, so it answers for every
    /// agent kind — including the hold Codex takes when it cannot be
    /// observed, which reads as mid-turn on purpose: unprovable-quiet errs
    /// AWAKE, and awake is the safe direction for a keep-awake hold.
    #[test]
    fn the_narrowing_answers_for_codex_too() {
        let t = ulid::Ulid::new();
        let running = session(t, SessionKind::Codex, "/r", SessionState::Running);
        assert!(is_mid_turn(&running), "an agent mid-turn is one whatever its vendor");
        // A pane and no fresh observation: `is_working`'s Codex clause holds
        // the checkout, and the machine stays up under it.
        let unobserved = session(
            t,
            SessionKind::Codex,
            "/r",
            SessionState::Idle { stop_reason: StopReason::EndTurn },
        );
        assert!(unobserved.observation_hold, "a fresh Codex record starts unobserved");
        assert!(is_working(&unobserved) && is_mid_turn(&unobserved));
        // But a turn stopped on a PERSON is still stopped on a person, hold
        // or no hold: that is the one clause this predicate exists for.
        let waiting = session(
            t,
            SessionKind::Codex,
            "/r",
            SessionState::RequiresAction { reason: Reason::Permission },
        );
        assert!(is_working(&waiting));
        assert!(!is_mid_turn(&waiting));
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
    fn cleanup_with_no_pane_owns_the_checkout_but_is_not_mid_turn() {
        // T-357: a Codex runtime that crashed before confirming cleanup keeps
        // `codex_stopping` for good. The checkout stays owned — that is the
        // doctrine — but no pane means no turn, so the keep-awake hold lets go.
        let mut rec = session(
            ulid::Ulid::new(),
            SessionKind::Codex,
            "/r",
            SessionState::Exited { reason: crate::board::ExitReason::Dismissed },
        );
        rec.codex_stopping = true;
        for state in [
            SessionState::Exited { reason: crate::board::ExitReason::Dismissed },
            SessionState::Exited { reason: crate::board::ExitReason::Crashed },
            SessionState::Sleeping,
        ] {
            rec.state = state;
            assert!(is_working(&rec), "{:?} still owns the checkout", rec.state);
            assert!(!is_mid_turn(&rec), "{:?} has no process to keep awake for", rec.state);
        }
        // With a pane the same flag is a runtime winding down: still mid-turn.
        rec.state = SessionState::Running;
        assert!(is_mid_turn(&rec));
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
