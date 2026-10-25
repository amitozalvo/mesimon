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
//! in, and a start that could not be made all end it. It rides `crown.json`,
//! so a restart neither loses a wake nor says it twice; one owed across the
//! restart says so (`after a restart`).

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
    /// When the turn began, wall clock, so a restart keeps the count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) began_ms: Option<u64>,
    /// The seat whose turn it is, from the moment it began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session: Option<uuid::Uuid>,
    /// The crown was woken for it: never again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) told: bool,
    /// Begun before a restart (`restore_crown`): a wake owed now says so.
    #[serde(skip)]
    pub(super) restored: bool,
}

impl Expect {
    pub(super) fn armed(ask: Option<ulid::Ulid>, expect_secs: u64) -> Self {
        Expect { ask, expect_secs, began_ms: None, session: None, told: false, restored: false }
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
    if e.told || !running {
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

/// The ledger's expectations back at start (`restore_crown`): the crown's
/// own ticket and a ticket gone from the board drop out; one told stays
/// told, so its wake is never said twice; one begun before the restart is
/// timed from then, and its wake, owed now, says `after a restart`.
pub(super) fn restore(
    expects: std::collections::BTreeMap<ulid::Ulid, Expect>,
    crown: ulid::Ulid,
    on_board: impl Fn(&ulid::Ulid) -> bool,
) -> std::collections::BTreeMap<ulid::Ulid, Expect> {
    expects
        .into_iter()
        .filter(|(w, _)| *w != crown && on_board(w))
        .map(|(w, mut e)| {
            e.restored = e.began_ms.is_some();
            (w, e)
        })
        .collect()
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
        if let Some(e) = self.crown_expects.get_mut(&ticket) {
            e.began_ms = Some(now);
            e.session = session;
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
        let o = overrun_of(self.crown_expects.get(&ticket)?, now_ms())?;
        Some(mesimon_core::command::AgentOverrunView {
            expected_secs: o.expected_secs,
            running_secs: o.running_secs,
        })
    }

    /// Whether an expectation still has a turn to time: its ticket on the
    /// board; begun, its seat still has a pane; not begun, its ask is still
    /// the crown's last on the ticket and not dropped. An ask the restart
    /// left unread (no record) keeps it: its words may still go in.
    fn expect_alive(&self, ticket: ulid::Ulid, e: &Expect) -> bool {
        if self.board.ticket(ticket).is_none() {
            return false;
        }
        if e.began_ms.is_some() {
            return e.session.is_none_or(|id| {
                self.board.sessions.iter().any(|s| s.id == id && s.state.has_pane())
            });
        }
        // The ticket holds a list of the crown's asks (T-974): its own ask
        // still among them and not dropped, or gone with the list's turnover.
        match (e.ask, self.crown_asks.get(&ticket)) {
            (Some(id), Some(list)) => list
                .iter()
                .find(|a| a.id == id)
                .is_some_and(|a| !matches!(a.stage, Stage::Dropped { .. })),
            _ => true,
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

    #[test]
    fn a_restart_keeps_what_was_told_and_dates_what_began() {
        let crown = ulid::Ulid::from_parts(1, 1);
        let (told, due, armed, gone) = (
            ulid::Ulid::from_parts(1, 2),
            ulid::Ulid::from_parts(1, 3),
            ulid::Ulid::from_parts(1, 4),
            ulid::Ulid::from_parts(1, 5),
        );
        let ledger = std::collections::BTreeMap::from([
            (told, Expect { told: true, ..begun(60, 1_000) }),
            (due, begun(60, 1_000)),
            (armed, Expect::armed(Some(ulid::Ulid::from_parts(1, 9)), 60)),
            (gone, begun(60, 1_000)),
            (crown, begun(60, 1_000)),
        ]);
        let back = restore(ledger, crown, |w| *w != gone);
        assert_eq!(back.keys().copied().collect::<Vec<_>>(), [told, due, armed]);
        let now = 3_600_000;
        assert_eq!(overrun_due(&back[&told], now, true), None, "told before: never again");
        assert!(overrun_of(&back[&told], now).is_some(), "get_ticket still reads it");
        assert!(back[&due].restored, "its wake says after a restart");
        assert_eq!(overrun_due(&back[&due], now, false), None, "not until it reads Running");
        assert!(overrun_due(&back[&due], now, true).is_some());
        assert!(!back[&armed].restored && back[&armed].began_ms.is_none(), "waits for its words");
    }

    #[test]
    fn an_expectation_round_trips_without_its_restart_flag() {
        let e = Expect { session: Some(uuid::Uuid::nil()), told: true, ..begun(60, 5) };
        let text = serde_json::to_string(&e).unwrap();
        let back: Expect = serde_json::from_str(&text).unwrap();
        assert_eq!(back, e);
        let r = Expect { restored: true, ..e.clone() };
        assert_eq!(serde_json::to_string(&r).unwrap(), text, "restored is never written");
        let bare: Expect = serde_json::from_str(r#"{"expect_secs": 9}"#).unwrap();
        assert_eq!(bare, Expect::armed(None, 9));
    }
}
