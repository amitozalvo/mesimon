//! `started.json`: every conversation a session mesimon SPAWNED has held
//! (T-441), so the External drawer never offers mesimon's own work as a
//! foreign session. A record forgets: `/clear` moves it to a new id, a
//! deleted ticket takes its records with it. This file does not — the drawer
//! listed 50 such conversations on the author's board before it existed.
//!
//! An adopted record contributes nothing. Its conversation was started
//! outside, and once the record is dead the drawer offering it again is the
//! re-import road (`m3_e2e`: "exited import must re-surface in the drawer").
//!
//! The file follows the other state files' contract: its own
//! `schema_version`, a newer build's bytes left untouched with writes
//! barred, an unparseable file quarantined rather than clobbered. A barred
//! set still filters for the run; it only stops being written.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use mesimon_core::board::{Provenance, SessionRecord};
use mesimon_core::command::Notice;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

pub const STARTED_SCHEMA: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct StartedFile {
    schema_version: u32,
    #[serde(default)]
    keys: Vec<String>,
}

/// Fold in the conversation key of every record mesimon spawned; true when
/// the set grew. `Daemon::persist_sessions` calls it, and every key change —
/// a spawn, a hook-learned `claude_session_id`, a fresh resume, a Codex
/// thread id — persists the sessions, so no mutator has to remember this.
pub fn fold(set: &mut HashSet<String>, sessions: &[SessionRecord]) -> bool {
    let mut grew = false;
    for record in sessions.iter().filter(|r| r.provenance == Provenance::Spawned) {
        let Some(adapter) = crate::agents::adapter(record.kind) else { continue };
        if let Some(key) = adapter.conversation_key(record) {
            grew |= set.insert(key);
        }
    }
    grew
}

/// Every id mesimon handed Claude as `--session-id`: each launch writes
/// `hooks/<uuid>.json` and nothing removes one, so the directory is the
/// history this file did not exist to keep. Read at every start, which is
/// also the migration. `<uuid>.codex.json` names a record, not a
/// conversation, and fails the parse on its own.
fn minted(hooks_dir: &Path) -> Vec<String> {
    let Ok(files) = std::fs::read_dir(hooks_dir) else { return Vec::new() };
    files
        .flatten()
        .filter_map(|f| {
            let name = f.file_name();
            let stem = name.to_str()?.strip_suffix(".json")?;
            stem.parse::<uuid::Uuid>().ok().map(|id| id.to_string())
        })
        .collect()
}

/// `Err(Some(v))` is a file from a NEWER mesimon: valid bytes this build
/// must refuse rather than guess at. `Err(None)` is genuinely unparseable.
fn parse(text: &str) -> std::result::Result<Vec<String>, (Option<u32>, String)> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| (None, e.to_string()))?;
    let found = v.get("schema_version").and_then(|s| s.as_u64()).unwrap_or(1) as u32;
    if found > STARTED_SCHEMA {
        return Err((Some(found), format!("schema {found}")));
    }
    serde_json::from_value::<StartedFile>(v).map(|f| f.keys).map_err(|e| (None, e.to_string()))
}

/// Startup loader: the file's keys, the minted ids under `hooks/` and the
/// keys `sessions` holds now; any notices; whether writes are barred. Writes
/// the union back when it grew.
pub fn load_or_recover(
    paths: &Paths,
    sessions: &[SessionRecord],
) -> (HashSet<String>, Vec<Notice>, bool) {
    let (mut keys, notices, barred) = read(paths);
    let before = keys.len();
    keys.extend(minted(&paths.hooks_dir()));
    fold(&mut keys, sessions);
    if keys.len() > before && !barred {
        let _ = save(paths, &keys);
    }
    (keys, notices, barred)
}

fn read(paths: &Paths) -> (HashSet<String>, Vec<Notice>, bool) {
    let f = paths.started_file();
    let mut notices = Vec::new();
    if !f.is_file() {
        return (HashSet::new(), notices, false);
    }
    let text = match std::fs::read_to_string(&f) {
        Ok(t) => t,
        Err(e) => {
            notices.push(
                Notice::new(
                    "quarantined",
                    "the sessions mesimon started could not be opened — not written to",
                )
                .with_path(f.display())
                .with_detail(e.to_string()),
            );
            return (HashSet::new(), notices, true);
        }
    };
    let detail = match parse(&text) {
        Ok(keys) => return (keys.into_iter().collect(), notices, false),
        Err((Some(found), _)) => {
            notices.push(
                Notice::new(
                    "future_version",
                    format!(
                        "started.json was written by a newer mesimon (schema {found}, this build \
                         reads {STARTED_SCHEMA}) — left untouched and not written to"
                    ),
                )
                .with_path(f.display()),
            );
            return (HashSet::new(), notices, true);
        }
        Err((None, detail)) => detail,
    };
    let moved = crate::store::quarantine(&f);
    notices.push(
        Notice::new(
            "quarantined",
            match &moved {
                Some(dest) => format!(
                    "the sessions mesimon started could not be read — the file was set aside as {}",
                    dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                ),
                None => {
                    "the sessions mesimon started could not be read — not written to".to_string()
                }
            },
        )
        .with_path(f.display())
        .with_detail(detail),
    );
    (HashSet::new(), notices, moved.is_none())
}

/// Sorted, so the file diffs as the set it is.
pub fn save(paths: &Paths, keys: &HashSet<String>) -> Result<()> {
    let mut keys: Vec<String> = keys.iter().cloned().collect();
    keys.sort();
    let file = StartedFile { schema_version: STARTED_SCHEMA, keys };
    crate::store::write_atomic(
        &paths.started_file(),
        &serde_json::to_string_pretty(&file)?,
        crate::store::PRIVATE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::{SessionKind, SessionState};

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut p = Paths::for_repo(&repo).unwrap();
        p.state_dir = dir.path().join("state");
        std::fs::create_dir_all(&p.state_dir).unwrap();
        (dir, p)
    }

    fn record(kind: SessionKind, provenance: Provenance) -> SessionRecord {
        let mut r = SessionRecord::new(
            uuid::Uuid::new_v4(),
            kind,
            ulid::Ulid::new(),
            vec![],
            "/repo".into(),
            SessionState::Sleeping,
        );
        r.provenance = provenance;
        r
    }

    #[test]
    fn fold_keeps_every_key_a_spawned_record_holds_and_nothing_adopted() {
        let mut set = HashSet::new();
        let mut spawned = record(SessionKind::Claude, Provenance::Spawned);
        let mut adopted = record(SessionKind::Claude, Provenance::Adopted);
        adopted.claude_session_id = Some(uuid::Uuid::new_v4());
        let shell = record(SessionKind::Bash, Provenance::Spawned);
        let mut codex = record(SessionKind::Codex, Provenance::Spawned);
        assert!(fold(&mut set, &[spawned.clone(), adopted.clone(), shell.clone(), codex.clone()]));
        assert_eq!(set, HashSet::from([spawned.id.to_string()]), "no thread id yet, no key");

        // `/clear`: the record moves to a new conversation; the old one stays.
        let cleared = uuid::Uuid::new_v4();
        spawned.claude_session_id = Some(cleared);
        codex.codex_thread_id = Some("thread-1".into());
        assert!(fold(&mut set, &[spawned.clone(), adopted.clone(), shell, codex.clone()]));
        assert_eq!(
            set,
            HashSet::from([spawned.id.to_string(), cleared.to_string(), "thread-1".to_string()])
        );
        assert!(!fold(&mut set, &[spawned, adopted, codex]), "nothing new, nothing to write");
    }

    #[test]
    fn load_seeds_from_the_minted_hook_files_and_the_records_and_writes_the_union() {
        let (_d, p) = paths();
        let hooks = p.hooks_dir();
        std::fs::create_dir_all(&hooks).unwrap();
        let minted = uuid::Uuid::new_v4();
        std::fs::write(hooks.join(format!("{minted}.json")), "{}").unwrap();
        std::fs::write(hooks.join(format!("{}.codex.json", uuid::Uuid::new_v4())), "{}").unwrap();
        std::fs::write(hooks.join("notes.json"), "{}").unwrap();
        let spawned = record(SessionKind::Claude, Provenance::Spawned);

        let (set, notices, barred) = load_or_recover(&p, std::slice::from_ref(&spawned));
        assert!(notices.is_empty() && !barred, "{notices:?}");
        assert_eq!(set, HashSet::from([minted.to_string(), spawned.id.to_string()]));

        // The record is gone (its ticket was deleted) and the key is not.
        let (back, _, _) = load_or_recover(&p, &[]);
        assert_eq!(back, set);
        let text = std::fs::read_to_string(p.started_file()).unwrap();
        assert!(text.contains(&format!("\"schema_version\": {STARTED_SCHEMA}")), "{text}");
    }

    #[test]
    fn a_missing_file_and_an_empty_board_write_nothing() {
        let (_d, p) = paths();
        let (set, notices, barred) = load_or_recover(&p, &[]);
        assert!(set.is_empty() && notices.is_empty() && !barred);
        assert!(!p.started_file().exists());
    }

    #[test]
    fn a_newer_schema_is_left_untouched_and_bars_writes() {
        let (_d, p) = paths();
        let text = format!("{{\"schema_version\": {}, \"keys\": []}}", STARTED_SCHEMA + 1);
        std::fs::write(p.started_file(), &text).unwrap();
        let spawned = record(SessionKind::Claude, Provenance::Spawned);
        let (set, notices, barred) = load_or_recover(&p, std::slice::from_ref(&spawned));
        assert!(barred);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, "future_version");
        assert!(set.contains(&spawned.id.to_string()), "a barred set still filters");
        assert_eq!(std::fs::read_to_string(p.started_file()).unwrap(), text, "bytes untouched");
    }

    #[test]
    fn garbage_is_quarantined_and_the_set_is_rebuilt() {
        let (_d, p) = paths();
        std::fs::write(p.started_file(), "{ not json").unwrap();
        let spawned = record(SessionKind::Claude, Provenance::Spawned);
        let (set, notices, barred) = load_or_recover(&p, std::slice::from_ref(&spawned));
        assert!(!barred, "the bytes were set aside, so the path is free again");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, "quarantined");
        assert_eq!(set, HashSet::from([spawned.id.to_string()]));
        let kept = std::fs::read_dir(&p.state_dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("started.json.quarantine-"));
        assert!(kept, "the bytes are preserved beside the file");
        assert!(p.started_file().is_file(), "and the rebuilt set is written");
    }
}
