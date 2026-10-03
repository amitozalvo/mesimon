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

use super::crownwake::{CrownWake, Heard, TurnAsk};
use super::*;

pub const CROWN_SCHEMA: u32 = 1;

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
}

/// `Err(Some(v))` is a file from a NEWER mesimon: valid bytes this build
/// must refuse rather than guess at. `Err(None)` is genuinely unparseable.
fn parse(text: &str) -> std::result::Result<Ledger, (Option<u32>, String)> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| (None, e.to_string()))?;
    let found = v.get("schema_version").and_then(|s| s.as_u64()).unwrap_or(1) as u32;
    if found > CROWN_SCHEMA {
        return Err((Some(found), format!("schema {found}")));
    }
    serde_json::from_value::<Ledger>(v).map_err(|e| (None, e.to_string()))
}

/// Startup loader: the ledger, any notices, and whether writes are barred.
pub(super) fn load_or_recover(paths: &Paths) -> (Ledger, Vec<Notice>, bool) {
    let f = paths.crown_file();
    let mut notices = Vec::new();
    if !f.is_file() {
        return (Ledger::default(), notices, false);
    }
    let text = match std::fs::read_to_string(&f) {
        Ok(t) => t,
        Err(e) => {
            notices.push(
                Notice::new(
                    "quarantined",
                    "the crown's ledger could not be opened — not written to",
                )
                .with_path(f.display())
                .with_detail(e.to_string()),
            );
            return (Ledger::default(), notices, true);
        }
    };
    let detail = match parse(&text) {
        Ok(ledger) => return (ledger, notices, false),
        Err((Some(found), _)) => {
            notices.push(
                Notice::new(
                    "future_version",
                    format!(
                        "crown.json was written by a newer mesimon (schema {found}, this build \
                         reads {CROWN_SCHEMA}) — left untouched and not written to"
                    ),
                )
                .with_path(f.display()),
            );
            return (Ledger::default(), notices, true);
        }
        Err((None, detail)) => detail,
    };
    let moved = store::quarantine(&f);
    notices.push(
        Notice::new(
            "quarantined",
            match &moved {
                Some(dest) => format!(
                    "the crown's ledger could not be read — the file was set aside as {}",
                    dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                ),
                None => "the crown's ledger could not be read — not written to".to_string(),
            },
        )
        .with_path(f.display())
        .with_detail(detail),
    );
    (Ledger::default(), notices, moved.is_none())
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
            wakes: self.crown_wakes.clone(),
            heard: self.crown_heard.iter().map(|(w, h)| (*w, h.clone())).collect(),
            turns_open: self.turns_open.iter().copied().collect(),
            asks: self.turn_asks.iter().map(|(w, a)| (*w, *a)).collect(),
            lingered: self.lingered.iter().copied().collect(),
        }
    }

    /// The single write path for `crown.json`: on the tick and on the way
    /// down, written only when it changed, so a quiet board writes nothing.
    pub(super) fn persist_crown(&mut self) {
        if self.crown_barred {
            return;
        }
        let Ok(text) = serde_json::to_string_pretty(&self.ledger()) else { return };
        if text == self.crown_written {
            return;
        }
        if store::write_atomic(&self.paths.crown_file(), &text, store::PRIVATE).is_ok() {
            self.crown_written = text;
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
        self.turn_asks = ledger.asks.into_iter().filter(|(w, _)| on_board(w)).collect();
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
        let open: Vec<ulid::Ulid> =
            self.turns_open.iter().chain(self.turn_asks.keys()).copied().collect();
        for worker in open.into_iter().filter(|w| *w != crown) {
            self.crown_recheck.entry(worker).or_default();
            self.crown_heard.entry(worker).or_default().restored = true;
        }
        self.crown_recheck_until = now_ms() + RECHECK_MS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ledger round-trips through its own bytes, and a newer build's
    /// file is refused rather than guessed at.
    #[test]
    fn the_ledger_round_trips_and_a_newer_one_is_refused() {
        let w = ulid::Ulid::from_parts(1, 7);
        let mut ledger = Ledger { schema_version: CROWN_SCHEMA, ..Ledger::default() };
        ledger.crown = Some(ulid::Ulid::from_parts(1, 1));
        ledger.turns_open.insert(w);
        ledger.asks.insert(w, TurnAsk::Crown(ulid::Ulid::from_parts(1, 1)));
        let text = serde_json::to_string_pretty(&ledger).unwrap();
        let back = parse(&text).unwrap();
        assert_eq!(serde_json::to_string_pretty(&back).unwrap(), text);
        let newer = format!("{{\"schema_version\": {}}}", CROWN_SCHEMA + 1);
        assert!(matches!(parse(&newer), Err((Some(_), _))));
        assert!(matches!(parse("{"), Err((None, _))));
        // An empty file of this schema is an empty ledger.
        assert!(parse("{\"schema_version\": 1}").unwrap().heard.is_empty());
    }
}
