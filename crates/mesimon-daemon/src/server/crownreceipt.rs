//! A receipt covers the write's own work, never a later person's turn (T-1042).
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Prompt {
    Launch { crown: ulid::Ulid },
    Delivery(ulid::Ulid),
    Ask(ulid::Ulid),
}

/// Captured before a synchronous write, after its seen gate. Keeping the
/// chain's identity here prevents a revocation during the write from reviving it.
pub(super) struct Write {
    seen: String,
    prior: Option<String>,
    seat: Option<uuid::Uuid>,
}

pub(super) struct Receipt {
    stamp: String,
    superseded: Vec<String>,
    owner: Option<uuid::Uuid>,
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

impl Receipt {
    pub(super) fn park(&mut self) {
        self.active = false;
        self.next.clear();
        self.beginning = false;
    }
}

impl Daemon {
    pub(super) fn receipt_holds(&self, id: ulid::Ulid, seen: &str) -> bool {
        self.receipts.get(&id).is_some_and(|r| {
            (r.stamp == seen || r.superseded.iter().any(|s| s == seen))
                && r.owner == self.receipt_owner()
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

    fn receipt_owner(&self) -> Option<uuid::Uuid> {
        self.board.crown_holder().and_then(|t| self.board.live_agent(t.id)).map(|s| s.id)
    }

    pub(super) fn receipt_write(&self, target: ulid::Ulid, seen: Option<&str>) -> Write {
        Write {
            // Notes and tags have no seen argument: their own edit supersedes
            // the current read token, not an arbitrary older read (T-1047).
            seen: seen.map(str::trim).map(str::to_owned).unwrap_or_else(|| self.seen_token(target)),
            prior: self
                .receipts
                .get(&target)
                .filter(|r| self.receipt_holds(target, &r.stamp))
                .map(|r| r.stamp.clone()),
            seat: self.board.live_agent(target).map(|s| s.id),
        }
    }

    /// Only a write's input read joins its chain; unrelated reads stay strict.
    /// Deferred replies have no write witness and keep the ordinary words check.
    pub(super) fn receipt_seen(&mut self, target: ulid::Ulid, write: Option<Write>) -> String {
        let board = &self.board;
        self.receipts.retain(|t, _| board.ticket(*t).is_some());
        self.moved_by_hand.retain(|t, _| board.ticket(*t).is_some());
        let full = self.seen_token(target);
        let words = self.words_token(target);
        let owner = self.receipt_owner();
        let prior = self.receipts.remove(&target).filter(|r| {
            r.owner == owner
                && self.seat_holds(target, r.seat)
                && match &write {
                    Some(w) => w.prior.as_deref() == Some(r.stamp.as_str()),
                    None => r.words == words && (r.full == full || r.active),
                }
        });
        let keep_input = write.as_ref().is_some_and(|w| {
            self.seat_holds(target, w.seat) && (w.prior.is_none() || prior.is_some())
        });
        let seat = self.board.live_agent(target).map(|s| s.id);
        let stamp = format!("receipt:{}", ulid::Ulid::new());
        let (active, next, beginning, mut superseded) =
            prior.map(|r| (r.active, r.next, r.beginning, r.superseded)).unwrap_or_default();
        if let Some(w) = write.filter(|_| keep_input) {
            if !superseded.contains(&w.seen) {
                superseded.push(w.seen);
            }
        }
        self.receipts.insert(
            target,
            Receipt {
                stamp: stamp.clone(),
                superseded,
                owner,
                full,
                words,
                seat,
                active,
                next,
                beginning,
            },
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

    fn with_ticket(f: impl FnOnce(&mut Daemon, ulid::Ulid)) {
        let dir = std::env::temp_dir()
            .join(format!("msmn-rcp-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        let (mut d, _, _rx) = crate::server::queued_person_push_tests::daemon(&dir);
        let id = d.board.sessions[0].ticket;
        d.board.tickets.push(Ticket {
            id,
            short_key: "T-1".into(),
            title: "worker".into(),
            column: "TODO".into(),
            order: "a".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            previous_column: None,
            picked: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            import_origin: None,
            envelope: None,
            raised: None,
            workspace: None,
            tags: vec![],
            notes: vec![],
            archived: None,
        });
        f(&mut d, id);
        drop(d);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn rename(d: &mut Daemon, id: ulid::Ulid, seen: &str, title: &str) -> String {
        d.check_seen(id, Some(seen)).unwrap();
        let write = d.receipt_write(id, Some(seen));
        d.board.ticket_mut(id).unwrap().title = title.into();
        d.receipt_seen(id, Some(write))
    }

    #[test]
    fn parallel_writes_keep_their_shared_read_and_receipt_inputs() {
        with_ticket(|d, id| {
            let seen = d.seen_token(id);
            rename(d, id, &seen, "first");
            let second = rename(d, id, &seen, "second");
            assert!(d.receipt_holds(id, &seen));
            rename(d, id, &second, "third");
            rename(d, id, &second, "fourth");
            assert!(d.receipt_holds(id, &seen), "the whole own-write chain survives");
            assert_eq!(d.receipts[&id].superseded.len(), 2, "shared inputs are deduplicated");
        });
    }

    #[test]
    fn an_own_park_keeps_the_input_and_prior_chain_without_owing_another_turn() {
        for prior in [false, true] {
            with_ticket(|d, id| {
                let seen = d.seen_token(id);
                if prior {
                    rename(d, id, &seen, "own write");
                    d.allow_receipt_prompt(id, Prompt::Ask(ulid::Ulid::new()));
                }
                let input = d.seen_token(id);
                let write = d.receipt_write(id, Some(&input));
                let sid = d.board.live_agent(id).unwrap().id;
                d.park_record(sid, None, 123, Park::Crown);
                d.receipt_seen(id, Some(write));
                assert!(d.receipt_holds(id, &input), "the park keeps its input read");
                assert!(d.receipt_holds(id, &seen), "the earlier chain survives");
                assert!(!d.receipts[&id].active);
                assert!(d.receipts[&id].next.is_empty());
                assert!(!d.receipts[&id].beginning);
                rename(d, id, &input, "parallel sibling");
                let seat = d.board.live_agent(id).unwrap().clone();
                d.receipt_state(
                    &seat,
                    &Change {
                        from: SessionState::Sleeping,
                        to: SessionState::Running,
                        attention_added: false,
                        confidence: Confidence::High,
                    },
                );
                assert!(!d.receipt_holds(id, &input), "a foreign turn still revokes");
            });
        }
        for park in [Park::Deliberate, Park::Automatic] {
            with_ticket(|d, id| {
                let seen = d.seen_token(id);
                rename(d, id, &seen, "own write");
                let sid = d.board.live_agent(id).unwrap().id;
                d.park_record(sid, None, 123, park);
                assert!(!d.receipt_holds(id, &seen), "a foreign park still revokes");
            });
        }
    }

    /// T-1072: a filing's receipt has no write witness. It passes while the
    /// ticket stands as filed and is distinct from a read's stamp; any edit
    /// or seat after it is stale, as a read's would be.
    #[test]
    fn a_filing_s_receipt_is_as_strict_as_a_read() {
        for edit in [false, true] {
            with_ticket(|d, id| {
                d.board.sessions.clear();
                let filed = d.receipt_seen(id, None);
                assert_ne!(filed, d.seen_token(id));
                assert!(d.check_seen(id, Some(&filed)).is_ok());
                if edit {
                    d.board.ticket_mut(id).unwrap().title = "a person's edit".into();
                } else {
                    d.board.ticket_mut(id).unwrap().column = "REVIEW".into();
                }
                assert!(d.check_seen(id, Some(&filed)).is_err());
            });
        }
    }

    #[test]
    fn an_unstamped_own_write_keeps_only_the_read_it_superseded() {
        with_ticket(|d, id| {
            let old = d.seen_token(id);
            d.board.ticket_mut(id).unwrap().title = "a person's edit".into();
            let current = d.seen_token(id);
            let write = d.receipt_write(id, None);
            d.board.ticket_mut(id).unwrap().title = "the crown's edit".into();
            d.receipt_seen(id, Some(write));
            assert!(d.check_seen(id, Some(&old)).is_err());
            rename(d, id, &current, "parallel sibling");
        });
    }

    #[test]
    fn a_foreign_move_or_hand_sort_stales_every_superseded_stamp() {
        for sort in [false, true] {
            with_ticket(|d, id| {
                let seen = d.seen_token(id);
                let receipt = rename(d, id, &seen, "own write");
                d.allow_receipt_prompt(id, Prompt::Ask(ulid::Ulid::new()));
                let t = d.board.ticket_mut(id).unwrap();
                if sort {
                    t.order = "b".into();
                } else {
                    t.column = "REVIEW".into();
                }
                *d.moved_by_hand.entry(id).or_default() += 1;
                assert!(d.check_seen(id, Some(&seen)).is_err());
                assert!(d.check_seen(id, Some(&receipt)).is_err());
                let fresh = d.seen_token(id);
                rename(d, id, &fresh, "after a fresh read");
                assert!(!d.receipt_holds(id, &seen), "a fresh chain cannot revive the old one");
            });
        }
    }

    #[test]
    fn superseded_reads_follow_only_the_seat_the_chain_started_or_saw() {
        for had_seat in [false, true] {
            with_ticket(|d, id| {
                let mut seat = d.board.sessions.remove(0);
                if had_seat {
                    d.board.sessions.push(seat.clone());
                }
                let seen = d.seen_token(id);
                let write = d.receipt_write(id, Some(&seen));
                seat.started_by = Some(ulid::Ulid::new());
                seat.state = SessionState::Running;
                if had_seat {
                    d.board.sessions[0] = seat.clone();
                } else {
                    d.board.sessions.push(seat.clone());
                }
                d.receipt_seen(id, Some(write));
                d.allow_receipt_prompt(id, Prompt::Launch { crown: seat.started_by.unwrap() });
                d.board.sessions[0].state_changed_at = Some(123);
                assert!(d.receipt_holds(id, &seen));
                d.board.sessions[0].id = uuid::Uuid::new_v4();
                assert!(!d.receipt_holds(id, &seen), "even another crown-started seat is foreign");
            });
        }
        with_ticket(|d, id| {
            let seat = d.board.sessions.remove(0);
            let seen = d.seen_token(id);
            let write = d.receipt_write(id, Some(&seen));
            d.board.sessions.push(seat);
            d.receipt_seen(id, Some(write));
            d.allow_receipt_prompt(id, Prompt::Ask(ulid::Ulid::new()));
            assert!(!d.receipt_holds(id, &seen), "a person's launch cannot inherit the read");
        });
    }

    #[test]
    fn revocation_drops_superseded_stamps_even_during_a_write() {
        for during_write in [false, true] {
            with_ticket(|d, id| {
                let seen = d.seen_token(id);
                let receipt = rename(d, id, &seen, "own write");
                let write = during_write.then(|| d.receipt_write(id, Some(&receipt)));
                let seat = d.board.sessions[0].clone();
                d.receipt_state(
                    &seat,
                    &Change {
                        from: seat.state.clone(),
                        to: SessionState::Running,
                        attention_added: false,
                        confidence: Confidence::High,
                    },
                );
                assert!(!d.receipts.contains_key(&id));
                d.receipt_seen(id, write);
                assert!(!d.receipt_holds(id, &seen));
                assert!(!d.receipt_holds(id, &receipt));
                assert!(d.receipts[&id].superseded.is_empty());
            });
        }
    }

    #[test]
    fn a_deferred_receipt_cannot_revive_stamps_after_foreign_state_motion() {
        with_ticket(|d, id| {
            let seen = d.seen_token(id);
            rename(d, id, &seen, "own write");
            d.board.sessions[0].state_changed_at = Some(123);
            assert!(!d.receipt_holds(id, &seen), "an ordinary chain stays strict");
            d.receipt_seen(id, None);
            assert!(!d.receipt_holds(id, &seen));
        });
    }

    #[test]
    fn another_crown_session_cannot_inherit_superseded_stamps() {
        with_ticket(|d, id| {
            let mut crown = d.board.tickets[0].clone();
            crown.id = ulid::Ulid::new();
            let mut seat = d.board.sessions[0].clone();
            seat.id = uuid::Uuid::new_v4();
            seat.ticket = crown.id;
            d.board.crown = Some(crown.id);
            d.board.tickets.push(crown);
            d.board.sessions.push(seat);
            let seen = d.seen_token(id);
            let receipt = rename(d, id, &seen, "first crown session");
            d.board.sessions[1].id = uuid::Uuid::new_v4();
            assert!(!d.receipt_holds(id, &seen));
            assert!(!d.receipt_holds(id, &receipt));
            let fresh = d.seen_token(id);
            rename(d, id, &fresh, "second crown session");
            assert!(!d.receipt_holds(id, &seen));
        });
    }

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
                superseded: vec![],
                owner: None,
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
