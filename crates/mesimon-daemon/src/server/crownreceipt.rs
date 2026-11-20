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

/// What a receipt saw of its ticket, part by part, for the words of a
/// refusal (T-1277): `word_parts`, the count of moves by hand, the column,
/// and whether its agent had a pane.
#[derive(Clone, Default)]
pub(super) struct Parts {
    words: Vec<(&'static str, u64)>,
    moves: u64,
    column: String,
    pane: Option<bool>,
}

/// A revoked chain's stamps and what revoked it (T-1277).
pub(super) struct Revoked {
    stamps: Vec<String>,
    why: &'static str,
}

pub(super) struct Receipt {
    stamp: String,
    superseded: Vec<String>,
    owner: Option<uuid::Uuid>,
    words: String,
    /// The parts at the mint, for a refusal's words; their `pane` is also
    /// what an ordinary chain holds the seat to (T-1277).
    parts: Parts,
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
                && self.receipt_current(id, r)
        })
    }

    /// The chain still stands as its last write left it (T-1035, T-1277):
    /// nobody's words, no move by hand, the same seat, and — unless the
    /// chain set work in motion — its agent still with or without a pane.
    /// What the agent does inside its pane (a turn ending, a background
    /// park) and the board's rules moving the card after it are not news;
    /// the news a seat can carry (a person's prompt, a dialog, a failure,
    /// a later turn) revokes the receipt as it lands, in `receipt_state`.
    fn receipt_current(&self, id: ulid::Ulid, r: &Receipt) -> bool {
        r.words == self.words_token(id)
            && self.seat_holds(id, r.seat)
            && (r.active || r.parts.pane == self.seat_pane(id))
    }

    fn seat_pane(&self, id: ulid::Ulid) -> Option<bool> {
        self.board.agent_or_corpse(id).map(|s| s.state.has_pane())
    }

    fn parts(&self, id: ulid::Ulid) -> Parts {
        let t = self.board.ticket(id);
        Parts {
            words: self.word_parts(id),
            moves: self.moved_by_hand.get(&id).copied().unwrap_or_default(),
            column: t.map(|t| t.column.clone()).unwrap_or_default(),
            pane: self.seat_pane(id),
        }
    }

    /// What a refused stamp missed (T-1277), when it is a receipt's: the
    /// parts that changed since the receipt, or what revoked its chain.
    /// `None` for a read's stamp, a bare hash with nothing kept beside it.
    pub(super) fn receipt_news(&self, id: ulid::Ulid, seen: &str) -> Option<Vec<String>> {
        let Some(r) = self
            .receipts
            .get(&id)
            .filter(|r| r.stamp == seen || r.superseded.iter().any(|s| s == seen))
        else {
            let revoked = self.receipts_revoked.get(&id)?;
            return revoked.stamps.iter().any(|s| s == seen).then(|| vec![revoked.why.to_string()]);
        };
        let now = self.parts(id);
        let mut news: Vec<String> = Vec::new();
        if r.owner != self.receipt_owner() {
            news.push("another crown session".into());
        }
        for ((word, was), (_, is)) in r.parts.words.iter().zip(&now.words) {
            if was != is {
                news.push(format!("{word} changed"));
            }
        }
        if r.parts.moves != now.moves {
            let who = self.moved_by.get(&id).map(String::as_str).unwrap_or("someone");
            if r.parts.column != now.column {
                news.push(format!("moved to {} by {who}", now.column));
            } else {
                news.push(format!("reordered in {} by {who}", now.column));
            }
        }
        if !self.seat_holds(id, r.seat) {
            news.push("another agent seat".into());
        } else if !r.active && r.parts.pane != now.pane {
            news.push(match now.pane {
                Some(true) => "its agent woke".into(),
                Some(false) => "its agent parked or exited".into(),
                None => "its agent left".into(),
            });
        }
        (!news.is_empty()).then_some(news)
    }

    /// A chain ends: its stamps refuse from here on, naming `why`.
    pub(super) fn revoke_receipt(&mut self, ticket: ulid::Ulid, why: &'static str) {
        if let Some(r) = self.receipts.remove(&ticket) {
            let mut stamps = r.superseded;
            stamps.push(r.stamp);
            self.receipts_revoked.insert(ticket, Revoked { stamps, why });
        }
    }

    /// Who moved a ticket, in a refusal's words (T-1277).
    pub(super) fn mover_word(&self, id: ulid::Ulid, by: &Principal) -> String {
        match by {
            Principal::Local | Principal::Paired { .. } => "a person",
            Principal::Remote { .. } => "a teammate",
            Principal::Peer { .. } => "another board",
            Principal::Automation { .. } => "the board",
            Principal::Agent { session } => {
                if self.board.live_agent(id).is_some_and(|s| s.id == *session) {
                    "its agent"
                } else if self.receipt_owner() == Some(*session) {
                    "the crown"
                } else {
                    "another agent"
                }
            }
        }
        .into()
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
        self.receipts_revoked.retain(|t, _| board.ticket(*t).is_some());
        self.moved_by_hand.retain(|t, _| board.ticket(*t).is_some());
        self.moved_by.retain(|t, _| board.ticket(*t).is_some());
        let words = self.words_token(target);
        let parts = self.parts(target);
        let owner = self.receipt_owner();
        let prior = self.receipts.remove(&target).filter(|r| {
            r.owner == owner
                && self.seat_holds(target, r.seat)
                && match &write {
                    Some(w) => w.prior.as_deref() == Some(r.stamp.as_str()),
                    None => self.receipt_current(target, r),
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
                words,
                parts,
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
    /// A background task's `<task-notification>` (T-1285) is no one's
    /// prompt: Claude Code fires `UserPromptSubmit` for it as it folds the
    /// notification into the running turn or wakes an idle seat with it. It
    /// takes no allowance and revokes nothing; a turn it begins is still
    /// one the crown did not start (`receipt_state`).
    pub(super) fn receipt_prompt(&mut self, session: uuid::Uuid, ahead: bool) {
        if self.frame_words.as_deref().is_some_and(mesimon_core::adopt::is_task_notification) {
            return;
        }
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
            self.revoke_receipt(ticket, "a prompt the crown did not send");
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
        let Some(beginning) = self.receipts.get(&s.ticket).map(|r| r.beginning) else { return };
        let dialog_left = matches!(change.from, SessionState::RequiresAction { .. })
            && change.to == SessionState::Running;
        let began = crownexpect::turn_begins(&change.from, &change.to);
        let why = match &change.to {
            SessionState::RequiresAction { reason: Reason::Plan } => Some("a plan"),
            SessionState::RequiresAction { reason: Reason::Question } => Some("a question"),
            SessionState::RequiresAction { .. } => Some("a dialog"),
            SessionState::Throttled => Some("a usage limit"),
            SessionState::Failed { .. } => Some("a failed turn"),
            _ if dialog_left => Some("an answer to its dialog"),
            to if !to.has_pane() => Some("its agent parked or exited"),
            _ if began && !beginning => Some("a turn the crown did not start"),
            _ => None,
        };
        if let Some(why) = why {
            self.revoke_receipt(s.ticket, why);
        } else if began || !crownexpect::turn_open(&change.to) {
            if let Some(r) = self.receipts.get_mut(&s.ticket) {
                r.beginning = false;
            }
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
                d.park_record(sid, 123, Park::Crown);
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
                d.park_record(sid, 123, park);
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
                    // A person's move; the board's own rule moves leave a
                    // receipt good (T-1035, restored for every receipt by
                    // T-1277), and no rule moves an agentless filing.
                    d.board.ticket_mut(id).unwrap().column = "REVIEW".into();
                    d.note_hand_move(id, &Principal::Local);
                }
                assert!(d.check_seen(id, Some(&filed)).is_err());
            });
        }
        // The receipt is the crown session's that filed, and it saw no seat.
        for replaced in [false, true] {
            with_ticket(|d, id| {
                let mut crown = d.board.tickets[0].clone();
                crown.id = ulid::Ulid::new();
                let mut seat = d.board.sessions.remove(0);
                let worker = seat.clone();
                seat.id = uuid::Uuid::new_v4();
                seat.ticket = crown.id;
                d.board.crown = Some(crown.id);
                d.board.tickets.push(crown);
                d.board.sessions.push(seat);
                let filed = d.receipt_seen(id, None);
                assert!(d.check_seen(id, Some(&filed)).is_ok());
                if replaced {
                    d.board.sessions[0].id = uuid::Uuid::new_v4();
                } else {
                    d.board.sessions.push(worker);
                }
                assert!(d.check_seen(id, Some(&filed)).is_err(), "replaced: {replaced}");
            });
        }
        // A seen-less own write (a note, a tag) supersedes the read token, not
        // the filing's receipt (T-1047, not widened here): a sibling carrying
        // the filing's stamp is refused, and the write's own stamp serves.
        with_ticket(|d, id| {
            d.board.sessions.clear();
            let filed = d.receipt_seen(id, None);
            let write = d.receipt_write(id, None);
            d.board.ticket_mut(id).unwrap().title = "the crown's own note".into();
            let noted = d.receipt_seen(id, Some(write));
            assert!(d.check_seen(id, Some(&filed)).is_err());
            assert!(d.check_seen(id, Some(&noted)).is_ok());
        });
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

    /// T-1277: a receipt with no allowance (a move's, a rename's) went
    /// stale when its seat only ended its turn. The seat's own motion is
    /// not news; a person's edit after it still is.
    #[test]
    fn an_ordinary_receipt_outlives_its_seat_s_own_state_motion() {
        with_ticket(|d, id| {
            let seen = d.seen_token(id);
            rename(d, id, &seen, "own write");
            assert!(!d.receipts[&id].active);
            for (to, at) in [
                (SessionState::Running, 100),
                (SessionState::Idle { stop_reason: StopReason::EndTurn }, 200),
            ] {
                let from = d.board.sessions[0].state.clone();
                let rec = &mut d.board.sessions[0];
                rec.set_state(to.clone());
                rec.state_changed_at = Some(at);
                let rec = rec.clone();
                if to == SessionState::Running {
                    // Its turn was running when the crown wrote.
                    let fresh = d.seen_token(id);
                    rename(d, id, &fresh, "own write again");
                    continue;
                }
                d.receipt_state(
                    &rec,
                    &Change { from, to, attention_added: false, confidence: Confidence::High },
                );
            }
            let receipt = d.receipts[&id].stamp.clone();
            assert!(d.check_seen(id, Some(&receipt)).is_ok(), "the turn's own end");
            assert!(d.check_seen(id, Some(&seen)).is_ok(), "the chain's first input too");
            // The board's `on_done` after it, a rule's move, is not news.
            d.board.ticket_mut(id).unwrap().column = "REVIEW".into();
            d.note_hand_move(id, &Principal::Automation { rule: "automove".into() });
            assert!(d.check_seen(id, Some(&receipt)).is_ok(), "the board's own move");
            d.board.ticket_mut(id).unwrap().title = "a person's edit".into();
            let refused = d.check_seen(id, Some(&receipt)).unwrap_err();
            assert!(refused.contains("its title"), "{refused}");
        });
    }

    /// The seat's own motion inside its pane no longer stales an ordinary
    /// chain (T-1277); a park or wake by a road no state change reports
    /// still does, and a deferred receipt cannot revive it.
    #[test]
    fn a_deferred_receipt_cannot_revive_stamps_after_foreign_state_motion() {
        with_ticket(|d, id| {
            let seen = d.seen_token(id);
            rename(d, id, &seen, "own write");
            d.board.sessions[0].state_changed_at = Some(123);
            assert!(d.receipt_holds(id, &seen), "the seat's own motion");
            let was = d.board.sessions[0].state.clone();
            d.board.sessions[0].set_state(SessionState::Sleeping);
            assert!(!d.receipt_holds(id, &seen), "an ordinary chain stays strict");
            let refused = d.check_seen(id, Some(&seen)).unwrap_err();
            assert!(refused.contains("its agent parked or exited"), "{refused}");
            d.receipt_seen(id, None);
            d.board.sessions[0].set_state(was);
            assert!(!d.receipt_holds(id, &seen));
        });
    }

    /// T-1277: a refusal of a receipt names what revoked or changed it.
    #[test]
    fn a_refused_receipt_names_what_changed() {
        with_ticket(|d, id| {
            let seen = d.seen_token(id);
            let receipt = rename(d, id, &seen, "own write");
            let t = d.board.ticket_mut(id).unwrap();
            t.column = "REVIEW".into();
            d.note_hand_move(id, &Principal::Local);
            let refused = d.check_seen(id, Some(&receipt)).unwrap_err();
            assert!(refused.contains(": moved to REVIEW by a person (now in REVIEW"), "{refused}");
        });
        with_ticket(|d, id| {
            let seen = d.seen_token(id);
            let receipt = rename(d, id, &seen, "own write");
            let seat = d.board.sessions[0].clone();
            d.receipt_state(
                &seat,
                &Change {
                    from: SessionState::Idle { stop_reason: StopReason::EndTurn },
                    to: SessionState::Running,
                    attention_added: false,
                    confidence: Confidence::High,
                },
            );
            for stamp in [&receipt, &seen] {
                let refused = d.check_seen(id, Some(stamp)).unwrap_err();
                assert!(refused.contains(": a turn the crown did not start ("), "{refused}");
            }
        });
        with_ticket(|d, id| {
            let read = d.seen_token(id);
            d.board.ticket_mut(id).unwrap().title = "a person's edit".into();
            let refused = d.check_seen(id, Some(&read)).unwrap_err();
            assert!(refused.contains("changed since it was read (now in"), "a read names no part");
        });
    }

    /// T-1285, T-1283's sequence: the crown's start on a ticket whose
    /// worktree is still being cut (no seat at the write), the brief the
    /// board sends down the seat's mod once it is, the turn it begins, and
    /// two background tasks ending inside that turn. Claude Code fires a
    /// `UserPromptSubmit` for each `<task-notification>` it folds in, with
    /// no words of anyone's owed; the receipt was revoked there as "a prompt
    /// the crown did not send". A person's prompt still revokes it.
    #[test]
    fn a_start_s_receipt_outlives_its_agent_s_own_task_notifications() {
        with_ticket(|d, id| {
            d.queued.clear();
            let mut crown = d.board.tickets[0].clone();
            crown.id = ulid::Ulid::new();
            crown.short_key = "T-0".into();
            let mut crown_seat = d.board.sessions[0].clone();
            crown_seat.id = uuid::Uuid::new_v4();
            crown_seat.ticket = crown.id;
            d.board.crown = Some(crown.id);
            d.board.tickets.push(crown.clone());
            // The start is written before the worktree is cut: no seat yet.
            let mut worker = d.board.sessions.remove(0);
            d.board.sessions.push(crown_seat);
            let seen = d.seen_token(id);
            let write = d.receipt_write(id, Some(&seen));
            let started = d.receipt_seen(id, Some(write));
            d.allow_receipt_prompt(id, Prompt::Launch { crown: crown.id });
            // The worktree is cut and the crown's seat comes up.
            let sid = worker.id;
            let start = SessionState::Idle { stop_reason: StopReason::Unknown };
            worker.started_by = Some(crown.id);
            worker.state = start.clone();
            worker.pane_key = Some("1:%1".into());
            d.board.sessions.push(worker);
            d.machines.insert(sid, Machine::new(start, 1_000));
            assert!(d.check_seen(id, Some(&started)).is_ok(), "the seat is the start's");
            let prompt = |text: &str| HookFrame {
                session: sid.to_string(),
                event: "UserPromptSubmit".into(),
                reason: None,
                pane: None,
                road: Road::Hooks,
                payload: serde_json::json!({ "prompt": text }),
            };
            let brief = Parked { text: String::new(), brief: true, title: true };
            d.owed.insert(sid, Owed::submitted(id, brief, "f1".into(), Ack::PROMPT, now_ms()));
            d.on_hook(prompt("worker"));
            assert_eq!(d.board.session(sid).unwrap().state, SessionState::Running);
            assert!(d.check_seen(id, Some(&started)).is_ok(), "the brief is the start's own");
            for task in ["b67k5e8hn", "b15tfxlui"] {
                d.on_hook(prompt(&format!(
                    "<task-notification>\n<task-id>{task}</task-id>\n<status>completed</status>\n\
                     <summary>Background command \"cargo ut\" completed (exit code 0)</summary>\n\
                     </task-notification>"
                )));
                let held = d.check_seen(id, Some(&started));
                assert!(held.is_ok(), "a task notification is the agent's own: {held:?}");
            }
            d.on_hook(prompt("a person's words"));
            let refused = d.check_seen(id, Some(&started)).unwrap_err();
            assert!(refused.contains(": a prompt the crown did not send ("), "{refused}");
        });
        // A notification that wakes an idle seat begins a turn the crown did
        // not start, and that still revokes an ordinary chain.
        with_ticket(|d, id| {
            d.queued.clear();
            let sid = d.board.sessions[0].id;
            let idle = SessionState::Idle { stop_reason: StopReason::EndTurn };
            d.board.sessions[0].pane_key = Some("1:%0".into());
            d.machines.insert(sid, Machine::new(idle, 1_000));
            let seen = d.seen_token(id);
            let receipt = rename(d, id, &seen, "own write");
            d.on_hook(HookFrame {
                session: sid.to_string(),
                event: "UserPromptSubmit".into(),
                reason: None,
                pane: None,
                road: Road::Hooks,
                payload: serde_json::json!({
                    "prompt": "<task-notification><task-id>x</task-id></task-notification>"
                }),
            });
            let refused = d.check_seen(id, Some(&receipt)).unwrap_err();
            assert!(refused.contains(": a turn the crown did not start ("), "{refused}");
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
                words: String::new(),
                parts: Parts::default(),
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
        assert!(matches!(d.hold_async_stop(id, stop, &frame, None, 1_000).0, Signal::TurnStarted));
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
