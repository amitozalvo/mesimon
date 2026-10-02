//! Where the crown's agents work (T-583), judged without I/O.
//!
//! The crown filed two tickets and started both with nothing said about a
//! workspace, so both took the board's default, the shared checkout, and two
//! agents were set to edit one tree at once. So the crown names a workspace
//! on every start, and on every ticket it files, and the daemon holds it to
//! three rules here:
//!
//! 1. **A ticket nobody has started takes the choice.** One with a worktree,
//!    or with a parked agent, already has a workspace, and a start naming the
//!    other one is refused in words naming what it has. A parked agent's
//!    workspace is its record's cwd, not the ticket's field: a wake replays
//!    the cwd it was parked in (`resume_session` never relocates), and the
//!    field may have moved under a parked record since.
//! 2. **The crown wakes what it parked.** A start on a ticket whose only
//!    agent sleeps and was started by the crown is that agent's wake; a
//!    person's parked agent stays the person's.
//! 3. **No second agent on a held checkout.** A `shared_checkout` start is
//!    refused while another ticket's agent is live there — working, idle, or
//!    parked with the checkout as its cwd, since a wake runs it there. A
//!    person's own start is the person's decision and is never refused by
//!    this; the rule is for the crown and the daemon's own automation. The
//!    crown's own ticket is the caller, not a holder: it coordinates, and a
//!    crown on the checkout would otherwise never start a checkout worker.

use crate::board::{Board, SessionRecord, WorkspaceStrategy};
use crate::principal::Principal;

/// A workspace as the tools spell it: `worktree` or `shared_checkout`. An
/// adopted worktree is a person's road and no tool names it.
pub fn parse_workspace(word: &str) -> Result<WorkspaceStrategy, String> {
    match word.trim() {
        "worktree" => Ok(WorkspaceStrategy::Worktree),
        "shared_checkout" => Ok(WorkspaceStrategy::SharedCheckout),
        other => Err(format!("workspace is worktree or shared_checkout, not {other}")),
    }
}

/// `parse_workspace`'s inverse, and the word `get_ticket` shows.
pub fn workspace_word(ws: WorkspaceStrategy) -> &'static str {
    match ws {
        WorkspaceStrategy::Worktree => "worktree",
        WorkspaceStrategy::SharedCheckout => "shared_checkout",
        WorkspaceStrategy::AdoptExisting => "adopt_existing",
    }
}

/// Where a record's next wake runs: the checkout when its cwd is the
/// checkout, a worktree otherwise. A worktree that was reclaimed under a
/// parked record is rebuilt by the wake (T-278), so a cwd that is gone still
/// reads as the worktree it was.
pub fn runs_in(rec: &SessionRecord, checkout: &str) -> WorkspaceStrategy {
    if rec.cwd == checkout {
        WorkspaceStrategy::SharedCheckout
    } else {
        WorkspaceStrategy::Worktree
    }
}

/// One ticket holding the checkout: its key and its agent's state word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub key: String,
    pub state: &'static str,
}

/// The tickets whose agent holds the shared checkout, besides `besides`:
/// an agent seat that is live — working, idle or parked — with the checkout
/// as its cwd, on a ticket the board shows. An archived ticket's parked
/// agent is off the board and wakes only after a person restores it.
pub fn checkout_holders(board: &Board, checkout: &str, besides: &[ulid::Ulid]) -> Vec<Holder> {
    let mut out: Vec<Holder> = Vec::new();
    for s in &board.sessions {
        if !s.holds_agent_seat() || s.cwd != checkout || besides.contains(&s.ticket) {
            continue;
        }
        let Some(t) = board.ticket(s.ticket).filter(|t| !t.is_archived()) else { continue };
        if !out.iter().any(|h| h.key == t.short_key) {
            out.push(Holder { key: t.short_key.clone(), state: s.state_word() });
        }
    }
    out
}

/// Why `by` may not put an agent on the shared checkout `holders` hold, if
/// it may not: who works there, as a clause each road finishes with its own
/// remedy. A person decides for themselves and is never refused here.
pub fn checkout_refusal(by: &Principal, holders: &[Holder]) -> Option<String> {
    if by.is_human() || holders.is_empty() {
        return None;
    }
    let names: Vec<String> = holders.iter().map(|h| format!("{} ({})", h.key, h.state)).collect();
    let (who, verb) = match names.as_slice() {
        [one] => (one.clone(), "works"),
        [rest @ .., last] => (format!("{} and {last}", rest.join(", ")), "work"),
        [] => unreachable!("holders is not empty"),
    };
    Some(format!("{who} {verb} on this checkout"))
}

/// What the crown's `start_agent` does on a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// A fresh agent. `apply` says the ticket's workspace has to be set to
    /// the crown's choice first, as `set_workspace` would.
    Spawn { apply: bool },
    /// The agent the crown started and parked, woken where it was parked.
    Wake(uuid::Uuid),
}

/// The crown's start on `target` in `wanted`, judged (T-583): what to do,
/// or the refusal in words. `own` is the crown's own ticket, `has_worktree`
/// whether `target` has a worktree binding, `checkout` the shared checkout's
/// path as a record's cwd spells it. The budget and the plan flag are the
/// daemon's to judge after this; nothing here mutates.
pub fn judge(
    board: &Board,
    own: ulid::Ulid,
    target: ulid::Ulid,
    wanted: WorkspaceStrategy,
    has_worktree: bool,
    checkout: &str,
) -> Result<Start, String> {
    let Some(t) = board.ticket(target) else { return Err("no such ticket".into()) };
    let key = t.short_key.as_str();
    let start = match board.live_agent(target) {
        Some(rec) if rec.state.has_pane() => {
            return Err(format!(
                "{key} already has an agent ({}); one agent per ticket",
                rec.state_word()
            ))
        }
        Some(rec) if rec.started_by.is_none() => {
            return Err(format!(
                "{key}'s agent was started by a person and is parked; a person wakes it \
                 (c on its card)"
            ))
        }
        Some(rec) => {
            let has = runs_in(rec, checkout);
            if has != wanted {
                return Err(format!(
                    "{key}'s parked agent works {}, and a wake runs it there: workspace {}",
                    match has {
                        WorkspaceStrategy::SharedCheckout => "on the shared checkout",
                        _ => "in its worktree",
                    },
                    workspace_word(has)
                ));
            }
            Start::Wake(rec.id)
        }
        None if has_worktree => {
            if wanted != WorkspaceStrategy::Worktree {
                return Err(format!(
                    "{key} has a worktree; start it there (workspace worktree) or archive it"
                ));
            }
            Start::Spawn { apply: false }
        }
        None => Start::Spawn { apply: t.workspace_strategy() != wanted },
    };
    if wanted == WorkspaceStrategy::SharedCheckout {
        let holders = checkout_holders(board, checkout, &[own, target]);
        let crown = Principal::Agent { session: uuid::Uuid::nil() };
        if let Some(who) = checkout_refusal(&crown, &holders) {
            return Err(format!("{who}; use worktree, or wait"));
        }
    }
    Ok(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{SessionKind, SessionState, StopReason, Ticket};
    use ulid::Ulid;

    const CHECKOUT: &str = "/repo";
    const CROWN: u128 = 1;
    const TARGET: u128 = 2;

    fn ticket(n: u128) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": Ulid(n).to_string(),
            "short_key": format!("T-{n}"),
            "title": "t",
            "column": "TODO",
            "order": format!("{n}"),
            "created_at": "@0",
        }))
        .expect("a ticket from its required fields")
    }

    fn agent(n: u128, cwd: &str, state: SessionState, by_crown: bool) -> SessionRecord {
        let mut s = SessionRecord::new(
            uuid::Uuid::from_u128(n),
            SessionKind::Claude,
            Ulid(n),
            vec!["claude".into()],
            cwd.into(),
            state,
        );
        s.started_by = by_crown.then_some(Ulid(CROWN));
        s
    }

    fn idle() -> SessionState {
        SessionState::Idle { stop_reason: StopReason::EndTurn }
    }

    /// Five tickets, the crown's on the checkout and working.
    fn board() -> Board {
        let mut b = Board::default();
        for n in 1..=5 {
            b.tickets.push(ticket(n));
        }
        b.sessions.push(agent(CROWN, CHECKOUT, SessionState::Running, false));
        b
    }

    fn judge_on(b: &Board, wanted: WorkspaceStrategy, has_worktree: bool) -> Result<Start, String> {
        judge(b, Ulid(CROWN), Ulid(TARGET), wanted, has_worktree, CHECKOUT)
    }

    #[test]
    fn the_two_words_parse_and_nothing_else() {
        assert_eq!(parse_workspace("worktree"), Ok(WorkspaceStrategy::Worktree));
        assert_eq!(parse_workspace(" shared_checkout "), Ok(WorkspaceStrategy::SharedCheckout));
        assert_eq!(
            parse_workspace("adopt_existing"),
            Err("workspace is worktree or shared_checkout, not adopt_existing".into())
        );
        for ws in [WorkspaceStrategy::Worktree, WorkspaceStrategy::SharedCheckout] {
            assert_eq!(parse_workspace(workspace_word(ws)), Ok(ws));
        }
    }

    /// An unstarted ticket takes the crown's choice: applied when it differs
    /// from the ticket's own, left alone when it is the same.
    #[test]
    fn an_unstarted_ticket_takes_the_choice() {
        let mut b = board();
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::Worktree, false),
            Ok(Start::Spawn { apply: true }),
            "the board's default is the checkout"
        );
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::SharedCheckout, false),
            Ok(Start::Spawn { apply: false })
        );
        b.tickets[1].workspace = Some(WorkspaceStrategy::Worktree);
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::Worktree, false),
            Ok(Start::Spawn { apply: false })
        );
        // A dead record relocates nothing: the ticket is still unstarted.
        b.sessions.push(agent(
            TARGET,
            CHECKOUT,
            SessionState::Exited { reason: crate::board::ExitReason::UserQuit },
            true,
        ));
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::SharedCheckout, false),
            Ok(Start::Spawn { apply: true })
        );
    }

    /// A ticket with a worktree starts there, and the other word is refused
    /// naming what it has.
    #[test]
    fn a_worktree_is_matched_or_refused() {
        let mut b = board();
        b.tickets[1].workspace = Some(WorkspaceStrategy::Worktree);
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::Worktree, true),
            Ok(Start::Spawn { apply: false })
        );
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::SharedCheckout, true),
            Err("T-2 has a worktree; start it there (workspace worktree) or archive it".into())
        );
        // An adopted worktree is a worktree to the crown, and no tool sets
        // the field over it.
        b.tickets[1].workspace = Some(WorkspaceStrategy::AdoptExisting);
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::Worktree, true),
            Ok(Start::Spawn { apply: false })
        );
    }

    /// An awake agent holds the seat, whoever started it.
    #[test]
    fn an_awake_agent_is_refused() {
        for by_crown in [false, true] {
            let mut b = board();
            b.sessions.push(agent(TARGET, CHECKOUT, idle(), by_crown));
            assert_eq!(
                judge_on(&b, WorkspaceStrategy::SharedCheckout, false),
                Err("T-2 already has an agent (idle); one agent per ticket".into())
            );
        }
    }

    /// The crown wakes what it parked, where it was parked; a person's
    /// parked agent stays the person's.
    #[test]
    fn a_parked_agent_is_the_crowns_to_wake_only_if_the_crown_started_it() {
        let mut b = board();
        b.sessions.push(agent(TARGET, CHECKOUT, SessionState::Sleeping, false));
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::SharedCheckout, false),
            Err("T-2's agent was started by a person and is parked; a person wakes it \
                 (c on its card)"
                .into())
        );
        let mut b = board();
        b.sessions.push(agent(TARGET, "/wt/T-2-x", SessionState::Sleeping, true));
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::Worktree, true),
            Ok(Start::Wake(uuid::Uuid::from_u128(TARGET)))
        );
        // Its worktree reclaimed under it: still the worktree it was.
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::Worktree, false),
            Ok(Start::Wake(uuid::Uuid::from_u128(TARGET)))
        );
    }

    /// A parked agent's workspace is its cwd, whatever the ticket's field
    /// says since (T-582: parked on the checkout, then set to a worktree).
    #[test]
    fn a_parked_agent_is_matched_by_its_cwd_not_the_field() {
        let mut b = board();
        b.tickets[1].workspace = Some(WorkspaceStrategy::Worktree);
        b.sessions.push(agent(TARGET, CHECKOUT, SessionState::Sleeping, true));
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::Worktree, false),
            Err("T-2's parked agent works on the shared checkout, and a wake runs it there: \
                 workspace shared_checkout"
                .into())
        );
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::SharedCheckout, false),
            Ok(Start::Wake(uuid::Uuid::from_u128(TARGET)))
        );
        let mut b = board();
        b.sessions.push(agent(TARGET, "/wt/T-2-x", SessionState::Sleeping, true));
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::SharedCheckout, true),
            Err("T-2's parked agent works in its worktree, and a wake runs it there: \
                 workspace worktree"
                .into())
        );
    }

    /// The checkout is held by another ticket's agent in each of its three
    /// live states, and by nothing else: not a worktree agent, not a dead
    /// record, not a shell, not an archived ticket, not the crown itself.
    #[test]
    fn the_checkout_is_held_working_idle_or_parked() {
        for (state, word) in [
            (SessionState::Running, "working"),
            (idle(), "idle"),
            (SessionState::Sleeping, "sleeping"),
        ] {
            let mut b = board();
            b.sessions.push(agent(3, CHECKOUT, state, false));
            assert_eq!(
                judge_on(&b, WorkspaceStrategy::SharedCheckout, false),
                Err(format!("T-3 ({word}) works on this checkout; use worktree, or wait")),
            );
            assert_eq!(
                judge_on(&b, WorkspaceStrategy::Worktree, false),
                Ok(Start::Spawn { apply: true }),
                "a worktree start is never held by the checkout"
            );
        }
        let mut b = board();
        b.sessions.push(agent(3, "/wt/T-3-x", SessionState::Running, true));
        b.sessions.push(agent(
            4,
            CHECKOUT,
            SessionState::Exited { reason: crate::board::ExitReason::UserQuit },
            true,
        ));
        let mut shell = agent(5, CHECKOUT, SessionState::Running, false);
        shell.kind = SessionKind::Bash;
        b.sessions.push(shell);
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::SharedCheckout, false),
            Ok(Start::Spawn { apply: false })
        );
        b.sessions.push(agent(4, CHECKOUT, SessionState::Sleeping, true));
        b.tickets[3].archived = Some(crate::board::Archived {
            at: "@0".into(),
            by: "local".into(),
            until: None,
            needs_you: false,
        });
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::SharedCheckout, false),
            Ok(Start::Spawn { apply: false }),
            "an archived ticket's parked agent is off the board"
        );
    }

    /// The wake of a parked checkout worker is a start on the checkout too.
    #[test]
    fn a_wake_on_a_held_checkout_is_refused() {
        let mut b = board();
        b.sessions.push(agent(TARGET, CHECKOUT, SessionState::Sleeping, true));
        b.sessions.push(agent(3, CHECKOUT, SessionState::Running, true));
        b.sessions.push(agent(4, CHECKOUT, SessionState::Sleeping, false));
        assert_eq!(
            judge_on(&b, WorkspaceStrategy::SharedCheckout, false),
            Err("T-3 (working) and T-4 (sleeping) work on this checkout; use worktree, or wait"
                .into())
        );
    }

    /// A person's start is the person's decision: the rule refuses the
    /// crown and the daemon's automation, never a person.
    #[test]
    fn a_persons_start_is_never_refused_by_the_checkout() {
        let holders = vec![Holder { key: "T-3".into(), state: "working" }];
        for person in
            [Principal::Local, Principal::Paired { grant: "g".into(), device: "d".into() }]
        {
            assert_eq!(checkout_refusal(&person, &holders), None, "{person:?}");
        }
        for not_a_person in [
            Principal::Agent { session: uuid::Uuid::nil() },
            Principal::Automation { rule: "queued_ask".into() },
        ] {
            assert_eq!(
                checkout_refusal(&not_a_person, &holders).as_deref(),
                Some("T-3 (working) works on this checkout"),
            );
        }
        assert_eq!(checkout_refusal(&Principal::Automation { rule: "x".into() }, &[]), None);
    }
}
