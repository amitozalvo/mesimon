//! Only the worker owns the serialized sessions cache and the atomic write.
//! While it is busy, the board itself holds the next snapshot. A burst costs
//! at most one in-flight copy, and a reply names the revision it needs.
use super::*;
use std::collections::{HashSet, VecDeque};

type AfterSave = Box<dyn FnOnce(&mut Daemon, Result<(), String>) + Send>;

struct Job {
    started: Option<HashSet<String>>,
    sessions: Option<Vec<SessionRecord>>,
}

pub(super) struct Saves {
    worker: Sender<Option<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
    // A failed or superseded write must include every conversation key
    // learned since the last durable snapshot, even if its seat has gone.
    pub(super) started_revision: u64,
    durable: u64,
    pub(super) requested: u64,
    pub(super) active: Option<u64>,
    pub(super) error: Option<String>,
    waiting: VecDeque<(u64, AfterSave)>,
}

impl Saves {
    pub(super) fn start(paths: Paths, mut written: String, tx: Sender<Msg>) -> Self {
        let (worker, jobs) = channel::<Option<Job>>();
        let thread = std::thread::spawn(move || {
            let gate = std::env::var_os("MESIMON_TEST_SESSION_SAVE_GATE");
            while let Ok(Some(job)) = jobs.recv() {
                let result = job
                    .write(&paths, &mut written, gate.as_deref())
                    .map_err(|e| format!("could not save sessions: {e}"));
                if tx.send(Msg::SessionsSaved(result)).is_err() {
                    break;
                }
            }
        });
        Self {
            worker,
            thread: Some(thread),
            started_revision: 0,
            durable: 0,
            requested: 0,
            active: None,
            error: None,
            waiting: VecDeque::new(),
        }
    }
}

impl Drop for Saves {
    fn drop(&mut self) {
        // Release the state directory only after its last writer has exited.
        let _ = self.worker.send(None);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Job {
    fn write(
        self,
        paths: &Paths,
        written: &mut String,
        gate: Option<&std::ffi::OsStr>,
    ) -> Result<()> {
        if let Some(keys) = self.started {
            crate::started::save(paths, &keys)?;
        }
        let Some(sessions) = self.sessions else { return Ok(()) };
        let text = store::sessions_text(&sessions)?;
        // A fixture holds a changed save before the atomic replacement.
        if let Some(path) = gate.filter(|_| text != *written) {
            if let Ok(mut socket) = UnixStream::connect(path) {
                socket.set_read_timeout(Some(Duration::from_secs(30)))?;
                std::io::Read::read_exact(&mut socket, &mut [0])?;
            }
        }
        store::write_sessions_if_changed(&paths.sessions_file(), text, written)
    }
}

impl Daemon {
    pub(super) fn start_session_save(&mut self) {
        self.session_saves.active = Some(self.session_saves.requested);
        let job = Job {
            started: (self.session_saves.started_revision > self.session_saves.durable)
                .then(|| self.started.clone()),
            sessions: (!self.sessions_barred).then(|| self.board.sessions.to_vec()),
        };
        if self.session_saves.worker.send(Some(job)).is_err() {
            let _ = self.tx.send(Msg::SessionsSaved(Err("session save worker stopped".into())));
        }
    }

    pub(super) fn retry_session_save(&mut self) {
        if self.session_saves.error.is_some() && self.session_saves.active.is_none() {
            self.start_session_save();
        }
    }

    pub(super) fn sessions_saved(&mut self, result: Result<(), String>) {
        let Some(saved) = self.session_saves.active.take() else { return };
        if let Err(message) = &result {
            if self.session_saves.error.as_ref() != Some(message) {
                self.journal.line(message);
            }
        }
        if result.is_ok() {
            self.session_saves.durable = saved;
        }
        self.session_saves.error = result.as_ref().err().cloned();
        if self.session_saves.requested > saved {
            self.start_session_save();
        }
        while self.session_saves.waiting.front().is_some_and(|(revision, _)| *revision <= saved) {
            if let Some((_, finish)) = self.session_saves.waiting.pop_front() {
                finish(self, result.clone());
            }
        }
    }

    pub(super) fn after_session_save(
        &mut self,
        wait: bool,
        finish: impl FnOnce(&mut Daemon, Result<(), String>) + Send + 'static,
    ) {
        if !wait {
            finish(self, Ok(()));
        } else if self.sessions_barred {
            finish(self, Err("sessions.json is protected and could not be saved".into()));
        } else if self.session_saves.active.is_some() {
            self.session_saves.waiting.push_back((self.session_saves.requested, Box::new(finish)));
        } else {
            let result = self.session_saves.error.clone().map_or(Ok(()), Err);
            finish(self, result);
        }
    }

    pub(super) fn session_reply(
        &mut self,
        reply: Sender<ClientReply>,
        response: Response,
        told: Option<(uuid::Uuid, u64)>,
        wait: bool,
    ) {
        self.after_session_save(wait, move |_, result| {
            let (response, told) = match result {
                Ok(()) => (response, told),
                Err(message) => (Response::Err { message }, None),
            };
            let _ = reply.send(ClientReply { response, told, delivered: None });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_save_refuses_its_receipt_and_later_revisions_share_one_retry() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, _, rx) = super::super::queued_person_push_tests::daemon(dir.path());
        let tmp = d.paths.sessions_file().with_extension("tmp");
        std::fs::create_dir(&tmp).unwrap();
        let (outcomes, receipts) = channel();
        for title in ["first", "second", "third"] {
            d.board.sessions[0].title = Some(title.into());
            d.persist_sessions();
            let outcomes = outcomes.clone();
            d.after_session_save(true, move |_, result| outcomes.send((title, result)).unwrap());
        }
        let Msg::SessionsSaved(failed) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("expected failed write")
        };
        assert!(failed.is_err());
        std::fs::remove_dir(&tmp).unwrap();
        d.sessions_saved(failed);
        let (title, result) = receipts.try_recv().unwrap();
        assert_eq!(title, "first");
        assert!(result.is_err());
        assert!(receipts.try_recv().is_err(), "a failed write cannot release later receipts");
        let Msg::SessionsSaved(saved) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
            panic!("expected the latest revision")
        };
        assert!(saved.is_ok());
        d.sessions_saved(saved);
        assert_eq!(receipts.try_recv().unwrap(), ("second", Ok(())));
        assert_eq!(receipts.try_recv().unwrap(), ("third", Ok(())));
        assert!(d.session_saves.active.is_none());
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(d.paths.sessions_file()).unwrap()).unwrap();
        assert_eq!(json["sessions"][0]["title"], "third");
    }

    #[test]
    #[ignore = "prints session save timings; run with --ignored --nocapture"]
    fn time_ordered_session_saves() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, _, rx) = super::super::queued_person_push_tests::daemon(dir.path());
        d.board.sessions.clear();
        store::tests::big_board(&mut d.board, dir.path());
        // Warm the worker's text cache and the conversation-key ledger.
        let mut dispatch = Duration::ZERO;
        let mut durable = Duration::ZERO;
        for n in 0..=20 {
            d.board.sessions[0].title = Some(format!("t{n}"));
            let start = Instant::now();
            d.persist_sessions();
            let handed = start.elapsed();
            let Msg::SessionsSaved(done) = rx.recv_timeout(Duration::from_secs(30)).unwrap() else {
                panic!("expected a session save")
            };
            done.as_ref().unwrap();
            d.sessions_saved(done);
            if n > 0 {
                dispatch += handed;
                durable += start.elapsed();
            }
        }
        println!(
            "one changed: writer {:?}, durable {:?} (20 saves, {} records)",
            dispatch / 20,
            durable / 20,
            d.board.sessions.len()
        );
    }
}
