//! `queue.json`: the ask queue's STARTS and WAKES, so a daemon restart does
//! not lose a column's worth of parked sessions (T-418: an agent's
//! `pkill -f "mesimon daemon"` after a rebuild dropped twenty queued BACKLOG
//! starts, silently).
//!
//! Only the seats a restart can still honour are written. A `Pane` entry is
//! words owed to a pane the next daemon "no longer understands"
//! (`Daemon::owed`'s parked words): the pane's state is re-derived at
//! Low confidence and the paste may land in a box mid-turn, so it dies with
//! the process as before. A `Start` has no pane at all — the title and the
//! words are the whole entry — and a `Wake` names a parked record that
//! `sessions.json` carries across the same restart. Both replay exactly.
//!
//! The file follows the other state files' contract: its own
//! `schema_version`, a newer build's bytes left untouched with writes
//! barred, an unparseable file quarantined rather than clobbered.

use std::path::{Path, PathBuf};

use anyhow::Result;
use mesimon_core::board::AgentProvider;
use mesimon_core::command::Notice;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

pub const QUEUE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueFile {
    pub schema_version: u32,
    #[serde(default)]
    pub entries: Vec<QueuedEntry>,
}

/// One parked start or wake. The checkout it waits on is not stored: a
/// `Start` waits on the shared root, a `Wake` on its record's own `cwd`, and
/// both are re-read at load so a moved checkout never pins a stale path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedEntry {
    pub ticket: ulid::Ulid,
    pub seat: PersistedSeat,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub queued_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PersistedSeat {
    /// A parked claude to wake and ask (T-294).
    Wake { session: uuid::Uuid },
    /// An empty seat to start on the ticket's title (T-294, T-379).
    Start { provider: AgentProvider },
}

pub fn queue_file(paths: &Paths) -> PathBuf {
    paths.queue_file()
}

/// `Err(Some(v))` is a file from a NEWER mesimon: valid bytes this build
/// must refuse rather than guess at. `Err(None)` is genuinely unparseable.
fn parse(text: &str) -> std::result::Result<Vec<QueuedEntry>, (Option<u32>, String)> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| (None, e.to_string()))?;
    let found = v.get("schema_version").and_then(|s| s.as_u64()).unwrap_or(1) as u32;
    if found > QUEUE_SCHEMA {
        return Err((Some(found), format!("schema {found}")));
    }
    serde_json::from_value::<QueueFile>(v).map(|f| f.entries).map_err(|e| (None, e.to_string()))
}

/// Startup loader: the entries, any notices, and whether writes are barred.
/// A missing file is an empty queue. Barred means bytes we could not read
/// (or a newer build's) are still on disk, so a save would destroy the only
/// copy — the daemon then keeps its queue in memory for the run, as before.
pub fn load_or_recover(paths: &Paths) -> (Vec<QueuedEntry>, Vec<Notice>, bool) {
    let f = queue_file(paths);
    let mut notices = Vec::new();
    if !f.is_file() {
        return (Vec::new(), notices, false);
    }
    let text = match std::fs::read_to_string(&f) {
        Ok(t) => t,
        Err(e) => {
            notices.push(
                Notice::new("quarantined", "queued asks could not be opened — not written to")
                    .with_path(f.display())
                    .with_detail(e.to_string()),
            );
            return (Vec::new(), notices, true);
        }
    };
    let detail = match parse(&text) {
        Ok(entries) => return (entries, notices, false),
        Err((Some(found), _)) => {
            notices.push(
                Notice::new(
                    "future_version",
                    format!(
                        "queue.json was written by a newer mesimon (schema {found}, this build \
                         reads {QUEUE_SCHEMA}) — left untouched and not written to"
                    ),
                )
                .with_path(f.display()),
            );
            return (Vec::new(), notices, true);
        }
        Err((None, detail)) => detail,
    };
    let moved = crate::store::quarantine(&f);
    notices.push(
        Notice::new(
            "quarantined",
            match &moved {
                Some(dest) => format!(
                    "queued asks could not be read — the file was set aside as {}",
                    short_name(dest)
                ),
                None => "queued asks could not be read — not written to".to_string(),
            },
        )
        .with_path(f.display())
        .with_detail(detail),
    );
    (Vec::new(), notices, moved.is_none())
}

pub fn save(paths: &Paths, entries: &[QueuedEntry]) -> Result<()> {
    let f = queue_file(paths);
    if entries.is_empty() {
        // An empty queue is no file: nothing to restore, nothing to parse.
        if f.exists() {
            std::fs::remove_file(&f)?;
        }
        return Ok(());
    }
    let qf = QueueFile { schema_version: QUEUE_SCHEMA, entries: entries.to_vec() };
    crate::store::write_atomic(&f, &serde_json::to_string_pretty(&qf)?, crate::store::PRIVATE)
}

fn short_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut p = Paths::for_repo(&repo).unwrap();
        p.state_dir = dir.path().join("state");
        std::fs::create_dir_all(&p.state_dir).unwrap();
        (dir, p)
    }

    fn entries() -> Vec<QueuedEntry> {
        vec![
            QueuedEntry {
                ticket: ulid::Ulid::new(),
                seat: PersistedSeat::Start { provider: AgentProvider::Codex },
                text: "read the ticket".into(),
                queued_at: 7,
            },
            QueuedEntry {
                ticket: ulid::Ulid::new(),
                seat: PersistedSeat::Wake { session: uuid::Uuid::new_v4() },
                text: String::new(),
                queued_at: 8,
            },
        ]
    }

    #[test]
    fn round_trips_starts_and_wakes() {
        let (_d, p) = paths();
        let e = entries();
        save(&p, &e).unwrap();
        let (back, notices, barred) = load_or_recover(&p);
        assert!(notices.is_empty(), "{notices:?}");
        assert!(!barred);
        assert_eq!(back, e);
        let text = std::fs::read_to_string(queue_file(&p)).unwrap();
        assert!(text.contains(&format!("\"schema_version\": {QUEUE_SCHEMA}")), "{text}");
        assert!(text.contains("\"kind\": \"start\""), "{text}");
    }

    #[test]
    fn a_missing_file_is_an_empty_queue_and_an_empty_save_removes_the_file() {
        let (_d, p) = paths();
        let (back, notices, barred) = load_or_recover(&p);
        assert!(back.is_empty() && notices.is_empty() && !barred);
        save(&p, &entries()).unwrap();
        assert!(queue_file(&p).is_file());
        save(&p, &[]).unwrap();
        assert!(!queue_file(&p).exists(), "an empty queue leaves no file behind");
    }

    #[test]
    fn a_newer_schema_is_left_untouched_and_bars_writes() {
        let (_d, p) = paths();
        let text = format!("{{\"schema_version\": {}, \"entries\": []}}", QUEUE_SCHEMA + 1);
        std::fs::write(queue_file(&p), &text).unwrap();
        let (back, notices, barred) = load_or_recover(&p);
        assert!(back.is_empty());
        assert!(barred);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, "future_version");
        assert_eq!(std::fs::read_to_string(queue_file(&p)).unwrap(), text, "bytes untouched");
    }

    #[test]
    fn garbage_is_quarantined_and_the_queue_comes_up_empty_and_writable() {
        let (_d, p) = paths();
        std::fs::write(queue_file(&p), "{ not json").unwrap();
        let (back, notices, barred) = load_or_recover(&p);
        assert!(back.is_empty());
        assert!(!barred, "the bytes were set aside, so the path is free again");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, "quarantined");
        assert!(!queue_file(&p).exists());
        let kept = std::fs::read_dir(&p.state_dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("queue.json.quarantine-"));
        assert!(kept, "the bytes are preserved beside the file");
    }
}
