//! Sampling the repo's own `CLAUDE.md` (T-217).
//!
//! `mesimon_core::claudemd` holds the text; this is the one piece of I/O
//! around it — "does the file already say it". A user who wrote the
//! instruction into their own CLAUDE.md has done the thing the agent-brief
//! offer (T-224, `mesimon_core::brief`) is for, and the offer stands down.
//!
//! Until T-224 this module also WROTE the snippet into `<repo>/CLAUDE.md`
//! behind the dialog. That road is gone: the brief lives in the system prompt
//! of the sessions mesimon starts, reaches only them, and writes nothing the
//! user tracks in git. `c` in the dialog still copies the snippet for a user
//! who would rather keep the words in their own file.

use std::path::{Path, PathBuf};

use mesimon_core::claudemd;
use mesimon_core::command::ClaudeMdStatus;

/// A sample, plus what it was taken from, so the next one can be skipped.
///
/// The gate is a `stat`, not a hash: a CLAUDE.md is commonly tens of kilobytes
/// (mesimon's own is ninety-five), a snapshot happens on every board change,
/// and re-reading that on each one to answer a question whose answer almost
/// never changes would be the most expensive thing in the tick.
#[derive(Debug, Clone, Default)]
pub struct Sampler {
    status: ClaudeMdStatus,
    /// `(len, mtime_ms)` of each file looked at, in `paths()` order. A file
    /// that is missing stamps `None`, so its arrival is a change too.
    stamps: Vec<Option<(u64, u64)>>,
}

/// The files a marker may live in: the repo root's `CLAUDE.md`, and
/// `.claude/CLAUDE.md`, which Claude Code reads just as happily. A user who put
/// the instruction in either has already done the thing the offer asks for and
/// must not be nagged for it.
fn paths(repo: &Path) -> [PathBuf; 2] {
    [repo.join("CLAUDE.md"), repo.join(".claude").join("CLAUDE.md")]
}

fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let ms = meta.modified().ok().and_then(mesimon_core::clock::epoch_ms).unwrap_or(0);
    Some((meta.len(), ms))
}

impl Sampler {
    /// The last answer. Cheap; call it per snapshot.
    pub fn status(&self) -> ClaudeMdStatus {
        self.status.clone()
    }

    /// Re-read only if a `stat` says something moved. Returns whether the
    /// answer changed, so the caller can broadcast on a real delta and stay
    /// quiet otherwise — the road `gitstatus::sample` already takes.
    pub fn refresh(&mut self, repo: &Path) -> bool {
        let files = paths(repo);
        let stamps: Vec<Option<(u64, u64)>> = files.iter().map(|p| stamp(p)).collect();
        if stamps == self.stamps && self.status.sampled {
            return false;
        }
        self.stamps = stamps;
        let present = files
            .iter()
            .any(|p| std::fs::read_to_string(p).is_ok_and(|b| claudemd::has_marker(&b)));
        // `offer` is answered per snapshot (`ClaudeMdStatus::for_board`): it
        // needs the board's switches, which this sampler does not hold.
        let next = ClaudeMdStatus {
            path: files[0].display().to_string(),
            sampled: true,
            present,
            offer: false,
        };
        let changed = next != self.status;
        self.status = next;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("msmn-claudemd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("test dir");
        d
    }

    /// A repo with no CLAUDE.md, or one that never said the words, is a repo
    /// the offer stands on; the words arriving — however — withdraw it.
    #[test]
    fn the_words_arriving_withdraw_the_offer() {
        let repo = dir("fresh");
        let mut s = Sampler::default();
        assert!(s.refresh(&repo), "the first sample is always news");
        assert!(!s.status().present);
        assert!(s.status().sampled, "and it says so: an unsampled repo offers nothing");
        assert_eq!(s.status().path, repo.join("CLAUDE.md").display().to_string());

        std::fs::write(repo.join("CLAUDE.md"), "# Rules\n").expect("seed");
        // The stamp moved, but the ANSWER did not: no marker, still offered,
        // and `refresh` reports a delta only when the answer moves.
        assert!(!s.refresh(&repo), "a file without the marker is not news");
        assert!(!s.status().present, "a file without the marker still offers");

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(repo.join("CLAUDE.md"), claudemd::appended("# Rules\n")).expect("grow");
        assert!(s.refresh(&repo));
        assert!(s.status().present, "the marker must withdraw the offer");
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// The instruction in `.claude/CLAUDE.md` counts: the user already did the
    /// thing, in a file Claude Code reads, and must not be asked again.
    #[test]
    fn the_dot_claude_copy_withdraws_the_offer() {
        let repo = dir("dotclaude");
        std::fs::create_dir_all(repo.join(".claude")).expect("mkdir");
        std::fs::write(
            repo.join(".claude/CLAUDE.md"),
            "call get_ticket when MESIMON_TICKET is set",
        )
        .expect("seed");
        let mut s = Sampler::default();
        s.refresh(&repo);
        assert!(s.status().present);
        // The path doctor names is still the root's.
        assert!(s.status().path.ends_with("CLAUDE.md"));
        assert!(!s.status().path.contains(".claude"));
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// A second sample with nothing moved does not re-read the file.
    #[test]
    fn an_unchanged_file_is_not_resampled() {
        let repo = dir("stable");
        std::fs::write(repo.join("CLAUDE.md"), "# Rules\n").expect("seed");
        let mut s = Sampler::default();
        assert!(s.refresh(&repo));
        assert!(!s.refresh(&repo), "nothing moved, so nothing to say");
        assert!(!s.status().present);
        let _ = std::fs::remove_dir_all(&repo);
    }
}
