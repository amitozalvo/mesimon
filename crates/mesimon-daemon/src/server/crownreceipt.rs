//! A receipt covers the write's own work, never a later person's turn (T-1042).
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Prompt {
    Launch { crown: ulid::Ulid },
    Delivery(ulid::Ulid),
    Ask(ulid::Ulid),
}

pub(super) struct Receipt {
    stamp: String,
    full: String,
    words: String,
    /// The live seat the receipt saw (T-1035, second commit).
    seat: Option<uuid::Uuid>,
    /// This chain set work in motion; its normal end and automatic moves
    /// may outlive `full`, but never a change to `words` or a revocation.
    active: bool,
    /// Identified prompts still owed by writes in this receipt chain.
    next: Vec<Prompt>,
    /// An attributed prompt has arrived ahead of its Running transition.
    /// Consumed at that edge so a later tool-only turn cannot borrow it.
    beginning: bool,
}

impl Daemon {
    pub(super) fn receipt_holds(&self, id: ulid::Ulid, seen: &str) -> bool {
        self.receipts.get(&id).is_some_and(|r| {
            r.stamp == seen
                && r.words == self.words_token(id)
                && self.seat_holds(id, r.seat)
                && (r.full == self.seen_token(id) || r.active)
        })
    }

    /// A receipt follows the record it saw. Before a deferred launch has
    /// a record, only a crown-started seat can be its consequence (T-1035).
    fn seat_holds(&self, id: ulid::Ulid, saw: Option<uuid::Uuid>) -> bool {
        match (saw, self.board.live_agent(id)) {
            (Some(saw), Some(seat)) => saw == seat.id,
            (None, Some(seat)) => seat.started_by.is_some(),
            (None, None) => true,
            (Some(_), None) => false,
        }
    }

    /// Distinct from a read even at the same instant: a read never grants
    /// permission to ignore the seat. A receipt chain keeps only allowances
    /// still valid when the next write returns.
    pub(super) fn receipt_seen(&mut self, target: ulid::Ulid) -> String {
        let board = &self.board;
        self.receipts.retain(|t, _| board.ticket(*t).is_some());
        self.moved_by_hand.retain(|t, _| board.ticket(*t).is_some());
        let full = self.seen_token(target);
        let words = self.words_token(target);
        let prior = self
            .receipts
            .remove(&target)
            .filter(|r| r.words == words && self.seat_holds(target, r.seat));
        let seat = self.board.live_agent(target).map(|s| s.id);
        let stamp = format!("receipt:{}", ulid::Ulid::new());
        let (active, next, beginning) =
            prior.map(|r| (r.active, r.next, r.beginning)).unwrap_or_default();
        self.receipts.insert(
            target,
            Receipt { stamp: stamp.clone(), full, words, seat, active, next, beginning },
        );
        stamp
    }

    pub(super) fn allow_receipt_prompt(&mut self, ticket: ulid::Ulid, next: Prompt) {
        if let Some(r) = self.receipts.get_mut(&ticket) {
            r.active = true;
            if !r.next.contains(&next) {
                r.next.push(next);
            }
        }
    }

    /// The same attribution that settles the owed words: a prompt ahead of
    /// a parked delivery is the person's. Every allowance is consumed once.
    pub(super) fn receipt_prompt(&mut self, session: uuid::Uuid, ahead: bool) {
        let Some(s) = self.board.sessions.iter().find(|s| s.id == session && s.kind.is_agent())
        else {
            return;
        };
        let ticket = s.ticket;
        let Some(r) = self.receipts.get_mut(&ticket) else { return };
        let owed = self.owed.get(&session);
        let matched = (!ahead)
            .then(|| {
                r.next.iter().position(|p| {
                    owed.is_some_and(|o| match p {
                        Prompt::Launch { crown } => {
                            s.started_by == Some(*crown)
                                && o.ask.is_none()
                                && o.sent.as_ref().or(o.parked.as_ref()).is_some_and(|p| p.brief)
                        }
                        Prompt::Delivery(id) => o.receipt_id == *id,
                        Prompt::Ask(id) => o.ask == Some(*id),
                    })
                })
            })
            .flatten();
        if let Some(i) = matched {
            r.next.remove(i);
            r.seat = Some(s.id);
            r.beginning = s.state != SessionState::Running;
        } else {
            self.receipts.remove(&ticket);
        }
    }

    pub(super) fn receipt_state(&mut self, s: &SessionRecord, change: &Change) {
        if !s.kind.is_agent()
            || (self.async_stop_pending(s.id) && change.to == SessionState::Running)
        {
            // Holding a Stop may promote an inferred idle to Running. That
            // synthetic edge neither begins a foreign turn nor consumes the
            // crown's allowance for the turn its own write started (T-1044).
            return;
        }
        let Some(r) = self.receipts.get_mut(&s.ticket) else { return };
        let needs_you = matches!(
            change.to,
            SessionState::RequiresAction { .. }
                | SessionState::Throttled
                | SessionState::Failed { .. }
        );
        let dialog_left = matches!(change.from, SessionState::RequiresAction { .. })
            && change.to == SessionState::Running;
        let began = crownexpect::turn_begins(&change.from, &change.to);
        if needs_you || dialog_left || !change.to.has_pane() || (began && !r.beginning) {
            self.receipts.remove(&s.ticket);
        } else if began || !crownexpect::turn_open(&change.to) {
            r.beginning = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_stop_keeps_its_crowns_receipt_until_a_real_foreign_turn() {
        let dir = std::env::temp_dir()
            .join(format!("msmn-hkr-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        let (mut d, _, _rx) = crate::server::queued_person_push_tests::daemon(&dir);
        let rec = &mut d.board.sessions[0];
        rec.state = SessionState::Running;
        rec.pane_key = Some("1:%0".into());
        let rec = rec.clone();
        let id = rec.id;
        d.machines.insert(id, Machine::new(SessionState::Running, 1_000));
        d.receipts.insert(
            rec.ticket,
            Receipt {
                stamp: "own-write".into(),
                full: String::new(),
                words: String::new(),
                seat: Some(id),
                active: true,
                next: vec![],
                beginning: true,
            },
        );
        let change =
            |from, to| Change { from, to, attention_added: false, confidence: Confidence::High };
        d.receipt_state(&rec, &change(SessionState::Spawning, SessionState::Running));
        assert!(!d.receipts[&rec.ticket].beginning, "own prompt consumed once");
        let frame = HookFrame {
            session: id.to_string(),
            event: "Stop".into(),
            reason: None,
            pane: None,
            road: Road::Hooks,
            payload: serde_json::json!({}),
        };
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
        assert!(matches!(d.hold_async_stop(id, stop, &frame, None, 1_000), Signal::TurnStarted));
        d.receipt_state(&rec, &change(SessionState::Running, SessionState::Running));
        // A held Stop can also promote an inferred idle. This is the
        // turn_begins edge that used to revoke the receipt after rebase.
        let idle = SessionState::Idle { stop_reason: StopReason::Unknown };
        assert!(crownexpect::turn_begins(&idle, &SessionState::Running));
        d.receipt_state(&rec, &change(idle, SessionState::Running));
        assert!(d.receipts.contains_key(&rec.ticket));
        assert!(!d.receipts[&rec.ticket].beginning, "no new prompt allowance");
        d.sample_async_stops(Some(&[]), Ok(&[]), 3_000);
        assert!(!d.async_stop_pending(id));
        let ended = d.machines.get_mut(&id).unwrap().tick(3_000).unwrap();
        d.receipt_state(&rec, &ended);
        assert!(d.receipts.contains_key(&rec.ticket), "release is its own turn ending");
        d.receipt_state(&rec, &change(ended.to, SessionState::Running));
        assert!(!d.receipts.contains_key(&rec.ticket), "a real foreign turn still revokes");
        drop(d);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
