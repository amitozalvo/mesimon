//! The merge train's candidate selection (2026-09-04) — pure, so it is
//! tested without git or tmux. The daemon feeds it the board, the worktree
//! flags it already keeps, the base branch's tip and its own memory of what
//! it asked; what comes back is what the train WOULD do, in board order,
//! and the pass takes the first of it once the board is quiet.
//!
//! Two lists — and which columns each reads is the column's own `train`
//! setting (T-117): `Merge` is where the template's REVIEW stands, `Rebase`
//! its IN PROGRESS. `merge`: a REVIEW ticket — where automove lands a finished
//! turn, and where HJKL back to IN PROGRESS opts a ticket out — whose
//! attached branch is ahead and fast-forwardable, with its claude idle after
//! an end of turn or no live claude at all (the card is in REVIEW and there
//! is nobody to wait for: exactly what a hand `m` would merge). `rebase`: an
//! IN PROGRESS or REVIEW ticket whose branch the base moved past and which
//! HAS COMMITS TO REPLAY, with an idle claude to ask, not fused, and not
//! already asked at THIS base tip — an agent that finished and is still
//! behind the same tip said no, and the human's `m` is the road from there.
//! The `ahead > 0` clause is T-351: a branch whose tip never left its
//! creation base has nothing to rebase, and the ask is not free — it spends
//! an agent's whole turn (the words end "run the tests and fix any failures
//! before we merge"), counts against the fuse, and lands the agent on a
//! branch with nothing on it. `merge_ticket` has refused that same state
//! since 2026-08-30 ("no commits on the branch yet — nothing to merge"); the
//! rebase road simply never learned it. A Sleeping claude is the user's
//! parking and never a candidate; `Interrupted` was their Esc. And a ticket
//! marked `manual_merge` (T-227, the `t` key) is on neither list: the user
//! took it off the train, and `m` is the only road for it. Neither is one
//! with a raised hand (T-107): an agent that ended its turn asking for a
//! person is saying a person looks before this goes anywhere, and merging it
//! — or asking it to rebase — would be the automation answering a question
//! addressed to somebody else. The hand also outlasts the ask: lowering it is
//! the person's gesture, so the skip lasts exactly as long as the question.

use std::collections::{HashMap, HashSet};

use crate::board::{Board, Confidence, SessionState, StopReason, TrainReach};

/// The `Principal::Automation { rule }` word, and the feed's.
pub const RULE: &str = "merge_train";

/// What the daemon knows about a ticket's worktree, as the planner needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WtFlags {
    pub attached: bool,
    pub ahead: u32,
    pub merged: bool,
    pub needs_rebase: bool,
    pub conflict: bool,
}

pub struct Input<'a> {
    pub board: &'a Board,
    pub flags: &'a HashMap<ulid::Ulid, WtFlags>,
    /// Each ticket's base tip now — a workspace ticket's is its legs' joined
    /// (T-368) — so an ask recorded at it is not repeated until it moves. A
    /// ticket with no sample yet reads `""`.
    pub base_tip: &'a HashMap<ulid::Ulid, String>,
    /// Ticket → the base tip it was last asked to rebase onto.
    pub asked: &'a HashMap<ulid::Ulid, String>,
    pub fused: &'a HashSet<ulid::Ulid>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub merge: Vec<ulid::Ulid>,
    pub rebase: Vec<ulid::Ulid>,
}

/// The ticket's claude seat, as the train reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    /// No live claude: nobody to wait for, nobody to ask.
    Empty,
    /// Idle after an end of turn, at a confidence worth acting on.
    Idle,
    /// Mid-turn, waiting on the user, interrupted, lost, or failed.
    Busy,
    /// Sleeping — parked by the user.
    Parked,
}

pub fn seat(board: &Board, ticket: ulid::Ulid) -> Seat {
    if board
        .sessions
        .iter()
        .any(|session| session.ticket == ticket && crate::quiet::is_working(session))
    {
        return Seat::Busy;
    }
    match board.live_agent(ticket) {
        None => Seat::Empty,
        Some(s) => match &s.state {
            SessionState::Sleeping => Seat::Parked,
            SessionState::Idle { stop_reason: StopReason::EndTurn }
                if matches!(s.confidence, Confidence::High | Confidence::Medium) =>
            {
                Seat::Idle
            }
            _ => Seat::Busy,
        },
    }
}

/// Everything the train would do, in board order — column order, then row
/// order (automove parks at the top, so the last ticket to finish goes
/// first). Not filtered by who is working: the pass does that, and the
/// snapshot wants the whole list so the cards can say what is coming.
pub fn plan(input: &Input) -> Plan {
    let mut plan = Plan::default();
    for col in input.board.sorted_columns() {
        for t in input.board.column_tickets(&col.name) {
            let Some(f) = input.flags.get(&t.id) else { continue };
            if !f.attached
                || f.conflict
                || f.merged
                || t.manual_merge
                || !t.effective_execution_policy().allows_automation()
                || t.hand_raised()
            {
                continue;
            }
            let seat = seat(input.board, t.id);
            if col.settings.train == TrainReach::Merge
                && f.ahead > 0
                && !f.needs_rebase
                && matches!(seat, Seat::Empty | Seat::Idle)
            {
                plan.merge.push(t.id);
            }
            if matches!(col.settings.train, TrainReach::Merge | TrainReach::Rebase)
                && f.needs_rebase
                && f.ahead > 0
                && seat == Seat::Idle
                && !input.fused.contains(&t.id)
                && input.asked.get(&t.id).is_none_or(|at| {
                    at != input.base_tip.get(&t.id).map(String::as_str).unwrap_or("")
                })
            {
                plan.rebase.push(t.id);
            }
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Column, SessionKind, SessionRecord, Ticket};
    use serde_json::json;

    fn ticket(n: u128, column: &str, order: &str) -> Ticket {
        serde_json::from_value(json!({
            "id": ulid::Ulid(n).to_string(),
            "short_key": format!("T-{n}"),
            "title": "t",
            "column": column,
            "order": order,
            "created_at": "@0",
        }))
        .expect("a ticket from its required fields")
    }

    const IN_PROGRESS: &str = "IN PROGRESS";
    const REVIEW: &str = "REVIEW";

    /// The template board, its columns carrying the template's reach: the
    /// train reads `TrainReach`, never a name, so the names here are only
    /// what `template_settings` keys on.
    fn board() -> Board {
        let mut b = Board::default();
        for (i, name) in ["TODO", IN_PROGRESS, REVIEW, "DONE"].iter().enumerate() {
            b.columns.push(Column::new(*name, format!("{i}")));
        }
        b.seed_template_settings();
        b
    }

    fn claude(ticket: u128, state: SessionState, confidence: Confidence) -> SessionRecord {
        let mut s = SessionRecord::new(
            uuid::Uuid::from_u128(ticket),
            SessionKind::Claude,
            ulid::Ulid(ticket),
            vec!["claude".into()],
            "/wt".into(),
            state,
        );
        s.confidence = confidence;
        s
    }

    fn idle() -> SessionState {
        SessionState::Idle { stop_reason: StopReason::EndTurn }
    }

    fn flags(ahead: u32, needs_rebase: bool) -> WtFlags {
        WtFlags { attached: true, ahead, merged: false, needs_rebase, conflict: false }
    }

    /// Every ticket 1..=4 stands on the same base tip.
    fn tips(tip: &str) -> HashMap<ulid::Ulid, String> {
        (1..=4).map(|n| (ulid::Ulid(n), tip.to_string())).collect()
    }

    fn run(board: &Board, flags: &HashMap<ulid::Ulid, WtFlags>) -> Plan {
        plan(&Input {
            board,
            flags,
            base_tip: &tips("tip1"),
            asked: &HashMap::new(),
            fused: &HashSet::new(),
        })
    }

    #[test]
    fn owner_only_survives_manual_merge_toggle_and_reload() {
        let mut b = board();
        let mut t = ticket(1, REVIEW, "a");
        t.execution_policy = crate::board::ExecutionPolicy::OwnerOnly;
        t.manual_merge = true;
        t.manual_merge = false;
        // The durable policy survives reload even with the user opt-out disabled.
        b.tickets.push(serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap());
        b.sessions.push(claude(1, idle(), Confidence::High));
        for needs_rebase in [false, true] {
            let flags = HashMap::from([(t.id, flags(1, needs_rebase))]);
            assert_eq!(run(&b, &flags), Plan::default());
        }
    }

    #[test]
    fn provenance_keeps_partial_imports_off_the_train() {
        let mut b = board();
        let mut t = ticket(1, REVIEW, "a");
        t.import_origin = Some(crate::content::ImportOrigin {
            source: ulid::Ulid::from(10),
            item: ulid::Ulid::from(20),
        });
        // Missing/default policy cannot widen a ticket carrying import provenance.
        assert!(t.execution_policy.allows_automation());
        assert!(!t.effective_execution_policy().allows_automation());
        b.tickets.push(serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap());
        b.sessions.push(claude(1, idle(), Confidence::High));
        for needs_rebase in [false, true] {
            assert_eq!(run(&b, &HashMap::from([(t.id, flags(1, needs_rebase))])), Plan::default());
        }
    }

    #[test]
    fn merges_in_board_order_from_review_only() {
        let mut b = board();
        b.tickets.push(ticket(1, REVIEW, "b"));
        b.tickets.push(ticket(2, REVIEW, "a"));
        b.tickets.push(ticket(3, IN_PROGRESS, "a"));
        b.tickets.push(ticket(4, "DONE", "a"));
        for n in 1..=4 {
            b.sessions.push(claude(n, idle(), Confidence::High));
        }
        let flags: HashMap<_, _> = (1..=4).map(|n| (ulid::Ulid(n), flags(1, false))).collect();
        let p = run(&b, &flags);
        assert_eq!(p.merge, vec![ulid::Ulid(2), ulid::Ulid(1)], "row order within REVIEW");
        assert!(p.rebase.is_empty());
    }

    #[test]
    fn the_seat_decides() {
        let cases = [
            (None, Seat::Empty, true),
            (Some((idle(), Confidence::High)), Seat::Idle, true),
            (Some((idle(), Confidence::Medium)), Seat::Idle, true),
            (Some((idle(), Confidence::Low)), Seat::Busy, false),
            (
                Some((
                    SessionState::Idle { stop_reason: StopReason::Interrupted },
                    Confidence::High,
                )),
                Seat::Busy,
                false,
            ),
            (
                Some((
                    SessionState::Idle { stop_reason: StopReason::Background },
                    Confidence::High,
                )),
                Seat::Busy,
                false,
            ),
            (Some((SessionState::Running, Confidence::High)), Seat::Busy, false),
            (Some((SessionState::unknown(), Confidence::High)), Seat::Busy, false),
            (Some((SessionState::Sleeping, Confidence::High)), Seat::Parked, false),
        ];
        for (session, want_seat, merges) in cases {
            let mut b = board();
            b.tickets.push(ticket(1, REVIEW, "a"));
            if let Some((state, conf)) = session {
                b.sessions.push(claude(1, state, conf));
            }
            assert_eq!(seat(&b, ulid::Ulid(1)), want_seat);
            let ahead: HashMap<_, _> = [(ulid::Ulid(1), flags(2, false))].into_iter().collect();
            assert_eq!(run(&b, &ahead).merge.is_empty(), !merges, "{want_seat:?}");
            // A rebase needs a claude to ask: an empty seat never rebases.
            let behind: HashMap<_, _> = [(ulid::Ulid(1), flags(2, true))].into_iter().collect();
            assert_eq!(
                run(&b, &behind).rebase.is_empty(),
                want_seat != Seat::Idle,
                "{want_seat:?}"
            );
        }
    }

    #[test]
    fn a_shell_beside_the_claude_changes_nothing() {
        let mut b = board();
        b.tickets.push(ticket(1, REVIEW, "a"));
        b.sessions.push(claude(1, idle(), Confidence::High));
        b.sessions.push(SessionRecord::new(
            uuid::Uuid::from_u128(99),
            SessionKind::Bash,
            ulid::Ulid(1),
            vec!["zsh".into()],
            "/wt".into(),
            SessionState::Running,
        ));
        let flags: HashMap<_, _> = [(ulid::Ulid(1), flags(1, false))].into_iter().collect();
        assert_eq!(run(&b, &flags).merge, vec![ulid::Ulid(1)]);
    }

    #[test]
    fn rebase_asks_go_to_in_progress_and_review_once_per_base_tip() {
        let mut b = board();
        b.tickets.push(ticket(1, "TODO", "a"));
        b.tickets.push(ticket(2, IN_PROGRESS, "a"));
        b.tickets.push(ticket(3, REVIEW, "a"));
        b.tickets.push(ticket(4, "DONE", "a"));
        for n in 1..=4 {
            b.sessions.push(claude(n, idle(), Confidence::High));
        }
        let flags: HashMap<_, _> = (1..=4).map(|n| (ulid::Ulid(n), flags(1, true))).collect();
        assert_eq!(run(&b, &flags).rebase, vec![ulid::Ulid(2), ulid::Ulid(3)]);
        // Asked at this tip: not again. Asked at an older tip: again.
        let asked: HashMap<_, _> =
            [(ulid::Ulid(2), "tip1".to_string()), (ulid::Ulid(3), "tip0".to_string())]
                .into_iter()
                .collect();
        let p = plan(&Input {
            board: &b,
            flags: &flags,
            base_tip: &tips("tip1"),
            asked: &asked,
            fused: &HashSet::new(),
        });
        assert_eq!(p.rebase, vec![ulid::Ulid(3)]);
        // The tip is the TICKET's (T-368): ticket 3's own base moved to
        // `tip2` while ticket 2's stands where it was asked — 3 is asked
        // again, 2 is not, whatever the other's base did.
        let asked: HashMap<_, _> =
            [(ulid::Ulid(2), "tip1".to_string()), (ulid::Ulid(3), "tip1".to_string())]
                .into_iter()
                .collect();
        let mut per_ticket = tips("tip1");
        per_ticket.insert(ulid::Ulid(3), "tip2".into());
        let p = plan(&Input {
            board: &b,
            flags: &flags,
            base_tip: &per_ticket,
            asked: &asked,
            fused: &HashSet::new(),
        });
        assert_eq!(p.rebase, vec![ulid::Ulid(3)]);
        // Fused: excluded from rebase asks only.
        let fused: HashSet<_> = [ulid::Ulid(3)].into_iter().collect();
        let p = plan(&Input {
            board: &b,
            flags: &flags,
            base_tip: &tips("tip1"),
            asked: &HashMap::new(),
            fused: &fused,
        });
        assert_eq!(p.rebase, vec![ulid::Ulid(2)]);
    }

    #[test]
    fn a_conflict_an_unattached_or_a_merged_branch_is_never_a_candidate() {
        let mut b = board();
        b.tickets.push(ticket(1, REVIEW, "a"));
        b.sessions.push(claude(1, idle(), Confidence::High));
        for f in [
            WtFlags { conflict: true, ..flags(1, false) },
            WtFlags { attached: false, ..flags(1, false) },
            WtFlags { merged: true, ..flags(1, false) },
            flags(0, false),
            WtFlags { conflict: true, ..flags(1, true) },
        ] {
            let flags: HashMap<_, _> = [(ulid::Ulid(1), f.clone())].into_iter().collect();
            let p = run(&b, &flags);
            assert!(p.merge.is_empty() && p.rebase.is_empty(), "{f:?}");
        }
        // And a branch that needs a rebase is not merged, it is asked.
        let flags: HashMap<_, _> = [(ulid::Ulid(1), flags(3, true))].into_iter().collect();
        let p = run(&b, &flags);
        assert!(p.merge.is_empty());
        assert_eq!(p.rebase, vec![ulid::Ulid(1)]);
    }

    /// `t` on the card (T-227): the ticket is on neither list, whatever
    /// its branch says, and comes back when the mark comes off.
    #[test]
    fn a_manual_merge_ticket_is_on_neither_list() {
        let mut b = board();
        b.tickets.push(ticket(1, REVIEW, "a"));
        b.tickets.push(ticket(2, IN_PROGRESS, "a"));
        b.sessions.push(claude(1, idle(), Confidence::High));
        b.sessions.push(claude(2, idle(), Confidence::High));
        let flags: HashMap<_, _> =
            [(ulid::Ulid(1), flags(2, false)), (ulid::Ulid(2), flags(1, true))]
                .into_iter()
                .collect();
        let p = run(&b, &flags);
        assert_eq!(p.merge, vec![ulid::Ulid(1)]);
        assert_eq!(p.rebase, vec![ulid::Ulid(2)]);
        for t in b.tickets.iter_mut() {
            t.manual_merge = true;
        }
        let p = run(&b, &flags);
        assert!(p.merge.is_empty() && p.rebase.is_empty(), "{p:?}");
        for t in b.tickets.iter_mut() {
            t.manual_merge = false;
        }
        assert_eq!(run(&b, &flags).merge, vec![ulid::Ulid(1)], "back on the train");
    }

    /// A raised hand (T-107) is on neither list either: an agent that ended
    /// its turn asking for a person is asking for a person, and merging its
    /// branch — or telling it to rebase — would be the automation answering.
    /// The skip lasts exactly as long as the hand, which only a person lowers.
    #[test]
    fn a_ticket_with_a_raised_hand_is_on_neither_list() {
        let mut b = board();
        b.tickets.push(ticket(1, REVIEW, "a"));
        b.tickets.push(ticket(2, IN_PROGRESS, "a"));
        b.sessions.push(claude(1, idle(), Confidence::High));
        b.sessions.push(claude(2, idle(), Confidence::High));
        let flags: HashMap<_, _> =
            [(ulid::Ulid(1), flags(2, false)), (ulid::Ulid(2), flags(1, true))]
                .into_iter()
                .collect();
        assert_eq!(run(&b, &flags).merge, vec![ulid::Ulid(1)]);
        for t in b.tickets.iter_mut() {
            t.raised = Some(crate::board::Raised {
                at: "@1".into(),
                by: "agent:x".into(),
                reason: "is this the right layer?".into(),
            });
        }
        let p = run(&b, &flags);
        assert!(p.merge.is_empty() && p.rebase.is_empty(), "{p:?}");
        for t in b.tickets.iter_mut() {
            t.raised = None;
        }
        assert_eq!(run(&b, &flags).merge, vec![ulid::Ulid(1)], "answered: back on the train");
    }

    /// T-351: a branch the base moved past but with NO COMMITS ON IT has
    /// nothing to replay, and the ask costs a whole agent turn. The same
    /// ticket joins the list the moment it has one commit.
    #[test]
    fn a_branch_with_no_commits_is_never_asked_to_rebase() {
        let mut b = board();
        b.tickets.push(ticket(1, IN_PROGRESS, "a"));
        b.tickets.push(ticket(2, REVIEW, "a"));
        for n in 1..=2 {
            b.sessions.push(claude(n, idle(), Confidence::High));
        }
        // Behind the base, nothing of its own: exactly the state a freshly
        // cut worktree sits in while main moves under it.
        let empty: HashMap<_, _> = (1..=2).map(|n| (ulid::Ulid(n), flags(0, true))).collect();
        assert_eq!(run(&b, &empty), Plan::default());
        // One commit on each and both are candidates again.
        let with_work: HashMap<_, _> = (1..=2).map(|n| (ulid::Ulid(n), flags(1, true))).collect();
        assert_eq!(run(&b, &with_work).rebase, vec![ulid::Ulid(1), ulid::Ulid(2)]);
    }

    #[test]
    fn an_archived_ticket_is_off_the_board() {
        let mut b = board();
        let mut t = ticket(1, REVIEW, "a");
        t.archived = Some(crate::board::Archived {
            at: "@1".into(),
            by: "local".into(),
            until: None,
            needs_you: false,
        });
        b.tickets.push(t);
        let flags: HashMap<_, _> = [(ulid::Ulid(1), flags(1, false))].into_iter().collect();
        assert!(run(&b, &flags).merge.is_empty());
    }
}
