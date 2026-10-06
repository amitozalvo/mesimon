//! The person's "turn finished" waits for the merge train (T-678).
//!
//! A worker's turn on a branch the armed train will take is not the end of
//! its work: the train asks the rebase, makes the merge and pastes the
//! merged notice, and each of those is a turn of the agent's own. A banner
//! at the first `Stop` told the person to look at work nobody needed them
//! for, and the next one said it again. So the snapshot carries the tickets
//! the train has yet to finish (`AutomationStatus::holding`) and the
//! person's differ leaves their finished turn unsaid until the ticket drops
//! off it: merged, told and its turn over, or let go by the train — a
//! refused merge, a rebase left behind the base it was asked at, the fuse,
//! a disarm. The crown's wake (T-554, T-596) asks the same two questions of
//! the same `pending_on`, so both hear a worker at the same moment.
//!
//! **"Will it" is judged on a look newer than the turn's end.** The flags
//! are sampled on the slow bucket, so at a `Stop` they predate the turn's
//! last commit: a branch with its first commit still reads `ahead 0`, and a
//! rebase the train asked for still reads behind the base tip it was asked
//! at, which is exactly the reading on which the train lets a ticket go.
//! A finished turn on a ticket the train could take is therefore held until
//! a sample started after it lands, and judged on that one. That sample is
//! the ticket's own (`look_after_turn`): its repositories and its branch
//! alone, `2 + legs` git forks, asked for at once, one in flight per ticket.
//! The board's sample stays on its 10 s bucket; whichever reads a ticket
//! later wins it (`wt_fresh`). A sample that never lands lets go after
//! `LOOK_MS`, judged on what is on hand: a hold is a delay, never a
//! silence.

use super::*;

/// How long a finished turn waits for a look at its branch before it is
/// judged on the flags at hand. A sample is a few git forks; this is the
/// bound for a git that hangs or a sample that is dropped.
const LOOK_MS: u64 = 30_000;

/// A finished turn on a train ticket, waiting for a sample newer than it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AwaitingLook {
    /// The sample sequence number the look has to reach (`wt_seq`).
    need: u64,
    since_ms: u64,
}

impl Daemon {
    /// A turn ended on `ticket`'s agent: on a branch the armed train could
    /// take, the person's banner waits for a sample started from here, and
    /// the ticket's own is asked for now rather than on the next bucket.
    pub(super) fn look_after_turn(&mut self, ticket: ulid::Ulid) {
        if !self.train_could_take(ticket) {
            return;
        }
        let need = self.wt_seq.wrapping_add(1);
        self.awaiting_looks.insert(ticket, AwaitingLook { need, since_ms: now_ms() });
        self.queue_ticket_flags(ticket);
    }

    /// The ticket's own flags sample on a worker, landing as
    /// `Msg::TicketFlags`. One in flight per ticket: asked again while one
    /// is out, the next goes when it lands, so the look a later turn waits
    /// for began after that turn.
    fn queue_ticket_flags(&mut self, ticket: ulid::Ulid) {
        if let Some(again) = self.ticket_looks.get_mut(&ticket) {
            *again = true;
            return;
        }
        let Some(base) = self.base_branch.clone() else { return };
        let mut queries = self.wt_queries(false, Some(ticket));
        if queries.is_empty() {
            return;
        }
        self.ticket_looks.insert(ticket, false);
        self.wt_seq = self.wt_seq.wrapping_add(1);
        let seq = self.wt_seq;
        let cached: HashMap<String, Option<String>> = self.upstreams.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // The board's sample's terms (`queue_worktree_flags`): the base
            // and the upstream filled in here where the writer had none.
            for q in &mut queries {
                if q.base.is_empty() {
                    q.base = base.clone();
                }
                if !cached.contains_key(&q.name) {
                    q.upstream = worktree::upstream_base(&q.repo, &q.base);
                }
            }
            let samples = worktree::compute_repo_flags(&queries);
            let _ = tx.send(Msg::TicketFlags(ticket, seq, samples));
        });
    }

    /// A ticket's own sample landed: absorbed for that ticket alone, then
    /// the train's pass on it and the turns it looked at — what the board's
    /// sample does on landing, for one ticket.
    pub(super) fn on_ticket_flags(
        &mut self,
        ticket: ulid::Ulid,
        seq: u64,
        samples: Vec<worktree::RepoSample>,
    ) {
        let again = self.ticket_looks.remove(&ticket).unwrap_or(false);
        let mut changed = false;
        let mut acted = false;
        if !samples.is_empty() && !self.tearing_down.contains(&ticket) {
            changed = self.absorb_worktree_flags(samples, seq, Some(ticket));
            acted = self.train_pass();
            if acted {
                self.persist_sessions();
            }
        }
        let settled = self.settle_looks();
        if again {
            self.queue_ticket_flags(ticket);
        }
        if changed || acted || settled {
            self.broadcast();
        }
    }

    /// The train's static half of `train::lane`, before any flag: armed,
    /// an attached binding, a column the train reaches, and a ticket a
    /// person has not kept for their own merge. Everything the flags decide
    /// is the look's.
    fn train_could_take(&self, ticket: ulid::Ulid) -> bool {
        if !self.train.is_armed() || self.worktrees_barred || self.base_branch.is_none() {
            return false;
        }
        let Some(t) = self.board.ticket(ticket).filter(|t| !t.is_archived()) else {
            return false;
        };
        let reach = self.board.column(&t.column).map(|c| c.settings.train).unwrap_or_default();
        reach != mesimon_core::board::TrainReach::Off
            && !t.manual_merge
            && self
                .worktrees
                .get(&ticket)
                .is_some_and(|b| b.status == BindingStatus::Attached && !b.branch.is_empty())
    }

    /// A sample landed, or the tick came round: the turns a sample started
    /// after them has read, and those past `LOOK_MS`, are judged now. True
    /// when one was let go, which the board must hear even when the sample
    /// changed nothing.
    pub(super) fn settle_looks(&mut self) -> bool {
        if self.awaiting_looks.is_empty() {
            return false;
        }
        let now = now_ms();
        let before = self.awaiting_looks.len();
        let fresh = &self.wt_fresh;
        self.awaiting_looks.retain(|t, a| {
            fresh.get(t).is_none_or(|f| *f < a.need) && now.saturating_sub(a.since_ms) < LOOK_MS
        });
        self.awaiting_looks.len() != before
    }

    /// The tickets the train has yet to finish (T-678), for the snapshot:
    /// of the agents sitting on a finished turn, those whose branch has not
    /// been looked at since, and those the crown's wake would hold for the
    /// train or for a merge step (`pending_on`) on the train's own sample.
    pub(super) fn train_holding(&self) -> Vec<ulid::Ulid> {
        let mut out: Vec<ulid::Ulid> = self
            .board
            .sessions
            .iter()
            .filter(|s| s.kind.is_agent() && s.holds_agent_seat())
            .filter(|s| matches!(s.state, SessionState::Idle { stop_reason: StopReason::EndTurn }))
            .map(|s| s.ticket)
            .filter(|t| {
                if self.awaiting_looks.contains_key(t) {
                    return true;
                }
                let pending = self.pending_on(*t, self.sampled(*t).as_ref());
                pending.train || pending.merge_step
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }
}
