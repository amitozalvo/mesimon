//! Slow tmux lifecycle work must not occupy the board's writer (T-1314).
//! Reservations stay on the writer until each worker's completion lands.
use super::*;

#[derive(Default)]
pub(super) struct Jobs {
    pub reaping: HashMap<String, Instant>,
    pub resumes: std::collections::HashSet<uuid::Uuid>,
    pub shell_wakes: std::collections::HashSet<uuid::Uuid>,
    pub opening: Option<Opening>,
    pub park_terminal: bool,
    pub status_writing: bool,
}

pub(super) struct Opening {
    ticket: Option<ulid::Ulid>,
    by: Weak<Mutex<UnixStream>>,
    peer: UnixStream,
    pub reply: Option<Sender<ClientReply>>,
}

impl Daemon {
    pub(super) fn sweep_reaping(&mut self) {
        if !self.responsive.reaping.is_empty() {
            return;
        }
        let now = Instant::now();
        let due: HashMap<String, Instant> = self
            .reaping
            .iter()
            .filter(|(sid, at)| {
                **at <= now
                    && !self.board.sessions.iter().any(|s| s.sid16() == **sid && s.codex_stopping)
            })
            .map(|(sid, at)| (sid.clone(), *at))
            .collect();
        if due.is_empty() {
            return;
        }
        self.responsive.reaping = due.clone();
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let began = Instant::now();
            let mut panes = None;
            let done = due
                .into_iter()
                .map(|(sid, generation)| {
                    let removed = backend.kill_session(&sid).is_ok()
                        || absence_proven(
                            panes.get_or_insert_with(|| backend.panes()),
                            &backend,
                            |panes| !panes.iter().any(|p| p.session_name == sid),
                        );
                    (sid, generation, removed)
                })
                .collect();
            let _ = tx.send(Msg::Reaped(done, began.elapsed()));
        });
    }

    pub(super) fn reaped(&mut self, done: Vec<(String, Instant, bool)>, took: Duration) {
        if took >= crate::journal::SLOW_TURN {
            self.journal.line(&format!(
                "slow reaper: {} sessions took {} ms, off the writer thread",
                done.len(),
                took.as_millis()
            ));
        }
        for (sid, generation, removed) in done {
            self.responsive.reaping.remove(&sid);
            if self.reaping.get(&sid) == Some(&generation) {
                if removed {
                    self.reaping.remove(&sid);
                } else {
                    self.reaping.insert(sid.clone(), Instant::now() + REAP_GRACE);
                }
            }
        }
        // Only wakes parked behind this worker replay here. A worktree still
        // provisioning has its own completion and must keep its own waiters.
        let ids = std::mem::take(&mut self.responsive.resumes);
        let (resumes, rest) =
            self.pending_resumes.drain(..).partition(|r| ids.contains(&r.session));
        self.pending_resumes = rest;
        self.replay_resumes(resumes);
        let wakes = std::mem::take(&mut self.responsive.shell_wakes);
        for id in wakes {
            self.wake_session(id);
        }
    }

    pub(super) fn terminal_opening(&self, ticket: ulid::Ulid) -> bool {
        self.responsive.opening.as_ref().is_some_and(|o| o.ticket == Some(ticket))
    }

    pub(super) fn begin_terminal_open(
        &mut self,
        ticket: Option<ulid::Ulid>,
        by: &Arc<Mutex<UnixStream>>,
        cwd: std::path::PathBuf,
        vars: Vec<(String, String)>,
    ) -> Response {
        let peer = match by.lock().ok().and_then(|s| s.try_clone().ok()) {
            Some(peer) => peer,
            None => return Response::Err { message: "terminal client disconnected".into() },
        };
        self.focus = Some(FocusHold { what: Focus::Terminal { ticket }, by: Arc::downgrade(by) });
        self.responsive.opening =
            Some(Opening { ticket, by: Arc::downgrade(by), peer, reply: None });
        self.responsive.park_terminal = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let launch = self.launch(std::slice::from_ref(&self.shell), &vars);
        std::thread::spawn(move || {
            let name = terminal_name(ticket);
            let mut timings = Vec::new();
            let result = (|| -> Result<(), String> {
                let began = Instant::now();
                // An inspection failure is not permission to destroy a live shell.
                let snapshot = backend.snapshot().map_err(|e| e.to_string());
                timings.push(("inspect", began.elapsed()));
                let alive = snapshot?.iter().any(|p| p.session_name == name && !p.pane_dead);
                if !alive {
                    let began = Instant::now();
                    let _ = backend.kill_session(&name);
                    timings.push(("remove old pane", began.elapsed()));
                    let began = Instant::now();
                    let spawned = backend.spawn(&name, &cwd, &launch).map_err(|e| e.to_string());
                    timings.push(("spawn", began.elapsed()));
                    spawned?;
                }
                Ok(())
            })();
            let _ = tx.send(Msg::TerminalOpened(result, timings));
        });
        Response::Ok // replaced by the parked reply in the writer loop
    }

    pub(super) fn terminal_opened(
        &mut self,
        result: Result<(), String>,
        timings: Vec<(&'static str, Duration)>,
    ) {
        let Some(opening) = self.responsive.opening.take() else { return };
        for (step, took) in timings {
            if took >= crate::journal::SLOW_TURN {
                self.journal.line(&format!(
                    "slow terminal open: {step} took {} ms, off the writer thread",
                    took.as_millis()
                ));
            }
        }
        let owns = self.focus.as_ref().is_some_and(|f| {
            f.by.ptr_eq(&opening.by) && f.what == (Focus::Terminal { ticket: opening.ticket })
        });
        let connected = !mesophon::permission_peer_closed(&opening.peer);
        let response = match result {
            Ok(()) => {
                self.terminals.entry(opening.ticket).or_insert(None);
                if owns && connected {
                    self.focus_label = "terminal".into();
                    self.refresh_status_line();
                    Response::Attach {
                        argv: self.backend.attach_argv(&terminal_name(opening.ticket)),
                    }
                } else {
                    Response::Err { message: "terminal opening cancelled".into() }
                }
            }
            Err(message) => Response::Err { message: format!("terminal open failed: {message}") },
        };
        if owns && matches!(response, Response::Err { .. }) {
            self.focus = None;
        }
        if let Some(reply) = opening.reply {
            let _ = reply.send(ClientReply { response, delivered: None, told: None });
        }
        self.broadcast();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_terminal_open_holds_teardown_until_completion() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, _, _rx) = queued_person_push_tests::daemon(dir.path());
        let ticket = ulid::Ulid(1);
        let (peer, _client) = UnixStream::pair().unwrap();
        let writer = Arc::new(Mutex::new(peer.try_clone().unwrap()));
        d.responsive.opening =
            Some(Opening { ticket: Some(ticket), by: Arc::downgrade(&writer), peer, reply: None });
        d.pending_teardown.push(Teardown { ticket, why: TeardownWhy::Archived, sids: Vec::new() });
        d.process_teardowns();
        assert_eq!(d.pending_teardown.len(), 1, "opening still owns its cwd");
        d.responsive.opening = None;
        d.process_teardowns();
        assert!(d.pending_teardown.is_empty(), "completion releases the queued teardown");
    }

    #[test]
    fn a_reaper_result_cannot_release_a_newer_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, sid, _rx) = queued_person_push_tests::daemon(dir.path());
        let old = Instant::now();
        let newer = old + REAP_GRACE;
        d.responsive.reaping.insert(sid.clone(), old);
        d.reaping.insert(sid.clone(), newer);
        d.reaped(vec![(sid.clone(), old, true)], Duration::ZERO);
        assert_eq!(d.reaping.get(&sid), Some(&newer));
        assert!(d.responsive.reaping.is_empty());
    }
}
