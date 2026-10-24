//! The crown's ask to another ticket's agent, as one record (T-938).
//!
//! T-930's worker was sent `ask_agent` with `deliver: now` on the mod road.
//! Claude Code holds a plugin's `submit` until the running turn ends, which
//! was an hour away; the receipt said `sent_now`, `get_ticket` said nothing,
//! and the crown's resend with `immediately` put a second copy in. The
//! receipt named the road the handler chose, not what the agent had.
//!
//! So each ticket keeps the crown's last ask as one record, which the
//! receipt and `get_ticket`'s `asked` both read, in one word:
//!
//! - **queued** — on the board's queue; a new ask replaces it;
//! - **sent** — the dispatch went through (the paste into the pane, the
//!   frame into the mod's outbox), and no edge of the agent's has named
//!   these words yet;
//! - **handed** — the agent's own edge for them: the `UserPromptSubmit`
//!   that acks the owed paste tagged with this ask, with nobody in the pane
//!   who could have typed it instead; the mod's `entered` for its frame;
//!   the Codex steer or launch counted;
//! - **taken** — the turn these words started has ended. It says nothing
//!   of absorption inside a turn already running (words that went into one
//!   stay `handed`), and an end, normal or not, says nothing of whether the
//!   work succeeded;
//! - **dropped** — a refusal or a cancellation confirmed before the agent
//!   had them, and by whom;
//! - **unknown** — an effect the board cannot read: keys that failed after
//!   the words were in the box, a mod frame never acknowledged, a pane
//!   replaced under the words, a restart. Never resent.
//!
//! A second ask is refused while the first reads `sent` or `handed`
//! (`blocks`), so a resend never puts in a second copy behind the board's
//! back. It lifts only at an edge the board saw: `taken`, `dropped`,
//! `unknown`, or the end of the turn the words went into, for words that
//! did not start a turn of their own; a mod `submit` always starts its own.
//! Without a request key a lost receipt, or a retry after a restart, can
//! still run twice: the rule refuses a second ask while the first is out,
//! and claims no more.
//!
//! Memory only, as every crown ask is (T-550): a restart keeps nothing but
//! the turn the crown's ask rode (`TurnAsk::Asked` in `crown.json`), which
//! comes back as `unknown`. A restart between `sent` and `handed` leaves no
//! trace, so no `asked` after a restart is no evidence, not proof that
//! nothing was sent.

use super::*;
use crownwake::Said;

/// Where the crown's last ask to a ticket stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Queued,
    Sent,
    Handed,
    Taken,
    Dropped { by: &'static str, why: String },
    Unknown { why: String },
}

impl Stage {
    pub(super) fn word(&self) -> &'static str {
        match self {
            Stage::Queued => "queued",
            Stage::Sent => "sent",
            Stage::Handed => "handed",
            Stage::Taken => "taken",
            Stage::Dropped { .. } => "dropped",
            Stage::Unknown { .. } => "unknown",
        }
    }
}

/// The crown's last ask to one ticket's agent.
#[derive(Clone, Debug)]
pub(super) struct CrownAsk {
    pub(super) id: ulid::Ulid,
    /// The asking ticket.
    pub(super) crown: ulid::Ulid,
    /// The seat the words went to, and its pane then: every callback is
    /// fenced by them.
    pub(super) session: Option<uuid::Uuid>,
    pub(super) pane: Option<String>,
    /// The mod frame the words went down in, when they did.
    pub(super) frame: Option<String>,
    /// That frame is a `submit`: Claude Code holds it to the running turn's
    /// end and runs it as a turn of its own (the T-930 road).
    pub(super) submit: bool,
    /// The words start a turn of their own: sent to an idle or parked
    /// agent, or down the mod as a `submit`.
    pub(super) own_turn: bool,
    /// The turn the words went into has ended: no longer a block.
    pub(super) released: bool,
    pub(super) stage: Stage,
    pub(super) since_ms: u64,
}

impl CrownAsk {
    pub(super) fn queued(crown: ulid::Ulid, now: u64) -> Self {
        CrownAsk {
            id: ulid::Ulid::new(),
            crown,
            session: None,
            pane: None,
            frame: None,
            submit: false,
            own_turn: false,
            released: false,
            stage: Stage::Queued,
            since_ms: now,
        }
    }

    fn set(&mut self, stage: Stage, now: u64) {
        self.stage = stage;
        self.since_ms = now;
    }

    /// Out with the agent and not yet settled: a second ask would be a
    /// second copy.
    pub(super) fn blocks(&self) -> bool {
        matches!(self.stage, Stage::Sent | Stage::Handed) && !self.released
    }

    pub(super) fn view(&self, now: u64) -> mesimon_core::command::AgentAskedView {
        let (by, why) = match &self.stage {
            Stage::Dropped { by, why } => ((*by).to_string(), why.clone()),
            Stage::Unknown { why } => (String::new(), why.clone()),
            _ => (String::new(), String::new()),
        };
        mesimon_core::command::AgentAskedView {
            status: self.stage.word().into(),
            by,
            since_secs: Some(now.saturating_sub(self.since_ms) / 1000),
            ask: self.id.to_string(),
            why,
        }
    }

    /// The refusal a second ask meets while this one is out.
    pub(super) fn refusal(&self, key: &str, now: u64) -> String {
        let secs = now.saturating_sub(self.since_ms) / 1000;
        let wait = if self.submit {
            "Claude Code runs a plugin's prompt when the running turn ends"
        } else {
            "the turn it went into has not ended"
        };
        format!(
            "{key}'s agent has ask {} ({}, {secs}s): {wait}. A second ask is refused while the \
             first is out; get_ticket's asked says when it is taken, dropped or unknown.",
            self.id,
            self.stage.word(),
        )
    }
}

/// What a restored ledger's ask comes back as (T-938): its turn was kept,
/// what became of it was not.
pub(super) const RESTORED_WHY: &str =
    "an earlier crown ask is recorded; its outcome was not retained";

impl Daemon {
    /// The crown's ask on `target`, while it is the latest one.
    fn crown_ask_mut(&mut self, target: ulid::Ulid, ask: ulid::Ulid) -> Option<&mut CrownAsk> {
        self.crown_asks.get_mut(&target).filter(|a| a.id == ask)
    }

    /// A new ask parked on the board: the record starts over.
    pub(super) fn crown_ask_queued(&mut self, target: ulid::Ulid, crown: ulid::Ulid) {
        let board = &self.board;
        self.crown_asks.retain(|t, _| board.ticket(*t).is_some());
        self.crown_asks.insert(target, CrownAsk::queued(crown, now_ms()));
    }

    /// The queued words were dispatched: into `session`'s pane, down its
    /// mod, or parked on its wake. `working` is whether its agent was in a
    /// turn just before. The owed entry the dispatch made carries the ask
    /// from here, so only its own ack hands it.
    pub(super) fn crown_ask_sent(
        &mut self,
        target: ulid::Ulid,
        session: uuid::Uuid,
        working: bool,
    ) {
        let owed = self.owed.get(&session).filter(|o| o.ticket == target);
        let frame = owed.and_then(|o| o.frame.clone());
        let submit = owed.is_some_and(|o| o.frame.is_some() && !o.fill);
        // A parked seat's pane is the one its wake makes: bound at the first
        // edge that names it (`bind_pane`), never the pane it slept in.
        let pane = self
            .board
            .sessions
            .iter()
            .find(|s| s.id == session && s.state.has_pane())
            .and_then(|s| s.pane_key.clone());
        let Some(ask) = self.crown_asks.get_mut(&target).filter(|a| a.stage == Stage::Queued)
        else {
            return;
        };
        ask.session = Some(session);
        ask.pane = pane;
        ask.frame = frame;
        ask.submit = submit;
        ask.own_turn = submit || !working;
        ask.set(Stage::Sent, now_ms());
        let id = ask.id;
        if let Some(o) = self.owed.get_mut(&session).filter(|o| o.ticket == target) {
            o.ask = Some(id);
        }
        // A wake behind a worktree rebuild parks its words on the resume
        // (T-278), and they are owed when it replays (`crown_ask_retag`).
    }

    /// The session a queued ask's seat sends to, and whether its agent is
    /// in a turn just before the send: words into a running turn do not
    /// start one of their own.
    pub(super) fn ask_seat(&self, seat: &QueuedSeat) -> (Option<uuid::Uuid>, bool) {
        match seat {
            QueuedSeat::Pane(id) => (Some(*id), !self.session_idle(*id)),
            QueuedSeat::Wake(id) => (Some(*id), false),
            QueuedSeat::Start(_) => (None, false),
        }
    }

    /// The crown's queued ask failed at its dispatch: what reached the pane
    /// before the error cannot be read, so it is unknown, never resent.
    pub(super) fn crown_ask_failed(
        &mut self,
        target: ulid::Ulid,
        by: Option<ulid::Ulid>,
        resp: &Response,
    ) {
        if by.is_none() {
            return;
        }
        let why = match resp {
            Response::Err { message } => {
                format!("the dispatch failed: {}", mesimon_core::text::cap_bytes(message, 160))
            }
            _ => "the dispatch failed".to_string(),
        };
        self.crown_ask_unknown(target, None, &why);
    }

    /// A wake's words parked again after a rebuild: the owed entry the
    /// replay made is the ask's, if its record waits on that seat.
    pub(super) fn crown_ask_retag(&mut self, target: ulid::Ulid, session: uuid::Uuid) {
        let Some(id) = self
            .crown_asks
            .get(&target)
            .filter(|a| a.stage == Stage::Sent && a.session == Some(session))
            .map(|a| a.id)
        else {
            return;
        };
        if let Some(o) = self.owed.get_mut(&session).filter(|o| o.ask.is_none()) {
            o.ask = Some(id);
        }
    }

    /// A refused `submit` whose words went in by the paste road after a
    /// stop (T-894's road): the ask is on that road now, no frame of the
    /// mod's to wait on, into a stopped agent's box as a turn of its own.
    /// Its release and settle rules are the paste's from here.
    pub(super) fn crown_ask_repasted(&mut self, session: uuid::Uuid, ask: ulid::Ulid) {
        let Some(a) = self
            .crown_asks
            .values_mut()
            .find(|a| a.id == ask && a.stage == Stage::Sent && a.session == Some(session))
        else {
            return;
        };
        a.frame = None;
        a.submit = false;
        a.own_turn = true;
    }

    /// The agent's own edge for the words: its turn now carries the ask.
    fn crown_ask_handed(&mut self, target: ulid::Ulid, ask: ulid::Ulid) -> bool {
        let Some(a) = self.crown_ask_mut(target, ask).filter(|a| a.stage == Stage::Sent) else {
            return false;
        };
        a.set(Stage::Handed, now_ms());
        let crown = a.crown;
        self.mark_turn(target, TurnAsk::Asked { crown, ask, said: Said::Handed });
        true
    }

    /// The owed entry tagged with `ask` was acked: `session`'s prompt hook,
    /// or a Codex steer counted or turn begun (`ack_edge` decides what it
    /// proves).
    pub(super) fn crown_ask_acked(
        &mut self,
        target: ulid::Ulid,
        session: uuid::Uuid,
        ask: ulid::Ulid,
    ) {
        let codex = self.session_kind(session) == SessionKind::Codex;
        let pane = self.pane_of(session);
        let Some(a) = self.crown_ask_mut(target, ask) else { return };
        if !bind_pane(a, pane.as_deref()) {
            return;
        }
        let crown = a.crown;
        match ack_edge(a, codex) {
            AckEdge::Hand => {
                self.crown_ask_handed(target, ask);
            }
            AckEdge::Mark => {
                self.mark_turn(target, TurnAsk::Asked { crown, ask, said: Said::Sent });
            }
            AckEdge::Nothing => {}
        }
    }

    /// The mod's `entered` for `frame` from `session`: found by its frame,
    /// so a report after the owed entry's window still lands.
    pub(super) fn crown_ask_entered(&mut self, session: uuid::Uuid, frame: &str) -> bool {
        let Some((target, ask)) = self.crown_ask_by_frame(session, frame) else { return false };
        self.crown_ask_handed(target, ask)
    }

    /// The ask a mod report from `session` about `frame` is for: the one
    /// whose seat, frame and pane it names. A report delayed from a pane
    /// since replaced is no one's. A parked seat's ask binds its pane here.
    fn crown_ask_by_frame(
        &mut self,
        session: uuid::Uuid,
        frame: &str,
    ) -> Option<(ulid::Ulid, ulid::Ulid)> {
        let pane = self.pane_of(session);
        let (target, a) = self
            .crown_asks
            .iter_mut()
            .find(|(_, a)| reports_to(a, session, frame, pane.as_deref()))?;
        bind_pane(a, pane.as_deref());
        Some((*target, a.id))
    }

    /// Whether a mod report from `session` about `frame` is a crown ask's,
    /// by the same fence `crown_ask_by_frame` keeps, binding nothing.
    pub(super) fn crown_ask_owns(&self, session: uuid::Uuid, frame: &str) -> bool {
        let pane = self.pane_of(session);
        self.crown_asks.values().any(|a| reports_to(a, session, frame, pane.as_deref()))
    }

    /// The pane `session` lives in now, if it has one.
    fn pane_of(&self, session: uuid::Uuid) -> Option<String> {
        self.board
            .sessions
            .iter()
            .find(|s| s.id == session && s.state.has_pane())
            .and_then(|s| s.pane_key.clone())
    }

    /// The mod's refusal of the frame (a confirmed one): dropped by the
    /// agent's side, with its reason.
    pub(super) fn crown_ask_refused(
        &mut self,
        session: uuid::Uuid,
        frame: &str,
        why: &str,
    ) -> bool {
        let Some((target, ask)) = self.crown_ask_by_frame(session, frame) else { return false };
        self.crown_ask_dropped(target, Some(ask), "agent", why)
    }

    /// The frame's effect cannot be read: unknown, never resent.
    pub(super) fn crown_ask_unread(&mut self, session: uuid::Uuid, frame: &str, why: &str) -> bool {
        let Some((target, ask)) = self.crown_ask_by_frame(session, frame) else { return false };
        self.crown_ask_unknown(target, Some(ask), why)
    }

    /// Dropped, confirmed before the agent had the words. `ask` fences it
    /// to one record; `None` is the ticket's latest, still queued or sent.
    pub(super) fn crown_ask_dropped(
        &mut self,
        target: ulid::Ulid,
        ask: Option<ulid::Ulid>,
        by: &'static str,
        why: &str,
    ) -> bool {
        let Some(a) = self.crown_asks.get_mut(&target) else { return false };
        if ask.is_some_and(|id| id != a.id) || !matches!(a.stage, Stage::Queued | Stage::Sent) {
            return false;
        }
        a.set(Stage::Dropped { by, why: why.to_string() }, now_ms());
        true
    }

    /// Unknown: an effect the board cannot read, from a dispatch that
    /// errored part-way (still `queued` then) or words already out.
    pub(super) fn crown_ask_unknown(
        &mut self,
        target: ulid::Ulid,
        ask: Option<ulid::Ulid>,
        why: &str,
    ) -> bool {
        let Some(a) = self.crown_asks.get_mut(&target) else { return false };
        if ask.is_some_and(|id| id != a.id)
            || !matches!(a.stage, Stage::Queued | Stage::Sent | Stage::Handed)
        {
            return false;
        }
        a.set(Stage::Unknown { why: why.to_string() }, now_ms());
        true
    }

    /// A turn of `worker`'s agent ended, proven (`turn_ended`), carrying
    /// `asked`. For the ask whose id it carries, the word at this end is the
    /// record's own (`end_said`): `taken` only where real evidence handed
    /// words that started this turn, nothing at all for one dropped. Any
    /// other ask out on the ticket went into the turn that just ended and
    /// no longer blocks; a mod `submit` waits for its own turn. The end of
    /// a turn in a pane the ask did not go to is not its: the ask is
    /// unknown from here, and the replacement's turn carries none of it.
    pub(super) fn crown_ask_turn_ended(
        &mut self,
        worker: ulid::Ulid,
        asked: Option<TurnAsk>,
    ) -> Option<TurnAsk> {
        let now = now_ms();
        let seat = self.board.live_agent(worker).map(|s| (s.id, s.pane_key.clone()));
        let Some(a) = self.crown_asks.get_mut(&worker) else { return asked };
        let mine = matches!(asked, Some(TurnAsk::Asked { ask, .. }) if ask == a.id);
        let same_seat = seat.as_ref().is_some_and(|(id, pane)| {
            a.session.is_none_or(|s| s == *id) && bind_pane(a, pane.as_deref())
        });
        if a.session.is_some() && !same_seat {
            if matches!(a.stage, Stage::Sent | Stage::Handed) {
                a.set(
                    Stage::Unknown {
                        why: "the pane it went to was replaced before the turn it went into ended"
                            .into(),
                    },
                    now,
                );
            }
            return if mine { None } else { asked };
        }
        let said = match asked {
            Some(TurnAsk::Asked { crown, ask, .. }) if mine => {
                let said = end_said(&a.stage, a.own_turn);
                if said == Some(Said::Taken) {
                    a.set(Stage::Taken, now);
                }
                said.map(|said| TurnAsk::Asked { crown, ask, said })
            }
            other => other,
        };
        if a.blocks() && !a.submit {
            a.released = true;
        }
        said
    }

    /// The tick's look at what the board already holds (no fork, no file,
    /// no clock of its own), for an ask still `sent`:
    ///
    /// - a mod `submit` whose seat is no longer the ticket's pane, or whose
    ///   pane was replaced, will never be entered by the mod it went to;
    /// - any other ask whose owed entry ended without its ack (the presses
    ///   ran out, the composer never came, the paste's window closed, the
    ///   pane went) while its agent is not in a turn: no turn end is coming
    ///   to settle it, and the words may stand in the box. A turn still
    ///   running settles it at its end (`crown_ask_turn_ended`).
    ///
    /// Neither says whether the agent read the words first: `unknown`.
    pub(super) fn settle_crown_asks(&mut self) -> bool {
        let gone: Vec<(ulid::Ulid, ulid::Ulid, &'static str)> = self
            .crown_asks
            .iter()
            .filter_map(|(t, a)| {
                let live = self.board.live_agent(*t).filter(|s| s.state.has_pane());
                let owed =
                    a.session.and_then(|s| self.owed.get(&s)).is_some_and(|o| o.ask == Some(a.id));
                let parked = self.pending_resumes.iter().any(|r| Some(r.session) == a.session);
                settle_why(a, live, owed, parked).map(|why| (*t, a.id, why))
            })
            .collect();
        let mut changed = false;
        for (target, ask, why) in gone {
            changed |= self.crown_ask_unknown(target, Some(ask), why);
        }
        changed
    }

    /// A restored ledger's crown asks (T-938): the turn each rode is back,
    /// what became of it is not.
    pub(super) fn restore_crown_asks(&mut self) {
        let now = now_ms();
        let worn = self.board.crown;
        let asks: Vec<(ulid::Ulid, ulid::Ulid, ulid::Ulid)> = self
            .turn_asks
            .iter()
            .filter_map(|(w, a)| match a {
                TurnAsk::Asked { crown, ask, .. } if Some(*crown) == worn => {
                    Some((*w, *crown, *ask))
                }
                _ => None,
            })
            .collect();
        for (worker, crown, ask) in asks {
            let mut a = CrownAsk::queued(crown, now);
            a.id = ask;
            a.set(Stage::Unknown { why: RESTORED_WHY.into() }, now);
            self.crown_asks.insert(worker, a);
        }
    }

    /// `get_ticket`'s `asked` for `target`: the record, whichever crown
    /// wears the crown now.
    pub(super) fn crown_asked_view(
        &self,
        target: ulid::Ulid,
    ) -> Option<mesimon_core::command::AgentAskedView> {
        self.crown_asks.get(&target).map(|a| a.view(now_ms()))
    }

    /// The second ask's refusal, while the first is out.
    pub(super) fn crown_ask_blocks(&self, target: ulid::Ulid, key: &str) -> Option<String> {
        self.crown_asks.get(&target).filter(|a| a.blocks()).map(|a| a.refusal(key, now_ms()))
    }
}

/// What an owed entry's ack proves for the crown's ask it carries.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum AckEdge {
    /// A receipt that names these words: a Codex steer or turn counted by
    /// its tag.
    Hand,
    /// A prompt hook, which names no words (a person's prompt fires one
    /// too): the turn now running ran after the ask went in, and the ask
    /// stays `sent`.
    Mark,
    /// A mod `submit`, held to the running turn's end: only its own
    /// `entered` speaks for it.
    Nothing,
}

pub(super) fn ack_edge(a: &CrownAsk, codex: bool) -> AckEdge {
    if a.stage != Stage::Sent || a.submit {
        return AckEdge::Nothing;
    }
    if codex {
        AckEdge::Hand
    } else {
        AckEdge::Mark
    }
}

/// Whether a mod report from `session`, about `frame`, with the session's
/// pane now `pane`, is `a`'s. An ask whose pane is not bound yet (sent to a
/// parked seat) takes the pane its wake made.
pub(super) fn reports_to(
    a: &CrownAsk,
    session: uuid::Uuid,
    frame: &str,
    pane: Option<&str>,
) -> bool {
    a.session == Some(session)
        && a.frame.as_deref() == Some(frame)
        && a.pane.as_deref().is_none_or(|p| Some(p) == pane)
}

/// Whether `pane` is the one `a` went to, binding it where the ask was sent
/// to a parked seat and this is the first edge that names its pane.
pub(super) fn bind_pane(a: &mut CrownAsk, pane: Option<&str>) -> bool {
    match (&a.pane, pane) {
        (None, Some(p)) => {
            a.pane = Some(p.to_string());
            true
        }
        (mine, now) => mine.as_deref() == now,
    }
}

/// The ask's word at the end of the turn that carried it, by its stage
/// alone: `taken` only for words real evidence handed and that started
/// this turn, `handed` for words handed into a turn already running,
/// `sent` for words no receipt named (the prompt hook's mark is an order,
/// not a match), `unknown` as it was; a dropped, queued or already taken
/// ask gives the turn nothing to say of it.
pub(super) fn end_said(stage: &Stage, own_turn: bool) -> Option<Said> {
    match stage {
        Stage::Handed if own_turn => Some(Said::Taken),
        Stage::Handed => Some(Said::Handed),
        Stage::Sent => Some(Said::Sent),
        Stage::Unknown { .. } => Some(Said::Unknown),
        Stage::Queued | Stage::Taken | Stage::Dropped { .. } => None,
    }
}

/// Why the tick moves an ask still out to `unknown`, from what the board
/// holds: `live` the ticket's agent with a pane, `owed` whether its owed
/// entry still carries this ask, `parked` whether its wake waits on a
/// worktree.
pub(super) fn settle_why(
    a: &CrownAsk,
    live: Option<&SessionRecord>,
    owed: bool,
    parked: bool,
) -> Option<&'static str> {
    if !a.blocks() {
        return None;
    }
    let moved =
        live.is_none_or(|s| Some(s.id) != a.session || a.pane.is_some() && s.pane_key != a.pane);
    // Handed into a pane since replaced: no end in the pane it went to is
    // coming.
    if a.stage == Stage::Handed {
        return (moved && a.pane.is_some())
            .then_some("the pane it went to was replaced before the turn it went into ended");
    }
    if a.stage != Stage::Sent {
        return None;
    }
    if a.submit {
        return moved
            .then_some("its session or pane changed before the mod reported the words entered");
    }
    let working = live.is_some_and(mesimon_core::quiet::is_working);
    (!owed && !parked && !working)
        .then_some("delivery is no longer tracked; the agent's receipt was not confirmed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(stage: Stage) -> CrownAsk {
        let mut a = CrownAsk::queued(ulid::Ulid::new(), 1_000);
        a.stage = stage;
        a
    }

    fn seat(state: SessionState) -> SessionRecord {
        let mut rec = SessionRecord::new(
            uuid::Uuid::from_u128(7),
            SessionKind::Claude,
            ulid::Ulid::from_parts(1, 7),
            vec!["claude".into()],
            "/tmp".into(),
            state,
        );
        rec.pane_key = Some("100:%1".into());
        rec
    }

    fn out(submit: bool) -> CrownAsk {
        let mut a = ask(Stage::Sent);
        a.session = Some(uuid::Uuid::from_u128(7));
        a.pane = Some("100:%1".into());
        a.frame = submit.then(|| "F1".into());
        a.submit = submit;
        a
    }

    /// T-938, correction 1: a paste-road ask whose owed entry ended
    /// unacked, at an agent in no turn and no parked wake, is no longer
    /// tracked; a working agent or a parked resume leaves it alone.
    #[test]
    fn the_tick_settles_an_untracked_delivery_and_nothing_still_in_flight() {
        let idle = seat(SessionState::Idle { stop_reason: StopReason::EndTurn });
        let why = settle_why(&out(false), Some(&idle), false, false);
        assert_eq!(
            why,
            Some("delivery is no longer tracked; the agent's receipt was not confirmed")
        );
        assert_eq!(settle_why(&out(false), Some(&idle), true, false), None, "still owed");
        assert_eq!(settle_why(&out(false), Some(&idle), false, true), None, "a parked resume");
        let working = seat(SessionState::Running);
        assert_eq!(settle_why(&out(false), Some(&working), false, false), None, "a turn runs");
        // A submit is the mod's: only its seat or pane changing settles it.
        assert_eq!(settle_why(&out(true), Some(&idle), false, false), None);
        let mut moved = idle.clone();
        moved.pane_key = Some("100:%2".into());
        assert!(settle_why(&out(true), Some(&moved), false, false).is_some());
        let mut released = out(false);
        released.released = true;
        assert_eq!(settle_why(&released, Some(&idle), false, false), None, "no longer a block");
    }

    /// T-938, corrections 2 and 3: a prompt hook names no words, so it hands
    /// no paste-road ask (it marks the turn after it); a held `submit` is
    /// spoken for by its own `entered` alone; a Codex steer's tag hands.
    #[test]
    fn only_a_receipt_that_names_the_words_hands_the_ask() {
        assert_eq!(ack_edge(&out(false), false), AckEdge::Mark);
        assert_eq!(ack_edge(&out(true), false), AckEdge::Nothing);
        assert_eq!(ack_edge(&out(false), true), AckEdge::Hand);
        assert_eq!(ack_edge(&ask(Stage::Handed), true), AckEdge::Nothing, "already handed");
    }

    /// T-938, correction 6: a mod report is the ask's only from its seat,
    /// about its frame, while the seat's pane is the one it went to; a
    /// delayed frame from a replaced pane marks nobody.
    #[test]
    fn a_delayed_report_from_a_replaced_pane_is_no_ones() {
        let a = out(true);
        let s = uuid::Uuid::from_u128(7);
        assert!(reports_to(&a, s, "F1", Some("100:%1")));
        assert!(!reports_to(&a, s, "F1", Some("100:%2")), "the pane was replaced");
        assert!(!reports_to(&a, s, "F0", Some("100:%1")), "another frame");
        assert!(!reports_to(&a, uuid::Uuid::from_u128(8), "F1", Some("100:%1")), "another seat");
    }

    /// T-938, round 2 B: the word a turn's end gives an ask is its stage's
    /// alone, matched case by case: `handed` only from real evidence, and a
    /// dropped ask gives the turn nothing to say of it.
    #[test]
    fn a_turn_end_says_only_what_the_ask_s_own_stage_proves() {
        assert_eq!(end_said(&Stage::Handed, true), Some(Said::Taken));
        assert_eq!(end_said(&Stage::Handed, false), Some(Said::Handed));
        assert_eq!(
            end_said(&Stage::Sent, true),
            Some(Said::Sent),
            "a mark is an order, not a match"
        );
        assert_eq!(end_said(&Stage::Unknown { why: "x".into() }, true), Some(Said::Unknown));
        let dropped = Stage::Dropped { by: "agent", why: "late".into() };
        assert_eq!(end_said(&dropped, false), None, "no attribution for a dropped ask");
        assert_eq!(end_said(&Stage::Queued, true), None);
        assert_eq!(end_said(&Stage::Taken, true), None, "taken once");
    }

    /// An ask sent to a parked seat binds the pane its wake made at the
    /// first edge that names one; a bound pane only ever matches itself.
    #[test]
    fn a_parked_seat_s_ask_binds_the_pane_its_wake_made() {
        let mut a = out(true);
        a.pane = None;
        let s = uuid::Uuid::from_u128(7);
        assert!(reports_to(&a, s, "F1", Some("200:%4")), "unbound: the wake's pane");
        assert!(bind_pane(&mut a, Some("200:%4")));
        assert_eq!(a.pane.as_deref(), Some("200:%4"));
        assert!(!bind_pane(&mut a, Some("200:%5")), "bound: another pane is not it");
        assert!(!reports_to(&a, s, "F1", Some("200:%5")));
    }

    /// Handed into a pane since replaced: the tick resolves it unknown, so
    /// it never blocks for good; handed in its own pane, it waits.
    #[test]
    fn a_handed_ask_whose_pane_was_replaced_does_not_block_for_good() {
        let mut a = out(true);
        a.stage = Stage::Handed;
        let idle = seat(SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(settle_why(&a, Some(&idle), false, false), None, "its own pane");
        let mut moved = idle.clone();
        moved.pane_key = Some("100:%2".into());
        assert!(settle_why(&a, Some(&moved), false, false).is_some_and(|w| w.contains("replaced")));
    }

    #[test]
    fn each_stage_has_one_word_and_only_the_out_ones_block() {
        let words: Vec<_> = [
            Stage::Queued,
            Stage::Sent,
            Stage::Handed,
            Stage::Taken,
            Stage::Dropped { by: "agent", why: "draft".into() },
            Stage::Unknown { why: "x".into() },
        ]
        .into_iter()
        .map(|s| (s.word(), ask(s).blocks()))
        .collect();
        assert_eq!(
            words,
            [
                ("queued", false),
                ("sent", true),
                ("handed", true),
                ("taken", false),
                ("dropped", false),
                ("unknown", false)
            ]
        );
        let mut released = ask(Stage::Handed);
        released.released = true;
        assert!(!released.blocks(), "the turn it went into ended");
    }

    #[test]
    fn the_view_says_who_and_why_only_where_it_has_them() {
        let a = ask(Stage::Dropped { by: "agent", why: "draft".into() });
        let v = a.view(5_000);
        assert_eq!(
            (v.status.as_str(), v.by.as_str(), v.why.as_str()),
            ("dropped", "agent", "draft")
        );
        assert_eq!(v.since_secs, Some(4));
        assert_eq!(v.ask, a.id.to_string());
        let v = ask(Stage::Sent).view(1_000);
        assert!(v.by.is_empty() && v.why.is_empty());
        let json = serde_json::to_value(&v).unwrap();
        // `by` is always written, for a reader from before T-938; `why` only
        // where there is one.
        assert!(json["by"] == "" && json.get("why").is_none(), "{json}");
    }

    #[test]
    fn the_refusal_names_the_first_ask_its_word_and_why_it_waits() {
        let mut a = ask(Stage::Sent);
        a.submit = true;
        let r = a.refusal("T-9", 31_000);
        assert!(r.contains(&a.id.to_string()) && r.contains("(sent, 30s)"), "{r}");
        assert!(r.contains("when the running turn ends"), "{r}");
        assert!(!r.contains("never"), "no promise the rule cannot keep: {r}");
        assert!(!r.to_lowercase().contains("you"), "words about the board, not to its reader: {r}");
    }
}
