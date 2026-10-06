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

use std::path::PathBuf;

use anyhow::Result;
use mesimon_core::board::AgentProvider;
use mesimon_core::command::Notice;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// v2 (T-434) adds `plan` to an entry. A bump for the columns file's
/// `claude_mode` reason: a v1 build reading the file would drop the flag and
/// start the restored session in the column's mode — auto, where the person
/// queued a read-only planning turn — which is a widening.
pub const QUEUE_SCHEMA: u32 = 2;

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
    /// The start or wake runs in plan mode (T-434).
    #[serde(default)]
    pub plan: bool,
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

/// Startup loader: the entries, any notices, and whether writes are barred.
/// A missing file is an empty queue. Barred means bytes we could not read
/// (or a newer build's) are still on disk, so a save would destroy the only
/// copy — the daemon then keeps its queue in memory for the run, as before.
pub fn load_or_recover(paths: &Paths) -> (Vec<QueuedEntry>, Vec<Notice>, bool) {
    let (file, notices, barred) = crate::store::load_versioned::<QueueFile>(
        &queue_file(paths),
        QUEUE_SCHEMA,
        "queued asks",
        "",
    );
    (file.map(|f| f.entries).unwrap_or_default(), notices, barred)
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
                plan: true,
            },
            QueuedEntry {
                ticket: ulid::Ulid::new(),
                seat: PersistedSeat::Wake { session: uuid::Uuid::new_v4() },
                text: String::new(),
                queued_at: 8,
                plan: false,
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
        assert!(text.contains("\"plan\": true"), "{text}");
    }

    /// A v1 file (T-418) has no `plan`: it reads as false, never as a
    /// refusal — the bump is for the OTHER direction (an older build must
    /// not drop the flag and widen the start).
    #[test]
    fn a_v1_file_reads_with_plan_off() {
        let (_d, p) = paths();
        let text = r#"{"schema_version": 1, "entries": [{"ticket": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "seat": {"kind": "start", "provider": "claude_code"}, "text": "x", "queued_at": 1}]}"#;
        std::fs::write(queue_file(&p), text).unwrap();
        let (back, notices, barred) = load_or_recover(&p);
        assert!(notices.is_empty(), "{notices:?}");
        assert!(!barred);
        assert_eq!(back.len(), 1);
        assert!(!back[0].plan);
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
