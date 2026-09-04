//! Sampling and writing the repo's own `CLAUDE.md` (T-217).
//!
//! `mesimon_core::claudemd` holds the text and the arithmetic; this is the two
//! pieces of I/O around it — "does the file already say it" and "append it" —
//! and the care that the second one needs.
//!
//! `<repo>/CLAUDE.md` is the ONLY file mesimon writes that the user tracks in
//! git, which is why nothing here runs without a keystroke behind it: the TUI
//! shows the exact bytes in a dialog first, and `Command::ClaudeMd` is the
//! answer to that dialog. `paths.rs::ensure_excluded` is the shape the append
//! copies — read, look for the marker, add, write — since it is mesimon's only
//! other write to the repo root.

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
    stamps: Vec<Option<(u64, u128)>>,
}

/// The files a marker may live in: the repo root's `CLAUDE.md`, which is where
/// the offer writes, and `.claude/CLAUDE.md`, which Claude Code reads just as
/// happily. A user who put the instruction in the second one has already done
/// the thing the offer asks for and must not be nagged for it.
fn paths(repo: &Path) -> [PathBuf; 2] {
    [repo.join("CLAUDE.md"), repo.join(".claude").join("CLAUDE.md")]
}

fn stamp(path: &Path) -> Option<(u64, u128)> {
    let meta = std::fs::metadata(path).ok()?;
    let ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
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
        let stamps: Vec<Option<(u64, u128)>> = files.iter().map(|p| stamp(p)).collect();
        if stamps == self.stamps && !self.status.path.is_empty() {
            return false;
        }
        self.stamps = stamps;
        let present = files
            .iter()
            .any(|p| std::fs::read_to_string(p).is_ok_and(|b| claudemd::has_marker(&b)));
        let next = ClaudeMdStatus {
            path: files[0].display().to_string(),
            exists: files[0].is_file(),
            present,
        };
        let changed = next != self.status;
        self.status = next;
        changed
    }

    /// Forget the stamps, so the next `refresh` reads whatever the state of
    /// the file is. Called after our own write — the write moves the mtime,
    /// but a same-millisecond write of the same length would not.
    pub fn invalidate(&mut self) {
        self.stamps.clear();
    }
}

/// Append the snippet to `<repo>/CLAUDE.md`, creating the file if it is not
/// there. `Ok(path)` names what was written; a file that already carries the
/// marker is left alone and still reports Ok, since the caller's request is
/// satisfied either way.
///
/// Two things are deliberate:
///
/// - **The symlink is followed, not replaced.** A CLAUDE.md symlinked into a
///   dotfiles repo is ordinary, and a temp-and-rename would swap the LINK for
///   a regular file — losing the user's arrangement and writing somewhere they
///   did not mean. `canonicalize` resolves it first, so the write lands on the
///   real file and the link survives.
/// - **It is still atomic.** Truncating the user's own tracked file with a
///   plain `write` and then crashing would cost them work that is not
///   mesimon's to lose, so the resolved path is written through the store's
///   temp+fsync+rename.
pub fn apply(repo: &Path) -> Result<PathBuf, String> {
    let target = repo.join("CLAUDE.md");
    let body = match std::fs::read_to_string(&target) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("could not read {}: {e}", target.display())),
    };
    if claudemd::has_marker(&body) {
        return Ok(target);
    }
    // Resolve before writing; a missing file has nothing to resolve.
    let real = std::fs::canonicalize(&target).unwrap_or_else(|_| target.clone());
    if real.exists() && !real.is_file() {
        return Err(format!("{} is not a regular file", real.display()));
    }
    crate::store::write_atomic(&real, &claudemd::appended(&body), 0o644)
        .map_err(|e| format!("could not write {}: {e}", real.display()))?;
    Ok(target)
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

    /// The offer stands on a repo with no CLAUDE.md, and applying creates it.
    #[test]
    fn a_repo_without_the_file_is_offered_and_gets_one() {
        let repo = dir("fresh");
        let mut s = Sampler::default();
        assert!(s.refresh(&repo), "the first sample is always news");
        assert!(!s.status().present);
        assert!(!s.status().exists);

        let written = apply(&repo).expect("apply");
        assert_eq!(written, repo.join("CLAUDE.md"));
        assert_eq!(std::fs::read_to_string(&written).expect("read"), claudemd::SNIPPET);

        s.invalidate();
        assert!(s.refresh(&repo));
        assert!(s.status().present, "the write must withdraw the offer");
        assert!(s.status().exists);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// Applying twice writes once: the marker the first pass left is what the
    /// second one refuses on.
    #[test]
    fn applying_twice_writes_once() {
        let repo = dir("twice");
        std::fs::write(repo.join("CLAUDE.md"), "# Rules\n").expect("seed");
        apply(&repo).expect("first");
        let once = std::fs::read_to_string(repo.join("CLAUDE.md")).expect("read");
        apply(&repo).expect("second");
        let twice = std::fs::read_to_string(repo.join("CLAUDE.md")).expect("read");
        assert_eq!(once, twice);
        assert_eq!(twice.matches("## mesimon").count(), 1, "{twice}");
        assert!(twice.starts_with("# Rules\n\n"), "{twice}");
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
        // The path the dialog would name is still the root's, which is where
        // an apply would write — the offer is simply not being made.
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

    /// A symlinked CLAUDE.md is followed: the link survives and the target is
    /// what grew.
    #[test]
    fn a_symlinked_file_is_followed_not_replaced() {
        let repo = dir("symlink");
        let real = repo.join("dotfiles-claude.md");
        std::fs::write(&real, "# Shared\n").expect("seed");
        std::os::unix::fs::symlink(&real, repo.join("CLAUDE.md")).expect("symlink");

        apply(&repo).expect("apply");
        let meta = std::fs::symlink_metadata(repo.join("CLAUDE.md")).expect("stat");
        assert!(meta.file_type().is_symlink(), "the link must survive the write");
        let body = std::fs::read_to_string(&real).expect("read target");
        assert!(body.starts_with("# Shared\n\n## mesimon"), "{body}");
        let _ = std::fs::remove_dir_all(&repo);
    }
}
