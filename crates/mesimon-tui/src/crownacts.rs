//! The crown's acts as the board's daemon keeps them (T-1162), read for
//! the ticket page's conversation and the `{ }` dialog: the
//! `crown-acts.jsonl` beside the board's other state, read straight from
//! the file as a transcript is (observe-tier, no daemon round-trip). The
//! UI thread only stats it, at most every `FRESH`; a read runs on a thread
//! of its own and lands at the next poll.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use mesimon_core::crownact::CrownAct;

/// How often the file is looked at while a conversation is on show.
const FRESH: Duration = Duration::from_millis(400);

type Read = ((u64, u64), Vec<CrownAct>);

pub(crate) struct Acts {
    path: Option<PathBuf>,
    acts: Rc<Vec<CrownAct>>,
    stamp: (u64, u64),
    checked: Option<Instant>,
    reading: Option<Receiver<Read>>,
    /// Moves each time `acts` does: what a conversation compares.
    pub version: u64,
}

impl Acts {
    pub(crate) fn new(repo_root: &std::path::Path) -> Self {
        let path = mesimon_daemon::Paths::for_repo(repo_root).ok().map(|p| p.crown_acts_file());
        Acts {
            path,
            acts: Rc::new(Vec::new()),
            stamp: (0, 0),
            checked: None,
            reading: None,
            version: 0,
        }
    }

    pub(crate) fn all(&self) -> Rc<Vec<CrownAct>> {
        Rc::clone(&self.acts)
    }

    /// Land a read that finished, and start another when the file moved.
    /// True when the acts changed.
    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.reading {
            match rx.try_recv() {
                Ok((stamp, acts)) => {
                    self.reading = None;
                    self.stamp = stamp;
                    if *self.acts != acts {
                        self.acts = Rc::new(acts);
                        self.version += 1;
                        changed = true;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.reading = None,
            }
        }
        let Some(path) = self.path.clone() else { return changed };
        if self.checked.is_some_and(|at| at.elapsed() < FRESH) {
            return changed;
        }
        self.checked = Some(Instant::now());
        let stamp = mesimon_daemon::crownacts::stamp(&path);
        if stamp == self.stamp {
            return changed;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let acts = mesimon_daemon::crownacts::read(&path);
            let _ = tx.send((stamp, acts));
        });
        self.reading = Some(rx);
        changed
    }

    /// For a test: the acts as if the file had held them.
    #[cfg(test)]
    pub(crate) fn set(&mut self, acts: Vec<CrownAct>) {
        self.path = None;
        self.acts = Rc::new(acts);
        self.version += 1;
    }
}
