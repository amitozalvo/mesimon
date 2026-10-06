//! The merge train's memory (2026-09-04) — the movegate's sibling. Whether
//! it is ARMED and by which connection, what it asked and at which base
//! tip, the fuse, and the merges it was refused. In memory on purpose, the
//! movegate's argument: a debounce and a safety, never a fact to persist —
//! a restart forgets, the board re-arms on its next snapshot, and one ask
//! per ticket may repeat under the fuse's cap.
//!
//! One part is kept (T-635): the asks whose turn is still open, in
//! `train.json`. That record is the hold (`in_rebase_turn`), and a `U`
//! handover in the middle of a rebase turn emptied it, so the new daemon's
//! first pass asked the next REVIEW ticket onto the same tip. The fuse, the
//! refusals and the settled asks stay in memory.
//!
//! Armed BY A CONNECTION: the train pastes into agents with no per-press
//! gesture, so somebody must be watching. The arming client's writer `Arc`
//! is held weakly; when its reader thread returns the `Arc` dies and
//! `is_armed` reads false on its own, and `Msg::ClientGone` says so out
//! loud. A closed board is a stopped train.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use mesimon_core::command::Notice;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// Rebase asks on one ticket that suspend the train for it.
pub const FUSE_LIMIT: usize = 6;
pub const FUSE_WINDOW: Duration = Duration::from_secs(2 * 3600);

struct Arm {
    by: Weak<Mutex<UnixStream>>,
    notice: bool,
}

/// One rebase ask, by the train or by hand (`m`): a hand ask also stops the
/// train re-asking at the same tip, and never counts toward the fuse.
pub struct AskRecord {
    pub base_oid: String,
    pub at_ms: u64,
    pub by_hand: bool,
    /// The turn the ask started has ended (T-435): the ticket's agent left
    /// its working state after the ask landed. Until then the ticket is in
    /// its rebase turn — the git step may have landed, the tests may still
    /// be running — and the train holds for it (`train_busy`); after, a
    /// later turn at the same tip is a bystander like any other.
    pub turn_over: bool,
}

#[derive(Default)]
pub struct Train {
    armed: Option<Arm>,
    asked: HashMap<ulid::Ulid, AskRecord>,
    /// Train asks only, pruned to the window.
    hits: HashMap<ulid::Ulid, Vec<Instant>>,
    fused: HashSet<ulid::Ulid>,
    /// A merge the checkout refused (a dirty main, a lock): remembered per
    /// `(branch tip, base tip)` so it is not retried every bucket. Value is
    /// the detail, for the card.
    refused: HashMap<ulid::Ulid, (String, String, String)>,
}

impl Train {
    pub fn arm(&mut self, by: &Arc<Mutex<UnixStream>>, notice: bool) {
        self.armed = Some(Arm { by: Arc::downgrade(by), notice });
    }

    pub fn disarm(&mut self) {
        self.armed = None;
    }

    pub fn is_armed(&self) -> bool {
        self.armed.as_ref().is_some_and(|a| a.by.strong_count() > 0)
    }

    /// Paste the merged notice after a train merge?
    pub fn notice(&self) -> bool {
        self.is_armed() && self.armed.as_ref().is_some_and(|a| a.notice)
    }

    pub fn owned_by(&self, s: &Arc<Mutex<UnixStream>>) -> bool {
        self.armed.as_ref().and_then(|a| a.by.upgrade()).is_some_and(|a| Arc::ptr_eq(&a, s))
    }

    /// Remember a delivered rebase ask. Returns whether the fuse blew NOW.
    pub fn record_ask(
        &mut self,
        ticket: ulid::Ulid,
        base_oid: String,
        at_ms: u64,
        by_hand: bool,
        now: Instant,
    ) -> bool {
        self.asked.insert(ticket, AskRecord { base_oid, at_ms, by_hand, turn_over: false });
        if by_hand {
            return false;
        }
        let hits = self.hits.entry(ticket).or_default();
        hits.push(now);
        hits.retain(|t| now.duration_since(*t) < FUSE_WINDOW);
        if hits.len() >= FUSE_LIMIT && self.fused.insert(ticket) {
            return true;
        }
        false
    }

    /// A person did something to the ticket — a hand `m`, a hand move: the
    /// fuse and the refusal memory clear, the way a hand move clears the
    /// movegate's.
    pub fn hand_touched(&mut self, ticket: ulid::Ulid) {
        self.fused.remove(&ticket);
        self.hits.remove(&ticket);
        self.refused.remove(&ticket);
    }

    pub fn forget(&mut self, ticket: ulid::Ulid) {
        self.hand_touched(ticket);
        self.asked.remove(&ticket);
    }

    pub fn is_fused(&self, ticket: ulid::Ulid) -> bool {
        self.fused.contains(&ticket)
    }

    pub fn fused_tickets(&self) -> impl Iterator<Item = &ulid::Ulid> {
        self.fused.iter()
    }

    pub fn asked(&self) -> &HashMap<ulid::Ulid, AskRecord> {
        &self.asked
    }

    /// The asks whose turn is still open, as `train.json` keeps them.
    pub fn open_asks(&self) -> BTreeMap<ulid::Ulid, OpenAsk> {
        self.asked
            .iter()
            .filter(|(_, r)| !r.turn_over)
            .map(|(t, r)| {
                (*t, OpenAsk { base_oid: r.base_oid.clone(), at_ms: r.at_ms, by_hand: r.by_hand })
            })
            .collect()
    }

    /// Read `train.json`'s asks back at start: each is in its turn again,
    /// and that turn's end settles it as it would have.
    pub fn restore(&mut self, asks: impl IntoIterator<Item = (ulid::Ulid, OpenAsk)>) {
        for (ticket, a) in asks {
            let record = AskRecord {
                base_oid: a.base_oid,
                at_ms: a.at_ms,
                by_hand: a.by_hand,
                turn_over: false,
            };
            self.asked.insert(ticket, record);
        }
    }

    /// The ticket's agent stopped working: whatever ask it was in is over.
    pub fn settle(&mut self, ticket: ulid::Ulid) {
        if let Some(r) = self.asked.get_mut(&ticket) {
            r.turn_over = true;
        }
    }

    /// Asked at `base_tip` and still in the turn that ask started.
    pub fn in_rebase_turn(&self, ticket: ulid::Ulid, base_tip: &str) -> bool {
        self.asked.get(&ticket).is_some_and(|r| !r.turn_over && r.base_oid == base_tip)
    }

    /// Ticket → the base tip it was last asked at, the planner's shape.
    pub fn asked_tips(&self) -> HashMap<ulid::Ulid, String> {
        self.asked.iter().map(|(t, r)| (*t, r.base_oid.clone())).collect()
    }

    pub fn refuse(&mut self, ticket: ulid::Ulid, tip: String, base_tip: String, detail: String) {
        self.refused.insert(ticket, (tip, base_tip, detail));
    }

    /// Forget every remembered refusal. The checkout that refused them has
    /// changed under them (T-289): the `(tip, base tip)` pair a refusal is
    /// keyed on does not move when the user STASHES, which is one of the two
    /// things the refusal itself asks for, so the git sample's own delta is
    /// what lets the train try again.
    pub fn forget_refusals(&mut self) {
        self.refused.clear();
    }

    /// The detail of a merge refused at exactly this `(tip, base tip)`.
    pub fn refusal(&self, ticket: ulid::Ulid, tip: &str, base_tip: &str) -> Option<&str> {
        self.refused
            .get(&ticket)
            .filter(|(t, b, _)| t == tip && b == base_tip)
            .map(|(_, _, d)| d.as_str())
    }
}

pub const TRAIN_SCHEMA: u32 = 1;

/// A rebase ask whose turn has not ended, as `train.json` keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAsk {
    pub base_oid: String,
    pub at_ms: u64,
    #[serde(default)]
    pub by_hand: bool,
}

/// The file. Every field defaults, so a file from an older build of this
/// schema reads as what it held.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TrainFile {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub asks: BTreeMap<ulid::Ulid, OpenAsk>,
}

impl TrainFile {
    pub fn of(train: &Train) -> Self {
        TrainFile { schema_version: TRAIN_SCHEMA, asks: train.open_asks() }
    }

    /// No asks, at this build's schema: what an absent file reads as.
    pub fn default_of_schema() -> Self {
        TrainFile { schema_version: TRAIN_SCHEMA, ..TrainFile::default() }
    }
}

/// Startup loader: the file, any notices, and whether writes are barred.
/// It follows the other state files' contract (`store::load_versioned`). A
/// barred train still holds for the run.
pub fn load_or_recover(paths: &Paths) -> (TrainFile, Vec<Notice>, bool) {
    let (file, notices, barred) = crate::store::load_versioned(
        &paths.train_file(),
        TRAIN_SCHEMA,
        "the merge train's open asks",
        "",
    );
    (file.unwrap_or_default(), notices, barred)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> std::result::Result<TrainFile, (Option<u32>, String)> {
        crate::store::parse_versioned(text, TRAIN_SCHEMA)
    }

    fn pair() -> Arc<Mutex<UnixStream>> {
        let (a, _b) = UnixStream::pair().expect("a socket pair");
        Arc::new(Mutex::new(a))
    }

    #[test]
    fn armed_by_a_connection_and_disarmed_when_it_goes() {
        let mut train = Train::default();
        assert!(!train.is_armed());
        let c1 = pair();
        train.arm(&c1, true);
        assert!(train.is_armed());
        assert!(train.notice());
        assert!(train.owned_by(&c1));
        let c2 = pair();
        assert!(!train.owned_by(&c2));
        // The latest arming connection owns it.
        train.arm(&c2, false);
        assert!(!train.owned_by(&c1));
        assert!(train.owned_by(&c2));
        assert!(!train.notice());
        drop(c2);
        assert!(!train.is_armed(), "the Arc died with the connection");
        assert!(!train.notice());
        train.arm(&c1, true);
        train.disarm();
        assert!(!train.is_armed());
    }

    #[test]
    fn the_fuse_blows_at_six_train_asks_in_two_hours_and_a_hand_clears_it() {
        let mut train = Train::default();
        let t = ulid::Ulid(1);
        let t0 = Instant::now();
        for i in 0..FUSE_LIMIT - 1 {
            assert!(!train.record_ask(
                t,
                format!("tip{i}"),
                0,
                false,
                t0 + Duration::from_secs(i as u64)
            ));
        }
        assert!(!train.is_fused(t));
        assert!(train.record_ask(t, "tip5".into(), 0, false, t0 + Duration::from_secs(5)));
        assert!(train.is_fused(t));
        assert!(
            !train.record_ask(t, "tip6".into(), 0, false, t0 + Duration::from_secs(6)),
            "already blown"
        );
        train.hand_touched(t);
        assert!(!train.is_fused(t));
        assert_eq!(train.asked_tips().get(&t).map(String::as_str), Some("tip6"), "the asks stay");
        train.forget(t);
        assert!(train.asked_tips().is_empty());
    }

    #[test]
    fn asks_spread_over_more_than_the_window_never_fuse() {
        let mut train = Train::default();
        let t = ulid::Ulid(1);
        let t0 = Instant::now();
        for i in 0..20u64 {
            let at = t0 + Duration::from_secs(i * 30 * 60);
            assert!(!train.record_ask(t, format!("tip{i}"), 0, false, at), "ask {i}");
        }
        assert!(!train.is_fused(t));
    }

    #[test]
    fn hand_asks_are_remembered_but_never_counted() {
        let mut train = Train::default();
        let t = ulid::Ulid(1);
        let now = Instant::now();
        for i in 0..20 {
            assert!(!train.record_ask(t, format!("tip{i}"), i, true, now));
        }
        assert!(!train.is_fused(t));
        assert!(train.asked().get(&t).is_some_and(|r| r.by_hand && r.at_ms == 19));
    }

    #[test]
    fn a_refusal_is_remembered_per_tip_pair() {
        let mut train = Train::default();
        let t = ulid::Ulid(1);
        train.refuse(t, "a".into(), "b".into(), "dirty".into());
        assert!(train.refusal(t, "a", "b").is_some());
        assert_eq!(train.refusal(t, "a", "b"), Some("dirty"));
        assert!(train.refusal(t, "a2", "b").is_none(), "the branch moved");
        assert!(train.refusal(t, "a", "b2").is_none(), "the base moved");
        train.hand_touched(t);
        assert!(train.refusal(t, "a", "b").is_none());
    }

    /// The checkout moved (T-289): every refusal it made goes with it, so a
    /// stash — which moves neither tip — lets the train try again.
    #[test]
    fn a_checkout_that_changes_forgets_every_refusal() {
        let mut train = Train::default();
        train.refuse(ulid::Ulid(1), "a".into(), "b".into(), "dirty".into());
        train.refuse(ulid::Ulid(2), "c".into(), "b".into(), "dirty".into());
        train.record_ask(ulid::Ulid(1), "b".into(), 0, false, Instant::now());
        train.forget_refusals();
        assert!(train.refusal(ulid::Ulid(1), "a", "b").is_none());
        assert!(train.refusal(ulid::Ulid(2), "c", "b").is_none());
        assert!(train.asked_tips().contains_key(&ulid::Ulid(1)), "the asks are not refusals");
    }

    /// The open asks round-trip through `train.json`'s bytes (T-635): a
    /// train ask and a hand ask come back in their turn, a settled one is
    /// not written, and a newer build's file is refused.
    #[test]
    fn the_open_asks_round_trip_and_a_settled_one_is_not_kept() {
        let mut train = Train::default();
        let now = Instant::now();
        let (a, b, c) = (ulid::Ulid(1), ulid::Ulid(2), ulid::Ulid(3));
        train.record_ask(a, "tip".into(), 10, false, now);
        train.record_ask(b, "tip".into(), 20, true, now);
        train.record_ask(c, "tip".into(), 30, false, now);
        train.settle(c);
        let text = serde_json::to_string_pretty(&TrainFile::of(&train)).unwrap();
        let back = parse(&text).unwrap();
        assert_eq!(back.asks.len(), 2, "{text}");
        let mut restored = Train::default();
        restored.restore(back.asks);
        assert!(restored.in_rebase_turn(a, "tip"));
        assert!(restored.in_rebase_turn(b, "tip"));
        assert!(!restored.in_rebase_turn(c, "tip"), "settled, not kept");
        assert!(restored.asked().get(&b).is_some_and(|r| r.by_hand && r.at_ms == 20));
        assert!(!restored.is_fused(a), "the fuse stays in memory");
        restored.settle(a);
        assert!(!restored.in_rebase_turn(a, "tip"), "its turn's end settles it");
        assert_eq!(TrainFile::of(&restored).asks.keys().collect::<Vec<_>>(), vec![&b]);
        // Nothing open writes no asks at all.
        restored.forget(b);
        let empty = serde_json::to_string(&TrainFile::of(&restored)).unwrap();
        assert_eq!(empty, format!("{{\"schema_version\":{TRAIN_SCHEMA}}}"));
        let newer = format!("{{\"schema_version\": {}}}", TRAIN_SCHEMA + 1);
        assert!(matches!(parse(&newer), Err((Some(_), _))));
        assert!(matches!(parse("{"), Err((None, _))));
    }
}
