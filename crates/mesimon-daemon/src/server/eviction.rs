//! Column arrival reclamation. The writer owns admission; a worker owns disk I/O.
use super::*;

impl Daemon {
    pub(super) fn process_column_evictions(&mut self) {
        let now = Instant::now();
        let candidates: Vec<_> = self
            .column_evictions
            .iter()
            .filter(|(id, e)| e.due <= now && !self.tearing_down.contains(id))
            .map(|(id, _)| *id)
            .collect();
        for id in candidates {
            let column = &self.column_evictions[&id].column;
            let eligible = self.board.ticket(id).is_some_and(|t| {
                !t.is_archived()
                    && t.column == *column
                    && t.workspace_strategy() == WorkspaceStrategy::Worktree
                    && self.board.column(column).is_some_and(|c| c.settings.free_worktree)
            });
            let Some(b) = self.worktrees.get(&id).cloned().filter(|_| eligible) else {
                if self.column_evictions.remove(&id).is_some_and(|e| e.refusal.is_some()) {
                    self.broadcast();
                }
                continue;
            };
            if b.status == BindingStatus::Evicted {
                // A cleanup failure remains visible until the next arrival.
                continue;
            }
            let by = Principal::Automation { rule: "free_worktree".into() };
            let mut legs = self.binding_legs(&b);
            let refusal = (|| {
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id })
                {
                    return Some(format!("denied: {reason}"));
                }
                if self.worktrees_barred {
                    return Some(self.barred_message("worktrees"));
                }
                if b.status != BindingStatus::Attached
                    || self.pending_teardown.iter().any(|t| t.ticket == id)
                {
                    return Some("worktree is busy".into());
                }
                let holders: Vec<_> = self
                    .board
                    .sessions
                    .iter()
                    .filter(|s| s.ticket == id || std::path::Path::new(&s.cwd).starts_with(&b.path))
                    .collect();
                if holders
                    .iter()
                    .any(|s| s.holds_process() || self.reaping.contains_key(&s.sid16()))
                    || self.pending_spawns.iter().any(|s| s.ticket == id)
                    || self.pending_resumes.iter().any(|s| s.ticket == id)
                {
                    return Some("waiting for agents and their reaper".into());
                }
                // Sleeping is a record state, not proof that its live pane is gone.
                let panes = match self.backend.snapshot() {
                    Ok(p) => p,
                    Err(_) => return Some("cannot verify panes are gone".into()),
                };
                if panes.iter().any(|p| {
                    !p.pane_dead
                        && (p.session_name == terminal_name(Some(id))
                            || holders.iter().any(|s| p.session_name == s.sid16()))
                }) {
                    return Some("a live pane holds the worktree".into());
                }
                match self.backend.live_directories() {
                    Ok(paths) if !paths.iter().any(|p| p.starts_with(&b.path)) => {}
                    Ok(_) => return Some("a live pane holds the worktree".into()),
                    Err(_) => return Some("cannot verify pane directories".into()),
                }
                // Stamp before the oracle: the worker refuses a tip changed since
                // this verdict, including a clean external commit after scheduling.
                for leg in &mut legs {
                    leg.branch_oid = worktree::branch_tip(&leg.repo, &b.branch);
                }
                if !self.ticket_merged(id) {
                    return Some("branch is not merged".into());
                }
                None
            })();
            let e = self.column_evictions.get_mut(&id).expect("arrival exists");
            e.due = now + Duration::from_secs(2);
            let refusal = refusal.map(|why| format!("worktree kept: {why}"));
            let changed = e.refusal != refusal;
            let refused = refusal.is_some();
            e.refusal = refusal;
            if changed {
                self.broadcast();
            }
            if refused {
                continue;
            }
            // All spawn roads, wakes and terminal opens now refuse or park until
            // completion. No worker mutates board state, nor kills a live pane.
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

    fn bed(dir: &std::path::Path) -> (Daemon, ulid::Ulid) {
        let (mut d, _, _) = super::super::queued_person_push_tests::daemon(dir);
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
        (d, id)
    }

    #[test]
    fn column_eviction_waits_for_barred_bindings_process_ownership_and_the_reaper() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, id) = bed(dir.path());
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
        let (mut d, id) = bed(dir.path());
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
