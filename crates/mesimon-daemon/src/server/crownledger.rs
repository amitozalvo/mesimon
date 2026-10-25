//! `crown.json` (T-602): the crown's ledger, kept through a restart.
//!
//! The crown's wakes (`crownwake`) are judged against what it was last
//! told of each worker, and a turn's end, a merge or a hold for the train
//! moves that baseline. All of it lived in memory, so a `U` handover forgot
//! it: a merge or a finished turn in the restart's window, a wake owed to a
//! working crown, or a delivery held for the train was never heard (T-600's
//! landing, 2026-10-03). This file is the ledger — the wakes owed, each
//! worker's `Heard`, the turns open and what they were asked for, the
//! lingering stretches told — written when it changes and on the way down,
//! and read back at start for the crown it belongs to. Read back, each
//! worker it names is looked at once (`hear_restored`), and what changed
//! while no daemon was looking is said once, `after a restart`; what the
//! crown already heard stays silent.
//!
//! The file follows the other state files' contract: its own
//! `schema_version`, a newer build's bytes left untouched with writes
//! barred, an unparseable file quarantined rather than clobbered. A barred
//! ledger still works for the run; it only stops being written.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::crownask::KeptAsk;
use super::crownexpect::{self, Engine, Expect, Found};
use super::crownwake::{CrownWake, Entering, Heard, ToldHand, TurnAsk};
use super::*;

/// 2 since T-936 and T-938, one schema for both (no released build carried
/// either): a wake may be `failed`, a turn's ask may be `TurnAsk::Asked`
/// (the crown's ask by its id), and a wake carries the ask it was owed for.
/// A schema-1 build cannot read those, and a same-schema file that fails to
/// parse is quarantined, which would lose the whole ledger; a newer schema
/// is left untouched with its writes barred instead. A schema-1 file reads
/// as it is. T-973's `WakeCause::Overrun` and `expects` ride 2 for the same
/// reason: no released build carried it.
pub const CROWN_SCHEMA: u32 = 2;

/// How long after a start the restart's look waits for a worker's agent to
/// settle (`hear_restored`) before letting it go to its turn's end.
pub(super) const RECHECK_MS: u64 = 60_000;

/// The file. Every field defaults, so a ledger from an older build of this
/// schema reads as what it held.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Ledger {
    schema_version: u32,
    /// The crown this ledger was kept for. Another one, or none, at start
    /// is owed nothing of it (T-414: a new crown has heard nothing).
    #[serde(skip_serializing_if = "Option::is_none")]
    crown: Option<ulid::Ulid>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    wakes: Vec<CrownWake>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    heard: BTreeMap<ulid::Ulid, Heard>,
    /// `Daemon::turns_open`: a turn running across the restart ends as a
    /// finished one (T-591).
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    turns_open: BTreeSet<ulid::Ulid>,
    /// `Daemon::turn_asks`: the turn running on the crown's ask answers it.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    asks: BTreeMap<ulid::Ulid, TurnAsk>,
    /// `Daemon::lingered`: a stretch told is not told again (T-599).
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    lingered: BTreeSet<uuid::Uuid>,
    /// `Daemon::crown_watched` (T-712): the tickets the crown watches, so a
    /// restart keeps the wait. Same schema: a build that drops the field
    /// ends the watches, and narrows nothing else.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    watched: BTreeSet<ulid::Ulid>,
    /// `Daemon::crown_held` (T-919): the train holds the crown made, so a
    /// restart keeps them the crown's to return. Same schema: a build that
    /// drops the field leaves each flag on its ticket, held as a person's,
    /// which narrows what the crown may do and widens nothing.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    held: BTreeSet<ulid::Ulid>,
    /// `Daemon::crown_hands` (T-922): a hand the crown was told of keeps
    /// its reason readable across a restart. Same schema: a build that
    /// drops the field loses the reasons, and narrows nothing else.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    hands: BTreeMap<ulid::Ulid, ToldHand>,
    /// `Daemon::crown_wake_entering` (T-918): the wake a Codex crown's box
    /// holds unentered, so the next daemon owns those words and does not
    /// read them as a person's draft. Same schema, no bump: the wakes it
    /// holds were moved out of `wakes`, so a build that drops the field
    /// loses them and what they would tell, reads the words in the box as
    /// a draft and waits on it, and pastes its next wake on top once the
    /// box is cleared. It sends nothing a person wrote, which is the bar.
    #[serde(skip_serializing_if = "Option::is_none")]
    entering: Option<Entering>,
    /// `Daemon::crown_wakes_taken` (T-990): the wakes told at their frame's
    /// take, kept for the frame's refusal or a lost pane, so a restart
    /// neither gives one a second window nor forgets what a refusal owes.
    /// Same schema, no bump: the wakes left `entering` at the take, so a
    /// build that drops the field resends none of them; it loses only the
    /// re-owe on a later refusal or a pane gone.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    taken: Vec<Entering>,
    /// `Daemon::crown_asks` (T-974): the crown's asks still out, `sent` or
    /// `handed`, each with the seat, pane and frame its receipts are fenced
    /// by, so a restart keeps their word and the turn's end still settles
    /// them. Same schema, no bump: a build that drops the field reads an
    /// ask a turn rode as `unknown`, as before, and refuses no less.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    out_asks: BTreeMap<ulid::Ulid, Vec<KeptAsk>>,
    /// `Daemon::crown_expects` (T-973): the turns the crown expects a length
    /// of, so a restart neither loses an overrun's wake nor says it twice.
    /// It rides schema 2 with `WakeCause::Overrun`: no released build
    /// carried schema 2, so none reads a wake it cannot parse.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    expects: BTreeMap<ulid::Ulid, Expect>,
}

/// Startup loader: the ledger, any notices, and whether writes are barred.
pub(super) fn load_or_recover(paths: &Paths) -> (Ledger, Vec<Notice>, bool) {
    let (ledger, notices, barred) =
        store::load_versioned(&paths.crown_file(), CROWN_SCHEMA, "the crown's ledger", "");
    (ledger.unwrap_or_default(), notices, barred)
}

impl Daemon {
    /// The ledger as it stands; empty while no crown is worn, so a board
    /// without one writes nothing past the first empty file.
    fn ledger(&self) -> Ledger {
        if self.board.crown.is_none() {
            return Ledger { schema_version: CROWN_SCHEMA, ..Ledger::default() };
        }
        Ledger {
            schema_version: CROWN_SCHEMA,
            crown: self.board.crown,
            wakes: self.crown_wakes.to_vec(),
            heard: self.crown_heard.iter().map(|(w, h)| (*w, h.clone())).collect(),
            turns_open: self.turns_open.iter().copied().collect(),
            asks: self.turn_asks.iter().map(|(w, a)| (*w, *a)).collect(),
            lingered: self.lingered.iter().copied().collect(),
            watched: self.crown_watched.iter().copied().collect(),
            held: self.crown_held.iter().copied().collect(),
            hands: self.crown_hands.iter().map(|(w, h)| (*w, h.clone())).collect(),
            entering: (*self.crown_wake_entering).clone(),
            taken: self.crown_wakes_taken.to_vec(),
            out_asks: self.kept_crown_asks(),
            expects: self.crown_expects.iter().map(|(t, e)| (*t, e.clone())).collect(),
        }
    }

    /// Whether the ledger may differ from the last one written: a field it
    /// is built from was reached for writing (`Tracked`, T-839) or the crown
    /// changed hands.
    fn ledger_dirty(&self) -> bool {
        self.board.crown != self.crown_written_for
            || self.crown_wakes.is_dirty()
            || self.crown_heard.is_dirty()
            || self.turns_open.is_dirty()
            || self.turn_asks.is_dirty()
            || self.lingered.is_dirty()
            || self.crown_watched.is_dirty()
            || self.crown_held.is_dirty()
            || self.crown_hands.is_dirty()
            || self.crown_wake_entering.is_dirty()
            || self.crown_wakes_taken.is_dirty()
            || self.crown_asks.is_dirty()
            || self.crown_expects.is_dirty()
    }

    fn mark_ledger_clean(&mut self) {
        self.crown_written_for = self.board.crown;
        self.crown_wakes.mark_clean();
        self.crown_heard.mark_clean();
        self.turns_open.mark_clean();
        self.turn_asks.mark_clean();
        self.lingered.mark_clean();
        self.crown_watched.mark_clean();
        self.crown_held.mark_clean();
        self.crown_hands.mark_clean();
        self.crown_wake_entering.mark_clean();
        self.crown_wakes_taken.mark_clean();
        self.crown_asks.mark_clean();
        self.crown_expects.mark_clean();
    }

    /// The single write path for `crown.json`: on the tick and on the way
    /// down, written only when it changed, so a quiet board writes nothing
    /// and, since T-839, builds nothing either: the 1 s bucket built and
    /// serialised the whole ledger to learn that.
    pub(super) fn persist_crown(&mut self) {
        if self.crown_barred || !self.ledger_dirty() {
            return;
        }
        let Ok(text) = serde_json::to_string_pretty(&self.ledger()) else { return };
        if store::write_if_changed(
            &self.paths.crown_file(),
            text,
            store::PRIVATE,
            &mut self.crown_written,
        )
        .is_ok()
        {
            self.mark_ledger_clean();
        }
    }

    /// Read the ledger back at start. What the last daemon was told of each
    /// worker comes back for the crown it was kept for, and every worker it
    /// names — a baseline, a turn open, an ask in flight — is owed one look
    /// (`hear_restored`). The wakes owed and the holds say `after a
    /// restart`. A ticket gone from the board takes its part with it.
    pub(super) fn restore_crown(&mut self, mut ledger: Ledger) {
        // No file reads as the empty ledger, which is then not written.
        ledger.schema_version = CROWN_SCHEMA;
        self.crown_written = serde_json::to_string_pretty(&ledger).unwrap_or_default();
        let Some(crown) = self.board.crown.filter(|c| ledger.crown == Some(*c)) else {
            return;
        };
        let on_board = |w: &ulid::Ulid| self.board.ticket(*w).is_some();
        self.lingered = ledger
            .lingered
            .into_iter()
            .filter(|id| self.board.sessions.iter().any(|s| s.id == *id))
            .collect();
        self.turns_open = ledger.turns_open.into_iter().filter(on_board).collect();
        self.crown_watched =
            ledger.watched.into_iter().filter(|w| *w != crown && on_board(w)).collect();
        // A hold the flag no longer stands behind is no hold to return.
        let flagged = |w: &ulid::Ulid| self.board.ticket(*w).is_some_and(|t| t.manual_merge);
        self.crown_held = ledger.held.into_iter().filter(|w| *w != crown && flagged(w)).collect();
        self.turn_asks = ledger.asks.into_iter().filter(|(w, _)| on_board(w)).collect();
        // A begun expectation comes back only where the engine's own id of
        // its turn finds that turn still open (T-983): Claude's prompt id in
        // its seat's transcript, read here; a Codex turn id, named again by
        // the runtime's first observation. One still waiting on its submit's
        // `entered` comes back as before.
        let found = |e: &Expect| {
            let seat = e.session.and_then(|id| self.board.sessions.iter().find(|s| s.id == id));
            match (e.turn.as_ref().map(|k| k.by), seat) {
                (Some(Engine::Codex), Some(s)) if s.kind == SessionKind::Codex => Found::Unverified,
                (Some(Engine::Prompt), Some(s)) if s.kind == SessionKind::Claude => {
                    let id = e.turn.as_ref().map(|k| k.id.as_str()).unwrap_or_default();
                    let open = s.transcript_path.as_deref().and_then(|p| {
                        crate::agents::claude::tail::prompt_still_open(std::path::Path::new(p), id)
                    });
                    if open == Some(true) {
                        Found::Open
                    } else {
                        Found::Gone
                    }
                }
                _ => Found::Gone,
            }
        };
        self.crown_expects = crownexpect::restore(ledger.expects, crown, on_board, found).into();
        // An archived worker's hand was forgotten as it went (T-922); one
        // still on file came from a ledger written before that.
        let showing = |w: &ulid::Ulid| self.board.ticket(*w).is_some_and(|t| !t.is_archived());
        // A turn begun while no daemon listened answered it too: its
        // `UserPromptSubmit` is lost, never queued, and the recovered turn
        // comes back at Low, which takes no hand down. The worker's
        // transcript dates its prompt; a prompt it does not show (none
        // written, or past the window) keeps the reason, and so does a
        // Codex worker, whose rollout is not that format.
        let answered = |w: &ulid::Ulid, hand: &ToldHand| {
            let seat = self.board.live_agent(*w).filter(|s| s.kind == SessionKind::Claude);
            seat.and_then(|s| s.transcript_path.as_deref()).is_some_and(|path| {
                crate::agents::claude::tail::prompted_since(
                    std::path::Path::new(path),
                    hand.told_ms,
                )
            })
        };
        self.crown_hands =
            ledger.hands.into_iter().filter(|(w, hand)| showing(w) && !answered(w, hand)).collect();
        self.crown_wake_entering = ledger
            .entering
            .filter(|e| e.crown == crown)
            .map(|mut e| {
                e.wakes.retain(|w| on_board(&w.worker));
                e
            })
            .into();
        // A taken wake (T-990) stays told; one owed again from here says
        // `after a restart`, as the owed do.
        self.crown_wakes_taken = ledger
            .taken
            .into_iter()
            .filter(|e| e.crown == crown)
            .map(|mut e| {
                e.wakes.retain(|w| on_board(&w.worker));
                e.wakes.iter_mut().for_each(|w| w.late = true);
                e
            })
            .collect();
        self.crown_wakes = ledger
            .wakes
            .into_iter()
            .filter(|w| on_board(&w.worker))
            .map(|mut w| {
                w.late = true;
                w
            })
            .collect();
        for (worker, mut heard) in ledger.heard {
            if worker == crown || !on_board(&worker) {
                continue;
            }
            heard.restored = true;
            let owed = heard.unjudged.unwrap_or_default();
            self.crown_recheck.insert(worker, owed);
            self.crown_heard.insert(worker, heard);
        }
        let out_asks: BTreeMap<ulid::Ulid, Vec<KeptAsk>> =
            ledger.out_asks.into_iter().filter(|(w, _)| *w != crown && on_board(w)).collect();
        let open: Vec<ulid::Ulid> = self
            .turns_open
            .iter()
            .chain(self.turn_asks.keys())
            .chain(out_asks.keys())
            .copied()
            .collect();
        for worker in open.into_iter().filter(|w| *w != crown) {
            self.crown_recheck.entry(worker).or_default();
            self.crown_heard.entry(worker).or_default().restored = true;
        }
        self.crown_recheck_until = now_ms() + RECHECK_MS;
        // The crown's asks still out come back at their word (T-974); one a
        // turn rode that no kept record names comes back `unknown` (T-938).
        self.restore_crown_asks(out_asks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> std::result::Result<Ledger, (Option<u32>, String)> {
        store::parse_versioned(text, CROWN_SCHEMA)
    }

    /// The ledger round-trips through its own bytes, and a newer build's
    /// file is refused rather than guessed at.
    #[test]
    fn the_ledger_round_trips_and_a_newer_one_is_refused() {
        let w = ulid::Ulid::from_parts(1, 7);
        let mut ledger = Ledger { schema_version: CROWN_SCHEMA, ..Ledger::default() };
        ledger.crown = Some(ulid::Ulid::from_parts(1, 1));
        ledger.turns_open.insert(w);
        ledger.asks.insert(w, TurnAsk::Crown(ulid::Ulid::from_parts(1, 1)));
        ledger.watched.insert(ulid::Ulid::from_parts(1, 9));
        ledger.held.insert(ulid::Ulid::from_parts(1, 11));
        ledger.hands.insert(w, ToldHand { reason: "which key?".into(), told_ms: 7 });
        ledger.entering = Some(Entering {
            crown: ulid::Ulid::from_parts(1, 1),
            session: uuid::Uuid::from_u128(5),
            text: "T-7 finished its turn".into(),
            wakes: Vec::new(),
            telling: Default::default(),
            person_came: true,
            frame: Some("01SUBMIT".into()),
            pane: Some("123:%4".into()),
            expires_ms: Some(9),
            parked: false,
        });
        // T-990: a wake told at its frame's take, kept for its refusal.
        ledger.taken = vec![Entering {
            crown: ulid::Ulid::from_parts(1, 1),
            session: uuid::Uuid::from_u128(5),
            text: "T-7 raised its hand".into(),
            wakes: Vec::new(),
            telling: Default::default(),
            person_came: false,
            frame: Some("01TAKEN".into()),
            pane: Some("123:%4".into()),
            expires_ms: Some(9),
            parked: false,
        }];
        ledger.out_asks.insert(
            w,
            vec![KeptAsk {
                id: ulid::Ulid::from_parts(1, 12),
                crown: ulid::Ulid::from_parts(1, 1),
                session: Some(uuid::Uuid::from_u128(5)),
                pane: Some("123:%4".into()),
                frame: Some("01SUBMIT".into()),
                submit: true,
                own_turn: true,
                released: false,
                handed: true,
                since_ms: 9,
            }],
        );
        let text = serde_json::to_string_pretty(&ledger).unwrap();
        let back = parse(&text).unwrap();
        assert_eq!(serde_json::to_string_pretty(&back).unwrap(), text);
        assert_eq!(back.out_asks, ledger.out_asks, "T-974: the asks out keep their word");
        let taken = back.taken.iter().filter_map(|e| e.frame.as_deref()).collect::<Vec<_>>();
        assert_eq!(taken, ["01TAKEN"], "T-990: a taken wake survives the restart");
        let newer = format!("{{\"schema_version\": {}}}", CROWN_SCHEMA + 1);
        assert!(matches!(parse(&newer), Err((Some(_), _))));
        assert!(matches!(parse("{"), Err((None, _))));
        // An empty file of this schema is an empty ledger.
        assert!(parse("{\"schema_version\": 1}").unwrap().heard.is_empty());
        assert!(parse("{\"schema_version\": 1}").unwrap().watched.is_empty());
        assert!(parse("{\"schema_version\": 1}").unwrap().held.is_empty());
        assert!(parse("{\"schema_version\": 1}").unwrap().hands.is_empty());
        assert!(parse("{\"schema_version\": 1}").unwrap().entering.is_none());
        assert!(parse("{\"schema_version\": 2}").unwrap().out_asks.is_empty());
    }

    /// T-936: a schema-1 ledger reads as it was under schema 2, its wakes
    /// and baselines kept; a schema-2 ledger holds a failed turn's wake
    /// with its reason through its own bytes.
    #[test]
    fn a_schema_1_ledger_reads_and_a_failed_wake_round_trips() {
        let crown = ulid::Ulid::from_parts(1, 1);
        let w = ulid::Ulid::from_parts(1, 7);
        let told = r#"{"tip": "aaa", "merge": "ahead", "ahead": 1, "column": "REVIEW"}"#;
        let v1 = format!(
            r#"{{"schema_version": 1, "crown": "{crown}",
                "wakes": [{{"worker": "{w}", "cause": "delivered", "to": {told}}}],
                "heard": {{"{w}": {{"judged": {told}, "told": {told}}}}}}}"#
        );
        let old = parse(&v1).unwrap();
        assert_eq!((old.wakes.len(), old.heard.len()), (1, 1));
        let v2 = format!(
            r#"{{"schema_version": 2, "crown": "{crown}",
                "wakes": [{{"worker": "{w}", "cause": "failed",
                    "failed": {{"session": "{}", "turn": "turn-a", "reason": "server"}}}}]}}"#,
            uuid::Uuid::nil()
        );
        let ledger = parse(&v2).unwrap();
        let text = serde_json::to_string_pretty(&ledger).unwrap();
        assert!(text.contains(r#""cause": "failed""#) && text.contains(r#""reason": "server""#));
        assert_eq!(serde_json::to_string_pretty(&parse(&text).unwrap()).unwrap(), text);
    }

    /// T-936, T-938: a schema-1 build (its reader asks for schema 1) meets
    /// a schema-2 ledger that holds a failed turn's wake and a crown ask by
    /// its id, neither of which it can read: the file is left byte for byte,
    /// its writes are barred, and nothing is quarantined, so a newer build
    /// gets its ledger back whole.
    #[test]
    fn a_schema_1_reader_leaves_a_schema_2_ledger_alone() {
        let dir = std::env::temp_dir().join(format!("msmn-crown-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("crown.json");
        let text = format!(
            r#"{{"schema_version": 2, "crown": "{c}", "asks": {{"{w}": {asked}}},
                "wakes": [{{"worker": "{w}", "cause": "failed",
                    "failed": {{"session": "{}", "turn": "turn-a", "reason": "server"}}}}]}}"#,
            uuid::Uuid::nil(),
            c = ulid::Ulid::from_parts(1, 1),
            w = ulid::Ulid::from_parts(1, 7),
            asked = serde_json::to_string(&TurnAsk::Asked {
                crown: ulid::Ulid::from_parts(1, 1),
                ask: ulid::Ulid::from_parts(1, 9),
                said: crownwake::Said::Sent,
            })
            .unwrap(),
        );
        std::fs::write(&file, &text).unwrap();
        let (ledger, notices, barred) =
            store::load_versioned::<Ledger>(&file, 1, "the crown's ledger", "");
        assert!(ledger.is_none(), "nothing read");
        assert!(barred, "its writes are barred: {notices:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text, "the bytes are untouched");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["crown.json"], "no quarantine");
        // This build reads it.
        let (ledger, _, barred) =
            store::load_versioned::<Ledger>(&file, CROWN_SCHEMA, "the crown's ledger", "");
        assert!(ledger.is_some_and(|l| l.wakes.len() == 1 && l.asks.len() == 1) && !barred);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A schema 1 ledger (before T-938) reads at schema 2: its crown turn
    /// is the old variant, its wake has no ask. An ask by id round-trips.
    #[test]
    fn a_schema_one_ledger_reads_and_an_ask_by_id_round_trips() {
        let w = ulid::Ulid::from_parts(1, 7);
        let c = ulid::Ulid::from_parts(1, 1);
        let v1 = format!(
            r#"{{"schema_version": 1, "crown": "{c}", "asks": {{"{w}": {{"crown": "{c}"}}}},
               "wakes": [{{"worker": "{w}", "cause": "answered", "finished": false, "late": false}}]}}"#
        );
        let old = parse(&v1).unwrap();
        assert_eq!(old.asks.get(&w), Some(&TurnAsk::Crown(c)));
        assert_eq!(old.wakes.len(), 1);
        let ask = ulid::Ulid::from_parts(1, 9);
        let mut ledger = Ledger { schema_version: CROWN_SCHEMA, ..Ledger::default() };
        ledger.asks.insert(w, TurnAsk::Asked { crown: c, ask, said: crownwake::Said::Taken });
        let text = serde_json::to_string(&ledger).unwrap();
        assert!(text.contains(r#""said":"taken""#), "{text}");
        assert_eq!(
            parse(&text).unwrap().asks.get(&w),
            Some(&TurnAsk::Asked { crown: c, ask, said: crownwake::Said::Taken })
        );
    }

    /// T-973: the crown's expectations ride the ledger through its own
    /// bytes, beside the overrun wake owed for one; a schema-1 reader leaves
    /// that ledger alone, as it does T-936's.
    #[test]
    fn an_expectation_and_its_overrun_wake_round_trip() {
        let w = ulid::Ulid::from_parts(1, 7);
        let v2 = format!(
            r#"{{"schema_version": 2, "crown": "{c}",
                "wakes": [{{"worker": "{w}", "cause": "overrun",
                    "overrun": {{"expected_secs": 60, "running_secs": 75}}}}],
                "expects": {{"{w}": {{"ask": "{a}", "expect_secs": 60, "began_ms": 5,
                    "session": "{s}", "told": true}}}}}}"#,
            c = ulid::Ulid::from_parts(1, 1),
            a = ulid::Ulid::from_parts(1, 9),
            s = uuid::Uuid::nil(),
        );
        let ledger = parse(&v2).unwrap();
        assert_eq!(ledger.wakes.len(), 1);
        let e = ledger.expects.get(&w).unwrap();
        assert!(e.told && e.began_ms == Some(5) && e.ask == Some(ulid::Ulid::from_parts(1, 9)));
        let text = serde_json::to_string_pretty(&ledger).unwrap();
        assert!(text.contains(r#""cause": "overrun""#) && text.contains(r#""running_secs": 75"#));
        assert_eq!(serde_json::to_string_pretty(&parse(&text).unwrap()).unwrap(), text);
        assert!(parse("{\"schema_version\": 1}").unwrap().expects.is_empty());
        let old = store::parse_versioned::<Ledger>(&text, 1);
        assert!(matches!(old, Err((Some(2), _))), "a schema-1 reader refuses it whole");
    }

    /// Not an assertion about wall clock (see `search.rs`): the number in
    /// STALE-MAP's T-839 block. `persist_crown` builds `Daemon::ledger` and
    /// its text to compare with the last write; this is that work on a
    /// ledger that heard 650 workers, a `Daemon` being no unit test's.
    #[test]
    #[ignore = "prints a timing; run with --ignored"]
    fn how_long_a_big_crown_ledger_check_takes() {
        let told = r#"{"tip":"0123456789abcdef0123456789abcdef01234567","merge":"ahead","ahead":3,"column":"REVIEW"}"#;
        let heard: Vec<String> = (0..650u128)
            .map(|n| {
                let w = ulid::Ulid::from_parts(1, n + 10);
                format!(r#""{w}": {{"judged": {told}, "told": {told}}}"#)
            })
            .collect();
        let text = format!(
            r#"{{"schema_version": 1, "crown": "{}", "heard": {{{}}}}}"#,
            ulid::Ulid::from_parts(1, 1),
            heard.join(",")
        );
        let ledger = parse(&text).unwrap();
        let mut held: Tracked<HashMap<ulid::Ulid, Heard>> =
            ledger.heard.clone().into_iter().collect();
        let build = |held: &HashMap<ulid::Ulid, Heard>| Ledger {
            schema_version: CROWN_SCHEMA,
            crown: ledger.crown,
            heard: held.iter().map(|(w, h)| (*w, h.clone())).collect(),
            ..Ledger::default()
        };
        let written = serde_json::to_string_pretty(&build(&held)).unwrap();
        let t0 = std::time::Instant::now();
        for _ in 0..20 {
            let now = serde_json::to_string_pretty(&build(&held)).unwrap();
            assert_eq!(now, written);
        }
        println!(
            "unchanged     {:>8.2} ms, {} bytes, built and compared",
            t0.elapsed().as_secs_f64() * 1e3 / 20.0,
            written.len()
        );
        // `persist_crown` since T-839: no field marked, nothing built.
        held.mark_clean();
        let t0 = std::time::Instant::now();
        for _ in 0..20 {
            if held.is_dirty() {
                let _ = serde_json::to_string_pretty(&build(&held)).unwrap();
            }
        }
        println!("unchanged     {:>8.5} ms, unmarked", t0.elapsed().as_secs_f64() * 1e3 / 20.0);
    }
}
