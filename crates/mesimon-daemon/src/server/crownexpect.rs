//! How long the crown expects a turn to take (T-973): `start_agent`'s and
//! `ask_agent`'s `expect_secs`.
//!
//! An expectation is armed on a ticket by the crown's start or ask and
//! begins with the turn those words begin: an ask's at its own mark
//! (`mark_turn` with the ask's id: the paste's ack on the hook set, the mod's
//! `entered`), a start's at its seat's first edge into `Running`. When that
//! turn has run past it with the agent still `Running`, the crown is woken
//! once (`WakeCause::Overrun`), and its `get_ticket` on the ticket carries
//! `overrun` until the turn ends. Nothing else happens: no kill, no park, no
//! move. The wait may be the work or a loop, and the board cannot tell
//! which (T-599's rule): it says so, and the crown decides.
//!
//! One wake per expectation: the crown arms the next with its next ask. A
//! turn's end, the seat leaving, the ask replaced or dropped before it went
//! in, and a start that could not be made all end it. It rides `crown.json`
//! with the engine's own id of the turn it times (T-983: Claude's prompt id,
//! Codex's turn id), so a restart neither loses a wake nor says it twice:
//! a begun one comes back only where that id finds its turn still open, and
//! a wake owed after it says so (`after a restart`).

use super::crownask::Stage;
use super::crownwake::{Owed, WakeCause};
use super::*;
use serde::{Deserialize, Serialize};

/// A turn past its expected duration, as the board saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Overrun {
    pub(super) expected_secs: u64,
    pub(super) running_secs: u64,
}

/// One ticket's armed expectation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Expect {
    /// The crown's ask whose words begin the turn (`CrownAsk::id`); `None`
    /// for a start's launch words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ask: Option<ulid::Ulid>,
    pub(super) expect_secs: u64,
    /// When the turn began, wall clock: the count, once a restart has found
    /// the turn again by `turn`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) began_ms: Option<u64>,
    /// The engine's own id of the turn it times (T-983), from the begin or
    /// the first one heard after it (`expect_heard`, `expect_observed`): the
    /// only thing a restart finds it again by. With none, a restart drops it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) turn: Option<TurnKey>,
    /// The seat whose turn it is, from its words' dispatch or the moment
    /// the turn began, and the pane it was in (`<server pid>:<pane id>`):
    /// a relaunch is another pane, so another turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pane: Option<String>,
    /// The mod `submit` frame an ask's words went down in (round 1): its
    /// `entered` begins the turn, after a restart too, when the ask's own
    /// record is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) frame: Option<String>,
    /// That submit's seat's last prompt id at the dispatch, and the others
    /// heard from it since (T-983, at most two): the mod relays in the
    /// engine's order, so the submit's own prompt is the one new id before
    /// its `entered`, and a second means another prompt ran first. With no
    /// id heard from the seat since this daemon started, the baseline is
    /// unknown and every id after the dispatch counts (round 2): a submit
    /// dispatched into a turn whose id was never heard then hears that
    /// turn's id and its own, and does not begin, a missed overrun and
    /// never a wrong one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) base: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) heard: Vec<String>,
    /// The crown was woken for it: never again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) told: bool,
    /// Begun before a restart (`restore_crown`): a wake owed now says so.
    #[serde(skip)]
    pub(super) restored: bool,
    /// A Codex turn restored before the runtime has named its turn again:
    /// owed nothing until its first observation says the same id.
    #[serde(skip)]
    pub(super) unverified: bool,
}

/// The engine's id of a turn (T-983). Claude's `prompt_id` is on every hook
/// frame and its transcript's `promptId` on every `user` record of the turn,
/// a prompt typed mid-turn included (it rides the running turn's); Codex's
/// is the app-server's turn id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TurnKey {
    pub(super) by: Engine,
    pub(super) id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Engine {
    Prompt,
    Codex,
}

/// What a restart found of a begun expectation's turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Found {
    /// Its id names the turn still open: it comes back.
    Open,
    /// Codex: the runtime names its turn at its first observation.
    Unverified,
    /// Closed, another turn since, or not found: it goes.
    Gone,
}

impl Expect {
    pub(super) fn armed(ask: Option<ulid::Ulid>, expect_secs: u64) -> Self {
        Expect {
            ask,
            expect_secs,
            began_ms: None,
            turn: None,
            session: None,
            pane: None,
            frame: None,
            base: None,
            heard: Vec::new(),
            told: false,
            restored: false,
            unverified: false,
        }
    }
}

/// Whether a seat in `state` is still inside a turn: working, at a dialog,
/// held by a rate limit, or not yet read (a restart, a stale observation).
/// Any other state — idle (a park on tasks included), failed, parked,
/// exited — ends it.
pub(super) fn turn_open(state: &SessionState) -> bool {
    matches!(
        state,
        SessionState::Spawning
            | SessionState::Running
            | SessionState::RequiresAction { .. }
            | SessionState::Throttled
            | SessionState::Unknown { .. }
    )
}

/// Whether the edge `from` → `to` begins a turn: into `Running` from no turn
/// at all, or from the launch. A seat re-read after a restart
/// (`Unknown` → `Running`) began its turn before this daemon saw it, so
/// that edge dates nothing.
pub(super) fn turn_begins(from: &SessionState, to: &SessionState) -> bool {
    *to == SessionState::Running && (!turn_open(from) || *from == SessionState::Spawning)
}

/// The turn past its expectation at `now`, begun or not; `None` before it
/// is due.
pub(super) fn overrun_of(e: &Expect, now: u64) -> Option<Overrun> {
    let running_ms = now.saturating_sub(e.began_ms?);
    (running_ms >= e.expect_secs.saturating_mul(1000))
        .then_some(Overrun { expected_secs: e.expect_secs, running_secs: running_ms / 1000 })
}

/// The overrun owed a wake: due, never told, and the seat `Running` (a
/// dialog, a question or a plan waits on someone, and has its own wake).
pub(super) fn overrun_due(e: &Expect, now: u64, running: bool) -> Option<Overrun> {
    if e.told || !running || e.unverified {
        return None;
    }
    overrun_of(e, now)
}

/// `has run its turn 25 min, past the 20 min it expected`: the turn's
/// whole length, not the excess, which reads `0 min` at the moment it is
/// owed.
pub(super) fn overrun_clause(o: Overrun) -> String {
    format!(
        "has run its turn {}, past the {} it expected",
        span(o.running_secs),
        span(o.expected_secs)
    )
}

/// `45 s` under two minutes, `25 min` under two hours, then `3 h`: the
/// lingering clause's units, and seconds for a short expectation.
fn span(secs: u64) -> String {
    if secs < 120 {
        format!("{secs} s")
    } else if secs < 120 * 60 {
        format!("{} min", secs / 60)
    } else {
        format!("{} h", secs / 3600)
    }
}

/// The ledger's expectations back at start (`restore_crown`). The crown's
/// own ticket and a ticket gone from the board drop out. One begun before
/// the restart comes back only where `found` finds its turn again by its
/// engine id (T-983; never a stamp near the begin, T-957): open on
/// Claude's transcript, or a Codex turn to be named again by the runtime.
/// It keeps its count, told stays told, and a wake owed now says `after a
/// restart`. One not yet begun comes back only where its words went down a
/// mod `submit` whose `entered` can still come (`frame`); a queued ask, a
/// paste whose ack the restart lost and a start are memory, so theirs go.
pub(super) fn restore(
    expects: std::collections::BTreeMap<ulid::Ulid, Expect>,
    crown: ulid::Ulid,
    on_board: impl Fn(&ulid::Ulid) -> bool,
    found: impl Fn(&Expect) -> Found,
) -> std::collections::BTreeMap<ulid::Ulid, Expect> {
    expects
        .into_iter()
        .filter(|(w, _)| *w != crown && on_board(w))
        .filter_map(|(w, mut e)| {
            if e.began_ms.is_none() {
                return (e.ask.is_some() && e.frame.is_some()).then_some((w, e));
            }
            match e.turn.as_ref().map_or(Found::Gone, |_| found(&e)) {
                Found::Gone => return None,
                Found::Unverified => e.unverified = true,
                Found::Open => {}
            }
            e.restored = true;
            Some((w, e))
        })
        .collect()
}

/// Whether a frame from `frame` speaks for the seat in `seat` (round 2):
/// two pane keys that differ say it came from a pane the seat left (a
/// wake's previous tenant), and such a frame is evidence of nothing. A
/// frame or a record with no key is trusted, as `straggler_death` trusts
/// it.
pub(super) fn speaks_for_seat(frame: Option<&str>, seat: Option<&str>) -> bool {
    frame.zip(seat).is_none_or(|(f, s)| f == s)
}

/// What a frame's `prompt_id` does to an expectation on its seat
/// (`Daemon::expect_heard`), the frame already known to be the seat's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hear {
    /// Begun and bound to another prompt: another turn.
    Retire,
    /// Begun unbound: the first id after its begin is its turn's.
    Bind,
    /// Waiting on its submit: a prompt new since the dispatch.
    New,
}

fn hearing(e: &Expect, pid: &str) -> Option<Hear> {
    match (&e.began_ms, &e.turn) {
        (Some(_), Some(k)) => (k.by == Engine::Prompt && k.id != pid).then_some(Hear::Retire),
        (Some(_), None) => Some(Hear::Bind),
        (None, _) if e.frame.is_none() => None,
        (None, _) => is_new(e, pid).then_some(Hear::New),
    }
}

/// Whether `pid` is a prompt id the submit's seat has not spoken by its
/// dispatch nor since: `heard` keeps at most two, as the second already
/// says what the `entered` needs.
fn is_new(e: &Expect, pid: &str) -> bool {
    e.base.as_deref() != Some(pid) && !e.heard.iter().any(|h| h == pid) && e.heard.len() < 2
}

impl Daemon {
    /// The crown's start or ask with `expect_secs` (T-973): one expectation
    /// per ticket, the latest. An ask without one leaves the last alone, as
    /// it belongs to its own words.
    pub(super) fn arm_expect(&mut self, ticket: ulid::Ulid, ask: Option<ulid::Ulid>, secs: u64) {
        self.crown_expects.insert(ticket, Expect::armed(ask, secs));
    }

    /// The turn the armed words began: an ask's at its mark, by its id, a
    /// start's (`ask` `None`) at its seat's first turn. Once.
    pub(super) fn expect_began(
        &mut self,
        ticket: ulid::Ulid,
        ask: Option<ulid::Ulid>,
        session: Option<uuid::Uuid>,
        now: u64,
    ) {
        // Asked before it is written: a `&mut` marks the ledger (T-839).
        if !self.crown_expects.get(&ticket).is_some_and(|e| e.began_ms.is_none() && e.ask == ask) {
            return;
        }
        let session = session.or_else(|| self.board.live_agent(ticket).map(|s| s.id));
        let pane = session.and_then(|id| self.pane_of(id));
        let turn = session.and_then(|id| self.turn_key(id));
        if let Some(e) = self.crown_expects.get_mut(&ticket) {
            e.began_ms = Some(now);
            e.session = session;
            e.pane = pane;
            e.turn = turn;
        }
    }

    /// The engine's id of the turn `session` begins now (T-983): Claude's
    /// `prompt_id` on the very frame that begins it (the hook set's
    /// `UserPromptSubmit`), never one cached from an earlier frame (round 1:
    /// that may be an earlier turn's); Codex's turn id on the observation
    /// that begins it. Anything else begins unbound, and the turn's own id
    /// binds it when it comes (`expect_heard`, `expect_observed`).
    fn turn_key(&self, session: uuid::Uuid) -> Option<TurnKey> {
        let rec = self.board.sessions.iter().find(|s| s.id == session)?;
        if rec.kind == SessionKind::Codex {
            let id = rec.codex_turn_id.clone()?;
            return Some(TurnKey { by: Engine::Codex, id });
        }
        let (seat, id) = self.frame_prompt.as_ref()?;
        (*seat == session).then(|| TurnKey { by: Engine::Prompt, id: id.clone() })
    }

    /// The armed ask's words went out (`crown_ask_sent`, `crown_ask_framed`,
    /// `crown_ask_repasted`): into `session`'s `pane`, down the mod as
    /// `frame` or pasted (`None`). Kept so a restart before the receipt
    /// still knows the receipt that begins the turn (round 1).
    pub(super) fn expect_dispatched(
        &mut self,
        ticket: ulid::Ulid,
        ask: ulid::Ulid,
        session: uuid::Uuid,
        frame: Option<String>,
    ) {
        let armed = self
            .crown_expects
            .get(&ticket)
            .is_some_and(|e| e.began_ms.is_none() && e.ask == Some(ask));
        if !armed {
            return;
        }
        let pane = self.pane_of(session);
        let base = self.prompt_ids.get(&session).cloned();
        if let Some(e) = self.crown_expects.get_mut(&ticket) {
            e.session = Some(session);
            e.pane = pane;
            e.frame = frame;
            e.base = base;
            e.heard.clear();
        }
    }

    /// `session` spoke under `pid` (T-983): every frame that carries a
    /// `prompt_id`, in the order the engine sent it, from the seat's own
    /// pane (`speaks_for_seat`, checked by the caller).
    /// - A begun expectation bound to another prompt goes (round 1): words
    ///   typed mid-turn ride the running turn's id, so another id means
    ///   another turn, whether the board saw the first one end or not (a
    ///   restart that found it open, a `Stop` lost). The same id keeps it.
    /// - One begun unbound (an `entered` ahead of its prompt's relay, the
    ///   native road) takes the first id heard after its begin.
    /// - One waiting on its `submit` hears it for its `entered`.
    pub(super) fn expect_heard(&mut self, session: uuid::Uuid, pid: &str) {
        let Some((ticket, hear)) = self.crown_expects.iter().find_map(|(t, e)| {
            (e.session == Some(session)).then(|| hearing(e, pid)).flatten().map(|h| (*t, h))
        }) else {
            return;
        };
        // Asked before it is written: a `&mut` marks the ledger (T-839).
        if hear == Hear::Retire {
            self.crown_expects.remove(&ticket);
            return;
        }
        let Some(e) = self.crown_expects.get_mut(&ticket) else { return };
        match hear {
            Hear::Bind => e.turn = Some(TurnKey { by: Engine::Prompt, id: pid.to_string() }),
            Hear::New => e.heard.push(pid.to_string()),
            Hear::Retire => {}
        }
    }

    /// The mod's `entered` for `frame` from `session` (round 1): the turn the
    /// expectation's words begin, fenced by seat, frame and the pane they
    /// went to. Whatever owns the frame otherwise (an ask whose record a
    /// restart dropped is no one's), its expectation begins here.
    ///
    /// And by its own prompt (T-983): the mod relays in the engine's order,
    /// so the submit's `UserPromptSubmit` is the one prompt id new since the
    /// dispatch, and that id is the turn's. A second new id means another
    /// prompt ran ahead of this `entered` (its report came late, a restart
    /// between): the words' turn is not the one running, and the
    /// expectation goes. None heard yet begins it unbound (`expect_heard`).
    pub(super) fn expect_entered(&mut self, session: uuid::Uuid, frame: &str) {
        let pane = self.pane_of(session);
        let Some(ticket) = self
            .crown_expects
            .iter()
            .find(|(_, e)| {
                e.began_ms.is_none()
                    && e.session == Some(session)
                    && e.frame.as_deref() == Some(frame)
                    && pane.is_some()
                    && e.pane == pane
            })
            .map(|(t, _)| *t)
        else {
            return;
        };
        if self.crown_expects.get(&ticket).is_some_and(|e| e.heard.len() > 1) {
            self.crown_expects.remove(&ticket);
            return;
        }
        if let Some(e) = self.crown_expects.get_mut(&ticket) {
            e.began_ms = Some(now_ms());
            e.turn = e.heard.first().map(|id| TurnKey { by: Engine::Prompt, id: id.clone() });
        }
    }

    /// A Codex seat's observation (T-983): its turn id. Another id than the
    /// one a begun expectation is bound to ends it, verified or not (round
    /// 1). One with no id yet takes the first one named while the turn is
    /// open (the runtime's `turn/started` can come a frame after the edge),
    /// and one a restart brought back unverified is the same turn once the
    /// runtime names the same id in an open turn; no open turn ends it.
    pub(super) fn expect_observed(
        &mut self,
        session: uuid::Uuid,
        turn: Option<&str>,
        state: &SessionState,
    ) {
        let Some(ticket) = self
            .crown_expects
            .iter()
            .find(|(_, e)| e.began_ms.is_some() && e.session == Some(session))
            .map(|(t, _)| *t)
        else {
            return;
        };
        let open = turn_open(state) && !matches!(state, SessionState::Unknown { .. });
        let Some(e) = self.crown_expects.get(&ticket) else { return };
        let ours = e.turn.as_ref().map(|k| k.id.as_str());
        if ours.is_some() && turn.is_some() && turn != ours {
            // Another turn, verified or not (round 1): the runtime moved on
            // between two observations, its end and the next start unseen.
            self.crown_expects.remove(&ticket);
        } else if e.unverified {
            if turn.is_some() && open {
                if let Some(e) = self.crown_expects.get_mut(&ticket) {
                    e.unverified = false;
                }
            } else if !turn_open(state) {
                self.crown_expects.remove(&ticket);
            }
        } else if e.turn.is_none() && open {
            if let (Some(id), Some(e)) = (turn, self.crown_expects.get_mut(&ticket)) {
                e.turn = Some(TurnKey { by: Engine::Codex, id: id.to_string() });
            }
        }
    }

    /// A turn began on `session` (`turn_begins`): an expectation a restart
    /// brought back timed an older one, whose end this daemon did not see,
    /// so it goes. A turn begun in this daemon's life was ended by
    /// `expect_left`.
    pub(super) fn expect_new_turn(&mut self, ticket: ulid::Ulid, session: uuid::Uuid) {
        let stale = self
            .crown_expects
            .get(&ticket)
            .is_some_and(|e| e.restored && e.session.is_none_or(|s| s == session));
        if stale {
            self.crown_expects.remove(&ticket);
        }
    }

    /// A wake's parked words were parked at last on the pane the wake made
    /// (`crown_ask_retag`, after `park` made their owed entry, round 2):
    /// that pane is the one the expectation waits in from here.
    pub(super) fn expect_bound(&mut self, ticket: ulid::Ulid, ask: ulid::Ulid) {
        let Some(session) = self
            .crown_expects
            .get(&ticket)
            .filter(|e| e.began_ms.is_none() && e.ask == Some(ask) && e.pane.is_none())
            .and_then(|e| e.session)
        else {
            return;
        };
        let pane = self.pane_of(session);
        if let Some(e) = self.crown_expects.get_mut(&ticket).filter(|_| pane.is_some()) {
            e.pane = pane;
        }
    }

    /// `session` left its turn (`turn_open` false): a begun expectation on
    /// it ends, told or not.
    pub(super) fn expect_left(&mut self, ticket: ulid::Ulid, session: uuid::Uuid) {
        let ends = self
            .crown_expects
            .get(&ticket)
            .is_some_and(|e| e.began_ms.is_some() && e.session.is_none_or(|s| s == session));
        if ends {
            self.crown_expects.remove(&ticket);
        }
    }

    /// A start the crown armed could not be made (T-777): nothing to time.
    pub(super) fn expect_unstarted(&mut self, ticket: ulid::Ulid) {
        if self.crown_expects.contains_key(&ticket) {
            self.crown_expects.remove(&ticket);
        }
    }

    /// `get_ticket`'s `overrun` on the crown's read (T-973).
    pub(super) fn overrun_view(
        &self,
        ticket: ulid::Ulid,
    ) -> Option<mesimon_core::command::AgentOverrunView> {
        let e = self.crown_expects.get(&ticket).filter(|e| !e.unverified)?;
        let o = overrun_of(e, now_ms())?;
        Some(mesimon_core::command::AgentOverrunView {
            expected_secs: o.expected_secs,
            running_secs: o.running_secs,
        })
    }

    /// Whether an expectation still has a turn to time: its ticket on the
    /// board; its seat, once known, still in the pane it was given (a
    /// relaunch or a park is another turn); not begun, its ask still the
    /// crown's last on the ticket and not dropped, or, with no record (a
    /// restart's), its `submit` frame still out in that pane.
    fn expect_alive(&self, ticket: ulid::Ulid, e: &Expect) -> bool {
        if self.board.ticket(ticket).is_none() {
            return false;
        }
        if let Some(id) = e.session {
            let Some(seat) = self.board.sessions.iter().find(|s| s.id == id) else {
                return false;
            };
            if seat.state.has_pane() {
                // In a pane: the one it was given, once one was.
                if e.pane.is_some() && seat.pane_key != e.pane {
                    return false;
                }
            } else if e.began_ms.is_some() || e.pane.is_some() {
                // It lost the pane its words went to, or its turn ran in.
                return false;
            } else if !self
                .pending_resumes
                .iter()
                .any(|p| p.session == id && p.ask.is_some() && p.ask == e.ask)
            {
                // Words parked on a wake still on its way (a worktree being
                // rebuilt) hold it only while that resume carries this very
                // ask (round 3): replaced by a person's words, dropped, or a
                // replay that failed, it goes.
                return false;
            }
        }
        if e.began_ms.is_some() {
            return true;
        }
        // The ticket holds a list of the crown's asks (T-974): its own ask
        // still among them and not dropped, or gone with the list's turnover.
        match (e.ask, self.crown_asks.get(&ticket)) {
            (Some(id), Some(list)) => list
                .iter()
                .find(|a| a.id == id)
                .is_some_and(|a| !matches!(a.stage, Stage::Dropped { .. })),
            (Some(_), None) => e.frame.is_some() && e.session.is_some(),
            (None, _) => true,
        }
    }

    /// The tick's look (T-973): expectations with no turn left to time go,
    /// and each turn past its own, its seat still `Running`, wakes the
    /// crown once. Nothing is stopped, parked or moved.
    pub(super) fn hear_overruns(&mut self) -> bool {
        let gone: Vec<ulid::Ulid> = self
            .crown_expects
            .iter()
            .filter(|(t, e)| !self.expect_alive(**t, e))
            .map(|(t, _)| *t)
            .collect();
        for t in &gone {
            self.crown_expects.remove(t);
        }
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let now = now_ms();
        let running = |e: &Expect| {
            e.session
                .and_then(|id| self.board.sessions.iter().find(|s| s.id == id))
                .is_some_and(|s| s.state == SessionState::Running)
        };
        let due: Vec<(ulid::Ulid, Overrun, bool)> = self
            .crown_expects
            .iter()
            .filter(|(t, _)| **t != crown)
            .filter_map(|(t, e)| overrun_due(e, now, running(e)).map(|o| (*t, o, e.restored)))
            .collect();
        for (worker, overrun, late) in &due {
            if let Some(e) = self.crown_expects.get_mut(worker) {
                e.told = true;
            }
            let owed = Owed { overrun: Some(*overrun), late: *late, ..Owed::default() };
            self.owe_wake(crown, *worker, WakeCause::Overrun, None, None, owed);
        }
        !due.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn begun(secs: u64, at: u64) -> Expect {
        Expect { began_ms: Some(at), ..Expect::armed(None, secs) }
    }

    #[test]
    fn an_overrun_is_due_from_its_expectation_once_and_only_while_running() {
        let e = begun(60, 1_000);
        assert_eq!(overrun_of(&Expect::armed(None, 60), 999_999), None, "not begun");
        assert_eq!(overrun_of(&e, 60_999), None, "a millisecond short");
        let at = Overrun { expected_secs: 60, running_secs: 60 };
        assert_eq!(overrun_of(&e, 61_000), Some(at));
        assert_eq!(overrun_due(&e, 61_000, true), Some(at));
        assert_eq!(overrun_due(&e, 61_000, false), None, "a dialog or an idle is no overrun");
        let told = Expect { told: true, ..e.clone() };
        assert_eq!(overrun_due(&told, 600_000, true), None, "told once");
        assert_eq!(overrun_of(&told, 600_000).map(|o| o.running_secs), Some(599), "still read");
        assert_eq!(overrun_of(&begun(u64::MAX, 0), 1 << 40), None, "no overflow");
        assert_eq!(overrun_of(&begun(5, 10_000), 1_000), None, "a clock stepped back");
    }

    #[test]
    fn an_overrun_says_the_whole_turn_in_its_units() {
        let line = |r, x| overrun_clause(Overrun { expected_secs: x, running_secs: r });
        assert_eq!(line(6, 3), "has run its turn 6 s, past the 3 s it expected");
        assert_eq!(line(1500, 1200), "has run its turn 25 min, past the 20 min it expected");
        assert_eq!(line(119, 60), "has run its turn 119 s, past the 60 s it expected");
        assert_eq!(line(3 * 3600 + 59, 7200), "has run its turn 3 h, past the 2 h it expected");
    }

    #[test]
    fn a_turn_is_open_through_dialogs_and_ends_at_any_idle() {
        let idle = |stop_reason| SessionState::Idle { stop_reason };
        assert!(turn_open(&SessionState::Running));
        assert!(turn_open(&SessionState::RequiresAction { reason: Reason::Permission }));
        assert!(turn_open(&SessionState::unknown()));
        for s in [
            idle(StopReason::EndTurn),
            idle(StopReason::Background),
            SessionState::Sleeping,
            SessionState::Failed { reason: mesimon_core::board::FailReason::Unknown },
        ] {
            assert!(!turn_open(&s), "{s:?}");
        }
        assert!(turn_begins(&idle(StopReason::EndTurn), &SessionState::Running));
        assert!(turn_begins(&SessionState::Spawning, &SessionState::Running), "the launch");
        assert!(!turn_begins(&SessionState::unknown(), &SessionState::Running), "a restart");
        let dialog = SessionState::RequiresAction { reason: Reason::Permission };
        assert!(!turn_begins(&dialog, &SessionState::Running), "a dialog left");
    }

    /// T-983: a begun expectation comes back only where its engine id finds
    /// its turn open (Claude), or to be named again by the runtime (Codex),
    /// keeping its count, told staying told, and its wake saying `after a
    /// restart`. One with no id goes whatever the transcript says. One whose
    /// words went down a mod submit, not yet entered, comes back; a queued
    /// ask's and a start's do not.
    #[test]
    fn a_restart_keeps_a_begun_expectation_only_by_its_turn_s_id() {
        let crown = ulid::Ulid::from_parts(1, 1);
        let t = |n| ulid::Ulid::from_parts(1, n);
        let key = |by, id: &str| Some(TurnKey { by, id: id.into() });
        let bound = |by, id: &str| Expect { turn: key(by, id), ..begun(60, 1_000) };
        let waiting = Expect {
            frame: Some("01SUBMIT".into()),
            session: Some(uuid::Uuid::nil()),
            ..Expect::armed(Some(t(9)), 60)
        };
        let ledger = std::collections::BTreeMap::from([
            (t(2), Expect { told: true, ..bound(Engine::Prompt, "open") }),
            (t(3), bound(Engine::Prompt, "open")),
            (t(4), bound(Engine::Prompt, "closed")),
            (t(5), begun(60, 1_000)),
            (t(6), bound(Engine::Codex, "turn-1")),
            (t(7), Expect::armed(Some(t(9)), 60)),
            (t(8), waiting.clone()),
            (t(10), bound(Engine::Prompt, "open")),
            (crown, bound(Engine::Prompt, "open")),
        ]);
        let found = |e: &Expect| match e.turn.as_ref().map(|k| (k.by, k.id.as_str())) {
            Some((Engine::Prompt, "open")) => Found::Open,
            Some((Engine::Codex, _)) => Found::Unverified,
            None => Found::Open,
            _ => Found::Gone,
        };
        let back = restore(ledger, crown, |w| *w != t(10), found);
        assert_eq!(back.keys().copied().collect::<Vec<_>>(), [t(2), t(3), t(6), t(8)]);
        let now = 3_600_000;
        assert_eq!(overrun_due(&back[&t(2)], now, true), None, "told before: never again");
        assert!(back[&t(3)].restored, "its wake says after a restart");
        assert_eq!(back[&t(3)].began_ms, Some(1_000), "the count is kept");
        assert!(overrun_due(&back[&t(3)], now, true).is_some());
        assert!(back[&t(6)].unverified && overrun_due(&back[&t(6)], now, true).is_none());
        assert_eq!(back[&t(8)], waiting, "its entered can still come");
        let start = std::collections::BTreeMap::from([(t(3), Expect::armed(None, 60))]);
        assert!(
            restore(start, crown, |_| true, |_| Found::Open).is_empty(),
            "a start's launch is memory"
        );
    }

    /// Round 1: a frame under another prompt retires a begun expectation;
    /// the same prompt keeps it; an unbound one takes the first id; one
    /// waiting on its submit counts every new id, an unknown baseline's
    /// included (round 2). A frame from a pane the seat left speaks for
    /// nothing, whatever it would have done (round 2).
    #[test]
    fn a_frame_s_prompt_id_retires_binds_or_is_heard() {
        let bound = Expect {
            turn: Some(TurnKey { by: Engine::Prompt, id: "A".into() }),
            ..begun(60, 1_000)
        };
        assert_eq!(hearing(&bound, "B"), Some(Hear::Retire), "another turn");
        assert_eq!(hearing(&bound, "A"), None, "a steer rides the same id");
        let codex = Expect { turn: Some(TurnKey { by: Engine::Codex, id: "T".into() }), ..bound };
        assert_eq!(hearing(&codex, "B"), None, "a Codex turn is its runtime's");
        assert_eq!(hearing(&begun(60, 1_000), "A"), Some(Hear::Bind));
        let unknown = Expect { frame: Some("F".into()), ..Expect::armed(None, 60) };
        assert_eq!(hearing(&unknown, "R"), Some(Hear::New), "an unknown baseline counts R");
        let known = Expect { base: Some("R".into()), ..unknown.clone() };
        assert_eq!(hearing(&known, "R"), None);
        assert_eq!(hearing(&known, "A"), Some(Hear::New));
        assert_eq!(hearing(&Expect::armed(None, 60), "A"), None, "a queued ask hears nothing");
        assert!(speaks_for_seat(Some("9:%1"), Some("9:%1")));
        assert!(!speaks_for_seat(Some("9:%0"), Some("9:%1")), "a pane the seat left");
        assert!(speaks_for_seat(None, Some("9:%1")), "a frame with no key");
        assert!(speaks_for_seat(Some("9:%0"), None), "a record with no key");
    }

    /// The submit's own prompt is the one new id since its dispatch: the
    /// base and a repeat are not, and a third is never kept.
    #[test]
    fn a_submit_hears_new_prompt_ids_only() {
        let mut e = Expect { base: Some("R".into()), ..Expect::armed(None, 60) };
        assert!(!is_new(&e, "R"), "the turn running at the dispatch");
        assert!(is_new(&e, "A"));
        e.heard.push("A".into());
        assert!(!is_new(&e, "A"), "a repeat");
        assert!(is_new(&e, "B"));
        e.heard.push("B".into());
        assert!(!is_new(&e, "C"), "two already say another prompt ran");
    }

    #[test]
    fn an_expectation_round_trips() {
        let e = Expect {
            session: Some(uuid::Uuid::nil()),
            told: true,
            turn: Some(TurnKey { by: Engine::Codex, id: "019a".into() }),
            base: Some("R".into()),
            heard: vec!["A".into()],
            ..begun(60, 5)
        };
        let text = serde_json::to_string(&e).unwrap();
        assert!(text.contains(r#""turn":{"by":"codex","id":"019a"}"#), "{text}");
        let back: Expect = serde_json::from_str(&text).unwrap();
        assert_eq!(back, e);
        let r = Expect { restored: true, unverified: true, ..e.clone() };
        assert_eq!(serde_json::to_string(&r).unwrap(), text, "a restart's flags are never written");
        let bare: Expect = serde_json::from_str(r#"{"expect_secs": 9}"#).unwrap();
        assert_eq!(bare, Expect::armed(None, 9));
    }
}
