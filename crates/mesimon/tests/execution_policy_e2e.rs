//! Durable owner-only policy survives import, duplication, preferences and restart.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use mesimon_core::board::{ExecutionPolicy, WorkspaceStrategy};
use mesimon_core::command::{Command, Response};
use mesimon_core::content::{ImportOrigin, ImportPlacement, PreparedImport, TicketContent};
use mesimon_daemon::store;

#[test]
fn owner_only_import_is_inert_and_copies_keep_the_restriction() {
    if !require_tmux() { return; }
    let fixture = TestFixture::new("owner-only");
    let repo = fixture.dir.join("repo");
    std::fs::create_dir(&repo).unwrap();
    let paths = fixture.paths(&repo);
    // Seed a persisted ticket before starting the daemon. No new remote transport
    // is exposed by this generic safety seam.
    let id = ulid::Ulid::new();
    let origin = ImportOrigin { source: ulid::Ulid::new(), item: ulid::Ulid::new() };
    let prepared = PreparedImport::prepare(
        &mesimon_core::Principal::Local,
        TicketContent { title: "Incoming question".into(), notes: vec!["Approved context".into()] },
        origin.clone(),
        ImportPlacement {
            id, short_key: "T-1".into(), column: "TODO".into(), order: "a0".into(),
            created_at: "@0".into(),
        },
        ulid::Ulid::new,
    ).unwrap();
    for (note, body) in &prepared.note_bodies {
        store::save_note(&paths, &prepared.ticket.short_key, *note, body).unwrap();
    }
    store::save_ticket(&paths, &prepared.ticket).unwrap();
    let mut loaded = store::load(&paths).unwrap();
    loaded.board.next_key = 1; // The imported ticket already owns T-1.
    loaded.board.columns.iter_mut().find(|c| c.name == "TODO").unwrap().settings.auto_run = true;
    store::save_columns(&paths, &loaded.board).unwrap();

    let daemon = fixture.daemon(&repo);
    let mut c = TestClient::connect(&paths.orch_sock());
    assert!(c.board().sessions.is_empty());
    for on in [true, false] {
        assert!(matches!(c.request(Command::SetManualMerge { id, on }), Response::Ok));
    }
    let copy = match c.request(Command::DuplicateTicket { id }) {
        Response::Created { id, started: false } => id,
        response => panic!("{response:?}"),
    };
    let board = c.board();
    for id in [id, copy] {
        let ticket = board.ticket(id).unwrap();
        assert_eq!(ticket.execution_policy, ExecutionPolicy::OwnerOnly);
        assert_eq!(ticket.import_origin, Some(origin.clone()));
        assert_eq!(ticket.workspace, Some(WorkspaceStrategy::Worktree));
        assert!(!ticket.manual_merge);
        assert_eq!(store::read_note(&paths, &ticket.short_key, ticket.notes[0].id).unwrap(), "Approved context");
    }
    assert!(board.sessions.is_empty());
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
    drop(c);
    // Restart reads the actual persisted files, not a test-only snapshot.
    let daemon = fixture.daemon(&repo);
    let mut c = TestClient::connect(&paths.orch_sock());
    let board = c.board();
    assert!(board.sessions.is_empty());
    for id in [id, copy] {
        assert_eq!(board.ticket(id).unwrap().execution_policy, ExecutionPolicy::OwnerOnly);
        assert_eq!(board.ticket(id).unwrap().import_origin, Some(origin.clone()));
    }
    let _ = c.request(Command::Shutdown);
    daemon.join().unwrap();
}
