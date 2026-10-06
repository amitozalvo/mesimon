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
//! a sample queued after it lands (`look_after_turn`, asked for at once),
//! and judged on that one. A sample that never lands lets go after
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
    /// take, the person's banner waits for a sample queued from here, and
    /// one is asked for now rather than on the next slow bucket.
    pub(super) fn look_after_turn(&mut self, ticket: ulid::Ulid) {
        if !self.train_could_take(ticket) {
            return;
        }
        let need = self.wt_seq.wrapping_add(1);
        self.awaiting_looks.insert(ticket, AwaitingLook { need, since_ms: now_ms() });
        self.queue_worktree_flags();
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

    /// A sample landed (`landed`), or the tick came round: the turns it has
    /// looked at, and those past `LOOK_MS`, are judged now. True when one
    /// was let go, which the board must hear even when the sample changed
    /// nothing.
    pub(super) fn settle_looks(&mut self, landed: bool) -> bool {
        if self.awaiting_looks.is_empty() {
            return false;
        }
        let (seq, now) = (self.wt_landed, now_ms());
        let before = self.awaiting_looks.len();
        self.awaiting_looks.retain(|_, a| a.need > seq && now.saturating_sub(a.since_ms) < LOOK_MS);
        // A look the sample in flight when the turn ended did not reach:
        // the next one goes now, not on the bucket. Only from a landing, so
        // a sample that never comes back is not asked for every tick.
        if landed && !self.awaiting_looks.is_empty() {
            self.queue_worktree_flags();
        }
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
