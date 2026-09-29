//! Durable owner-only policy survives import, duplication, preferences and restart.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use mesimon_core::board::{ExecutionPolicy, WorkspaceStrategy};
use mesimon_core::command::{Command, Response};
use mesimon_core::content::{ImportOrigin, TicketContent};
use mesimon_core::Principal;
use mesimon_daemon::store;

#[test]
fn owner_only_import_is_inert_and_copies_keep_the_restriction() {
    if !require_tmux() {
        return;
    }
    let fixture = TestFixture::new("owner-only");
    let repo = fixture.dir.join("repo");
    std::fs::create_dir(&repo).unwrap();
    let paths = fixture.paths(&repo);
    std::fs::create_dir_all(paths.board_dir.join("board/tickets")).unwrap();
    let origin = ImportOrigin { source: ulid::Ulid::new(), item: ulid::Ulid::new() };
    let import = Command::ImportTicket {
        column: "TODO".into(),
        content: TicketContent {
            title: "Incoming question".into(),
            notes: vec!["Approved context".into()],
        },
        origin: origin.clone(),
    };

    let daemon = fixture.daemon(&repo);
    let mut c = TestClient::connect(&paths.orch_sock());
    for principal in [
        Principal::Agent { session: uuid::Uuid::nil() },
        Principal::Automation { rule: "remote".into() },
    ] {
        assert!(matches!(c.send(principal, import.clone()), Response::Err { .. }));
    }
    assert!(c.board().tickets.is_empty());
    let (id, key) = match c.request(import.clone()) {
        Response::Imported { id, key, created: true } => (id, key),
        other => panic!("{other:?}"),
    };
    match c.request(import.clone()) {
        Response::Imported { id: replay, key: replay_key, created: false } => {
            assert_eq!(replay, id);
            assert_eq!(replay_key, key);
        }
        other => panic!("{other:?}"),
    }
    assert!(c.board().sessions.is_empty());
    for on in [true, false] {
        assert!(matches!(c.request(Command::SetManualMerge { id, on }), Response::Ok));
    }
    let copy = match c.request(Command::DuplicateTicket { id }) {
        Response::Created { id } => id,
        response => panic!("{response:?}"),
    };
    let board = c.board();
    for id in [id, copy] {
        let ticket = board.ticket(id).unwrap();
        assert_eq!(ticket.execution_policy, ExecutionPolicy::OwnerOnly);
        assert_eq!(ticket.import_origin, Some(origin.clone()));
        assert_eq!(ticket.workspace, Some(WorkspaceStrategy::Worktree));
        assert!(!ticket.manual_merge);
        assert_eq!(
            store::read_note(&paths, &ticket.short_key, ticket.notes[0].id).unwrap(),
            "Approved context"
        );
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
    // Retries still identify the original after a copy, restart, local edits and
    // deletion. They cannot attach to a copy or recreate a deliberately removed card.
    let note = c.board().ticket(id).unwrap().notes[0].id;
    assert!(matches!(
        c.request(Command::WriteNote {
            ticket: id,
            note: Some(note),
            text: "Private working reply".into()
        }),
        Response::NoteWritten { .. }
    ));
    assert!(
        matches!(c.request(import.clone()), Response::Imported { id: original, created: false, .. } if original == id)
    );
    assert_eq!(store::read_note(&paths, &key, note).unwrap(), "Private working reply");
    let mut changed = import.clone();
    if let Command::ImportTicket { content, .. } = &mut changed {
        content.title = "Different request".into();
    }
    assert!(matches!(c.request(changed), Response::Err { .. }));
    assert!(matches!(
        c.request(Command::DeleteTicket { id, discard_worktree: false }),
        Response::Ok
    ));
    assert!(
        matches!(c.request(import.clone()), Response::Imported { id: original, created: false, .. } if original == id)
    );
    assert!(c.board().ticket(id).is_none());
    assert!(c.board().ticket(copy).is_some());
    assert!(c.board().sessions.is_empty());
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
    drop(c);
    let daemon = fixture.daemon(&repo);
    let mut c = TestClient::connect(&paths.orch_sock());
    assert!(
        matches!(c.request(import), Response::Imported { id: original, created: false, .. } if original == id)
    );
    assert_eq!(c.board().tickets.len(), 1);
    assert!(c.board().ticket(id).is_none());
    let _ = c.request(Command::Shutdown);
    daemon.join().unwrap();
}
