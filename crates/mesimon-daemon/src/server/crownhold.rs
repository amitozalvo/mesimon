//! The crown's train hold (T-919): `t` on another ticket's page, for review.
//!
//! A crown that reviews a worker's branch before it lands could only ask
//! the worker not to merge, and the worker never merges: the merge train
//! does, the moment the branch is ahead in a column it reaches and the
//! agent's turn is over. So the person turned the whole train off to keep
//! one review. `hold_ticket` takes one ticket out instead, with the
//! person's own flag (`Ticket::manual_merge`, which the planner, the train's
//! busy test and the crown's wake already read), and hands it back.
//!
//! **Whose hold it is** is the crown ledger's (`Daemon::crown_held`, kept in
//! `crown.json`): the crown returns only a hold it made, and a person's `t`
//! on the ticket makes the hold the person's or ends it. A crown leaving
//! leaves its holds standing as the person's: a branch held for a review
//! nobody will finish waits for a person, never for the train.
//!
//! **What a hold does not do.** A merge is made on the writer thread in one
//! piece, so a hold lands before it or after it, never during; after it the
//! branch reads merged and the hold is refused in words. A rebase the train
//! already asked of the agent runs its turn: words that reached the agent
//! are not taken back, and the train merges nothing after that turn while
//! the hold stands. Nothing is said to the agent either way.
//!
//! **What a return does not do.** It forces nothing: the ticket is back on
//! the planner's lists exactly as an unheld ticket is, behind the column's
//! train setting, the branch's state, the agent's seat, the board's order,
//! the train's fuse and its refused-merge memory — which a person's `t`
//! clears and the crown's return does not. Under a supervised crown the
//! return is a person's, as the crown's own merge is.

use super::*;

impl Daemon {
    /// Whose train hold `id` is under, in `get_ticket`'s word: `crown`,
    /// `person`, or `None` while the train may take it.
    pub(super) fn train_hold_word(&self, id: ulid::Ulid) -> Option<&'static str> {
        let t = self.board.ticket(id)?;
        if !t.manual_merge {
            return None;
        }
        Some(if self.crown_held.contains(&id) { "crown" } else { "person" })
    }

    /// A person's `t` (`SetManualMerge`) on `id`: whatever it sets, the hold
    /// is no longer the crown's — off ends it, on makes it the person's.
    pub(super) fn person_touched_hold(&mut self, id: ulid::Ulid) {
        if self.crown_held.contains(&id) {
            self.crown_held.remove(&id);
            self.persist_crown();
        }
    }

    /// The crown left, or another ticket took it: its holds stay on the
    /// tickets, as the person's from here, each said once in the feed.
    pub(super) fn release_crown_holds(&mut self) {
        if self.crown_held.is_empty() {
            return;
        }
        for id in std::mem::take(&mut *self.crown_held) {
            self.feed.board("automation", "hold_kept", Some(id));
        }
    }

    /// `Command::AgentHoldTicket` from `session` on `own`.
    pub(super) fn agent_hold_ticket(
        &mut self,
        own: ulid::Ulid,
        session: uuid::Uuid,
        key: String,
        hold: bool,
        seen: Option<String>,
    ) -> Response {
        let target = match self.keyed_target(own, Some(&key), seen.as_deref(), false) {
            Ok(t) => t,
            Err(message) => return Response::Err { message },
        };
        if target == own {
            return Response::Err {
                message: format!(
                    "{key} is this session's own ticket; hold_ticket is for a worker's ticket, \
                     and this one's merge is the board's"
                ),
            };
        }
        let by = Principal::Agent { session };
        let action = Action::HoldFromTrain { crown: self.board.is_crowned(own) };
        if let Decision::Deny { reason } = authorize(&by, &action, &Resource::Ticket { id: target })
        {
            return Response::Err { message: format!("denied: {reason}") };
        }
        let Some(t) = self.board.ticket(target) else { return no_such_ticket() };
        let key = t.short_key.clone();
        if t.workspace_strategy() != WorkspaceStrategy::Worktree {
            return Response::Err {
                message: format!(
                    "{key} works in the shared checkout, whose commits are on the base the \
                     moment they exist: the merge train has no branch of it to hold"
                ),
            };
        }
        let merged = self.merge_state_word(target).as_deref() == Some("merged");
        let changed = match (hold, self.train_hold_word(target)) {
            (true, _) if merged => {
                return Response::Err {
                    message: format!(
                        "{key}'s branch is already merged; a hold takes back no merge that \
                         happened"
                    ),
                };
            }
            (true, Some(_)) | (false, None) => false,
            (false, Some("person")) => {
                return Response::Err {
                    message: format!(
                        "{key} was taken out of the merge train by a person (t on its page), \
                         who returns it; raise_hand on this session's ticket asks them"
                    ),
                };
            }
            (false, Some(_)) if !self.board.crown_mode.merges() => {
                return Response::Err {
                    message: format!(
                        "Settings → Agents → Crown mode is supervised: a person returns {key} \
                         to the merge train, t on its page; the crown's hold stands until then"
                    ),
                };
            }
            (true, None) | (false, Some(_)) => true,
        };
        if changed {
            if hold {
                self.crown_held.insert(target);
            } else {
                self.crown_held.remove(&target);
            }
            self.persist_crown();
            self.with_ticket(target, |t| t.manual_merge = hold);
            let cmd = if hold { "hold_ticket" } else { "return_ticket" };
            self.feed.board(by.actor(), cmd, Some(target));
            self.crown_touched(own, target, if hold { "held" } else { "returned" });
        }
        let held = self.train_hold_word(target);
        Response::AgentHeld {
            key: key.clone(),
            held: held.is_some(),
            by: held.unwrap_or_default().to_string(),
            changed,
            merge: self.merge_state_word(target).is_some().then(|| self.merge_by(target)),
            detail: self.hold_detail(target, &key, held, changed),
            seen: Some(self.receipt_seen(target)),
        }
    }

    /// The receipt's one sentence: what the hold now does, and what it does
    /// not — a rebase already asked runs, a pull request's merge is the
    /// forge's, a returned ticket waits for the train's own gates.
    fn hold_detail(
        &self,
        target: ulid::Ulid,
        key: &str,
        held: Option<&str>,
        changed: bool,
    ) -> String {
        let prs = self.through_prs();
        match held {
            Some("person") => format!(
                "{key} is held out of the merge train by a person (t on its page); the hold \
                 and its return stay the person's"
            ),
            Some(_) => {
                let mut s = if changed {
                    format!(
                        "{key} is out of the merge train: the train neither merges its branch \
                         nor asks its agent to rebase until hold_ticket returns it, and its \
                         delivery wakes this session; merge_ticket still merges it"
                    )
                } else {
                    format!("{key} was already held by the crown; nothing changed")
                };
                if self.train.in_rebase_turn(target, self.base_tip_of(target)) {
                    s.push_str(
                        ". A rebase the train asked of its agent before the hold is not taken \
                         back: that turn runs, and the train merges nothing after it",
                    );
                }
                if prs {
                    s.push_str(
                        ". This board merges through pull requests: the hold stops the \
                         train's rebase asks, never the forge's merge",
                    );
                }
                s
            }
            None => {
                let mut s = if changed {
                    format!(
                        "{key} is back in the merge train's reach: nothing merges it now, and \
                         the train takes it when its own gates pass (its column's train \
                         setting, a branch ahead of its base, an agent whose turn is over), in \
                         board order"
                    )
                } else {
                    format!("{key} is not held; the train's own gates decide its merge")
                };
                if !self.train.is_armed() {
                    s.push_str(
                        ". The merge train is off, so until a person turns it on nothing \
                         merges it on the train's account",
                    );
                }
                s
            }
        }
    }
}
