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
//! - **handed** — a receipt of the agent's that names these words: the
//!   mod's `entered` for their frame, from the seat and live pane it went
//!   to, or the Codex steer or launch counted. A Claude prompt hook names no
//!   words, so a paste is never handed: its ack only marks the turn, and
//!   the ask stays `sent`;
//! - **taken** — the turn these words started has ended. It says nothing
//!   of absorption inside a turn already running (words that went into one
//!   stay `handed`), and an end, normal or not, says nothing of whether the
//!   work succeeded;
//! - **dropped** — a refusal or a cancellation confirmed before the agent
//!   had them, and by whom: a person's take-back or replacement, the
//!   board's seat gone, or the mod's own `refused`, `dropped` or `rejected`;
//! - **unknown** — an effect the board cannot read: keys that failed after
//!   the words were in the box (a paste whose Enter failed included), a mod
//!   frame never acknowledged or reported in a word the board does not
//!   read, a pane replaced or gone under the words, a restart. Never
//!   resent.
//!
//! A second ask with `deliver: idle` is refused while the first reads
//! `sent` or `handed` (`blocks`), so a resend never puts in a second copy
//! behind the board's back. It lifts only at an edge the board saw:
//! `taken`, `dropped`, `unknown`, or the end of the turn the words went
//! into, for words that did not start a turn of their own; a mod `submit`
//! always starts its own. Without a request key a lost receipt, or a retry
//! after a restart, can still run twice: the rule refuses a second ask while
//! the first is out, and claims no more.
//!
//! **A steer goes beside it (T-974).** `now` or `immediately` to a working
//! agent the crown may send to is a person's send-now made on the crown's
//! say-so, and a person's is not refused for words still out: it is
//! admitted, a record of its own beside the one still out, and replaces
//! nothing. So a ticket holds a list, oldest first: the latest is what
//! `asked` reads, the others ride it as `earlier`, each with its own word,
//! and every receipt finds its record by id. The turn running keeps the ask
//! that rode it (`keeps_turn`): a steer's mark never takes the turn from
//! the ask still out on it, so that turn's end speaks of the first.
//!
//! `crown.json` keeps the asks still out (`KeptAsk`, T-974): a restart
//! brings each back at its own word, fenced by the seat, pane and frame it
//! went to, and the restart's look owns its next end. An ask a turn rode
//! but no kept record names (a ledger an older build wrote) comes back
//! `unknown`, as before.

use super::*;
use crownwake::Said;
use serde::{Deserialize, Serialize};

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

/// One of the crown's asks to a ticket's agent.
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
            earlier: Vec::new(),
        }
    }

    /// The refusal a second ask meets while this one is out.
    pub(super) fn refusal(&self, key: &str, now: u64) -> String {
        let secs = now.saturating_sub(self.since_ms) / 1000;
        let (wait, lifts) = if self.submit {
            (
                "Claude Code runs a plugin's prompt when the running turn ends",
                "the turn it starts ends",
            )
        } else {
            ("the turn it went into has not ended", "that turn ends")
        };
        format!(
            "{key}'s agent has ask {} ({}, {secs}s): {wait}. A second ask is refused while the \
             first is out, until {lifts} or get_ticket's asked reads taken, dropped or unknown.",
            self.id,
            self.stage.word(),
        )
    }
}

/// What a restored ledger's ask comes back as (T-938): its turn was kept,
/// what became of it was not.
pub(super) const RESTORED_WHY: &str =
    "an earlier crown ask is recorded; its outcome was not retained";

/// An ask still out, as `crown.json` keeps it (T-974): what its receipts
/// and its turn's end are fenced by, and whether a receipt handed it. Only
/// `sent` and `handed` are kept; the rest settle nothing after a restart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct KeptAsk {
    pub(super) id: ulid::Ulid,
    pub(super) crown: ulid::Ulid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) frame: Option<String>,
    #[serde(default)]
    pub(super) submit: bool,
    #[serde(default)]
    pub(super) own_turn: bool,
    #[serde(default)]
    pub(super) released: bool,
    #[serde(default)]
    pub(super) handed: bool,
    #[serde(default)]
    pub(super) since_ms: u64,
}

impl KeptAsk {
    /// The record as the ledger keeps it, if it is out.
    pub(super) fn of(a: &CrownAsk) -> Option<KeptAsk> {
        let handed = match a.stage {
            Stage::Sent => false,
            Stage::Handed => true,
            _ => return None,
        };
        Some(KeptAsk {
            id: a.id,
            crown: a.crown,
            session: a.session,
            pane: a.pane.clone(),
            frame: a.frame.clone(),
            submit: a.submit,
            own_turn: a.own_turn,
            released: a.released,
            handed,
            since_ms: a.since_ms,
        })
    }

    /// The record back, at the word it was kept at.
    pub(super) fn record(self) -> CrownAsk {
        CrownAsk {
            id: self.id,
            crown: self.crown,
            session: self.session,
            pane: self.pane,
            frame: self.frame,
            submit: self.submit,
            own_turn: self.own_turn,
            released: self.released,
            stage: if self.handed { Stage::Handed } else { Stage::Sent },
            since_ms: self.since_ms,
        }
    }
}

impl Daemon {
    /// The crown's ask `ask` on `target`, while the board holds it.
    fn crown_ask_mut(&mut self, target: ulid::Ulid, ask: ulid::Ulid) -> Option<&mut CrownAsk> {
        self.crown_asks.get_mut(&target)?.iter_mut().find(|a| a.id == ask)
    }

    /// The ticket's latest ask.
    fn crown_ask_latest(&mut self, target: ulid::Ulid) -> Option<&mut CrownAsk> {
        self.crown_asks.get_mut(&target)?.last_mut()
    }

    /// Every ask the board holds, with its ticket.
    fn all_crown_asks(&self) -> impl Iterator<Item = (ulid::Ulid, &CrownAsk)> {
        self.crown_asks.iter().flat_map(|(t, list)| list.iter().map(move |a| (*t, a)))
    }

    /// A new ask parked on the board: the latest from here. One still out
    /// stays beside it (a steer's, T-974); one settled or replaced on the
    /// queue goes.
    pub(super) fn crown_ask_queued(&mut self, target: ulid::Ulid, crown: ulid::Ulid) {
        let board = &self.board;
        self.crown_asks.retain(|t, _| board.ticket(*t).is_some());
        let list = self.crown_asks.entry(target).or_default();
        list.retain(CrownAsk::blocks);
        list.push(CrownAsk::queued(crown, now_ms()));
    }

    /// A record `take_queued` took, put back as it was (T-958).
    pub(super) fn crown_ask_restore(&mut self, target: ulid::Ulid, ask: CrownAsk) {
        let list = self.crown_asks.entry(target).or_default();
        match list.iter_mut().find(|a| a.id == ask.id) {
            Some(a) => *a = ask,
            None => list.push(ask),
        }
    }

    /// The ticket's latest ask, as `take_queued` keeps it to put back.
    pub(super) fn crown_ask_latest_clone(&self, target: ulid::Ulid) -> Option<CrownAsk> {
        self.crown_asks.get(&target).and_then(|l| l.last()).cloned()
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
        // A parked seat's pane is the one its wake makes: bound when its
        // frame is dispatched there (`crown_ask_framed`), or for a paste at
        // the first edge of its own that names it (`bind_pane`), never the
        // pane it slept in.
        let pane = self
            .board
            .sessions
            .iter()
            .find(|s| s.id == session && s.state.has_pane())
            .and_then(|s| s.pane_key.clone());
        let Some(ask) = self.crown_ask_latest(target).filter(|a| a.stage == Stage::Queued) else {
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
        let Some(id) = self.crown_asks.get(&target).and_then(|l| {
            l.iter()
                .rev()
                .find(|a| a.stage == Stage::Sent && a.session == Some(session))
                .map(|a| a.id)
        }) else {
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
            .flatten()
            .find(|a| a.id == ask && a.stage == Stage::Sent && a.session == Some(session))
        else {
            return;
        };
        a.frame = None;
        a.submit = false;
        a.own_turn = true;
    }

    /// The agent's own edge for the words: its turn now carries the ask,
    /// unless another ask still out rides it (`keeps_turn`).
    fn crown_ask_handed(&mut self, target: ulid::Ulid, ask: ulid::Ulid) -> bool {
        let Some(a) = self.crown_ask_mut(target, ask).filter(|a| a.stage == Stage::Sent) else {
            return false;
        };
        a.set(Stage::Handed, now_ms());
        let crown = a.crown;
        self.mark_ask_turn(target, TurnAsk::Asked { crown, ask, said: Said::Handed });
        true
    }

    /// Mark the running turn with a crown ask, unless it already carries
    /// another of this ticket's asks still out (T-974): a steer went into
    /// the turn that ask rode, and that turn's end is the first's.
    fn mark_ask_turn(&mut self, target: ulid::Ulid, mark: TurnAsk) {
        let list = self.crown_asks.get(&target).map(Vec::as_slice).unwrap_or_default();
        if !keeps_turn(self.turn_asks.get(&target), &mark, list) {
            self.mark_turn(target, mark);
        }
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
                self.mark_ask_turn(target, TurnAsk::Asked { crown, ask, said: Said::Sent });
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
    /// whose seat, frame and pane it names, that pane live now. A report
    /// delayed from a pane since replaced or gone is no one's, and a report
    /// binds nothing: the pane is the frame's, bound at its dispatch.
    fn crown_ask_by_frame(
        &self,
        session: uuid::Uuid,
        frame: &str,
    ) -> Option<(ulid::Ulid, ulid::Ulid)> {
        let pane = self.pane_of(session);
        self.all_crown_asks()
            .find(|(_, a)| reports_to(a, session, frame, pane.as_deref()))
            .map(|(target, a)| (target, a.id))
    }

    /// A parked seat's words went down its mod as `frame`, in the pane its
    /// wake made: the ask is that frame's and that pane's from here, a turn
    /// of its own (round 3: bound at dispatch, never by a later report).
    pub(super) fn crown_ask_framed(&mut self, session: uuid::Uuid, ask: ulid::Ulid, frame: String) {
        let pane = self.pane_of(session);
        if let Some(a) = self.crown_asks.values_mut().flatten().find(|a| a.id == ask) {
            a.frame = Some(frame);
            a.submit = true;
            a.own_turn = true;
            a.pane = pane;
        }
    }

    /// Whether a mod report from `session` about `frame` is a crown ask's,
    /// by the same fence `crown_ask_by_frame` keeps, binding nothing.
    pub(super) fn crown_ask_owns(&self, session: uuid::Uuid, frame: &str) -> bool {
        let pane = self.pane_of(session);
        self.all_crown_asks().any(|(_, a)| reports_to(a, session, frame, pane.as_deref()))
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
        let a = match ask {
            Some(id) => self.crown_ask_mut(target, id),
            None => self.crown_ask_latest(target),
        };
        let Some(a) = a else { return false };
        if !matches!(a.stage, Stage::Queued | Stage::Sent) {
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
        let a = match ask {
            Some(id) => self.crown_ask_mut(target, id),
            None => self.crown_ask_latest(target),
        };
        let Some(a) = a else { return false };
        if !matches!(a.stage, Stage::Queued | Stage::Sent | Stage::Handed) {
            return false;
        }
        a.set(Stage::Unknown { why: why.to_string() }, now_ms());
        true
    }

    /// A turn of `worker`'s agent ended, proven (`turn_ended`), carrying
    /// `asked`. For the ask whose id it carries, the word at this end is the
    /// record's own (`end_said`): `taken` only where real evidence handed
    /// words that started this turn, nothing at all for one dropped. Any
    /// other ask out on the ticket (a steer beside it, T-974) went into the
    /// turn that just ended and no longer blocks; a mod `submit` waits for
    /// its own turn. The end of a turn in a pane an ask did not go to is not
    /// its: that ask is unknown from here, and the replacement's turn
    /// carries none of it.
    pub(super) fn crown_ask_turn_ended(
        &mut self,
        worker: ulid::Ulid,
        asked: Option<TurnAsk>,
    ) -> Option<TurnAsk> {
        let now = now_ms();
        let seat = self.board.live_agent(worker).map(|s| (s.id, s.pane_key.clone()));
        let Some(list) = self.crown_asks.get_mut(&worker) else { return asked };
        let mut said = asked;
        for a in list.iter_mut() {
            let mine = matches!(asked, Some(TurnAsk::Asked { ask, .. }) if ask == a.id);
            let same_seat = seat.as_ref().is_some_and(|(id, pane)| {
                a.session.is_none_or(|s| s == *id) && bind_pane(a, pane.as_deref())
            });
            if a.session.is_some() && !same_seat {
                if matches!(a.stage, Stage::Sent | Stage::Handed) {
                    a.set(
                        Stage::Unknown {
                            why: "the pane it went to was replaced before the turn it went into \
                                  ended"
                                .into(),
                        },
                        now,
                    );
                }
                if mine {
                    said = None;
                }
                continue;
            }
            if let (true, Some(TurnAsk::Asked { crown, ask, .. })) = (mine, asked) {
                let word = end_said(&a.stage, a.own_turn);
                if word == Some(Said::Taken) {
                    a.set(Stage::Taken, now);
                }
                said = word.map(|said| TurnAsk::Asked { crown, ask, said });
            } else if a.stage == Stage::Handed && a.own_turn {
                // Handed words that started a turn of their own, under
                // another ask's mark (round 1): that turn began before this
                // proven end, so it has ended. The wake is the mark's.
                a.set(Stage::Taken, now);
            }
            if a.blocks() && !a.submit {
                a.released = true;
            }
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
        // A worker the restart has yet to look at (T-602) is left to that
        // look: its kept ask's owed entry was memory, and the turn's end
        // that settles it is the look's (T-974).
        let gone: Vec<(ulid::Ulid, ulid::Ulid, &'static str)> = self
            .all_crown_asks()
            .filter(|(t, _)| !self.crown_recheck.contains_key(t))
            .filter_map(|(t, a)| {
                let live = self.board.live_agent(t).filter(|s| s.state.has_pane());
                let owed =
                    a.session.and_then(|s| self.owed.get(&s)).is_some_and(|o| o.ask == Some(a.id));
                let parked = self.pending_resumes.iter().any(|r| Some(r.session) == a.session);
                settle_why(a, live, owed, parked).map(|why| (t, a.id, why))
            })
            .collect();
        let mut changed = false;
        for (target, ask, why) in gone {
            changed |= self.crown_ask_unknown(target, Some(ask), why);
        }
        changed
    }

    /// A restored ledger's crown asks: each kept one at its own word
    /// (T-974); an ask a turn rode that no kept record names, from a ledger
    /// an older build wrote, comes back `unknown` (T-938). Kept for the crown
    /// worn now and tickets on the board alone (`restore_crown` filtered
    /// them).
    pub(super) fn restore_crown_asks(
        &mut self,
        kept: std::collections::BTreeMap<ulid::Ulid, Vec<KeptAsk>>,
    ) {
        let now = now_ms();
        let worn = self.board.crown;
        for (worker, list) in kept {
            let list: Vec<CrownAsk> =
                list.into_iter().filter(|k| Some(k.crown) == worn).map(KeptAsk::record).collect();
            if !list.is_empty() {
                self.crown_asks.insert(worker, list);
            }
        }
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
            if self.all_crown_asks().any(|(_, a)| a.id == ask) {
                continue;
            }
            let mut a = CrownAsk::queued(crown, now);
            a.id = ask;
            a.set(Stage::Unknown { why: RESTORED_WHY.into() }, now);
            self.crown_asks.entry(worker).or_default().push(a);
        }
    }

    /// The asks still out, as `crown.json` keeps them (T-974).
    pub(super) fn kept_crown_asks(&self) -> std::collections::BTreeMap<ulid::Ulid, Vec<KeptAsk>> {
        self.crown_asks
            .iter()
            .map(|(t, list)| (*t, list.iter().filter_map(KeptAsk::of).collect::<Vec<_>>()))
            .filter(|(_, list)| !list.is_empty())
            .collect()
    }

    /// `get_ticket`'s `asked` for `target`: the latest record, and the
    /// others the board holds as `earlier` (T-974), whichever crown wears
    /// the crown now.
    pub(super) fn crown_asked_view(
        &self,
        target: ulid::Ulid,
    ) -> Option<mesimon_core::command::AgentAskedView> {
        let now = now_ms();
        let (latest, earlier) = self.crown_asks.get(&target)?.split_last()?;
        let mut view = latest.view(now);
        view.earlier = earlier.iter().map(|a| a.view(now)).collect();
        Some(view)
    }

    /// The ticket's other asks still out, beside its latest, as a feed line
    /// names them (`<id> <word>`, T-974).
    pub(super) fn crown_asks_beside(&self, target: ulid::Ulid) -> Vec<String> {
        let Some((_, earlier)) = self.crown_asks.get(&target).and_then(|l| l.split_last()) else {
            return Vec::new();
        };
        earlier
            .iter()
            .filter(|a| a.blocks())
            .map(|a| format!("{} {}", a.id, a.stage.word()))
            .collect()
    }

    /// The ticket's latest ask's id, once it is sent (the feed's `ask`).
    pub(super) fn crown_ask_sent_id(&self, target: ulid::Ulid) -> Option<ulid::Ulid> {
        self.crown_asks
            .get(&target)
            .and_then(|l| l.last())
            .filter(|a| !matches!(a.stage, Stage::Queued))
            .map(|a| a.id)
    }

    /// Whether an ask is a steer (T-974): `now` or `immediately`, on a road
    /// the crown sends (`sends`), at a pane whose agent is in a turn the
    /// board observed running: `Running`, proven (no guess off the tail).
    /// Not `Failed`, `Unknown` (a restart's agent before a hook speaks), a
    /// dialog, a start, an idle with background tasks (words there start a
    /// turn, they steer none) or a parked seat (round 1). Only a steer
    /// passes an ask still out; every other ask keeps the one-ask rule.
    pub(super) fn steer_admits(&self, seat: &QueuedSeat, deliver: Deliver, sends: bool) -> bool {
        let QueuedSeat::Pane(id) = seat else { return false };
        sends
            && deliver.at_once()
            && self
                .board
                .sessions
                .iter()
                .any(|s| s.id == *id && s.state == SessionState::Running && !crownwake::guessed(s))
    }

    /// The second ask's refusal, while the first is out: the oldest ask
    /// that still blocks is the one it names.
    pub(super) fn crown_ask_blocks(&self, target: ulid::Ulid, key: &str) -> Option<String> {
        let list = self.crown_asks.get(&target)?;
        list.iter().find(|a| a.blocks()).map(|a| a.refusal(key, now_ms()))
    }
}

/// Whether the running turn's mark `current` stays over `mark` (T-974): it
/// names another of the ticket's asks still out (`list`), whose turn this
/// is; a steer that went into it does not take it over. An ask that starts
/// a turn of its own (a held `submit` entered, words to an idle agent)
/// always takes the mark (round 1): the turn running from here is its, even
/// where the board saw no end between it and the steer's (the engine's
/// `Stop` and the held submit's prompt 0.3 s apart, inside the settle). Any
/// other mark gives way, as `mark_turn` decides.
pub(super) fn keeps_turn(current: Option<&TurnAsk>, mark: &TurnAsk, list: &[CrownAsk]) -> bool {
    let (Some(TurnAsk::Asked { ask: rode, .. }), TurnAsk::Asked { ask, .. }) = (current, mark)
    else {
        return false;
    };
    let own_turn = list.iter().any(|a| a.id == *ask && a.own_turn);
    !own_turn && rode != ask && list.iter().any(|a| a.id == *rode && a.blocks())
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
        && pane.is_some()
        && a.pane.as_deref() == pane
}

/// Whether `pane` is the one `a` went to, binding it where a paste was sent
/// to a parked seat and this is the first edge of the seat's own (its ack,
/// its turn's end) that names its pane. A frame's pane is bound at its
/// dispatch (`crown_ask_framed`), never here.
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
        return moved
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

    /// Round 3: a frame report needs a live pane, the one the frame was
    /// dispatched to. An unbound ask is no report's (its pane is bound at
    /// the dispatch, never chosen by a report), a seat with no pane reports
    /// nothing, and a paste's own edge binds the pane its wake made.
    #[test]
    fn a_frame_report_needs_the_live_pane_its_frame_went_to() {
        let mut a = out(true);
        let s = uuid::Uuid::from_u128(7);
        assert!(!reports_to(&a, s, "F1", None), "no live pane: no one's");
        a.pane = None;
        assert!(!reports_to(&a, s, "F1", Some("200:%4")), "unbound: no wildcard");
        assert!(!reports_to(&a, s, "F1", None));
        assert!(bind_pane(&mut a, Some("200:%4")));
        assert_eq!(a.pane.as_deref(), Some("200:%4"));
        assert!(reports_to(&a, s, "F1", Some("200:%4")));
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
        // Round 3: an unbound handed ask whose seat is gone resolves too.
        a.pane = None;
        assert!(settle_why(&a, None, false, false).is_some_and(|w| w.contains("replaced")));
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

/// T-974: the ledger as the daemon holds it, on a daemon assembled as `run`
/// assembles one (a temporary state dir, a tmux socket nothing listens on)
/// with one working Claude seat on the worker's ticket.
#[cfg(test)]
mod ledger_tests {
    use super::*;

    fn ticket(n: u128, key: &str) -> Ticket {
        Ticket {
            id: ulid::Ulid(n),
            short_key: key.into(),
            title: key.into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            envelope: None,
            workspace: None,
            import_origin: None,
            raised: None,
            previous_column: None,
            picked: None,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        }
    }

    struct Bed {
        d: Daemon,
        crown: ulid::Ulid,
        worker: ulid::Ulid,
        seat: uuid::Uuid,
        _dir: std::path::PathBuf,
    }

    fn bed(name: &str) -> Bed {
        let dir = std::env::temp_dir().join(format!("msmn-crownask-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let paths = Paths {
            repo_root: repo.clone(),
            proj16: "0974097409740974".into(),
            rt_dir: dir.join("rt"),
            state_dir: dir.join("state"),
            board_dir: repo.join(".mesimon"),
        };
        paths.ensure_dirs().unwrap();
        let (tx, _rx) = channel();
        let backend = TmuxBackend::new(dir.join("tmux.sock"), &paths.state_dir, None).unwrap();
        let mut board = Board::with_default_columns();
        let (crown, worker) = (ulid::Ulid(1), ulid::Ulid(2));
        board.tickets.push(ticket(1, "T-1"));
        board.tickets.push(ticket(2, "T-2"));
        board.tickets.push(ticket(3, "T-3"));
        board.crown = Some(crown);
        let seat = uuid::Uuid::from_u128(974);
        let mut rec = SessionRecord::new(
            seat,
            SessionKind::Claude,
            worker,
            vec!["claude".into()],
            repo.display().to_string(),
            SessionState::Running,
        );
        rec.pane_key = Some("100:%1".into());
        board.sessions.push(rec);
        let feed = FeedWriter::open(&paths.activity_log()).unwrap();
        let journal = crate::journal::Journal::open(&paths.daemon_log());
        let d = Daemon::assemble(Seed {
            board,
            backend,
            journal,
            notices: Vec::new(),
            exe_stamp: None,
            own_exe: None,
            started_at: String::new(),
            worktrees_barred: false,
            queue_barred: false,
            started_barred: false,
            costs_barred: false,
            machines: HashMap::new(),
            queued: Vec::new(),
            train_barred: false,
            forge_barred: false,
            feed,
            started: Default::default(),
            costs: Default::default(),
            worktrees: Default::default(),
            crown_barred: false,
            modroad: bridge::ModRoad::start(&paths),
            git_fetch_every: Duration::from_secs(60),
            columns_barred: false,
            sessions_barred: false,
            terminals: Default::default(),
            tx,
            self_exe: dir.join("mesimon"),
            paths,
        });
        Bed { d, crown, worker, seat, _dir: dir }
    }

    impl Bed {
        /// A new ask, sent into the seat as `stage`, starting a turn of its
        /// own or not: its id.
        fn ask(&mut self, stage: Stage, own_turn: bool) -> ulid::Ulid {
            self.d.crown_ask_queued(self.worker, self.crown);
            let a = self.d.crown_ask_latest(self.worker).unwrap();
            a.session = Some(self.seat);
            a.pane = Some("100:%1".into());
            a.own_turn = own_turn;
            a.stage = stage;
            a.id
        }

        fn word_of(&self, ask: ulid::Ulid) -> &'static str {
            let list = &self.d.crown_asks[&self.worker];
            list.iter().find(|a| a.id == ask).unwrap().stage.word()
        }
    }

    /// A steer beside the ask out: both records held, the idle rule still
    /// refuses naming the first, the steer's ack leaves the turn the
    /// first's, and that turn's end takes the first and releases the steer.
    #[test]
    fn a_steer_rides_beside_the_ask_out_and_the_turn_s_end_is_the_first_s() {
        let mut b = bed("steer");
        let first = b.ask(Stage::Handed, true);
        let crown = b.crown;
        b.d.mark_turn(b.worker, TurnAsk::Asked { crown, ask: first, said: Said::Handed });
        let steer = b.ask(Stage::Sent, false);
        assert_eq!(b.d.crown_asks[&b.worker].len(), 2, "the first stays beside the steer");
        let refusal = b.d.crown_ask_blocks(b.worker, "T-2").unwrap();
        assert!(refusal.contains(&first.to_string()), "the oldest out is named: {refusal}");
        // The steer's prompt hook: a mark, which leaves the turn the first's.
        b.d.crown_ask_acked(b.worker, b.seat, steer);
        assert_eq!(
            b.d.turn_asks.get(&b.worker),
            Some(&TurnAsk::Asked { crown, ask: first, said: Said::Handed })
        );
        let v = b.d.crown_asked_view(b.worker).unwrap();
        assert_eq!((v.ask.as_str(), v.status.as_str()), (steer.to_string().as_str(), "sent"));
        let earlier: Vec<_> = v.earlier.iter().map(|e| (e.ask.clone(), e.status.clone())).collect();
        assert_eq!(earlier, [(first.to_string(), "handed".to_string())]);
        assert_eq!(b.d.crown_asks_beside(b.worker), [format!("{first} handed")]);
        // The turn's end: the first's word is `taken`, and nothing blocks.
        let asked = b.d.turn_asks.remove(&b.worker);
        let said = b.d.crown_ask_turn_ended(b.worker, asked);
        assert_eq!(said, Some(TurnAsk::Asked { crown, ask: first, said: Said::Taken }));
        assert_eq!((b.word_of(first), b.word_of(steer)), ("taken", "sent"));
        assert_eq!(b.d.crown_ask_blocks(b.worker, "T-2"), None, "the steer went into that turn");
        // The next ask drops what no longer blocks.
        let next = b.ask(Stage::Queued, false);
        let ids: Vec<_> = b.d.crown_asks[&b.worker].iter().map(|a| a.id).collect();
        assert_eq!(ids, [next]);
    }

    /// Round 1, ST2's sequence on the real mod: a held submit out, a steer
    /// (a fill) into the running turn and its prompt hook, then the submit's
    /// `entered` before the board saw that turn end (the engine's Stop and
    /// the held prompt 0.3 s apart, inside the settle). The submit takes the
    /// turn's mark, the one end the board sees takes it and names it, the
    /// steer is released, and nothing blocks after.
    #[test]
    fn a_held_submit_entered_after_a_steer_takes_the_turn_it_starts() {
        let mut b = bed("held");
        let crown = b.crown;
        let held = b.ask(Stage::Sent, true);
        b.d.crown_asks.get_mut(&b.worker).unwrap()[0].submit = true;
        let steer = b.ask(Stage::Sent, false);
        b.d.crown_ask_acked(b.worker, b.seat, steer);
        assert_eq!(
            b.d.turn_asks.get(&b.worker),
            Some(&TurnAsk::Asked { crown, ask: steer, said: Said::Sent })
        );
        assert!(b.d.crown_ask_handed(b.worker, held));
        assert_eq!(
            b.d.turn_asks.get(&b.worker),
            Some(&TurnAsk::Asked { crown, ask: held, said: Said::Handed }),
            "the submit's own turn takes the mark"
        );
        let asked = b.d.turn_asks.remove(&b.worker);
        let said = b.d.crown_ask_turn_ended(b.worker, asked);
        assert_eq!(said, Some(TurnAsk::Asked { crown, ask: held, said: Said::Taken }));
        assert_eq!((b.word_of(held), b.word_of(steer)), ("taken", "sent"));
        assert_eq!(b.d.crown_ask_blocks(b.worker, "T-2"), None);
        // A handed submit under another own-turn ask's mark is taken at
        // the end too, never left blocking.
        let a1 = b.ask(Stage::Handed, true);
        b.d.crown_asks.get_mut(&b.worker).unwrap().last_mut().unwrap().submit = true;
        let a2 = b.ask(Stage::Handed, true);
        let mark = Some(TurnAsk::Asked { crown, ask: a2, said: Said::Handed });
        b.d.crown_ask_turn_ended(b.worker, mark);
        assert_eq!((b.word_of(a1), b.word_of(a2)), ("taken", "taken"));
        assert_eq!(b.d.crown_ask_blocks(b.worker, "T-2"), None);
    }

    /// The asks still out ride `crown.json` at their word and come back so
    /// after a restart; one a turn rode that no kept record names is
    /// `unknown`, as from a ledger an older build wrote.
    #[test]
    fn a_kept_ask_comes_back_at_its_word_and_an_unkept_one_unknown() {
        let mut b = bed("kept");
        let first = b.ask(Stage::Handed, true);
        let steer = b.ask(Stage::Sent, false);
        let kept = b.d.kept_crown_asks();
        assert_eq!(kept[&b.worker].len(), 2);
        let text = serde_json::to_string(&kept).unwrap();
        let kept: std::collections::BTreeMap<ulid::Ulid, Vec<KeptAsk>> =
            serde_json::from_str(&text).unwrap();
        // A second daemon on the same board, seat and pane: the restart.
        let mut again = bed("kept-again");
        let (w, c) = (again.worker, again.crown);
        let lost = ulid::Ulid::new();
        let other = ulid::Ulid(3);
        again.d.turn_asks.insert(w, TurnAsk::Asked { crown: c, ask: first, said: Said::Handed });
        again.d.turn_asks.insert(other, TurnAsk::Asked { crown: c, ask: lost, said: Said::Sent });
        again.d.restore_crown_asks(kept);
        assert_eq!((again.word_of(first), again.word_of(steer)), ("handed", "sent"));
        let v = again.d.crown_asked_view(other).unwrap();
        assert_eq!((v.ask.as_str(), v.status.as_str()), (lost.to_string().as_str(), "unknown"));
        assert_eq!(v.why, RESTORED_WHY);
        // The restart's look owns a kept ask's end: the tick settles nothing
        // for a worker it has yet to look at.
        again.d.board.sessions[0].state = SessionState::Idle { stop_reason: StopReason::EndTurn };
        again.d.crown_recheck.insert(w, Default::default());
        assert!(!again.d.settle_crown_asks());
        assert_eq!(again.word_of(steer), "sent");
        again.d.crown_recheck.remove(&w);
        assert!(again.d.settle_crown_asks(), "an untracked paste at an idle agent settles");
        assert_eq!(again.word_of(steer), "unknown");
        assert_eq!(again.word_of(first), "handed", "handed in its own pane waits for its end");
    }

    /// Round 1: the steer's exception asks for a turn the board observed
    /// running. With a submit still out, `now` and `immediately` pass it at
    /// a proven `Running` agent the crown sends to (one it started, or a
    /// person's whose ticket it watches, T-734), and nowhere else: not a
    /// guessed run, `Failed`, `Unknown`, `Spawning`, an idle (background
    /// tasks or not), a dialog, a parked seat, an unwatched person's agent,
    /// or `idle`.
    #[test]
    fn a_steer_needs_a_turn_observed_running() {
        let mut b = bed("admits");
        b.d.board.crown_mode = CrownMode::Autonomous;
        b.d.board.sessions[0].started_by = Some(b.crown);
        let mut a = CrownAsk::queued(b.crown, 1);
        a.stage = Stage::Sent;
        a.submit = true;
        a.session = Some(b.seat);
        b.d.crown_asks.insert(b.worker, vec![a]);
        let admits = |b: &Bed, deliver: Deliver| {
            let seat = b.d.seat_of(b.worker);
            let sends = b.d.crown_ask_hold(b.crown, b.worker, &seat).is_none();
            b.d.steer_admits(&seat, deliver, sends)
        };
        let set = |b: &mut Bed, state: SessionState, confidence: Confidence| {
            b.d.board.sessions[0].state = state;
            b.d.board.sessions[0].confidence = confidence;
        };
        set(&mut b, SessionState::Running, Confidence::High);
        assert!(admits(&b, Deliver::Now) && admits(&b, Deliver::Immediately));
        assert!(!admits(&b, Deliver::Idle), "idle keeps the rule");
        assert!(b.d.crown_ask_blocks(b.worker, "T-2").is_some(), "the submit is out");
        set(&mut b, SessionState::Running, Confidence::Low);
        assert!(!admits(&b, Deliver::Now), "a run guessed off the tail");
        for state in [
            SessionState::Failed { reason: mesimon_core::board::FailReason::Server },
            SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
            SessionState::Spawning,
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::Idle { stop_reason: StopReason::Background },
            SessionState::RequiresAction { reason: Reason::Permission },
            SessionState::Sleeping,
        ] {
            set(&mut b, state.clone(), Confidence::High);
            assert!(!admits(&b, Deliver::Now), "{state:?}");
            assert!(!admits(&b, Deliver::Immediately), "{state:?}");
        }
        // A person's agent: held for a person unless the crown watches its
        // ticket, which sends to it as to one it started (T-734, round 2).
        set(&mut b, SessionState::Running, Confidence::High);
        b.d.board.sessions[0].started_by = None;
        assert!(!admits(&b, Deliver::Now), "an unwatched person's agent");
        b.d.crown_watched.insert(b.worker);
        assert!(admits(&b, Deliver::Now), "a watched person's agent");
        assert!(admits(&b, Deliver::Immediately), "a watched person's agent");
    }

    /// A settled record is not kept; a queued one is the queue's.
    #[test]
    fn only_an_ask_out_is_kept() {
        let mut a = CrownAsk::queued(ulid::Ulid::new(), 7);
        assert_eq!(KeptAsk::of(&a), None);
        a.stage = Stage::Handed;
        a.frame = Some("F1".into());
        a.submit = true;
        let k = KeptAsk::of(&a).unwrap();
        let back = k.record();
        assert_eq!(
            (back.id, back.stage, back.frame, back.submit),
            (a.id, Stage::Handed, a.frame.clone(), true)
        );
        for stage in [Stage::Taken, Stage::Unknown { why: "x".into() }] {
            a.stage = stage;
            assert_eq!(KeptAsk::of(&a), None);
        }
    }

    /// The turn keeps the ask that rode it while that one is out; any other
    /// mark gives way as before.
    #[test]
    fn the_turn_keeps_the_ask_that_rode_it_only_while_it_is_out() {
        let c = ulid::Ulid::new();
        let mut first = CrownAsk::queued(c, 1);
        first.stage = Stage::Handed;
        let rode = TurnAsk::Asked { crown: c, ask: first.id, said: Said::Handed };
        let steer = TurnAsk::Asked { crown: c, ask: ulid::Ulid::new(), said: Said::Sent };
        let list = [first.clone()];
        assert!(keeps_turn(Some(&rode), &steer, &list));
        assert!(!keeps_turn(Some(&rode), &rode, &list), "its own mark");
        assert!(!keeps_turn(Some(&TurnAsk::Merge), &steer, &list));
        assert!(!keeps_turn(None, &steer, &list));
        assert!(!keeps_turn(Some(&rode), &steer, &[]), "the board no longer holds it");
        // A held submit entered after the steer: its own turn takes the
        // mark from the steer that rode the turn before (round 1, ST2).
        let mut held = CrownAsk::queued(c, 2);
        held.stage = Stage::Handed;
        held.submit = true;
        held.own_turn = true;
        let entered = TurnAsk::Asked { crown: c, ask: held.id, said: Said::Handed };
        assert!(!keeps_turn(Some(&rode), &entered, &[first.clone(), held]), "its own turn");
        first.released = true;
        assert!(!keeps_turn(Some(&rode), &steer, &[first]), "no longer out");
    }
}
