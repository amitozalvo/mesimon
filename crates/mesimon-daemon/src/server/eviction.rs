//! Column arrival reclamation. The writer owns admission; a worker owns disk I/O.
//!
//! Nothing here forks on the writer (T-857). The tick runs the in-memory
//! checks and reads the merge verdict off the card's own flags, the ones the
//! worktree sampler last landed (`wt_repos`); the two tmux reads the verdict
//! needs — the live panes and their directories — ride the pane sampler and
//! are judged in [`Daemon::absorb_evictions`] as the sample lands. The git
//! reads (an unchanged tip, a clean tree) stay the eviction worker's, off
//! the writer: each leg carries the tip its merge verdict was read at, and
//! the worker refuses a tip that has moved since.
use super::*;

/// How long an asked sample may stay out before the arrival is judged afresh.
const PROBE_GRACE: Duration = Duration::from_secs(10);
/// How soon a refused arrival is judged again.
const RETRY: Duration = Duration::from_secs(2);

impl Daemon {
    /// The arrival still wants its worktree freed: the ticket is on the
    /// board, in the column it arrived in, a worktree ticket, and the column
    /// still asks.
    fn eviction_eligible(&self, id: ulid::Ulid) -> bool {
        let Some(column) = self.column_evictions.get(&id).map(|e| e.column.as_str()) else {
            return false;
        };
        self.board.ticket(id).is_some_and(|t| {
            !t.is_archived()
                && t.column == column
                && t.workspace_strategy() == WorkspaceStrategy::Worktree
                && self.board.column(column).is_some_and(|c| c.settings.free_worktree)
        }) && self.worktrees.contains_key(&id)
    }

    /// The session names whose pane would hold the worktree: the ticket's
    /// own, and any record whose cwd is under it.
    fn eviction_holders(&self, id: ulid::Ulid, b: &worktree::Binding) -> Vec<&SessionRecord> {
        self.board
            .sessions
            .iter()
            .filter(|s| s.ticket == id || std::path::Path::new(&s.cwd).starts_with(&b.path))
            .collect()
    }

    /// The in-memory admission, forking nothing: the binding and its legs,
    /// each leg's `branch_oid` stamped with the tip its merge verdict was
    /// read at, or the refusal in words.
    fn eviction_admission(
        &self,
        id: ulid::Ulid,
    ) -> Result<(worktree::Binding, Vec<worktree::Leg>), String> {
        let by = Principal::Automation { rule: "free_worktree".into() };
        let b = self.worktrees.get(&id).cloned().ok_or_else(|| "worktree is gone".to_string())?;
        if let Decision::Deny { reason } = authorize(&by, &Action::Mutate, &Resource::Ticket { id })
        {
            return Err(format!("denied: {reason}"));
        }
        if self.worktrees_barred {
            return Err(self.barred_message("worktrees"));
        }
        if b.status != BindingStatus::Attached
            || self.pending_teardown.iter().any(|t| t.ticket == id)
        {
            return Err("worktree is busy".into());
        }
        if self
            .eviction_holders(id, &b)
            .iter()
            .any(|s| s.holds_process() || self.reaping.contains_key(&s.sid16()))
            || self.pending_spawns.iter().any(|s| s.ticket == id)
            || self.pending_resumes.iter().any(|s| s.ticket == id)
        {
            return Err("waiting for agents and their reaper".into());
        }
        // The merge verdict is the card's: the flags the sampler last landed
        // for every leg. `ticket_merged` would fork git here; the gates may,
        // on a keypress, a tick stage may not (T-857). A merge made since
        // the last sample is a bucket's wait, never a wrong answer, and the
        // worker refuses a tip moved since the verdict.
        if self.open_pr(id).is_some() {
            return Err("branch is not merged".into());
        }
        let mut legs = self.binding_legs(&b);
        let flags = self.wt_repos.get(&id).ok_or_else(|| "merge state not read yet".to_string())?;
        for leg in &mut legs {
            let f = flags
                .iter()
                .find(|f| f.name == leg.name)
                .filter(|f| !f.unchecked && !f.tip.is_empty())
                .ok_or_else(|| "merge state not read yet".to_string())?;
            if !f.merged {
                return Err("branch is not merged".into());
            }
            leg.branch_oid = f.tip.clone();
        }
        Ok((b, legs))
    }

    /// Record the verdict on the arrival's card: true when the card changed.
    fn eviction_verdict(&mut self, id: ulid::Ulid, refusal: Option<String>, now: Instant) -> bool {
        let Some(e) = self.column_evictions.get_mut(&id) else { return false };
        e.due = now + RETRY;
        e.probing = false;
        let refusal = refusal.map(|why| format!("worktree kept: {why}"));
        let changed = e.refusal != refusal;
        e.refusal = refusal;
        changed
    }

    /// Drop an arrival that no longer wants freeing: true when a refusal it
    /// showed goes with it.
    fn eviction_dropped(&mut self, id: ulid::Ulid) -> bool {
        self.column_evictions.remove(&id).is_some_and(|e| e.refusal.is_some())
    }

    /// The tick's half: in-memory admission, then the pane sample the
    /// verdict needs, asked of the sampler.
    pub(super) fn process_column_evictions(&mut self) {
        let now = Instant::now();
        let candidates: Vec<_> = self
            .column_evictions
            .iter()
            .filter(|(id, e)| e.due <= now && !self.tearing_down.contains(id))
            .map(|(id, _)| *id)
            .collect();
        for id in candidates {
            if !self.eviction_eligible(id) {
                if self.eviction_dropped(id) {
                    self.broadcast();
                }
                continue;
            }
            if self.worktrees.get(&id).is_some_and(|b| b.status == BindingStatus::Evicted) {
                // A cleanup failure remains visible until the next arrival.
                continue;
            }
            match self.eviction_admission(id) {
                Err(why) => {
                    if self.eviction_verdict(id, Some(why), now) {
                        self.broadcast();
                    }
                }
                Ok(_) => {
                    // Sleeping is a record state, not proof that its live
                    // pane is gone: the sample says.
                    self.pane_due.facts = true;
                    self.pane_due.directories = true;
                    let e = self.column_evictions.get_mut(&id).expect("arrival exists");
                    e.probing = true;
                    e.due = now + PROBE_GRACE;
                }
            }
        }
    }

    /// The sample's half (T-857): the arrivals whose sample this is, judged
    /// against the panes and directories it read, and admitted to the
    /// worker. True when a card changed.
    pub(super) fn absorb_evictions(
        &mut self,
        facts: Option<&[mesimon_backend_tmux::PaneFacts]>,
        directories: Option<&Result<Vec<std::path::PathBuf>, ()>>,
    ) -> bool {
        let now = Instant::now();
        let probing: Vec<_> =
            self.column_evictions.iter().filter(|(_, e)| e.probing).map(|(id, _)| *id).collect();
        let mut changed = false;
        for id in probing {
            if self.tearing_down.contains(&id) {
                continue;
            }
            if !self.eviction_eligible(id) {
                changed |= self.eviction_dropped(id);
                continue;
            }
            // Admission again, as the board stands: a record that moved on
            // while the read was out drops its answer.
            let verdict = self.eviction_admission(id).and_then(|(b, legs)| {
                let Some(facts) = facts else {
                    return Err("cannot verify panes are gone".to_string());
                };
                let holders: Vec<String> =
                    self.eviction_holders(id, &b).iter().map(|s| s.sid16()).collect();
                if facts.iter().any(|p| {
                    !p.pane_dead
                        && (p.session_name == terminal_name(Some(id))
                            || holders.contains(&p.session_name))
                }) {
                    return Err("a live pane holds the worktree".to_string());
                }
                match directories {
                    Some(Ok(paths)) if !paths.iter().any(|p| p.starts_with(&b.path)) => {}
                    Some(Ok(_)) => return Err("a live pane holds the worktree".to_string()),
                    _ => return Err("cannot verify pane directories".to_string()),
                }
                Ok((b, legs))
            });
            match verdict {
                Err(why) => changed |= self.eviction_verdict(id, Some(why), now),
                Ok((b, legs)) => {
                    changed |= self.eviction_verdict(id, None, now);
                    // All spawn roads, wakes and terminal opens now refuse or
                    // park until completion. No worker mutates board state,
                    // nor kills a live pane.
                    self.tearing_down.insert(id);
                    let root = self.paths.worktrees_root();
                    let repo = self.paths.repo_root.clone();
                    let tx = self.tx.clone();
                    std::thread::spawn(move || {
                        let outcome = worktree::evict(&repo, &root, id, &b, &legs);
                        let _ = tx.send(Msg::ColumnEvicted { ticket: id, outcome });
                    });
                }
            }
        }
        changed
    }

    pub(super) fn on_column_evicted(&mut self, ticket: ulid::Ulid, outcome: worktree::Eviction) {
        self.tearing_down.remove(&ticket);
        if outcome.evicted {
            if let Some(b) = self.worktrees.get_mut(&ticket) {
                b.status = BindingStatus::Evicted;
                b.locked = false;
            }
            self.feed.board("automation", "worktree_freed", Some(ticket));
            self.persist_worktrees();
        }
        if let Some(message) = outcome.trouble {
            self.feed.board_outcome(
                "automation",
                if outcome.evicted { "worktree_cleanup_failed" } else { "worktree_free_refused" },
                Some(ticket),
                &message,
            );
            if let Some(e) = self.column_evictions.get_mut(&ticket) {
                e.refusal = Some(if outcome.evicted {
                    message
                } else {
                    format!("worktree kept: {message}")
                });
                e.due = Instant::now() + Duration::from_secs(10);
            }
        } else {
            self.column_evictions.remove(&ticket);
        }
        // A wake accepted during removal gets the existing branch rebuilt before
        // adapter preparation or pane spawn. On refusal, replay against the tree
        // still standing instead of provisioning over it.
        if self.pending_spawns.iter().any(|s| s.ticket == ticket)
            || self.pending_resumes.iter().any(|r| r.ticket == ticket)
        {
            if outcome.evicted {
                self.queue_provision(ticket);
            } else {
                self.replay_worktree_waiters(ticket);
            }
        }
        self.queue_worktree_flags();
        self.broadcast();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bed(dir: &std::path::Path) -> (Daemon, ulid::Ulid, std::sync::mpsc::Receiver<Msg>) {
        let (d, id, rx) = bed_with_rx(dir);
        (d, id, rx)
    }

    fn bed_with_rx(dir: &std::path::Path) -> (Daemon, ulid::Ulid, std::sync::mpsc::Receiver<Msg>) {
        let (mut d, _, rx) = super::super::queued_person_push_tests::daemon(dir);
        crate::testrepo::init(&d.paths.repo_root, "file", "base");
        let id = match d.create_ticket(
            &Principal::Local,
            "TODO".into(),
            "eviction".into(),
            Some(WorkspaceStrategy::Worktree),
            None,
        ) {
            Response::Created { id } => id,
            other => panic!("{other:?}"),
        };
        let b = worktree::provision(
            &d.paths.repo_root,
            &d.paths.worktrees_root(),
            id,
            "T-1",
            "eviction",
            false,
        )
        .unwrap();
        d.board.sessions[0].ticket = id;
        d.board.sessions[0].cwd = b.path.display().to_string();
        d.board.sessions[0].set_state(SessionState::Sleeping);
        d.worktrees.insert(id, b);
        d.place_ticket(id, "DONE", Position::Top, &Principal::Local, None, "test").unwrap();
        (d, id, rx)
    }

    /// The card's flags for the bed's one root leg: merged or not, at `tip`.
    fn flags(d: &mut Daemon, id: ulid::Ulid, merged: bool, tip: &str) {
        d.wt_repos.insert(
            id,
            vec![worktree::RepoFlags {
                name: String::new(),
                merged,
                tip: tip.into(),
                ..Default::default()
            }],
        );
    }

    fn refusal(d: &Daemon, id: ulid::Ulid) -> String {
        d.column_evictions[&id].refusal.clone().unwrap_or_default()
    }

    /// T-857: the tick forks nothing. The merge verdict is the card's flags,
    /// the pane reads ride the sampler, and the worker gets the tip the
    /// verdict was read at.
    #[test]
    fn admission_reads_the_cards_flags_and_judges_the_panes_off_the_sample() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, id, rx) = bed(dir.path());
        let sid = d.board.sessions[0].sid16();
        let path = d.worktrees[&id].path.clone();
        let judge = |d: &mut Daemon| {
            d.column_evictions.get_mut(&id).unwrap().due = Instant::now();
            d.process_column_evictions();
        };
        judge(&mut d);
        assert_eq!(refusal(&d, id), "worktree kept: merge state not read yet");
        assert!(!d.pane_due.directories && !d.column_evictions[&id].probing);
        flags(&mut d, id, false, "abc");
        judge(&mut d);
        assert_eq!(refusal(&d, id), "worktree kept: branch is not merged");
        flags(&mut d, id, true, "abc");
        judge(&mut d);
        assert!(d.column_evictions[&id].probing, "admitted: the sample decides");
        assert!(d.pane_due.directories && d.pane_due.facts);
        assert!(!d.tearing_down.contains(&id), "nothing admitted before the sample");
        // The sample: a live pane of the holder, then one in the directory,
        // then reads that did not answer. Each keeps the tree.
        let live = |name: &str| mesimon_backend_tmux::PaneFacts {
            session_name: name.into(),
            ..Default::default()
        };
        let empty: Result<Vec<std::path::PathBuf>, ()> = Ok(Vec::new());
        for (facts, dirs, why) in [
            (Some(vec![live(&sid)]), Some(empty.clone()), "a live pane holds the worktree"),
            (Some(vec![]), Some(Ok(vec![path.join("sub")])), "a live pane holds the worktree"),
            (Some(vec![]), Some(Err(())), "cannot verify pane directories"),
            (None, Some(empty.clone()), "cannot verify panes are gone"),
        ] {
            d.column_evictions.get_mut(&id).unwrap().probing = true;
            let before = refusal(&d, id);
            let changed = d.absorb_evictions(facts.as_deref(), dirs.as_ref());
            assert_eq!(refusal(&d, id), format!("worktree kept: {why}"));
            assert_eq!(changed, before != refusal(&d, id), "a card changes once per new word");
            assert!(!d.column_evictions[&id].probing && !d.tearing_down.contains(&id));
        }
        // A clean sample admits the worker, with the verdict's tip on the leg:
        // the real branch is elsewhere, so the worker refuses the moved tip.
        d.column_evictions.get_mut(&id).unwrap().probing = true;
        d.absorb_evictions(Some(&[]), Some(&empty));
        assert!(d.tearing_down.contains(&id));
        let outcome = loop {
            match rx.recv_timeout(Duration::from_secs(20)).expect("the worker reports") {
                Msg::ColumnEvicted { ticket, outcome } if ticket == id => break outcome,
                _ => {}
            }
        };
        assert!(!outcome.evicted);
        assert!(outcome.trouble.as_deref().unwrap_or_default().contains("branch changed"));
        assert!(path.join("file").exists());
    }

    /// A sample out past its grace does not hold the arrival: the tick
    /// judges it afresh and asks again.
    #[test]
    fn an_unanswered_sample_is_asked_again_after_its_grace() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, id, _rx) = bed(dir.path());
        flags(&mut d, id, true, "abc");
        d.column_evictions.get_mut(&id).unwrap().due = Instant::now();
        d.process_column_evictions();
        assert!(d.column_evictions[&id].probing);
        d.pane_due.directories = false;
        d.process_column_evictions();
        assert!(!d.pane_due.directories, "within the grace the ask stands");
        d.column_evictions.get_mut(&id).unwrap().due = Instant::now();
        d.process_column_evictions();
        assert!(d.pane_due.directories, "past it the arrival is asked again");
    }

    #[test]
    fn column_eviction_waits_for_barred_bindings_process_ownership_and_the_reaper() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, id, _rx) = bed(dir.path());
        for guard in ["barred", "process", "reaper", "pending spawn", "pending resume"] {
            d.worktrees_barred = guard == "barred";
            d.board.sessions[0].codex_stopping = guard == "process";
            let sid = d.board.sessions[0].sid16();
            let session = d.board.sessions[0].id;
            d.reaping.clear();
            d.pending_spawns.clear();
            d.pending_resumes.clear();
            if guard == "reaper" {
                d.reaping.insert(sid, Instant::now() + Duration::from_secs(5));
            }
            if guard == "pending spawn" {
                d.pending_spawns.push(PendingSpawn {
                    ticket: id,
                    kind: SessionKind::Bash,
                    submit_prompt: false,
                    prompt: None,
                    started_by: None,
                    plan: false,
                });
            }
            if guard == "pending resume" {
                d.pending_resumes.push(PendingResume {
                    ticket: id,
                    session,
                    confirm: false,
                    prompt: None,
                    brief: false,
                    plan: false,
                    ack: Ack::PROMPT,
                    ask: None,
                });
            }
            let e = d.column_evictions.get_mut(&id).unwrap();
            e.due = Instant::now();
            e.refusal = None;
            let version = d.board_version;
            d.process_column_evictions();
            assert!(d.board_version > version, "the card must receive the refusal");
            let version = d.board_version;
            d.column_evictions.get_mut(&id).unwrap().due = Instant::now();
            d.process_column_evictions();
            assert_eq!(d.board_version, version, "an unchanged refusal is not a new frame");
            assert!(!d.tearing_down.contains(&id), "{guard}");
            assert!(d.column_evictions[&id].refusal.is_some(), "{guard}");
            assert!(d.worktrees[&id].path.join("file").exists(), "{guard}");
        }
        // Turning the setting off clears an already visible refusal as well as
        // cancelling the pending removal, even without another session event.
        d.board.columns.iter_mut().find(|c| c.name == "DONE").unwrap().settings.free_worktree =
            false;
        d.column_evictions.get_mut(&id).unwrap().due = Instant::now();
        let version = d.board_version;
        d.process_column_evictions();
        assert!(!d.column_evictions.contains_key(&id));
        assert!(d.board_version > version);
        assert!(d.worktrees[&id].path.join("file").exists());
    }

    #[test]
    fn teardown_seals_fresh_spawn_resume_and_terminal_even_while_the_directory_exists() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, id, _rx) = bed(dir.path());
        let sid = d.board.sessions[0].id;
        assert!(d.worktrees[&id].path.is_dir());
        d.tearing_down.insert(id);
        assert!(matches!(
            d.spawn_session(id, SessionKind::Bash, false, None, None, false),
            Response::Provisioning
        ));
        assert_eq!(d.pending_spawns.len(), 1);
        d.pending_spawns.clear();
        for kind in [SessionKind::Claude, SessionKind::Codex] {
            d.board.sessions[0].kind = kind;
            d.pending_resumes.clear();
            assert!(
                matches!(d.resume_session_in(sid, true, false), Response::Provisioning),
                "{kind:?}"
            );
            assert_eq!(d.pending_resumes.len(), 1);
        }
        let (socket, _peer) = UnixStream::pair().unwrap();
        let by = Arc::new(Mutex::new(socket));
        assert!(matches!(d.open_terminal(Some(id), &by), Response::Err { .. }));
        // Completion must replay even a failed eviction, rather than leave the
        // accepted wake stranded. The replay below fails the archive gate; no
        // external process is needed to prove the waiter was consumed.
        d.board.ticket_mut(id).unwrap().archived = Some(mesimon_core::board::Archived {
            at: now_iso(),
            by: "local".into(),
            until: None,
            needs_you: false,
        });
        d.on_column_evicted(
            id,
            worktree::Eviction { evicted: false, trouble: Some("dirty".into()) },
        );
        assert!(!d.tearing_down.contains(&id));
        assert!(d.pending_resumes.is_empty());
        assert!(d.worktrees[&id].path.join("file").exists());
    }
}
