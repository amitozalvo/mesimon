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

use crate::board::{Board, Confidence, MergeInto, SessionState, StopReason, Ticket, TrainReach};

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
    /// A pull request is bound to the ticket, open, merged or closed, on
    /// any repository (T-739, T-778). Read only on a board that merges
    /// through pull requests (T-810): a board that merges into main reads
    /// no forge.
    pub has_pr: bool,
    /// The forge reports a conflict on an open PR.
    pub pr_conflict: bool,
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
    /// Tickets whose PR conflict the forge still reports was already asked
    /// about (T-770): one ask per conflict, whatever the local base did.
    pub conflict_asked: &'a HashSet<ulid::Ulid>,
    pub fused: &'a HashSet<ulid::Ulid>,
    /// Tickets the daemon owes words to (`seat`'s `words_owed`).
    pub owed: &'a HashSet<ulid::Ulid>,
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
    /// At its composer with no turn in flight and nothing owed to it, at a
    /// confidence worth acting on (`at_composer`).
    Idle,
    /// Mid-turn, waiting on the user, interrupted, lost, failed, or owed
    /// words a turn is about to start on.
    Busy,
    /// Sleeping — parked by the user.
    Parked,
}

impl Seat {
    /// The seat lets the train merge past it: nobody there, or an agent at
    /// its composer. The one reading `lane` and `get_ticket`'s merge share
    /// (T-809).
    pub fn lets_merge(self) -> bool {
        matches!(self, Seat::Empty | Seat::Idle)
    }
}

/// `words_owed`: the daemon owes one of the ticket's sessions an ack for
/// words it is pasting (its `owed` ledger) — a launch's brief waiting for
/// the composer, a queued ask in flight. A turn is about to start on them,
/// so the seat is busy whatever the record says (T-809).
pub fn seat(board: &Board, ticket: ulid::Ulid, words_owed: bool) -> Seat {
    if words_owed
        || board
            .sessions
            .iter()
            .any(|session| session.ticket == ticket && crate::quiet::is_working(session))
    {
        return Seat::Busy;
    }
    match board.live_agent(ticket) {
        None => Seat::Empty,
        Some(s) if s.state == SessionState::Sleeping => Seat::Parked,
        Some(s) if at_composer(s) => Seat::Idle,
        Some(_) => Seat::Busy,
    }
}

/// An agent sitting at its composer with no turn in flight (T-809): after
/// an end of turn at High or Medium, or at `Idle{Unknown}` at High — the
/// state a `SessionStart` lands, which is where every relaunch that keeps
/// the conversation comes up (a tier switch, a wake, the mod road's
/// relaunch on the hook set) — with no launch words left undelivered. The
/// owed Enter (`pending_submit`) is `is_working`'s, and words still being
/// pasted are the caller's `words_owed`. A restart's re-derived
/// `Idle{Unknown}` is Low and holds (T-663).
fn at_composer(s: &crate::board::SessionRecord) -> bool {
    match s.state {
        SessionState::Idle { stop_reason: StopReason::EndTurn } => {
            matches!(s.confidence, Confidence::High | Confidence::Medium)
        }
        SessionState::Idle { stop_reason: StopReason::Unknown } => {
            s.confidence == Confidence::High && s.unsent.is_none()
        }
        _ => false,
    }
}

/// Which of the two lists a ticket is on. A branch is behind its base or it
/// is not, so never both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Merge,
    Rebase,
}

/// One ticket as the train reads it. `plan` builds one per ticket from the
/// board and the daemon's memory; the crown's wake (T-554) builds one from a
/// turn's own look, to ask what the train will do with a delivery.
pub struct Reading<'a> {
    /// Where the board's branches land (T-810).
    pub merge_into: MergeInto,
    /// The ticket's column's `train` setting.
    pub reach: TrainReach,
    pub flags: &'a WtFlags,
    pub seat: Seat,
    /// The ticket's base tip now (`""` with no sample yet).
    pub base_tip: &'a str,
    /// The base tip it was last asked to rebase onto.
    pub asked: Option<&'a str>,
    /// The conflict the forge reports on its PR was already asked about.
    pub conflict_asked: bool,
    pub fused: bool,
}

/// The list `t` is on, if any: the one predicate `plan` runs per ticket.
///
/// One workflow per board (T-810). Merging into main, the train is the
/// board before the forge (T-739): it merges a branch ahead and
/// fast-forwardable, and asks a rebase once per base tip. Through pull
/// requests it never merges: the forge does. Its one ask there is the
/// rebase a conflict the forge reports on an open PR needs, once per
/// conflict (T-770); the local base tip is no memo for it, since a forge
/// merge never moves the checkout's base. A branch with no PR yet is asked
/// nothing: the base its PR will target is the forge's, and the forge says
/// whether the PR conflicts once it is opened. So neither mode waits for a
/// forge read before it acts: under main nothing is read, and under pull
/// requests the train makes no merge a read could have to rule out.
pub fn lane(t: &Ticket, r: &Reading) -> Option<Lane> {
    let f = r.flags;
    if !f.attached
        || f.conflict
        || f.merged
        || t.manual_merge
        || !t.effective_execution_policy().allows_automation()
        || t.hand_raised()
    {
        return None;
    }
    let prs = r.merge_into.pull_requests();
    if !prs && r.reach == TrainReach::Merge && f.ahead > 0 && !f.needs_rebase && r.seat.lets_merge()
    {
        return Some(Lane::Merge);
    }
    let owed = if prs {
        f.has_pr && f.pr_conflict && !r.conflict_asked
    } else {
        f.needs_rebase && r.asked.is_none_or(|at| at != r.base_tip)
    };
    (matches!(r.reach, TrainReach::Merge | TrainReach::Rebase)
        && owed
        && f.ahead > 0
        && r.seat == Seat::Idle
        && !r.fused)
        .then_some(Lane::Rebase)
}

/// Everything the train would do, in board order — column order, then row
/// order (automove parks at the top, so the last ticket to finish goes
/// first). Not filtered by who is working: the pass does that, and the
/// snapshot wants the whole list so the cards can say what is coming.
pub fn plan(input: &Input) -> Plan {
    let mut plan = Plan::default();
    for col in input.board.sorted_columns() {
        for t in input.board.column_tickets(&col.name) {
            let Some(flags) = input.flags.get(&t.id) else { continue };
            let reading = Reading {
                merge_into: input.board.merge_into,
                reach: col.settings.train,
                flags,
                seat: seat(input.board, t.id, input.owed.contains(&t.id)),
                base_tip: input.base_tip.get(&t.id).map(String::as_str).unwrap_or(""),
                asked: input.asked.get(&t.id).map(String::as_str),
                conflict_asked: input.conflict_asked.contains(&t.id),
                fused: input.fused.contains(&t.id),
            };
            match lane(t, &reading) {
                Some(Lane::Merge) => plan.merge.push(t.id),
                Some(Lane::Rebase) => plan.rebase.push(t.id),
                None => {}
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
        WtFlags {
            attached: true,
            ahead,
            merged: false,
            needs_rebase,
            conflict: false,
            has_pr: false,
            pr_conflict: false,
        }
    }

    /// T-810: one workflow per board. Into main the train merges a branch
    /// ahead and asks a rebase of one behind, once per base tip, and a PR is
    /// never read. Through pull requests it merges nothing, and its one ask
    /// is the rebase a conflict the forge reports needs: a branch behind its
    /// local base, PR or not, is the forge's.
    #[test]
    fn a_board_merges_into_main_or_through_pull_requests_never_both() {
        let mut b = board();
        b.sessions.push(claude(1, idle(), Confidence::High));
        let t = ticket(1, REVIEW, "a");
        let read = |into: MergeInto, f: &WtFlags| {
            lane(
                &t,
                &Reading {
                    merge_into: into,
                    reach: TrainReach::Merge,
                    flags: f,
                    seat: seat(&b, t.id, false),
                    base_tip: "tip",
                    asked: None,
                    conflict_asked: false,
                    fused: false,
                },
            )
        };
        let ahead = flags(2, false);
        let behind = flags(2, true);
        let conflict = WtFlags { has_pr: true, pr_conflict: true, ..flags(2, true) };
        assert_eq!(read(MergeInto::Main, &ahead), Some(Lane::Merge), "main: the train merges");
        assert_eq!(read(MergeInto::Main, &behind), Some(Lane::Rebase), "and asks the rebase");
        let prs = MergeInto::PullRequests;
        assert_eq!(read(prs, &ahead), None, "pull requests: the forge merges");
        assert_eq!(read(prs, &behind), None, "a branch with no PR yet is asked nothing");
        let open = WtFlags { has_pr: true, ..flags(2, true) };
        assert_eq!(read(prs, &open), None, "a base moved past a PR branch is not a reason");
        assert_eq!(read(prs, &conflict), Some(Lane::Rebase), "a conflict the forge reports is");
        let parked = Reading {
            merge_into: prs,
            reach: TrainReach::Merge,
            flags: &conflict,
            seat: Seat::Busy,
            base_tip: "tip",
            asked: None,
            conflict_asked: false,
            fused: false,
        };
        assert_eq!(lane(&t, &parked), None, "asked only of an idle agent");
    }

    /// T-770: with a PR open the forge's conflict is the reason whatever the
    /// local base says, and it is asked once. The local base tip, which a
    /// forge merge never moves, is no memo either way: a conflict asked
    /// about stays answered when the local base moves, and a ticket asked
    /// at this local tip is asked again about a conflict it was not asked
    /// about.
    #[test]
    fn a_pr_conflict_is_asked_once_whatever_the_local_base_says() {
        let mut b = board();
        b.merge_into = MergeInto::PullRequests;
        b.tickets.push(ticket(1, REVIEW, "a"));
        b.sessions.push(claude(1, idle(), Confidence::High));
        let t = &b.tickets[0];
        let read = |f: &WtFlags, asked: Option<&str>, conflict_asked: bool| {
            lane(
                t,
                &Reading {
                    merge_into: MergeInto::PullRequests,
                    reach: TrainReach::Merge,
                    flags: f,
                    seat: seat(&b, t.id, false),
                    base_tip: "tip1",
                    asked,
                    conflict_asked,
                    fused: false,
                },
            )
        };
        let conflict = WtFlags { has_pr: true, pr_conflict: true, ..flags(1, false) };
        assert_eq!(read(&conflict, None, false), Some(Lane::Rebase), "a clean local base asks");
        assert_eq!(
            read(&conflict, Some("tip1"), false),
            Some(Lane::Rebase),
            "asked at this local tip, but not about this conflict"
        );
        assert_eq!(read(&conflict, None, true), None, "this conflict was asked about");
        assert_eq!(read(&conflict, Some("tip0"), true), None, "a moved local base changes nothing");
        let open = WtFlags { has_pr: true, ..flags(1, true) };
        assert_eq!(read(&open, None, false), None, "a PR behind its base and clean is the forge's");
        // And `plan` reads the same memo.
        let flags = HashMap::from([(t.id, conflict)]);
        let asked_about = HashSet::from([t.id]);
        let p = |conflict_asked: &HashSet<ulid::Ulid>| {
            plan(&Input {
                board: &b,
                flags: &flags,
                base_tip: &tips("tip1"),
                asked: &HashMap::from([(t.id, "tip1".to_string())]),
                conflict_asked,
                fused: &HashSet::new(),
                owed: &HashSet::new(),
            })
        };
        assert_eq!(p(&HashSet::new()).rebase, vec![t.id]);
        assert_eq!(p(&asked_about), Plan::default());
    }

    /// T-778, T-810: on a board that merges through pull requests no
    /// branch is the train's to merge, PR or not, and a base that moved asks
    /// no rebase. A PR merged at another head leaves its branch reading
    /// ahead of a local base the forge never moved, and landing it there
    /// lands the work a second time. The same branches on a board that
    /// merges into main are the train's: `plan` reads the board's mode.
    #[test]
    fn a_board_through_pull_requests_is_never_the_trains_to_merge() {
        let mut b = board();
        b.merge_into = MergeInto::PullRequests;
        b.tickets.push(ticket(1, REVIEW, "a"));
        b.tickets.push(ticket(2, REVIEW, "b"));
        b.sessions.push(claude(1, idle(), Confidence::High));
        let landed = WtFlags { has_pr: true, ..flags(4, false) };
        let behind = WtFlags { has_pr: true, ..flags(4, true) };
        let pr = HashMap::from([(ulid::Ulid(1), landed), (ulid::Ulid(2), behind)]);
        assert_eq!(run(&b, &pr), Plan::default(), "ahead or behind, the forge's");
        let plain =
            HashMap::from([(ulid::Ulid(1), flags(4, false)), (ulid::Ulid(2), flags(4, true))]);
        assert_eq!(run(&b, &plain), Plan::default(), "no PR yet: the forge's all the same");
        b.merge_into = MergeInto::Main;
        assert_eq!(run(&b, &plain).merge, vec![ulid::Ulid(1)], "into main: the train's");
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
            conflict_asked: &HashSet::new(),
            fused: &HashSet::new(),
            owed: &HashSet::new(),
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
            assert_eq!(seat(&b, ulid::Ulid(1), false), want_seat);
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

    /// T-809: an agent relaunched on its conversation (a tier switch, a
    /// wake) comes up at `Idle{Unknown}` from its `SessionStart`, at its
    /// composer with nothing in flight: a free seat, as an end of turn is.
    /// A start whose launch words have not landed is not, nor one whose
    /// words never went, nor a restart's re-derived Low reading (T-663).
    #[test]
    fn a_relaunched_agent_at_its_composer_is_a_free_seat() {
        let unknown = SessionState::Idle { stop_reason: StopReason::Unknown };
        // (state, confidence, unsent, pending_submit, words owed, seat)
        let cases = [
            (unknown.clone(), Confidence::High, false, false, false, Seat::Idle),
            (unknown.clone(), Confidence::Medium, false, false, false, Seat::Busy),
            (unknown.clone(), Confidence::Low, false, false, false, Seat::Busy),
            (unknown.clone(), Confidence::High, true, false, false, Seat::Busy),
            (unknown.clone(), Confidence::High, false, true, false, Seat::Busy),
            (unknown.clone(), Confidence::High, false, false, true, Seat::Busy),
            (idle(), Confidence::High, false, false, true, Seat::Busy),
            (idle(), Confidence::High, true, false, false, Seat::Idle),
        ];
        for (state, conf, unsent, submit, owed, want) in cases {
            let mut b = board();
            b.tickets.push(ticket(1, REVIEW, "a"));
            let mut s = claude(1, state.clone(), conf);
            s.unsent = unsent.then(Default::default);
            s.pending_submit = submit;
            b.sessions.push(s);
            let case = format!("{state:?} {conf:?} unsent={unsent} submit={submit} owed={owed}");
            assert_eq!(seat(&b, ulid::Ulid(1), owed), want, "{case}");
            let owed_set: HashSet<_> = owed.then_some(ulid::Ulid(1)).into_iter().collect();
            let read = |needs_rebase| {
                plan(&Input {
                    board: &b,
                    flags: &HashMap::from([(ulid::Ulid(1), flags(2, needs_rebase))]),
                    base_tip: &tips("tip1"),
                    asked: &HashMap::new(),
                    conflict_asked: &HashSet::new(),
                    fused: &HashSet::new(),
                    owed: &owed_set,
                })
            };
            let free = want == Seat::Idle;
            assert_eq!(read(false).merge.is_empty(), !free, "merges: {case}");
            assert_eq!(read(true).rebase.is_empty(), !free, "rebase asked: {case}");
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
            conflict_asked: &HashSet::new(),
            fused: &HashSet::new(),
            owed: &HashSet::new(),
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
            conflict_asked: &HashSet::new(),
            fused: &HashSet::new(),
            owed: &HashSet::new(),
        });
        assert_eq!(p.rebase, vec![ulid::Ulid(3)]);
        // Fused: excluded from rebase asks only.
        let fused: HashSet<_> = [ulid::Ulid(3)].into_iter().collect();
        let p = plan(&Input {
            board: &b,
            flags: &flags,
            base_tip: &tips("tip1"),
            asked: &HashMap::new(),
            conflict_asked: &HashSet::new(),
            fused: &fused,
            owed: &HashSet::new(),
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

    fn lane_of(f: &WtFlags, reach: TrainReach, seat: Seat, asked: Option<&str>) -> Option<Lane> {
        let t = ticket(1, REVIEW, "a");
        lane(
            &t,
            &Reading {
                merge_into: MergeInto::Main,
                reach,
                flags: f,
                seat,
                base_tip: "tip1",
                asked,
                conflict_asked: false,
                fused: false,
            },
        )
    }

    /// One ticket read alone, as the crown's wake reads a turn's look
    /// (T-554), is on the list `plan` would put it on. A rebase asked at
    /// this base tip that left the branch behind is on neither: the train is
    /// done with it. A branch ready to merge merges only where the column
    /// reaches that far, and never past a parked seat.
    #[test]
    fn one_ticket_is_read_the_way_plan_reads_it() {
        let behind = flags(1, true);
        let ready = flags(1, false);
        assert_eq!(lane_of(&behind, TrainReach::Merge, Seat::Idle, None), Some(Lane::Rebase));
        assert_eq!(
            lane_of(&behind, TrainReach::Merge, Seat::Idle, Some("tip0")),
            Some(Lane::Rebase)
        );
        assert_eq!(lane_of(&behind, TrainReach::Merge, Seat::Idle, Some("tip1")), None);
        assert_eq!(lane_of(&ready, TrainReach::Merge, Seat::Idle, Some("tip1")), Some(Lane::Merge));
        assert_eq!(lane_of(&ready, TrainReach::Merge, Seat::Empty, None), Some(Lane::Merge));
        assert_eq!(lane_of(&ready, TrainReach::Merge, Seat::Parked, None), None);
        assert_eq!(lane_of(&ready, TrainReach::Rebase, Seat::Idle, None), None);
        assert_eq!(lane_of(&ready, TrainReach::Off, Seat::Idle, None), None);
        // And `plan` agrees, over the same ticket on the template board.
        let mut b = board();
        b.tickets.push(ticket(1, REVIEW, "a"));
        b.sessions.push(claude(1, idle(), Confidence::High));
        let flags = HashMap::from([(ulid::Ulid(1), behind)]);
        let asked = HashMap::from([(ulid::Ulid(1), "tip1".to_string())]);
        let p = plan(&Input {
            board: &b,
            flags: &flags,
            base_tip: &tips("tip1"),
            asked: &asked,
            conflict_asked: &HashSet::new(),
            fused: &HashSet::new(),
            owed: &HashSet::new(),
        });
        assert_eq!(p, Plan::default());
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
