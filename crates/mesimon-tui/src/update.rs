//! New-binary watch: a dev rebuild and a production upgrade are the same
//! signal — the executable at our own path gains a newer mtime and holds it
//! for one full check interval (the hold is the debounce: a binary still
//! being linked keeps moving). Detection only; the reload itself is the
//! user's U key, never automatic.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const CHECK_EVERY: Duration = Duration::from_secs(2);

pub struct UpdateWatch {
    exe: Option<PathBuf>,
    start: Option<SystemTime>,
    /// A changed mtime seen once — must repeat on the next check to count.
    candidate: Option<SystemTime>,
    last_check: Instant,
    ready: bool,
}

impl UpdateWatch {
    pub fn new() -> Self {
        let exe = std::env::current_exe().ok();
        let start = exe.as_deref().and_then(mtime);
        Self { exe, start, candidate: None, last_check: Instant::now(), ready: false }
    }

    /// Rate-limited poll; true exactly once, when a new binary has settled.
    pub fn tick(&mut self) -> bool {
        if self.ready || self.start.is_none() || self.last_check.elapsed() < CHECK_EVERY {
            return false;
        }
        self.last_check = Instant::now();
        let Some(m) = self.exe.as_deref().and_then(mtime) else {
            return false;
        };
        if Some(m) == self.start {
            self.candidate = None;
        } else if self.candidate == Some(m) {
            self.ready = true;
            return true;
        } else {
            self.candidate = Some(m);
        }
        false
    }

    pub fn ready(&self) -> bool {
        self.ready
    }

    #[cfg(test)]
    pub(crate) fn force_ready(&mut self) {
        self.ready = true;
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok().and_then(|m| m.modified().ok())
}
