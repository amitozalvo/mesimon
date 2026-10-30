//! Signed-record behavior with a real board and note store, no relay or sessions.
use super::*;
use mesimon_core::team::SharedTicket;
use mesimon_team::crypto::DeviceKeys;
use mesimon_team::wire::Receipt;
use std::sync::mpsc::{channel, Receiver};

struct Bed {
    d: Daemon,
    author: DeviceKeys,
    jobs: Receiver<Job>,
    _dir: tempfile::TempDir,
}

impl Bed {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let paths = Paths {
            repo_root: repo.clone(),
            proj16: "0656065606560656".into(),
            rt_dir: dir.path().join("rt"),
            state_dir: dir.path().join("state"),
            board_dir: repo.join(".mesimon"),
        };
        paths.ensure_dirs().unwrap();
        let (tx, _rx) = channel();
        let backend =
            TmuxBackend::new(dir.path().join("tmux.sock"), &paths.state_dir, None).unwrap();
        let feed = FeedWriter::open(&paths.activity_log()).unwrap();
        let journal = crate::journal::Journal::open(&paths.daemon_log());
        let mut d = Daemon::assemble(Seed {
            board: Board::with_default_columns(),
            backend,
            journal,
            notices: Vec::new(),
            exe_stamp: None,
            own_exe: None,
            started_at: String::new(),
            worktrees_barred: false,
            queue_barred: false,
            started_barred: false,
            costs_barred: false,
            machines: HashMap::new(),
            queued: Vec::new(),
            train_barred: false,
            forge_barred: false,
            feed,
            started: Default::default(),
            costs: Default::default(),
            worktrees: Default::default(),
            crown_barred: false,
            modroad: bridge::ModRoad::start(&paths),
            git_fetch_every: Duration::from_secs(60),
            columns_barred: false,
            sessions_barred: false,
            terminals: Default::default(),
            tx,
            self_exe: dir.path().join("mesimon"),
            paths,
        });
        let (jobs, rx) = channel();
        d.team = TeamCtx::with_jobs(jobs);
        d.team.device = Some(DeviceFile::fresh(
            "Dana".into(),
            RelayEndpoint::parse("relay.example:9000").unwrap(),
        ));
        let author = DeviceKeys::generate();
        let mut state = TeamState::new(BoardId::random(), Role::Contributor, true, "Amit".into());
        state.pin_owner(author.public(), "Amit".into());
        let local = d.team.device.as_ref().unwrap().keys().unwrap();
        state.verified_members.insert(
            local.id().to_hex(),
            VerifiedMember {
                public: local.public(),
                display_name: "Dana".into(),
                role: Role::Contributor,
                active: true,
            },
        );
        state.add_key(0, &BoardKey::generate());
        state.members.push(Member {
            device: author.id(),
            public: author.public(),
            display_name: "Amit".into(),
            role: Role::Owner,
            status: MemberStatus::Active,
            proof: None,
            epochs: vec![0],
        });
        d.team.state = Some(state);
        Self { d, author, jobs: rx, _dir: dir }
    }

    fn state(&self) -> &TeamState {
        self.d.team.state.as_ref().unwrap()
    }

    fn record(&self, id: ulid::Ulid, revision: u64, epoch: u32, body: RecordBody) -> StoredRecord {
        let object = ObjectId::from(id);
        let scope = RecordScope { board: self.state().board_id().unwrap(), object, revision };
        StoredRecord {
            object,
            revision,
            seq: revision,
            record: crypto::seal(
                &self.state().key(epoch).unwrap(),
                epoch,
                scope,
                &self.author,
                &serde_json::to_vec(&body).unwrap(),
            )
            .unwrap(),
        }
    }

    fn apply(&mut self, record: StoredRecord) {
        // Exercise the receiver under the unchanged environment gate without
        // mutating the unit-test process's environment.
        self.d.team_receive_record(record);
    }

    fn restart_state(&mut self) {
        self.state().save(&self.d.paths.team_file()).unwrap();
        self.d.team.state = TeamState::load(&self.d.paths.team_file()).unwrap();
    }

    fn note(&self, ticket: ulid::Ulid, note: ulid::Ulid) -> String {
        let ticket = self.d.board.ticket(ticket).unwrap();
        store::read_note(&self.d.paths, &ticket.short_key, note).unwrap()
    }
}

fn ticket(title: &str) -> RecordBody {
    RecordBody::new(SharedObject::Ticket(SharedTicket {
        title: title.into(),
        column: "TODO".into(),
        order: "a0".into(),
        created_at: "@1790000000".into(),
        created_by: "member:Amit".into(),
        archived: false,
        notes: Vec::new(),
        deleted: false,
    }))
}

fn stone() -> RecordBody {
    let SharedObject::Ticket(t) = ticket("gone").object else { unreachable!() };
    project::ticket_tombstone(&t)
}

#[test]
fn older_and_equal_records_with_later_sequences_leave_ticket_note_and_publication_unchanged() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    let mut current = ticket("current");
    if let SharedObject::Ticket(t) = &mut current.object {
        t.column = "REVIEW".into();
    }
    b.apply(b.record(t, 3, 0, current));
    b.apply(b.record(n, 3, 0, project::note_body(t, "current note".into())));
    let board = b.d.board.clone();
    let state = b.state().clone();
    for revision in [2, 3] {
        for (id, body) in [(t, ticket("old")), (n, project::note_body(t, "old note".into()))] {
            let mut record = b.record(id, revision, 0, body);
            record.seq = 100 + revision;
            b.apply(record);
        }
    }
    assert_eq!(serde_json::to_value(&b.d.board).unwrap(), serde_json::to_value(&board).unwrap());
    assert_eq!(b.state(), &state);
    assert_eq!(b.note(t, n), "current note");
    b.d.feed.flush().unwrap();
    let feed = std::fs::read_to_string(b.d.paths.activity_log()).unwrap();
    let refused: Vec<serde_json::Value> = feed
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .filter(|v: &serde_json::Value| v["cmd"] == "team_revision_refused")
        .collect();
    assert_eq!(refused.len(), 4);
    let detail = refused[0]["detail"].as_str().unwrap();
    assert!(detail.contains(&ObjectId::from(t).to_hex()));
    assert!(detail.contains("revision 2") && detail.contains("revision 3"));
}

#[test]
fn remote_tombstones_hold_after_local_removal_and_restart() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 3, 0, ticket("current")));
    b.apply(b.record(n, 3, 0, project::note_body(t, "current note".into())));
    b.apply(b.record(n, 4, 0, project::note_tombstone(t)));
    b.apply(b.record(t, 4, 0, stone()));
    assert!(b.d.board.ticket(t).unwrap().is_archived());
    b.apply(b.record(t, 3, 0, ticket("old")));
    assert!(b.d.board.ticket(t).unwrap().is_archived());
    b.d.board.tickets.clear();
    b.d.team_after_broadcast();
    // Forget digests as deletion does; leave no pending edit to mask a replay.
    b.d.team.state.as_mut().unwrap().forget_published(&ObjectId::from(n).to_hex());
    b.d.team.state.as_mut().unwrap().outbox.clear();
    b.restart_state();
    let state = b.state().clone();
    let next_key = b.d.board.next_key;
    b.apply(b.record(t, 3, 0, ticket("old")));
    b.apply(b.record(n, 3, 0, project::note_body(t, "old note".into())));
    assert!(b.d.board.tickets.is_empty());
    assert_eq!(b.d.board.next_key, next_key);
    assert!(b.state().pending_records.is_empty());
    assert_eq!(b.state(), &state);
    assert_eq!(b.state().revision(&ObjectId::from(t).to_hex()), Some(4));
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), Some(4));
}

#[test]
fn a_note_before_its_parent_is_published_only_when_it_can_apply() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(n, 3, 0, project::note_body(t, "deferred".into())));
    assert_eq!(b.state().pending_records.len(), 1);
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), None);
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.d.team_retry_pending_records();
    assert_eq!(b.note(t, n), "deferred");
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), Some(3));
    assert!(b.state().pending_records.is_empty());
}

#[test]
fn a_deferred_note_cannot_replace_a_later_tombstone() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(n, 3, 0, project::note_body(t, "deferred".into())));
    b.apply(b.record(n, 4, 0, project::note_tombstone(t)));
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.d.team_retry_pending_records();
    assert!(b.d.board.ticket(t).unwrap().notes.is_empty());
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), Some(4));
}

#[test]
fn rotation_keeps_historical_catchup_but_does_not_reset_revision_floors() {
    let mut b = Bed::new();
    let (t, untouched) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 3, 0, ticket("current")));
    b.d.team.state.as_mut().unwrap().add_key(1, &BoardKey::generate());
    b.restart_state();
    b.apply(b.record(t, 2, 0, ticket("old")));
    assert_eq!(b.d.board.ticket(t).unwrap().title, "current");
    b.apply(b.record(untouched, 1, 0, ticket("untouched before rotation")));
    assert_eq!(b.d.board.ticket(untouched).unwrap().title, "untouched before rotation");
    b.apply(b.record(t, 4, 1, ticket("new epoch")));
    assert_eq!(b.d.board.ticket(t).unwrap().title, "new epoch");
}

#[test]
fn pending_local_edits_keep_their_body_and_send_against_the_newer_floor() {
    let mut b = Bed::new();
    let t = ulid::Ulid::new();
    b.apply(b.record(t, 1, 0, ticket("first")));
    b.d.board.ticket_mut(t).unwrap().title = "local".into();
    b.d.team_after_broadcast();
    let local = b.state().outbox[0].body.clone();
    b.apply(b.record(t, 3, 0, ticket("remote")));
    b.apply(b.record(t, 2, 0, ticket("older remote")));
    assert_eq!(b.d.board.ticket(t).unwrap().title, "local");
    assert_eq!(b.state().outbox[0].body, local);
    b.d.team_pump();
    let Job::Call { request: Request::Put { expected, record, object, .. }, .. } =
        b.jobs.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
    else {
        panic!("expected put");
    };
    assert_eq!(expected, Some(3));
    let keys = b.d.team.device.as_ref().unwrap().keys().unwrap();
    let plain = crypto::open(
        &b.state().key(0).unwrap(),
        RecordScope { board: b.state().board_id().unwrap(), object, revision: 4 },
        &record,
        &keys.public(),
    )
    .unwrap();
    assert_eq!(serde_json::from_slice::<RecordBody>(&plain).unwrap(), local);
}

#[test]
fn local_deletion_uses_and_persists_the_tombstone_revision_without_requeueing() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 3, 0, ticket("current")));
    b.apply(b.record(n, 3, 0, project::note_body(t, "note".into())));
    b.d.board.tickets.clear();
    b.d.team_after_broadcast();
    assert_eq!(b.state().outbox.len(), 2);
    for _ in 0..2 {
        b.d.team_pump();
        let Job::Call { tag: Tag::Put(operation), request: Request::Put { expected, object, .. } } =
            b.jobs.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
        else {
            panic!("expected put");
        };
        assert_eq!(expected, Some(3));
        b.d.team.inflight.remove(&Tag::Put(operation.clone()));
        b.d.team_on_put(
            &operation,
            Ok(Wire::Accepted { receipt: Receipt { object, revision: 4, seq: 10 } }),
        );
        b.d.team_after_broadcast();
    }
    assert!(b.state().outbox.is_empty());
    assert!(b.state().published.is_empty());
    b.restart_state();
    b.apply(b.record(t, 3, 0, ticket("old")));
    b.apply(b.record(n, 3, 0, project::note_body(t, "old note".into())));
    assert!(b.d.board.tickets.is_empty());
    assert!(b.state().pending_records.is_empty());
    assert_eq!(b.state().revision(&ObjectId::from(t).to_hex()), Some(4));
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), Some(4));
}

#[test]
fn a_late_put_receipt_cannot_lower_the_revision_or_replace_a_newer_digest() {
    let mut b = Bed::new();
    let t = ulid::Ulid::new();
    b.apply(b.record(t, 1, 0, ticket("first")));
    b.d.board.ticket_mut(t).unwrap().title = "local".into();
    b.d.team_after_broadcast();
    b.d.team_pump();
    let Job::Call { tag: Tag::Put(operation), request: Request::Put { object, .. } } =
        b.jobs.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
    else {
        panic!("expected put");
    };
    b.apply(b.record(t, 3, 0, ticket("remote")));
    let published = b.state().published.clone();
    b.d.team_on_put(
        &operation,
        Ok(Wire::Accepted { receipt: Receipt { object, revision: 2, seq: 2 } }),
    );
    assert_eq!(b.state().published, published);
    assert_eq!(b.state().revision(&object.to_hex()), Some(3));
    assert_eq!(b.d.board.ticket(t).unwrap().title, "local");
}

#[test]
fn unverifiable_records_and_mismatched_receipts_do_not_advance_the_floor() {
    let mut b = Bed::new();
    let t = ulid::Ulid::new();
    b.apply(b.record(t, 1, 0, ticket("first")));
    let mut forged = b.record(t, 2, 0, ticket("bad"));
    forged.revision = 100;
    b.apply(forged);
    assert_eq!(b.state().revision(&ObjectId::from(t).to_hex()), Some(1));
    b.d.board.ticket_mut(t).unwrap().title = "local".into();
    b.d.team_after_broadcast();
    b.d.team_pump();
    let operation = b.state().outbox[0].operation.clone();
    for (object, revision) in [(ObjectId::from(t), 100), (ObjectId::random(), 2)] {
        b.d.team_on_put(
            &operation,
            Ok(Wire::Accepted { receipt: Receipt { object, revision, seq: 100 } }),
        );
        assert_eq!(b.state().revision(&ObjectId::from(t).to_hex()), Some(1));
        assert_eq!(b.state().outbox.len(), 1);
    }
}

#[test]
fn a_note_local_edit_survives_a_late_receipt_and_retries_above_the_floor() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "first".into())));
    let key = b.d.board.ticket(t).unwrap().short_key.clone();
    store::save_note(&b.d.paths, &key, n, "local").unwrap();
    b.d.board.ticket_mut(t).unwrap().notes[0].rev += 1;
    b.d.team_after_broadcast();
    // Note ordering also changes the parent projection; isolate the note put.
    b.d.team.state.as_mut().unwrap().outbox.retain(|o| o.object == ObjectId::from(n).to_hex());
    b.d.team_pump();
    let Job::Call { tag: Tag::Put(operation), request: Request::Put { object, .. } } =
        b.jobs.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
    else {
        panic!("expected put");
    };
    b.apply(b.record(n, 3, 0, project::note_body(t, "remote".into())));
    b.d.team.inflight.remove(&Tag::Put(operation.clone()));
    b.d.team_on_put(
        &operation,
        Ok(Wire::Accepted { receipt: Receipt { object, revision: 2, seq: 2 } }),
    );
    assert_eq!(b.note(t, n), "local");
    let retry = &b.state().outbox[0];
    assert_ne!(retry.operation, operation);
    assert_eq!(retry.expected, Some(3));
    assert_eq!(retry.body, project::note_body(t, "local".into()));
}

#[test]
fn a_note_changed_again_during_a_put_queues_its_latest_body_after_the_receipt() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "first".into())));
    let key = b.d.board.ticket(t).unwrap().short_key.clone();
    store::save_note(&b.d.paths, &key, n, "local one").unwrap();
    b.d.board.ticket_mut(t).unwrap().notes[0].rev += 1;
    b.d.team_after_broadcast();
    b.d.team.state.as_mut().unwrap().outbox.retain(|o| o.object == ObjectId::from(n).to_hex());
    b.d.team_pump();
    let operation = b.state().outbox[0].operation.clone();
    store::save_note(&b.d.paths, &key, n, "local two").unwrap();
    b.d.board.ticket_mut(t).unwrap().notes[0].rev += 1;
    b.d.team_after_broadcast();
    assert!(b.state().outbox[0].dirty);
    b.d.team.inflight.remove(&Tag::Put(operation.clone()));
    b.d.team_on_put(
        &operation,
        Ok(Wire::Accepted { receipt: Receipt { object: ObjectId::from(n), revision: 2, seq: 2 } }),
    );
    let retry = b.state().outbox.iter().find(|o| o.object == ObjectId::from(n).to_hex()).unwrap();
    assert_eq!(retry.expected, Some(2));
    assert_eq!(retry.body, project::note_body(t, "local two".into()));
}

#[test]
fn deleting_a_note_during_its_put_sends_a_tombstone_after_the_receipt() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "first".into())));
    let key = b.d.board.ticket(t).unwrap().short_key.clone();
    store::save_note(&b.d.paths, &key, n, "local").unwrap();
    b.d.board.ticket_mut(t).unwrap().notes[0].rev += 1;
    b.d.team_after_broadcast();
    b.d.team.state.as_mut().unwrap().outbox.retain(|o| o.object == ObjectId::from(n).to_hex());
    b.d.team_pump();
    let operation = b.state().outbox[0].operation.clone();
    b.d.board.ticket_mut(t).unwrap().notes.clear();
    b.d.team_after_broadcast();
    assert!(b.state().outbox[0].dirty);
    b.d.team.inflight.remove(&Tag::Put(operation.clone()));
    b.d.team_on_put(
        &operation,
        Ok(Wire::Accepted { receipt: Receipt { object: ObjectId::from(n), revision: 2, seq: 2 } }),
    );
    let retry = b.state().outbox.iter().find(|o| o.object == ObjectId::from(n).to_hex()).unwrap();
    assert_eq!(retry.expected, Some(2));
    assert_eq!(retry.body, project::note_tombstone(t));
}

#[test]
fn keys_unlock_a_pending_note_and_later_parent_in_one_retry() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    let note = b.record(n, 3, 0, project::note_body(t, "deferred for keys".into()));
    let parent = b.record(t, 1, 0, ticket("parent"));
    let key = b.state().key(0).unwrap();
    b.d.team.state.as_mut().unwrap().keys.clear();
    b.apply(note);
    b.apply(parent);
    assert_eq!(b.state().pending_records.len(), 2);
    // No key yet: the retry terminates with both records still pending.
    b.d.team_retry_pending_records();
    assert_eq!(b.state().pending_records.len(), 2);
    b.d.team.state.as_mut().unwrap().add_key(0, &key);
    b.d.team_retry_pending_records();
    assert!(b.state().pending_records.is_empty());
    assert_eq!(b.note(t, n), "deferred for keys");
    assert_eq!(b.d.board.ticket(t).unwrap().notes[0].rev, 1);
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), Some(3));
}

impl Bed {
    fn note_path(&self, ticket: ulid::Ulid, note: ulid::Ulid) -> PathBuf {
        self.d
            .paths
            .board_dir
            .join("board/tickets")
            .join(&self.d.board.ticket(ticket).unwrap().short_key)
            .join("notes")
            .join(format!("{note}.md"))
    }

    /// Restore only what a previous process actually persisted, not its memory.
    fn reload(&mut self) {
        self.d.team.state = TeamState::load(&self.d.paths.team_file()).unwrap();
        self.d.board = store::load(&self.d.paths).unwrap().board;
        self.d.team.note_revs.clear();
        for ticket in &self.d.board.tickets {
            for note in &ticket.notes {
                self.d.team.note_revs.insert(note.id, (note.rev, ticket.id));
            }
        }
        self.d.team.blocked_digests.clear();
        self.d.team_capture_blocked_digests();
    }
}

#[test]
fn a_failed_note_write_retains_the_exact_revision_across_paging_and_restart() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "old body".into())));
    let before = b.d.board.ticket(t).unwrap().notes[0].rev;
    let incoming = b.record(n, 3, 0, project::note_body(t, "new body".into()));
    let blocked = b.note_path(t, n).with_extension("tmp");
    std::fs::create_dir(&blocked).unwrap();
    assert_eq!(b.d.team_receive_record(incoming.clone()), ApplyOutcome::Deferred);
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), Some(3));
    assert_eq!(b.state().published[&ObjectId::from(n).to_hex()].revision, 1);
    assert_eq!(b.d.board.ticket(t).unwrap().notes[0].rev, before);
    assert!(b.d.team.error.is_some());
    assert!(b.d.team_save_cursor(100));
    std::fs::remove_dir(&blocked).unwrap();
    b.reload();
    assert_eq!(b.state().cursor, 100);
    assert_eq!(b.state().pending_records.len(), 1);
    b.d.team_after_broadcast();
    assert!(!b.state().outbox.iter().any(|o| o.object == ObjectId::from(n).to_hex()));
    assert_eq!(
        b.d.team_receive_record(b.record(n, 2, 0, project::note_body(t, "older replay".into()))),
        ApplyOutcome::Rejected
    );
    assert_eq!(b.note(t, n), "old body");
    assert_eq!(b.d.team_receive_record(incoming.clone()), ApplyOutcome::Applied);
    assert_eq!(b.note(t, n), "new body");
    assert_eq!(b.d.board.ticket(t).unwrap().notes[0].rev, before + 1);
    assert!(b.state().pending_records.is_empty());
    assert_eq!(b.d.team_receive_record(incoming), ApplyOutcome::Rejected);
    assert_eq!(b.d.board.ticket(t).unwrap().notes[0].rev, before + 1);
}

#[test]
fn state_save_failure_blocks_content_and_cursor_until_the_intent_is_durable() {
    let mut b = Bed::new();
    let t = ulid::Ulid::new();
    b.apply(b.record(t, 1, 0, ticket("old")));
    let path = b.d.paths.team_file();
    let backup = path.with_extension("backup");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    let incoming = b.record(t, 3, 0, ticket("new"));
    assert_eq!(b.d.team_receive_record(incoming), ApplyOutcome::Deferred);
    assert_eq!(b.d.board.ticket(t).unwrap().title, "old");
    assert_eq!(store::load(&b.d.paths).unwrap().board.ticket(t).unwrap().title, "old");
    assert!(!b.d.team_save_cursor(50));
    assert_eq!(b.state().cursor, 0);
    assert!(b.d.team.state_dirty);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    // Even before disk recovery, a lower record cannot displace the intent in memory.
    assert_eq!(b.d.team_receive_record(b.record(t, 2, 0, ticket("older"))), ApplyOutcome::Rejected);
    assert!(b.d.team_save_cursor(50));
    b.reload();
    b.d.team_retry_pending_records();
    assert_eq!(b.d.board.ticket(t).unwrap().title, "new");
    assert_eq!(b.state().revision(&ObjectId::from(t).to_hex()), Some(3));
    assert!(b.state().pending_records.is_empty());
}

#[test]
fn a_crash_after_content_write_recovers_from_the_saved_intent_without_double_note_revision() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "old body".into())));
    let before = b.d.board.ticket(t).unwrap().notes[0].rev;
    b.d.team.before_record_finish = Some(|d| {
        let path = d.paths.team_file();
        std::fs::rename(&path, path.with_extension("backup")).unwrap();
        std::fs::create_dir(&path).unwrap();
    });
    assert_eq!(
        b.d.team_receive_record(b.record(n, 3, 0, project::note_body(t, "new body".into()))),
        ApplyOutcome::Deferred
    );
    assert_eq!(b.note(t, n), "new body", "content reached disk before finalization failed");
    let path = b.d.paths.team_file();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("backup"), &path).unwrap();
    b.reload();
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), Some(3));
    assert_eq!(b.state().published[&ObjectId::from(n).to_hex()].revision, 1);
    assert_eq!(b.d.board.ticket(t).unwrap().notes[0].rev, before + 1);
    b.d.team_after_broadcast();
    assert!(!b.state().outbox.iter().any(|o| o.object == ObjectId::from(n).to_hex()));
    b.d.team_retry_pending_records();
    assert!(b.state().pending_records.is_empty());
    assert_eq!(b.state().published[&ObjectId::from(n).to_hex()].revision, 3);
    assert_eq!(b.note(t, n), "new body");
    assert_eq!(b.d.board.ticket(t).unwrap().notes[0].rev, before + 1);
}

#[test]
fn later_pages_can_supply_deferred_author_key_and_parent_after_restart() {
    let mut b = Bed::new();
    // Missing peer authority must not remove the invite-pinned owner.
    b.author = DeviceKeys::generate();
    b.d.team.state.as_mut().unwrap().verified_members.insert(
        b.author.id().to_hex(),
        VerifiedMember {
            public: b.author.public(),
            display_name: "Peer".into(),
            role: Role::Contributor,
            active: true,
        },
    );
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    let note = b.record(n, 3, 0, project::note_body(t, "later dependencies".into()));
    let parent = b.record(t, 1, 0, ticket("parent"));
    let key = b.state().key(0).unwrap();
    let members = b.state().verified_members.clone();
    b.d.team.state.as_mut().unwrap().keys.clear();
    b.d.team.state.as_mut().unwrap().verified_members.remove(&b.author.id().to_hex());
    assert_eq!(b.d.team_receive_record(note), ApplyOutcome::Deferred);
    assert!(b.d.team_save_cursor(10));
    b.reload();
    assert_eq!(b.state().cursor, 10);
    assert!(b.state().revision_floors.is_empty());
    b.d.team.state.as_mut().unwrap().add_key(0, &key);
    b.d.team_retry_pending_records();
    assert_eq!(b.state().pending_records.len(), 1);
    b.d.team.state.as_mut().unwrap().verified_members = members;
    b.d.team_retry_pending_records();
    assert_eq!(b.state().pending_records.len(), 1, "parent still absent");
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), None);
    b.apply(parent);
    b.d.team_retry_pending_records();
    assert!(b.d.team_save_cursor(20));
    assert_eq!(b.note(t, n), "later dependencies");
    assert!(b.state().pending_records.is_empty());
}

#[test]
fn columns_barred_defers_a_new_ticket_without_publishing_it() {
    let mut b = Bed::new();
    let t = ulid::Ulid::new();
    b.d.columns_barred = true;
    assert_eq!(
        b.d.team_receive_record(b.record(t, 3, 0, ticket("deferred"))),
        ApplyOutcome::Deferred
    );
    assert_eq!(b.state().revision(&ObjectId::from(t).to_hex()), None);
    assert!(b.d.board.tickets.is_empty());
    assert!(b.d.team_save_cursor(10));
    b.d.team.state = TeamState::load(&b.d.paths.team_file()).unwrap();
    b.d.columns_barred = false;
    b.d.team_retry_pending_records();
    assert_eq!(b.d.board.ticket(t).unwrap().title, "deferred");
    assert_eq!(b.state().published[&ObjectId::from(t).to_hex()].revision, 3);
    assert!(b.state().pending_records.is_empty());
}

#[test]
fn local_changes_made_during_a_failed_incoming_write_still_win() {
    let mut b = Bed::new();
    let t = ulid::Ulid::new();
    b.apply(b.record(t, 1, 0, ticket("old")));
    let path = b.d.paths.team_file();
    let backup = path.with_extension("backup");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        b.d.team_receive_record(b.record(t, 3, 0, ticket("remote"))),
        ApplyOutcome::Deferred
    );
    b.d.team_after_broadcast();
    assert!(b.state().outbox.is_empty(), "unchanged old body is not republished");
    b.d.board.ticket_mut(t).unwrap().title = "local latest".into();
    b.d.team_after_broadcast();
    assert_eq!(b.state().outbox.len(), 1);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    b.d.team_retry_pending_records();
    assert_eq!(b.d.board.ticket(t).unwrap().title, "local latest");
    assert!(b.state().pending_records.is_empty());
    assert_eq!(b.state().outbox[0].expected, Some(3));
}

#[test]
fn an_unverified_high_revision_cannot_replace_a_retained_authenticated_intent() {
    let mut b = Bed::new();
    let t = ulid::Ulid::new();
    b.apply(b.record(t, 1, 0, ticket("old")));
    let path = b.d.paths.team_file();
    let backup = path.with_extension("backup");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    let authenticated = b.record(t, 3, 0, ticket("authenticated"));
    assert_eq!(b.d.team_receive_record(authenticated.clone()), ApplyOutcome::Deferred);
    let mut unknown = b.record(t, 999, 0, ticket("untrusted"));
    unknown.record.author = DeviceKeys::generate().id();
    assert_eq!(b.d.team_receive_record(unknown), ApplyOutcome::Deferred);
    assert_eq!(b.state().revision(&ObjectId::from(t).to_hex()), Some(3));
    assert!(b
        .state()
        .pending_records
        .iter()
        .any(|p| p.matches(&authenticated) && p.publication.is_some()));
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    assert!(b.d.team_save_cursor(100));
    b.reload();
    b.d.team_retry_pending_records();
    assert_eq!(b.d.board.ticket(t).unwrap().title, "authenticated");
    assert_eq!(b.state().published[&ObjectId::from(t).to_hex()].revision, 3);
    assert_eq!(b.state().pending_records.len(), 1);
    assert!(b.state().pending_records[0].publication.is_none());
}

#[test]
fn a_later_local_note_survives_finalization_failure_and_restart() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "original".into())));
    b.d.team.before_record_finish = Some(|d| {
        let path = d.paths.team_file();
        std::fs::rename(&path, path.with_extension("backup")).unwrap();
        std::fs::create_dir(&path).unwrap();
    });
    assert_eq!(
        b.d.team_receive_record(b.record(n, 3, 0, project::note_body(t, "remote".into()))),
        ApplyOutcome::Deferred
    );
    assert!(matches!(
        b.d.write_note(t, Some(n), "later local".into(), Some(2), &Principal::Local),
        Response::NoteWritten { .. }
    ));
    assert_eq!(b.d.board.ticket(t).unwrap().notes[0].rev, 3);
    assert_eq!(b.state().pending_records[0].note_rev, Some(2));
    assert!(b.state().outbox.iter().any(|o| o.object == ObjectId::from(n).to_hex()));
    let path = b.d.paths.team_file();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("backup"), &path).unwrap();
    b.reload();
    assert!(b.state().outbox.is_empty(), "the unsaved outbox did not survive");
    b.d.team_retry_pending_records();
    assert_eq!(b.note(t, n), "later local");
    let queued = b.state().outbox.iter().find(|o| o.object == ObjectId::from(n).to_hex()).unwrap();
    assert_eq!(queued.body, project::note_body(t, "later local".into()));
    assert_eq!(queued.expected, Some(3));
    assert!(b.state().pending_records.is_empty());
}

#[test]
fn a_later_durable_ticket_revert_survives_restart_without_its_unsaved_outbox() {
    let mut b = Bed::new();
    let t = ulid::Ulid::new();
    b.apply(b.record(t, 1, 0, ticket("original")));
    b.d.team.before_record_finish = Some(|d| {
        let path = d.paths.team_file();
        std::fs::rename(&path, path.with_extension("backup")).unwrap();
        std::fs::create_dir(&path).unwrap();
    });
    assert_eq!(
        b.d.team_receive_record(b.record(t, 3, 0, ticket("remote"))),
        ApplyOutcome::Deferred
    );
    assert!(matches!(b.d.with_ticket(t, |ticket| ticket.title = "original".into()), Response::Ok));
    let path = b.d.paths.team_file();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("backup"), &path).unwrap();
    b.reload();
    assert!(b.state().outbox.is_empty());
    b.d.team_retry_pending_records();
    assert_eq!(b.d.board.ticket(t).unwrap().title, "original");
    assert_eq!(b.state().outbox[0].expected, Some(3));
    let SharedObject::Ticket(queued) = &b.state().outbox[0].body.object else {
        panic!("ticket edit")
    };
    assert_eq!(queued.title, "original");
}

fn deferred(record: StoredRecord) -> PendingRecord {
    PendingRecord {
        record,
        publication: None,
        note_rev: None,
        local_before: None,
        local_after: None,
        local_intermediate: Vec::new(),
        applied_at: None,
    }
}

#[test]
fn retained_capacity_preserves_records_and_cursor_then_recovers_without_eviction() {
    let mut b = Bed::new();
    let candidate = b.record(ulid::Ulid::new(), 1, 0, ticket("later page"));
    let mut unknown = candidate.clone();
    unknown.record.epoch = 1;
    b.d.team.state.as_mut().unwrap().pending_records = (0..MAX_PENDING_RECORDS)
        .map(|i| {
            let mut record = unknown.clone();
            record.object = ObjectId::from(ulid::Ulid((i + 100) as u128));
            deferred(record)
        })
        .collect();
    let retained = b.state().pending_records.clone();
    assert_eq!(b.d.team_receive_record(candidate.clone()), ApplyOutcome::Deferred);
    assert_eq!(b.state().pending_records, retained);
    assert!(b.state().revision_floors.is_empty());
    assert!(!b.d.team_save_cursor(50));
    b.reload();
    assert_eq!(b.state().cursor, 0);
    assert_eq!(b.state().pending_records, retained);
    assert!(b.d.team.error.as_ref().unwrap().contains("retained input limit"));
    // Key recovery lets signature verification reject these malformed inputs.
    let key = b.state().key(0).unwrap();
    b.d.team.state.as_mut().unwrap().add_key(1, &key);
    b.d.team_retry_pending_records();
    assert_eq!(b.state().pending_records.len(), MAX_PENDING_RECORDS - MAX_RETRY_RECORDS);
    b.d.team.retention_full = false; // a fresh fetch of the unchanged page
    assert_eq!(b.d.team_receive_record(candidate), ApplyOutcome::Applied);
    assert!(b.d.team_save_cursor(50));
    assert_eq!(b.state().cursor, 50);
}

#[test]
fn retained_bytes_are_bounded_independently_of_count() {
    let mut b = Bed::new();
    let candidate = b.record(ulid::Ulid::new(), 1, 0, ticket("candidate"));
    let mut oversized = candidate.clone();
    oversized.record.ciphertext.resize(MAX_PENDING_BYTES, 0);
    b.d.team.state.as_mut().unwrap().pending_records.push(deferred(oversized));
    assert!(!b.d.team_can_retain(&b.record(ulid::Ulid::new(), 1, 0, ticket("another"))));
    assert_eq!(b.state().pending_records.len(), 1);
    assert!(b.d.team.retention_full);
}

#[test]
fn retry_budget_is_fixed_and_rotates_to_dependencies_beyond_the_first_batch() {
    let mut b = Bed::new();
    let id = ulid::Ulid::new();
    let candidate = b.record(id, 1, 0, ticket("unblocked"));
    let mut unknown = candidate.clone();
    unknown.record.epoch = 1;
    b.d.team.state.as_mut().unwrap().pending_records = (0..MAX_RETRY_RECORDS)
        .map(|i| {
            let mut record = unknown.clone();
            record.object = ObjectId::from(ulid::Ulid((i + 100) as u128));
            deferred(record)
        })
        .chain(std::iter::once(deferred(candidate)))
        .collect();
    b.d.team_retry_pending_records();
    assert!(b.d.board.ticket(id).is_none());
    b.d.team_retry_pending_records();
    assert_eq!(b.d.board.ticket(id).unwrap().title, "unblocked");
    assert_eq!(b.state().pending_records.len(), MAX_RETRY_RECORDS);
}

#[test]
fn reconciliation_preserves_inflight_bytes_and_dirty_followup_until_the_receipt() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "original".into())));
    b.d.team.before_record_finish = Some(|d| {
        let path = d.paths.team_file();
        std::fs::rename(&path, path.with_extension("backup")).unwrap();
        std::fs::create_dir(&path).unwrap();
    });
    assert_eq!(
        b.d.team_receive_record(b.record(n, 3, 0, project::note_body(t, "remote".into()))),
        ApplyOutcome::Deferred
    );
    let path = b.d.paths.team_file();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("backup"), &path).unwrap();
    assert!(matches!(
        b.d.write_note(t, Some(n), "local A".into(), Some(2), &Principal::Local),
        Response::NoteWritten { .. }
    ));
    b.d.team.state.as_mut().unwrap().outbox.retain(|o| o.object == ObjectId::from(n).to_hex());
    b.d.team_pump();
    let sent = b.state().outbox[0].clone();
    assert_eq!(sent.body, project::note_body(t, "local A".into()));
    assert!(matches!(
        b.d.write_note(t, Some(n), "local B".into(), Some(3), &Principal::Local),
        Response::NoteWritten { .. }
    ));
    b.d.team_retry_pending_records();
    let flight = b.state().outbox.iter().find(|o| o.operation == sent.operation).unwrap();
    assert_eq!(flight.body, sent.body);
    assert_eq!(flight.expected, sent.expected);
    assert!(flight.dirty);
    b.d.team.inflight.remove(&Tag::Put(sent.operation.clone()));
    b.d.team_on_put(
        &sent.operation,
        Ok(Wire::Accepted { receipt: Receipt { object: ObjectId::from(n), revision: 4, seq: 40 } }),
    );
    let next = b.state().outbox.iter().find(|o| o.object == ObjectId::from(n).to_hex()).unwrap();
    assert_ne!(next.operation, sent.operation);
    assert_eq!(next.expected, Some(4));
    assert_eq!(next.body, project::note_body(t, "local B".into()));
    assert_eq!(b.note(t, n), "local B");
}

#[test]
fn a_failed_ticket_write_never_persists_the_intermediate_move() {
    let mut b = Bed::new();
    let t = ulid::Ulid::new();
    b.apply(b.record(t, 1, 0, ticket("original")));
    let blocked =
        b.d.paths
            .board_dir
            .join("board/tickets")
            .join(&b.d.board.ticket(t).unwrap().short_key)
            .join("ticket.tmp");
    std::fs::create_dir(&blocked).unwrap();
    let mut body = ticket("renamed");
    if let SharedObject::Ticket(t) = &mut body.object {
        t.column = "REVIEW".into();
    }
    assert_eq!(b.d.team_receive_record(b.record(t, 3, 0, body)), ApplyOutcome::Deferred);
    let stored = store::load(&b.d.paths).unwrap().board;
    assert_eq!(stored.ticket(t).unwrap().title, "original");
    assert_eq!(stored.ticket(t).unwrap().column, "TODO");
    std::fs::remove_dir(&blocked).unwrap();
    b.reload();
    b.d.team_retry_pending_records();
    assert_eq!(b.d.board.ticket(t).unwrap().title, "renamed");
    assert_eq!(b.d.board.ticket(t).unwrap().column, "REVIEW");
    assert!(b.state().outbox.is_empty());
}

#[test]
fn a_later_note_edit_with_a_colliding_local_revision_survives_partial_write_and_restart() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "original".into())));
    let blocked =
        b.d.paths
            .board_dir
            .join("board/tickets")
            .join(&b.d.board.ticket(t).unwrap().short_key)
            .join("ticket.tmp");
    std::fs::create_dir(&blocked).unwrap();
    assert_eq!(
        b.d.team_receive_record(b.record(n, 3, 0, project::note_body(t, "remote".into()))),
        ApplyOutcome::Deferred
    );
    assert_eq!(b.note(t, n), "remote");
    assert_eq!(b.d.board.ticket(t).unwrap().notes[0].rev, 1);
    std::fs::remove_dir(&blocked).unwrap();
    let state_path = b.d.paths.team_file();
    std::fs::rename(&state_path, state_path.with_extension("backup")).unwrap();
    std::fs::create_dir(&state_path).unwrap();
    assert!(matches!(
        b.d.write_note(t, Some(n), "later local".into(), Some(1), &Principal::Local),
        Response::NoteWritten { .. }
    ));
    assert_eq!(b.d.board.ticket(t).unwrap().notes[0].rev, 2);
    assert_eq!(b.state().pending_records[0].note_rev, Some(2));
    std::fs::remove_dir(&state_path).unwrap();
    std::fs::rename(state_path.with_extension("backup"), &state_path).unwrap();
    b.reload();
    assert!(b.state().outbox.is_empty());
    b.d.team_retry_pending_records();
    assert_eq!(b.note(t, n), "later local");
    let next = b.state().outbox.iter().find(|o| o.object == ObjectId::from(n).to_hex()).unwrap();
    assert_eq!(next.body, project::note_body(t, "later local".into()));
    assert_eq!(next.expected, Some(3));
}

#[test]
fn recovery_preserves_withheld_local_notes_without_queuing_their_bodies() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "original".into())));
    b.d.team.state.as_mut().unwrap().notes_withheld = true;
    b.d.team.before_record_finish = Some(|d| {
        let path = d.paths.team_file();
        std::fs::rename(&path, path.with_extension("backup")).unwrap();
        std::fs::create_dir(&path).unwrap();
    });
    assert_eq!(
        b.d.team_receive_record(b.record(n, 3, 0, project::note_body(t, "remote".into()))),
        ApplyOutcome::Deferred
    );
    assert!(matches!(
        b.d.write_note(t, Some(n), "private local".into(), Some(2), &Principal::Local),
        Response::NoteWritten { .. }
    ));
    let path = b.d.paths.team_file();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("backup"), &path).unwrap();
    b.reload();
    b.d.team_retry_pending_records();
    assert_eq!(b.note(t, n), "private local");
    assert!(!b.state().outbox.iter().any(|o| o.object == ObjectId::from(n).to_hex()));
}

#[test]
fn unavailable_local_inspection_defers_without_overwriting_bytes_then_preserves_the_edit() {
    let mut b = Bed::new();
    let (t, n) = (ulid::Ulid::new(), ulid::Ulid::new());
    b.apply(b.record(t, 1, 0, ticket("parent")));
    b.apply(b.record(n, 1, 0, project::note_body(t, "original".into())));
    b.d.team.before_record_finish = Some(|d| {
        let path = d.paths.team_file();
        std::fs::rename(&path, path.with_extension("backup")).unwrap();
        std::fs::create_dir(&path).unwrap();
    });
    let remote = b.record(n, 3, 0, project::note_body(t, "remote".into()));
    assert_eq!(b.d.team_receive_record(remote.clone()), ApplyOutcome::Deferred);
    assert!(matches!(
        b.d.write_note(t, Some(n), "later local".into(), Some(2), &Principal::Local),
        Response::NoteWritten { .. }
    ));
    let path = b.d.paths.team_file();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("backup"), &path).unwrap();
    b.reload();
    let note_path = b.note_path(t, n);
    let unreadable = b"later local\xff";
    std::fs::write(&note_path, unreadable).unwrap();
    let retained = b.state().pending_records.clone();
    assert_eq!(b.d.team_receive_record(remote), ApplyOutcome::Deferred);
    assert_eq!(std::fs::read(&note_path).unwrap(), unreadable);
    assert_eq!(b.state().pending_records, retained);
    assert_eq!(b.state().revision(&ObjectId::from(n).to_hex()), Some(3));
    assert!(b.state().outbox.is_empty());
    assert!(b.d.team.error.as_ref().unwrap().contains("could not be inspected"));
    std::fs::write(&note_path, "later local").unwrap();
    b.d.team_retry_pending_records();
    assert_eq!(b.note(t, n), "later local");
    let queued = b.state().outbox.iter().find(|o| o.object == ObjectId::from(n).to_hex()).unwrap();
    assert_eq!(queued.body, project::note_body(t, "later local".into()));
    assert_eq!(queued.expected, Some(3));
    assert!(b.state().pending_records.is_empty());
}
