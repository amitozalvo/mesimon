//! What restrains a mover that is not a person (T-84).
//!
//! Three things can move a card: the human, `automove`, and — since T-84 — the
//! agent asking over MCP. M5 adds a fourth when column policies grow on-enter
//! actions. Authority does not separate them: every one of them is *allowed* to
//! move a ticket. What separates them is this gate.
//!
//! A human is never refused here. Everything else is subject to three rules,
//! and they exist because the alternative is a board that argues with itself
//! while the user watches:
//!
//! * **No undo.** An automatic mover may not perform the exact reverse of a
//!   move someone else just made. Without it, a human dragging a running
//!   ticket back to TODO watches `automove` snap it forward again, and an
//!   agent announcing REVIEW watches `automove` drag it back on its next
//!   `Running`.
//! * **Depth zero.** An automation may not fire inside another automation's
//!   move. Nothing recurses today — `automove` has exactly one call site — so
//!   this costs nothing now and is the reason M5's on-enter actions cannot
//!   turn into a cascade later.
//! * **A fuse.** Enough automatic moves of one ticket in one window and
//!   automation stops for that ticket until a human moves it, or until the
//!   machines have gone a whole window without trying. This is the general
//!   protection: it holds for whatever automation is added next, including
//!   ones written after everybody has forgotten this file. One thing is not
//!   a flap and is not counted: a session's card following its own turns
//!   (T-468) — see `MoveGate::record`.
//!
//! The state is in memory on purpose. It is a debounce, not a security
//! control, so losing it across a daemon restart is harmless — and it needs no
//! schema field, no migration, and no `#[serde(default)]` to get wrong.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use mesimon_core::Principal;

/// How long a move stays "just made" for the purposes of the no-undo rule.
/// Long enough to cover a turn boundary, short enough that a genuine later
/// reversal — the user replies to a REVIEW ticket ten minutes on — goes
/// through.
const PINGPONG_WINDOW: Duration = Duration::from_secs(60);
/// The fuse: this many automatic moves of one ticket …
const FLAP_LIMIT: usize = 6;
/// … inside this window trips it, and a blown fuse lapses once this long
/// passes without an automatic attempt to move the ticket.
const FLAP_WINDOW: Duration = Duration::from_secs(120);
/// `FLAP_WINDOW` as the notice and the refusal say it.
const FLAP_WINDOW_WORDS: &str = "two minutes";

/// Where a move lands the ticket in its new column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Position {
    /// The top of the column. Automatic moves use this: the move is fresh news
    /// (just started, just finished), so it outranks what was already there.
    Top,
    /// A human drag, which carries its own ordering.
    Before(Option<ulid::Ulid>),
}

/// Why the gate said no. Rendered to whoever asked — the agent sees it as a
/// tool error, and every refusal is written to the activity feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The exact reverse of somebody else's recent move.
    PingPong { by: String, from: String, to: String },
    /// The fuse is blown for this ticket.
    Fused,
    /// An automation tried to fire inside another automation's move.
    Cascade,
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Refusal::PingPong { by, from, to } => format!(
                "refused: this would undo a move {by} just made ({from} → {to}). \
                 A move by hand overrides."
            ),
            Refusal::Fused => format!(
                "refused: automatic moves are suspended for this ticket — it moved too \
                 many times too quickly. They resume when a person moves it to another \
                 column, or after {FLAP_WINDOW_WORDS} with no automatic attempt to move it."
            ),
            Refusal::Cascade => "refused: an automation cannot move a ticket from inside another \
                 automation's move"
                .to_string(),
        }
    }

    pub fn tag(&self) -> &'static str {
        match self {
            Refusal::PingPong { .. } => "ping_pong",
            Refusal::Fused => "fused",
            Refusal::Cascade => "cascade",
        }
    }
}

/// The board's advisory for the tickets whose fuse is blown, by key. It
/// names the act that clears the fuse — a move of THAT ticket to another
/// column, since moving any other card or reordering this one does nothing
/// (T-468: "moving one by hand" read as "move any ticket").
pub fn suspended_notice(keys: &[String]) -> String {
    match keys {
        [one] => format!(
            "automatic moves suspended for {one} — it moved too often, too fast. \
             Moving {one} to another column by hand resumes them, and so do \
             {FLAP_WINDOW_WORDS} with no automatic attempt to move it."
        ),
        _ => format!(
            "automatic moves suspended for {} — they moved too often, too fast. \
             Moving one of them to another column by hand resumes its moves, and so do \
             {FLAP_WINDOW_WORDS} with no automatic attempt to move it.",
            keys.join(", ")
        ),
    }
}

#[derive(Debug, Clone)]
struct LastMove {
    actor: String,
    from: String,
    to: String,
    at: Instant,
    /// The session whose own turn edge made this move, when one did.
    turn_of: Option<uuid::Uuid>,
}

#[derive(Debug, Default)]
pub struct MoveGate {
    last: HashMap<ulid::Ulid, LastMove>,
    /// Timestamps of counted automatic moves, pruned to `FLAP_WINDOW`.
    recent: HashMap<ulid::Ulid, Vec<Instant>>,
    /// Blown fuses, each with the last time it had a reason to hold: when it
    /// tripped, or the latest automatic attempt it refused since.
    fused: HashMap<ulid::Ulid, Instant>,
    /// How many automation-driven moves are on the stack right now.
    depth: u32,
}

impl MoveGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// The no-undo window, overridable for tests that cannot wait a minute.
    fn pingpong_window() -> Duration {
        match std::env::var("MESIMON_PINGPONG_MS").ok().and_then(|v| v.parse().ok()) {
            Some(ms) => Duration::from_millis(ms),
            None => PINGPONG_WINDOW,
        }
    }

    /// May `by` move `ticket` from `from` to `to` right now?
    ///
    /// A human is always allowed: the gate restrains machines, and a person
    /// who is being told "no" by their own kanban board has been failed by it.
    ///
    /// A blown fuse holds while something keeps trying: each attempt it
    /// refuses restarts its window, and one that finds the window passed
    /// quietly lets the fuse lapse and is judged like any other move.
    pub fn check(
        &mut self,
        ticket: ulid::Ulid,
        from: &str,
        to: &str,
        by: &Principal,
        now: Instant,
    ) -> Result<(), Refusal> {
        if by.is_human() {
            return Ok(());
        }
        if self.depth > 0 {
            return Err(Refusal::Cascade);
        }
        if let Some(held) = self.fused.get_mut(&ticket) {
            if now.saturating_duration_since(*held) < FLAP_WINDOW {
                *held = now;
                return Err(Refusal::Fused);
            }
            self.fused.remove(&ticket);
            self.recent.remove(&ticket);
        }
        if let Some(last) = self.last.get(&ticket) {
            let reversal = last.to == from && last.from == to;
            let someone_else = last.actor != by.actor();
            let fresh = now.saturating_duration_since(last.at) < Self::pingpong_window();
            if reversal && someone_else && fresh {
                return Err(Refusal::PingPong {
                    by: last.actor.clone(),
                    from: last.from.clone(),
                    to: last.to.clone(),
                });
            }
        }
        Ok(())
    }

    /// Record a move that actually happened.
    ///
    /// `turn_of` is the session whose own turn edge made the move — automove
    /// following a session from `Running` to `Idle` and back — and `None` for
    /// everything else. It exists for the fuse alone. A card that follows one
    /// session's turns moves as often as a person answers that agent: six
    /// quick questions and answers in two minutes blew the fuse on a single
    /// agent's REVIEW ⇄ IN PROGRESS cadence, and the ticket sat in REVIEW
    /// while its agent worked (T-468). So a turn-edge move does not count
    /// when the ticket's previous move was that same session's turn edge.
    /// What the fuse is for still counts: two sessions' turns dragging one
    /// card between them (each move follows the OTHER session's), and an
    /// agent calling `move_ticket` back and forth (no turn edge at all).
    pub fn record(
        &mut self,
        ticket: ulid::Ulid,
        from: &str,
        to: &str,
        by: &Principal,
        turn_of: Option<uuid::Uuid>,
        now: Instant,
    ) {
        let cadence =
            turn_of.is_some() && self.last.get(&ticket).is_some_and(|l| l.turn_of == turn_of);
        self.last.insert(
            ticket,
            LastMove {
                actor: by.actor().to_string(),
                from: from.into(),
                to: to.into(),
                at: now,
                turn_of,
            },
        );
        if by.is_human() {
            // A person took charge. Whatever the machines were arguing about
            // is settled, and the fuse is theirs to reset by acting.
            self.fused.remove(&ticket);
            self.recent.remove(&ticket);
            return;
        }
        if cadence {
            return;
        }
        let hits = self.recent.entry(ticket).or_default();
        hits.retain(|t| now.saturating_duration_since(*t) < FLAP_WINDOW);
        hits.push(now);
        if hits.len() >= FLAP_LIMIT {
            self.fused.insert(ticket, now);
        }
    }

    /// Let every fuse that has gone `FLAP_WINDOW` without an automatic
    /// attempt lapse, so the advisory does not outlive the flap by hours
    /// (T-468). Whether any did — the board then has news.
    pub fn expire(&mut self, now: Instant) -> bool {
        let lapsed: Vec<ulid::Ulid> = self
            .fused
            .iter()
            .filter(|(_, held)| now.saturating_duration_since(**held) >= FLAP_WINDOW)
            .map(|(t, _)| *t)
            .collect();
        for t in &lapsed {
            self.fused.remove(t);
            self.recent.remove(t);
        }
        !lapsed.is_empty()
    }

    /// A column was renamed (T-117): the memory of where a ticket came from
    /// and went follows it, or a ping-pong check after the rename would
    /// compare the old name and never fire.
    pub fn rename_column(&mut self, from: &str, to: &str) {
        for l in self.last.values_mut() {
            if l.from == from {
                l.from = to.to_string();
            }
            if l.to == from {
                l.to = to.to_string();
            }
        }
    }

    /// A person asked the ticket's agent to work (a prompt reached it): the
    /// person's own last move stops being one to protect. The no-undo rule
    /// exists so a hand drag is not snapped back by a machine — but the drag
    /// and the ask are the SAME hand, and the newer act is the intent
    /// (dogfood 2026-09-04, T-186: `<<` to TODO then Shift+Enter, and the
    /// card sat in TODO for a minute while the agent worked). Somebody
    /// else's move — an agent that announced REVIEW — keeps its protection:
    /// the person did not make it, so their ask cannot supersede it. The
    /// fuse is untouched; moving by hand is still what clears it.
    pub fn asked_by_hand(&mut self, ticket: ulid::Ulid) {
        if self.last.get(&ticket).is_some_and(|l| l.actor == Principal::Local.actor()) {
            self.last.remove(&ticket);
        }
    }

    /// Is automation suspended for this ticket? Drives the card's mark.
    pub fn is_fused(&self, ticket: ulid::Ulid) -> bool {
        self.fused.contains_key(&ticket)
    }

    pub fn fused_tickets(&self) -> impl Iterator<Item = &ulid::Ulid> {
        self.fused.keys()
    }

    /// Bracket the mutation of an automatic move, so an automation that fires
    /// from inside it (M5's on-enter actions) sees `depth > 0` and is refused.
    pub fn enter(&mut self) {
        self.depth += 1;
    }

    pub fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Forget a ticket entirely — it was deleted.
    pub fn forget(&mut self, ticket: ulid::Ulid) {
        self.last.remove(&ticket);
        self.recent.remove(&ticket);
        self.fused.remove(&ticket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> ulid::Ulid {
        ulid::Ulid::nil()
    }
    fn auto() -> Principal {
        Principal::Automation { rule: "automove".into() }
    }
    fn agent() -> Principal {
        Principal::Agent { session: uuid::Uuid::nil() }
    }

    #[test]
    fn a_human_is_never_refused() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        g.record(t(), "TODO", "IN PROGRESS", &auto(), None, now);
        g.enter();
        // Fused, mid-cascade, and reversing an automation — all of it.
        for _ in 0..FLAP_LIMIT {
            g.record(t(), "A", "B", &auto(), None, now);
        }
        assert!(g.is_fused(t()));
        assert!(g.check(t(), "IN PROGRESS", "TODO", &Principal::Local, now).is_ok());
    }

    /// The one the user actually hits: drag a running ticket back to TODO and
    /// watch automove snap it forward again.
    #[test]
    fn automation_may_not_undo_a_human() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        g.record(t(), "IN PROGRESS", "TODO", &Principal::Local, None, now);
        let err = g.check(t(), "TODO", "IN PROGRESS", &auto(), now).unwrap_err();
        assert!(matches!(err, Refusal::PingPong { .. }));
        assert!(err.message().contains("IN PROGRESS"));
    }

    /// The one T-84 introduces: the agent says REVIEW, automove yanks it back.
    #[test]
    fn automation_may_not_undo_an_agent() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        g.record(t(), "IN PROGRESS", "REVIEW", &agent(), None, now);
        assert!(g.check(t(), "REVIEW", "IN PROGRESS", &auto(), now).is_err());
    }

    /// … and symmetrically, an agent may not undo automove.
    #[test]
    fn an_agent_may_not_undo_automation() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        g.record(t(), "TODO", "IN PROGRESS", &auto(), None, now);
        assert!(g.check(t(), "IN PROGRESS", "TODO", &agent(), now).is_err());
    }

    /// Not every follow-on move is a reversal. A human parking a card in TODO
    /// and the agent then starting work is the normal flow, not a fight.
    #[test]
    fn a_different_destination_is_not_a_reversal() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        g.record(t(), "REVIEW", "TODO", &Principal::Local, None, now);
        assert!(g.check(t(), "TODO", "IN PROGRESS", &auto(), now).is_ok());
    }

    #[test]
    fn the_window_expires() {
        let mut g = MoveGate::new();
        let then = Instant::now();
        g.record(t(), "IN PROGRESS", "TODO", &Principal::Local, None, then);
        let later = then + PINGPONG_WINDOW + Duration::from_secs(1);
        assert!(g.check(t(), "TODO", "IN PROGRESS", &auto(), later).is_ok());
    }

    /// A mover does not block itself — only somebody else's move counts.
    #[test]
    fn a_mover_may_reverse_its_own_move() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        g.record(t(), "TODO", "IN PROGRESS", &auto(), None, now);
        assert!(g.check(t(), "IN PROGRESS", "TODO", &auto(), now).is_ok());
    }

    #[test]
    fn the_fuse_trips_and_a_human_clears_it() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        for i in 0..FLAP_LIMIT {
            assert!(!g.is_fused(t()), "fused after only {i} moves");
            g.record(t(), "A", "B", &auto(), None, now);
        }
        assert!(g.is_fused(t()));
        assert_eq!(g.check(t(), "A", "B", &auto(), now), Err(Refusal::Fused));
        assert!(g.check(t(), "A", "B", &Principal::Local, now).is_ok());

        g.record(t(), "A", "B", &Principal::Local, None, now);
        assert!(!g.is_fused(t()));
        assert!(g.check(t(), "B", "C", &auto(), now).is_ok());
    }

    #[test]
    fn the_fuse_forgets_moves_outside_its_window() {
        let mut g = MoveGate::new();
        let start = Instant::now();
        for i in 0..FLAP_LIMIT - 1 {
            g.record(t(), "A", "B", &auto(), None, start + Duration::from_secs(i as u64));
        }
        assert!(!g.is_fused(t()));
        // Long after: the old hits age out, so this is hit number one again.
        g.record(t(), "A", "B", &auto(), None, start + FLAP_WINDOW + Duration::from_secs(10));
        assert!(!g.is_fused(t()));
    }

    /// T-468, replayed from T-456's feed: one agent, a run of sub-minute
    /// turns, REVIEW's on-working rule and IN PROGRESS's on-done rule taking
    /// turns. That is the card following its agent, not a flap.
    #[test]
    fn one_sessions_turn_cadence_never_fuses() {
        let mut g = MoveGate::new();
        let s = Some(uuid::Uuid::from_u128(1));
        let start = Instant::now();
        let mut at = start;
        g.record(t(), "TODO", "IN PROGRESS", &auto(), s, at);
        for i in 0..4 * FLAP_LIMIT {
            at += Duration::from_secs(15);
            let (from, to) =
                if i % 2 == 0 { ("IN PROGRESS", "REVIEW") } else { ("REVIEW", "IN PROGRESS") };
            assert_eq!(g.check(t(), from, to, &auto(), at), Ok(()), "move {i} refused");
            g.record(t(), from, to, &auto(), s, at);
        }
        assert!(!g.is_fused(t()));
    }

    /// … while two sessions' turns dragging one card between them is the
    /// fight the fuse caught before one agent per ticket (2026-09-02) — a
    /// board written before that keeps both seats. Each move follows the
    /// OTHER session's.
    #[test]
    fn two_sessions_turns_on_one_card_still_fuse() {
        let mut g = MoveGate::new();
        let (a, b) = (Some(uuid::Uuid::from_u128(1)), Some(uuid::Uuid::from_u128(2)));
        let now = Instant::now();
        for i in 0..FLAP_LIMIT {
            assert!(!g.is_fused(t()), "fused after only {i} moves");
            let (from, to, by) = if i % 2 == 0 {
                ("IN PROGRESS", "REVIEW", a)
            } else {
                ("REVIEW", "IN PROGRESS", b)
            };
            g.record(t(), from, to, &auto(), by, now);
        }
        assert!(g.is_fused(t()));
    }

    /// … and so is an agent calling `move_ticket` back and forth: no turn
    /// edge made those moves (`mcp_e2e`'s fuse section, on the real wire).
    #[test]
    fn an_agent_moving_back_and_forth_still_fuses() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        for i in 0..FLAP_LIMIT {
            let (from, to) = if i % 2 == 0 { ("A", "B") } else { ("B", "A") };
            g.record(t(), from, to, &agent(), None, now);
        }
        assert!(g.is_fused(t()));
    }

    /// A blown fuse holds while something keeps trying, and lapses once a
    /// whole window goes by with nothing trying.
    #[test]
    fn a_blown_fuse_lapses_after_a_quiet_window() {
        let mut g = MoveGate::new();
        let tripped = Instant::now();
        for _ in 0..FLAP_LIMIT {
            g.record(t(), "A", "B", &auto(), None, tripped);
        }
        let tried = tripped + FLAP_WINDOW - Duration::from_secs(1);
        assert_eq!(g.check(t(), "B", "C", &auto(), tried), Err(Refusal::Fused));
        // The attempt restarted the window: past the trip's, inside its own.
        let after_trip = tripped + FLAP_WINDOW + Duration::from_secs(30);
        assert_eq!(g.check(t(), "B", "C", &auto(), after_trip), Err(Refusal::Fused));
        let quiet = after_trip + FLAP_WINDOW;
        assert_eq!(g.check(t(), "B", "C", &auto(), quiet), Ok(()));
        assert!(!g.is_fused(t()));
        // It lapsed clean: the flap's old hits are not waiting to re-trip it.
        g.record(t(), "B", "C", &auto(), None, quiet);
        assert!(!g.is_fused(t()));
    }

    /// The tick's road to the same lapse, so the advisory goes without
    /// waiting for an attempt that may never come.
    #[test]
    fn expire_drops_a_quiet_fuse_and_says_so() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        for _ in 0..FLAP_LIMIT {
            g.record(t(), "A", "B", &auto(), None, now);
        }
        assert!(!g.expire(now + FLAP_WINDOW / 2));
        assert!(g.is_fused(t()));
        assert!(g.expire(now + FLAP_WINDOW));
        assert!(!g.is_fused(t()));
        assert!(!g.expire(now + FLAP_WINDOW * 2), "nothing left to lapse");
    }

    /// T-468: "Moving one by hand clears it" read as "move any ticket", and
    /// the user moved five other cards by hand while the notice stood.
    #[test]
    fn the_notice_names_the_ticket_and_the_act_that_clears_it() {
        let one = suspended_notice(&["T-456".into()]);
        assert!(one.contains("Moving T-456 to another column by hand"), "{one}");
        assert!(!one.contains("Moving one by hand"), "{one}");
        let two = suspended_notice(&["T-1".into(), "T-2".into()]);
        assert!(two.contains("T-1, T-2") && two.contains("to another column by hand"), "{two}");
        // The words say the window the code keeps.
        assert_eq!(FLAP_WINDOW, Duration::from_secs(120));
        assert!(
            one.contains(FLAP_WINDOW_WORDS) && Refusal::Fused.message().contains("two minutes")
        );
    }

    #[test]
    fn depth_blocks_a_cascade_and_unwinds() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        assert!(g.check(t(), "A", "B", &auto(), now).is_ok());
        g.enter();
        assert_eq!(g.check(t(), "A", "B", &auto(), now), Err(Refusal::Cascade));
        // A human still gets through mid-cascade.
        assert!(g.check(t(), "A", "B", &Principal::Local, now).is_ok());
        g.leave();
        assert!(g.check(t(), "A", "B", &auto(), now).is_ok());
    }

    #[test]
    fn leaving_more_than_entering_does_not_wrap() {
        let mut g = MoveGate::new();
        g.leave();
        g.leave();
        assert!(g.check(t(), "A", "B", &auto(), Instant::now()).is_ok());
    }

    /// The bug: a human parks a running ticket in TODO, then asks the agent
    /// again. The `Running` edge that follows is the exact reverse of the
    /// park, inside the window — and it is what the human just asked for.
    #[test]
    fn a_prompt_by_hand_supersedes_the_hands_own_park() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        g.record(t(), "IN PROGRESS", "TODO", &Principal::Local, None, now);
        assert!(g.check(t(), "TODO", "IN PROGRESS", &auto(), now).is_err(), "the guard is armed");
        g.asked_by_hand(t());
        assert_eq!(g.check(t(), "TODO", "IN PROGRESS", &auto(), now), Ok(()));
    }

    /// ...but not somebody else's move: the agent said REVIEW, and the
    /// human's next prompt does not hand automove permission to undo that.
    #[test]
    fn a_prompt_by_hand_leaves_an_agents_move_protected() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        g.record(t(), "IN PROGRESS", "REVIEW", &agent(), None, now);
        g.asked_by_hand(t());
        assert!(matches!(
            g.check(t(), "REVIEW", "IN PROGRESS", &auto(), now),
            Err(Refusal::PingPong { .. })
        ));
        // And the fuse is not the ask's to clear.
        for _ in 0..FLAP_LIMIT {
            g.record(t(), "A", "B", &auto(), None, now);
        }
        g.asked_by_hand(t());
        assert!(g.is_fused(t()), "only a move by hand clears the fuse");
    }

    #[test]
    fn forget_clears_everything_for_a_ticket() {
        let mut g = MoveGate::new();
        let now = Instant::now();
        for _ in 0..FLAP_LIMIT {
            g.record(t(), "A", "B", &auto(), None, now);
        }
        assert!(g.is_fused(t()));
        g.forget(t());
        assert!(!g.is_fused(t()));
        assert!(g.check(t(), "B", "A", &auto(), now).is_ok());
    }

    #[test]
    fn every_refusal_says_something_actionable() {
        for r in [
            Refusal::PingPong { by: "local".into(), from: "A".into(), to: "B".into() },
            Refusal::Fused,
            Refusal::Cascade,
        ] {
            assert!(r.message().starts_with("refused: "));
            assert!(!r.tag().is_empty());
        }
    }
}
