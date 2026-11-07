//! `crown-acts.jsonl` (T-1162): what the crown did, kept for the ticket
//! page's conversation. One `CrownAct` per line, appended by the daemon's
//! main thread as each act lands (acts are rare: a crown's tool call, a
//! wake), read by the TUI off its draw thread. Board facts only, never a
//! prompt's text (D11): `CrownAct::words` is a key of the words.
//!
//! The file rotates to `.1` by size, so a board's history is the two files
//! together. A log that cannot open writes nothing and fails nothing: the
//! board is more important than this record.

use std::io::Write;
use std::path::{Path, PathBuf};

use mesimon_core::crownact::CrownAct;

/// Rotate at 1 MiB: some five thousand acts, months of a busy crown.
pub const ROTATE_BYTES: u64 = 1024 * 1024;

pub struct Log {
    path: PathBuf,
    file: Option<std::fs::File>,
    bytes: u64,
    rotate_at: u64,
}

fn open(path: &Path) -> Option<std::fs::File> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

impl Log {
    pub fn open(path: &Path) -> Self {
        Self::open_with_limit(path, ROTATE_BYTES)
    }

    fn open_with_limit(path: &Path, rotate_at: u64) -> Self {
        let file = open(path);
        let bytes = file.as_ref().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len());
        Self { path: path.to_path_buf(), file, bytes, rotate_at }
    }

    /// One act, written at once: the page that is open reads it next.
    pub fn append(&mut self, act: &CrownAct) {
        let Some(file) = self.file.as_mut() else { return };
        let Ok(mut line) = serde_json::to_string(act) else { return };
        line.push('\n');
        if file.write_all(line.as_bytes()).is_err() {
            return;
        }
        self.bytes += line.len() as u64;
        if self.bytes >= self.rotate_at {
            let _ = std::fs::rename(&self.path, rotated(&self.path));
            self.file = open(&self.path);
            self.bytes = 0;
        }
    }
}

fn rotated(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

/// (len, mtime ms) of the log and its rotation: what moves when a line is
/// written.
pub fn stamp(path: &Path) -> (u64, u64) {
    let one = |p: &Path| {
        std::fs::metadata(p)
            .map(|m| {
                let ms = m.modified().ok().and_then(mesimon_core::clock::epoch_ms).unwrap_or(0);
                (m.len(), ms)
            })
            .unwrap_or_default()
    };
    let (a, b) = (one(&rotated(path)), one(path));
    (a.0 + b.0, a.1.max(b.1))
}

/// Every act the two files hold, oldest first. A line that does not parse
/// (a torn write, a newer build's) is skipped.
pub fn read(path: &Path) -> Vec<CrownAct> {
    let mut out = Vec::new();
    for p in [rotated(path), path.to_path_buf()] {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        out.extend(text.lines().filter_map(|l| serde_json::from_str::<CrownAct>(l).ok()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(n: u64) -> CrownAct {
        CrownAct {
            at_ms: n,
            crown: ulid::Ulid(1),
            ticket: ulid::Ulid(2),
            act: "moved".into(),
            detail: "REVIEW".into(),
            words: None,
            cause: None,
        }
    }

    #[test]
    fn acts_read_back_in_order_across_a_rotation() {
        let dir = std::env::temp_dir().join(format!("msmn-crownacts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("crown-acts.jsonl");
        let mut log = Log::open_with_limit(&path, 300);
        for n in 0..6 {
            log.append(&act(n));
        }
        assert!(rotated(&path).exists(), "the log rotated");
        let read = read(&path);
        let at: Vec<u64> = read.iter().map(|a| a.at_ms).collect();
        // The rotation keeps one older file: the newest acts are all there,
        // in order, and nothing older than the rotation's file.
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{at:?}");
        assert_eq!(at.last(), Some(&5));
        // A torn line is skipped, not the file.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"at_ms\":")
            .unwrap();
        assert_eq!(super::read(&path).len(), read.len());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_log_that_cannot_open_writes_nothing_and_fails_nothing() {
        let mut log = Log::open(Path::new("/dev/null/no/crown-acts.jsonl"));
        log.append(&act(1));
        assert!(read(Path::new("/dev/null/no/crown-acts.jsonl")).is_empty());
    }
}
