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

impl AwaitingLook {
    /// Still out: no sample that began after the turn has read the ticket
    /// (`fresh`, its `wt_fresh`), and `LOOK_MS` has not run.
    fn out(&self, fresh: Option<&u64>, now: u64) -> bool {
        fresh.is_none_or(|f| *f < self.need) && now.saturating_sub(self.since_ms) < LOOK_MS
    }
}

impl Daemon {
    /// A turn ended on `ticket`'s agent: on a branch the armed train could
    /// take, the person's banner waits for a sample started from here, and
    /// the ticket's own is asked for now rather than on the next bucket.
    /// A worker the crown hears is looked at too, train or not: its forge
    /// half is the read the crown's wake waits for (T-773). And so is any
    /// worktree ticket on a board that merges through pull requests
    /// (T-895, `forge_follows`): a turn's `pr create` or push is read
    /// within the burst's floor, whoever hears the turn.
    pub(super) fn look_after_turn(&mut self, ticket: ulid::Ulid) {
        if self.train_could_take(ticket) {
            let need = self.wt_seq.wrapping_add(1);
            self.awaiting_looks.insert(ticket, AwaitingLook { need, since_ms: now_ms() });
        } else if !self.crown_hears_turn(ticket) && !self.forge_follows(ticket) {
            return;
        }
        self.queue_ticket_flags(ticket);
    }

    /// A ticket whose branch a forge may read (T-895): a worktree ticket on
    /// a board that merges through pull requests. Its turn's end is looked
    /// at — the legs' pushes and the transcript's tail — train, crown or
    /// neither, so the card and a column's `Arrives here when` see the PR it
    /// opened in seconds, not on the minute.
    fn forge_follows(&self, ticket: ulid::Ulid) -> bool {
        self.through_prs() && self.worktrees.get(&ticket).is_some_and(|b| !b.branch.is_empty())
    }

    /// The settled Stop and the seat that owns it. A fresh flags sample checks
    /// every leg's tree on its worker only when this Stop has new tasks to name.
    pub(super) fn parked_for_task_fact(&self, ticket: ulid::Ulid) -> Option<(uuid::Uuid, u64)> {
        let seat = self.board.pane_target(ticket)?;
        let since = self.tasks_since.get(&seat.id).copied()?;
        (!self.board.prompts.background_task_notice_off
            && seat.kind == SessionKind::Claude
            && seat.state.on_tasks()
            && !seat.observe_only()
            && seat.background_tasks.needs_report()
            && !self.async_stop_pending(seat.id))
        .then_some((seat.id, since))
    }

    fn tell_parked_worker(&mut self, ticket: ulid::Ulid, park: (uuid::Uuid, u64)) -> bool {
        if self.parked_for_task_fact(ticket) != Some(park)
            || !self.committed_branch_waits(ticket)
            || self.board.ticket(ticket).is_none_or(|t| {
                t.is_archived()
                    || t.hand_raised()
                    || !t.effective_execution_policy().allows_automation()
            })
            || !self.session_takes_words(park.0)
            || self.owed.contains_key(&park.0)
            || self.queued.iter().any(|q| q.ticket == ticket)
        {
            return false;
        }
        let by = Principal::Automation { rule: "background_task_notice".into() };
        if authorize(&by, &Action::Mutate, &Resource::Session { id: park.0 }).denied() {
            return false;
        }
        let Some(tasks) = self.board.session(park.0).map(|s| &s.background_tasks) else {
            return false;
        };
        let text = self.board.prompts.render(
            mesimon_core::prompts::AgentPrompt::BackgroundTaskNotice,
            &[("count", &tasks.count().to_string()), ("tasks", &tasks.task_list())],
        );
        let ack = Ack { by: "automation", word: "background_task_notice" };
        if self.paste_to_ticket(ticket, &text, ack).is_err() {
            return false;
        }
        if let Some(s) = self.board.sessions.iter_mut().find(|s| s.id == park.0) {
            s.background_tasks.mark_reported();
        }
        true
    }

    /// The ticket's own flags sample on a worker, landing as
    /// `Msg::TicketFlags`. One in flight per ticket: asked again while one
    /// is out, the next goes when it lands, so the look a later turn waits
    /// for began after that turn.
    pub(super) fn queue_ticket_flags(&mut self, ticket: ulid::Ulid) {
        if let Some(again) = self.ticket_looks.get_mut(&ticket) {
            *again = true;
            return;
        }
        let base = self.worktree_base_cached();
        let mut queries = self.wt_queries(false, Some(ticket));
        if queries.is_empty() {
            return;
        }
        self.ticket_looks.insert(ticket, false);
        self.wt_seq = self.wt_seq.wrapping_add(1);
        let seq = self.wt_seq;
        let gen = self.wt_gen;
        // What the forge would need to read again (T-761): the legs a forge
        // may read and the transcripts' tails, looked at beside the flags.
        let forge_legs = self.look_legs(ticket);
        let tails = if forge_legs.is_empty() {
            Vec::new()
        } else {
            self.costs.tails(ticket, &self.live_transcripts(ticket))
        };
        if !forge_legs.is_empty() {
            self.forge.looking.insert(ticket);
        }
        let park = self.parked_for_task_fact(ticket).and_then(|park| {
            let binding = self.worktrees.get(&ticket)?;
            Some((park, self.binding_legs(binding)))
        });
        let uncached = self.uncached_upstreams(&queries);
        self.off_writer(move || {
            let local =
                (!forge_legs.is_empty()).then(|| crate::forge::look(&forge_legs, &tails, now_ms()));
            // The board's sample's terms (`queue_worktree_flags`): the base
            // and the upstream filled in here where the writer had none.
            sample_terms(&mut queries, base.as_ref(), &uncached);
            let samples = worktree::compute_repo_flags(&queries);
            let clean_park = park.and_then(|(park, legs)| {
                (!legs.is_empty()
                    && legs
                        .iter()
                        .all(|l| worktree::audit(&l.path).is_ok_and(|(dirty, _)| dirty == 0)))
                .then_some(park)
            });
            Msg::TicketFlags(ticket, gen, seq, samples, local, clean_park)
        });
    }

    /// A ticket's own sample landed: absorbed for that ticket alone, then
    /// the train's pass on it and the turns it looked at — what the board's
    /// sample does on landing, for one ticket.
    pub(super) fn on_ticket_flags(
        &mut self,
        ticket: ulid::Ulid,
        gen: u64,
        seq: u64,
        samples: Vec<worktree::RepoSample>,
        local: Option<crate::forge::Local>,
        clean_park: Option<(uuid::Uuid, u64)>,
    ) {
        let again = self.ticket_looks.remove(&ticket).unwrap_or(false);
        self.forge.looking.remove(&ticket);
        // A target switch or synchronous refresh supersedes both halves of
        // this look, including conflicts and forge data outside wt_fresh.
        if gen != self.wt_gen {
            self.queue_ticket_flags(ticket);
            return;
        }
        let mut changed = false;
        let mut acted = false;
        // The look's forge half (T-761), before the train's pass reads it:
        // a turn that pushed or opened something waits for a read, asked
        // now for this ticket's repositories alone.
        if let Some(local) = local {
            let at_ms = local.at_ms;
            self.forge.local.insert(ticket, local);
            if self.board.ticket(ticket).is_some() && !self.forge_fresh(ticket) {
                self.forge.owed.insert(ticket, at_ms);
                self.queue_forge_burst();
            }
        }
        if !samples.is_empty() && !self.tearing_down.contains(&ticket) {
            changed = self.absorb_worktree_flags(samples, seq, Some(ticket));
            acted = self.train_pass();
            if let Some(park) = clean_park.filter(|_| self.wt_fresh.get(&ticket) == Some(&seq)) {
                acted |= self.tell_parked_worker(ticket, park);
            }
            if acted {
                self.persist_sessions();
            }
        }
        let settled = self.settle_looks();
        if again {
            self.queue_ticket_flags(ticket);
        }
        // A look that found nothing to read lets the crown's wake go (T-773).
        if self.hear_forged(HashMap::new()) {
            self.drain_crown_wakes(None);
            changed = true;
        }
        if changed || acted || settled {
            self.broadcast();
        }
    }

    /// The legs of `ticket` a forge may read, for its look (T-761).
    fn look_legs(&mut self, ticket: ulid::Ulid) -> Vec<crate::forge::LegQuery> {
        // A board that merges into main reads no forge (T-810).
        if !self.through_prs() {
            return Vec::new();
        }
        let legs = match self.worktrees.get(&ticket) {
            Some(b) if !b.branch.is_empty() => self.binding_legs(b),
            _ => return Vec::new(),
        };
        self.forge_legs(&legs)
    }

    /// The train's static half of `train::lane`, before any flag: armed,
    /// an attached binding, a column the train reaches, and a ticket a
    /// person has not kept for their own merge. Everything the flags decide
    /// is the look's.
    fn train_could_take(&self, ticket: ulid::Ulid) -> bool {
        if !self.train.is_armed() || self.worktrees_barred || !self.base_known_for(ticket) {
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
        self.awaiting_looks.retain(|t, a| a.out(fresh.get(t), now));
        self.awaiting_looks.len() != before
    }

    /// A ticket is off the rebase turn the train asked for at this base tip
    /// and the look at what that turn left is still out (T-1246). The ask
    /// settles at the turn's end, and until the look lands the flags are
    /// from before it: a rebase made in the turn's last seconds still reads
    /// behind the tip it was asked at, which puts the ticket on neither
    /// list. A pass in that window would merge the next branch over the one
    /// that had just rebased, or ask the next one behind onto a tip about
    /// to move (T-435's wasted turn, by another door); read off the code,
    /// never seen in a feed. The pass waits; the look's landing runs one,
    /// and `LOOK_MS` bounds a look that never lands.
    pub(super) fn rebase_look_out(&self) -> bool {
        let now = now_ms();
        self.awaiting_looks.iter().any(|(t, a)| {
            a.out(self.wt_fresh.get(t), now)
                && self.train.asked().get(t).is_some_and(|r| r.base_oid == self.base_tip_of(*t))
        })
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

#[cfg(test)]
mod task_tests {
    use super::*;
    use mesimon_core::background::{Transition, BACKGROUND_COMMAND};
    use mesimon_core::train::{seat, Seat};

    #[test]
    fn committed_suite_gets_one_fact_and_holds_the_train_through_the_linger_alert() {
        let mut b = crownask::ledger_tests::bed("committed-suite");
        let d = &mut b.d;
        d.board.crown = None;
        let rec = &mut d.board.sessions[0];
        let id = rec.id;
        rec.road = Road::Mod;
        rec.set_state(SessionState::Idle { stop_reason: StopReason::Background });
        rec.background_tasks.record(
            "suite",
            Some(BACKGROUND_COMMAND),
            None,
            Transition::Started,
            None,
        );
        rec.background_tasks.label("suite", Some("cargo nextest run --workspace"));
        d.modroad.seat_bridge(id, &["submit"]);
        d.tasks_since.insert(id, 100);
        let park = (id, 100);
        assert!(!d.tell_parked_worker(b.worker, park), "no commits yet");
        d.wt_agg.insert(b.worker, worktree::Aggregate { ahead: 1, ..Default::default() });
        assert!(!d.tell_parked_worker(b.worker, (id, 99)), "late sample from another turn");
        d.board.prompts.background_task_notice_off = true;
        assert!(!d.tell_parked_worker(b.worker, park));
        assert!(!d.owed.contains_key(&id), "an off switch sends no prompt");
        d.board.prompts.background_task_notice_off = false;
        d.board.prompts.set(
            mesimon_core::prompts::AgentPrompt::BackgroundTaskNotice,
            Some("{count} remaining. {tasks}".into()),
        );
        assert!(d.tell_parked_worker(b.worker, park));
        let sent = d.owed.remove(&id).unwrap().sent.unwrap().text;
        assert_eq!(sent, "1 remaining. - suite — cargo nextest run --workspace");
        // Claude names the running suite and stops again with the same task.
        d.board.sessions[0].background_tasks.record(
            "suite",
            Some("shell"),
            Some("running"),
            Transition::Listed,
            None,
        );
        assert!(!d.tell_parked_worker(b.worker, park), "the fact cannot start a loop");
        assert_eq!(seat(&d.board, b.worker, false), Seat::Busy);
        d.board.sessions[0].set_state(SessionState::Idle { stop_reason: StopReason::Monitoring });
        d.board.prompts.background_task_notice_off = true;
        let due = 100 + crownwake::LINGER_MS;
        assert!(!d.route_lingering(due - 1, crownwake::LINGER_MS));
        assert!(d.route_lingering(due, crownwake::LINGER_MS));
        assert!(d.board.needs_you_tickets(due).contains(&b.worker));
        assert_eq!(seat(&d.board, b.worker, false), Seat::Busy);
        d.board.crown = Some(b.crown);
        assert!(d.route_lingering(due, crownwake::LINGER_MS));
        assert!(!d.board.needs_you_tickets(due).contains(&b.worker));
        assert!(d.crown_wakes.iter().any(|w| w.worker == b.worker));
        assert!(!d.route_lingering(due + 1, crownwake::LINGER_MS));
        assert_eq!(seat(&d.board, b.worker, false), Seat::Busy);
    }
}
