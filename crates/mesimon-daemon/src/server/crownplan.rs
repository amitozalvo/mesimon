//! The crown accepts a plan (T-582): `accept_plan` on a plan dialog a claude
//! the crown started stopped on, under the board's `crown_answers` switch.
//!
//! The press is the board's own accept (T-420, `press_plan`): Enter on the
//! dialog's default row, only once the screen shows the dialog with the
//! cursor there, and only while the worker's checkout is quiet (T-429) —
//! the crown's own ticket not counted, since its turn waits on this
//! receipt and writes nothing meanwhile. The receipt waits for the plan's
//! own hook edge, as `answer_agent`'s does (T-569): `accepted` only on the
//! `PostToolUse` that ends the projected `ExitPlanMode`, `input_sent` for an
//! Enter no edge followed inside `PLAN_ACCEPT_CONFIRM_MS`, `queued` when a
//! busy checkout still holds the press past `CROWN_PLAN_WAIT_MS` (the
//! press keeps its place and goes when the checkout is quiet), and
//! `unknown` with a reason when no Enter went: a person answered first,
//! the dialog never showed at its default row, or what allowed the call no
//! longer holds. Sending a plan back stays a person's: the crown raises its
//! hand for that, and mesimon reads no plan's words to decide anything.

use super::*;

/// How long the receipt waits for a press its checkout holds back before it
/// answers `queued` (T-582). The press itself keeps waiting.
pub(super) const CROWN_PLAN_WAIT_MS: u64 = 30_000;

/// One crown accept on its way, keyed by the worker's session.
pub(super) struct CrownPlan {
    /// The crown's ticket, and the crown's session as the principal every
    /// step re-checks.
    crown: ulid::Ulid,
    session: uuid::Uuid,
    /// The worker's ticket, and the plan dialog the call named.
    ticket: ulid::Ulid,
    request: String,
    /// The call's reply, parked by the writer loop; `None` once a `queued`
    /// receipt answered it, or for a caller that is not the writer loop.
    reply: Option<Sender<ClientReply>>,
    asked_at: u64,
    /// The deadline for the hook edge, once the Enter went in.
    pressed: Option<u64>,
    /// Passes that found no dialog to press, and the last reason.
    tries: u8,
    miss: &'static str,
    /// What the worker's turn was asked for before the press marked it the
    /// crown's, given back when the Enter left the dialog standing.
    prior: Option<Option<TurnAsk>>,
}

impl Daemon {
    /// The crown's `accept_plan` (T-582), past the gates `handle_agent` runs
    /// first (the crown, the crown's own ticket, the stamp, `authorize`):
    /// the board's switch, the agent, its provenance and kind, the stop,
    /// the dialog, and a press already on its way, each refused in words.
    /// `Ok` is the worker's session; the receipt waits for the press to
    /// settle (`crown_plan_park_reply`).
    pub(super) fn crown_accept_plan(
        &mut self,
        crown: ulid::Ulid,
        session: uuid::Uuid,
        target: ulid::Ulid,
        key: &str,
        request: &str,
    ) -> std::result::Result<uuid::Uuid, String> {
        if !self.board.crown_answers {
            return Err(format!(
                "this board leaves {key}'s plan to a person (Settings → Agents → Crown answers \
                 questions is off); raise_hand on the crown's own ticket names the worker and the \
                 plan for them"
            ));
        }
        let Some(rec) = self.board.live_agent(target) else {
            return Err(format!("{key} has no agent, so no plan to accept"));
        };
        // T-539's line, as `answer_agent` draws it: the one who started an
        // agent is the one who accepts its plan.
        if rec.started_by.is_none() {
            return Err(format!(
                "a person started {key}'s agent, and the one who started it accepts its plan, in \
                 the pane, on the board or from Remote Control"
            ));
        }
        let id = rec.id;
        match rec.state {
            SessionState::RequiresAction { reason: Reason::Plan } => {}
            SessionState::RequiresAction { reason: Reason::Question } => {
                return Err(format!(
                    "{key}'s agent is stopped on a question, not a plan; answer_agent answers a \
                     question"
                ));
            }
            SessionState::RequiresAction { reason } => {
                let word = agent_reason_word(reason);
                return Err(format!(
                    "{key}'s agent is stopped on a {word}, not a plan; a {word} is never the \
                     crown's to answer: a person answers it, in the pane or from Remote Control"
                ));
            }
            ref other => {
                return Err(format!(
                    "{key}'s agent is not stopped on a plan (it is {}); accept_plan accepts the \
                     plan get_ticket shows in needs_you",
                    agent_state_word(other)
                ));
            }
        }
        if rec.kind != SessionKind::Claude {
            return Err(format!(
                "{key}'s agent is not a claude; the board accepts a claude's plan for the crown, \
                 and this one is a person's"
            ));
        }
        // A person's answer always wins: a plan the hook stream ended, or
        // one replaced by another, refuses the crown's accept.
        if self.control_plan(id).is_none_or(|(current, _)| current != request) {
            return Err("plan changed; read get_ticket again".into());
        }
        let queued = self.queued.iter().any(|q| {
            matches!(q.seat, QueuedSeat::Pane(s) if s == id) && (q.accept_plan || q.send_on_accept)
        });
        if self.crown_plans.contains_key(&id) || self.plan_accept.contains_key(&id) || queued {
            return Err(format!("an accept for {key}'s plan is already on its way"));
        }
        self.crown_plans.insert(
            id,
            CrownPlan {
                crown,
                session,
                ticket: target,
                request: request.into(),
                reply: None,
                asked_at: now_ms(),
                pressed: None,
                tries: 0,
                miss: "",
                prior: None,
            },
        );
        // The dialog is up: press now where the checkout lets it, rather
        // than a second from now.
        self.service_crown_plans();
        Ok(id)
    }

    /// The writer's reply for the `accept_plan` call that registered the
    /// press for `id` (T-582): kept with it, answered when it settles.
    /// Handed back when there is no such press, to be sent now.
    pub(super) fn crown_plan_park_reply(
        &mut self,
        id: uuid::Uuid,
        reply: Sender<ClientReply>,
    ) -> Option<Sender<ClientReply>> {
        match self.crown_plans.get_mut(&id) {
            Some(plan) if plan.reply.is_none() => {
                plan.reply = Some(reply);
                None
            }
            _ => Some(reply),
        }
    }

    /// Whether the crown's press may still go (T-582): what allowed it at
    /// the call holds — the crown on its ticket, the board's switch, a
    /// claude the crown started on that ticket at its plan, and the crown's
    /// `Mutate` on it.
    fn crown_plan_allowed(&self, plan: &CrownPlan, id: uuid::Uuid) -> bool {
        let by = Principal::Agent { session: plan.session };
        self.board.is_crowned(plan.crown)
            && self.board.crown_answers
            && self.board.live_agent(plan.ticket).is_some_and(|rec| {
                rec.id == id
                    && rec.started_by.is_some()
                    && rec.kind == SessionKind::Claude
                    && rec.state == SessionState::RequiresAction { reason: Reason::Plan }
            })
            && !authorize(&by, &Action::Mutate, &Resource::Ticket { id: plan.ticket }).denied()
    }

    /// Press every crown accept whose worker's checkout is quiet (T-582),
    /// one per checkout per pass, after the board's own flagged asks
    /// (`service_plan_accepts`) — a press of theirs in flight holds the
    /// checkout here too (`accept_holders`). The crown's own ticket is no
    /// holder: its turn is waiting on this receipt. A press the screen does
    /// not show at its default row is retried on the next pass, up to
    /// `PLAN_ACCEPT_TRIES`, and never blind.
    pub(super) fn service_crown_plans(&mut self) -> bool {
        let mut seen: Vec<String> = Vec::new();
        let mut due: Vec<uuid::Uuid> = Vec::new();
        for (id, plan) in &self.crown_plans {
            if plan.pressed.is_some() || self.plan_accept.contains_key(id) {
                continue;
            }
            if !self.crown_plan_allowed(plan, *id) {
                continue;
            }
            let Some(cwd) = self.board.sessions.iter().find(|s| s.id == *id).map(|s| s.cwd.clone())
            else {
                continue;
            };
            if seen.contains(&cwd) || !self.crown_plan_quiet(plan.crown, &cwd) {
                continue;
            }
            seen.push(cwd);
            due.push(*id);
        }
        let mut changed = false;
        for id in due {
            let pressed = self.press_plan(id);
            let Some(plan) = self.crown_plans.get_mut(&id) else { continue };
            match pressed {
                Ok(_) => {
                    plan.pressed = Some(now_ms() + PLAN_ACCEPT_CONFIRM_MS);
                    // The accepted plan's turn is the crown's from here: its
                    // end wakes the crown, as the turn that takes an answer
                    // does (T-569).
                    let (ticket, crown) = (plan.ticket, plan.crown);
                    let prior = self.turn_asks.get(&ticket).copied();
                    if let Some(plan) = self.crown_plans.get_mut(&id) {
                        plan.prior = Some(prior);
                    }
                    self.mark_turn(ticket, TurnAsk::Crown(crown));
                    changed = true;
                }
                Err(why) => {
                    plan.tries = plan.tries.saturating_add(1);
                    plan.miss = why;
                }
            }
        }
        changed
    }

    /// Whether `cwd` is quiet for the crown's press: nothing working there
    /// against a plan accept but the crown itself, and the checkout known.
    fn crown_plan_quiet(&self, crown: ulid::Ulid, cwd: &str) -> bool {
        self.crown_plan_holders(crown, cwd).is_empty() && !self.checkout_unresolved(cwd)
    }

    fn crown_plan_holders(&self, crown: ulid::Ulid, cwd: &str) -> Vec<ulid::Ulid> {
        self.accept_holders(cwd).into_iter().filter(|t| *t != crown).collect()
    }

    /// Settle every crown accept that has its answer (T-582), each tick:
    /// the plan's own hook edge after the press is `accepted`, an Enter
    /// with no edge by the deadline (or a dialog that left another way) is
    /// `input_sent`, and before any press a person's answer, a dialog never
    /// shown at its default row, or a call no longer allowed is `unknown`.
    /// A press its checkout still holds past `CROWN_PLAN_WAIT_MS` answers
    /// `queued` and keeps waiting.
    pub(super) fn settle_crown_plans(&mut self) -> bool {
        if self.crown_plans.is_empty() {
            return false;
        }
        let now = now_ms();
        let mut settled: Vec<(uuid::Uuid, &'static str, Option<String>)> = Vec::new();
        let mut queued: Vec<(uuid::Uuid, String)> = Vec::new();
        for (id, plan) in &self.crown_plans {
            let at_plan = self.session_at_plan(*id);
            let edge = self.control_dialog_edge(*id, &plan.request);
            if let Some(deadline) = plan.pressed {
                match edge {
                    Some(mesophon::DialogEdge::Answered) => settled.push((*id, "accepted", None)),
                    Some(_) => settled.push((*id, "input_sent", None)),
                    // No edge by the deadline with the record still at its
                    // plan: the dialog stands (`crown_plan_settled`).
                    None if !at_plan || now >= deadline => settled.push((*id, "input_sent", None)),
                    None => {}
                }
                continue;
            }
            if edge == Some(mesophon::DialogEdge::Answered) {
                settled.push((*id, "unknown", Some("a_person_answered".into())));
            } else if !at_plan || edge.is_some() || !self.crown_plan_allowed(plan, *id) {
                settled.push((*id, "unknown", Some("state_changed".into())));
            } else if plan.tries >= PLAN_ACCEPT_TRIES {
                settled.push((*id, "unknown", Some(miss_word(plan.miss).into())));
            } else if plan.reply.is_some() && now >= plan.asked_at + CROWN_PLAN_WAIT_MS {
                let cwd = self.board.sessions.iter().find(|s| s.id == *id).map(|s| s.cwd.clone());
                let holders = cwd.map(|c| self.crown_plan_holders(plan.crown, &c));
                let keys = self.keys_of(&holders.unwrap_or_default());
                let why = if keys.is_empty() {
                    "checkout_unresolved".to_string()
                } else {
                    format!("after {}", keys.join(", "))
                };
                queued.push((*id, why));
            }
        }
        for (id, why) in queued {
            let Some(plan) = self.crown_plans.get_mut(&id) else { continue };
            let (ticket, reply) = (plan.ticket, plan.reply.take());
            self.crown_plan_reply(reply, ticket, "queued", Some(why));
        }
        let changed = !settled.is_empty();
        for (id, status, reason) in settled {
            if let Some(plan) = self.crown_plans.remove(&id) {
                self.crown_plan_settled(id, plan, status, reason);
            }
        }
        changed
    }

    /// Everyone sees the crown's accept (T-582): the feed's `accept_plan`
    /// with its outcome, and once the Enter went in and the dialog did not
    /// stay standing, `♛ accepted plan` on the card and the claude's line
    /// `plan accepted by T-411` until the next state edge. An Enter that
    /// left the dialog standing gives the turn its mark back; an accept no
    /// Enter carried shows nothing but its feed line.
    fn crown_plan_settled(
        &mut self,
        id: uuid::Uuid,
        plan: CrownPlan,
        status: &'static str,
        reason: Option<String>,
    ) {
        self.feed.board_outcome("agent", "accept_plan", Some(plan.ticket), status);
        // Standing is judged by the edge, not the state: a record leaving
        // its plan still reads `Plan` through the attention settle (1.5 s)
        // after the `PostToolUse` that accepted it.
        let edge = self.control_dialog_edge(id, &plan.request);
        let standing = edge.is_none() && self.session_at_plan(id);
        if plan.pressed.is_some() && standing {
            if let Some(prior) = plan.prior {
                match prior {
                    Some(ask) => self.turn_asks.insert(plan.ticket, ask),
                    None => self.turn_asks.remove(&plan.ticket),
                };
            }
        } else if plan.pressed.is_some() {
            let by = self.board.ticket(plan.crown).map(|t| t.short_key.clone()).unwrap_or_default();
            let line = format!("plan accepted by {by}");
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.detail = Some(line.clone());
                self.crown_answer_lines.insert(id, line);
                self.persist_sessions();
            }
            self.crown_touched(plan.crown, plan.ticket, "accepted plan");
            self.broadcast();
        }
        self.crown_plan_reply(plan.reply, plan.ticket, status, reason);
    }

    fn crown_plan_reply(
        &mut self,
        reply: Option<Sender<ClientReply>>,
        ticket: ulid::Ulid,
        outcome: &str,
        reason: Option<String>,
    ) {
        let Some(reply) = reply else { return };
        let key = self.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
        let response = Response::AgentPlanAccepted {
            key,
            outcome: outcome.into(),
            reason,
            seen: Some(self.seen_token(ticket)),
        };
        // The receipt goes mid-tick, before the tick's own flush: what it
        // says is on disk first, so a reader of the feed after it finds it.
        let _ = self.feed.flush();
        let _ = reply.send(ClientReply { response, delivered: None });
    }
}

/// `press_plan`'s refusal as the receipt's reason word.
fn miss_word(miss: &str) -> &'static str {
    if miss == crate::plan_dialog::NOT_AT_DEFAULT {
        "not_at_default"
    } else if miss == crate::plan_dialog::NOT_RECOGNISED || miss.is_empty() {
        "shape_unrecognised"
    } else if miss == "could not press enter" {
        "pane_unreachable"
    } else {
        "state_changed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The receipt's reason words for a press that never went (T-582).
    #[test]
    fn a_miss_is_named_in_a_word() {
        assert_eq!(miss_word(crate::plan_dialog::NOT_AT_DEFAULT), "not_at_default");
        assert_eq!(miss_word(crate::plan_dialog::NOT_RECOGNISED), "shape_unrecognised");
        assert_eq!(miss_word(""), "shape_unrecognised");
        assert_eq!(miss_word("could not press enter"), "pane_unreachable");
        assert_eq!(miss_word("no plan waiting"), "state_changed");
    }
}
