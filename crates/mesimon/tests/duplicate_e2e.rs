//! Board duplication copies durable content, never session or automation state.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use mesimon_core::board::{SessionKind, WorkspaceStrategy};
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;

fn create(c: &mut TestClient, title: &str) -> ulid::Ulid {
    match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: Some(WorkspaceStrategy::SharedCheckout),
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    }
}

fn duplicate(c: &mut TestClient, id: ulid::Ulid) -> ulid::Ulid {
    match c.request(Command::DuplicateTicket { id }) {
        Response::Created { id, started: false } => id,
        other => panic!("{other:?}"),
    }
}

#[test]
fn duplicate_copies_content_below_source_without_starting_an_agent() {
    let Some(h) = Harness::boot("duplicate", Some("#!/bin/sh\nwhile read -r line; do :; done\n"))
    else {
        return;
    };
    let mut c = h.client("duplicate");
    let source = create(&mut c, "Copy this 日本語");
    let following = create(&mut c, "Following card");
    for body in ["# Description\n\nExact **markdown**.\n", "# Plan\n\nSecond note ✓\n"] {
        assert!(matches!(
            c.request(Command::WriteNote { ticket: source, note: None, text: body.into() }),
            Response::NoteWritten { .. }
        ));
    }
    assert!(matches!(
        c.request(Command::SetTag { id: source, group: 1, name: Some("FEATURE".into()) }),
        Response::Ok
    ));
    assert!(matches!(c.request(Command::SetManualMerge { id: source, on: true }), Response::Ok));
    let session = match c.request(Command::SpawnSession {
        ticket: source,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("{other:?}"),
    };
    // A live agent cannot reach the human duplication command.
    assert!(matches!(
        c.send(Principal::Agent { session }, Command::DuplicateTicket { id: source }),
        Response::Err { .. }
    ));
    let mut settings = c.board().column("TODO").unwrap().settings.clone();
    settings.auto_run = true;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "TODO".into(), settings }),
        Response::Ok
    ));
    let before = c.board();
    let original = before.ticket(source).unwrap();
    let copied = duplicate(&mut c, source);
    let board = c.board();
    let copy = board.ticket(copied).unwrap();
    assert_eq!(
        board.column_tickets("TODO").iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![source, copied, following]
    );
    assert_ne!(copy.short_key, original.short_key);
    assert_eq!(copy.title, original.title);
    assert_eq!(copy.tags, original.tags);
    assert_eq!(copy.workspace, original.workspace);
    assert_eq!(copy.created_by, "local");
    assert!(
        copy.created_from.is_none()
            && copy.archived.is_none()
            && copy.raised.is_none()
            && copy.woke_at.is_none()
    );
    assert!(!copy.manual_merge);
    assert_eq!(board.sessions.len(), before.sessions.len());
    assert!(!board.sessions.iter().any(|s| s.ticket == copied));
    assert_eq!(copy.notes.len(), original.notes.len());
    for (new, old) in copy.notes.iter().zip(&original.notes) {
        assert_ne!(new.id, old.id);
        let mut expected = old.clone();
        expected.id = new.id;
        assert_eq!(*new, expected);
        let body = mesimon_daemon::store::read_note(&h.paths, &original.short_key, old.id).unwrap();
        match c.request(Command::ReadNote { ticket: copied, note: new.id }) {
            Response::Note { text, .. } => assert_eq!(text, body),
            other => panic!("{other:?}"),
        }
    }
    // A new store load sees the full copy and the reserved key.
    let loaded = mesimon_daemon::store::load(&h.paths).unwrap();
    assert_eq!(loaded.board.next_key, board.next_key);
    assert_eq!(loaded.board.ticket(copied).unwrap().notes, copy.notes);
    assert_eq!(loaded.board.ticket(copied).unwrap().order, copy.order);
    // Editing the copy leaves the source note unchanged.
    assert!(matches!(
        c.request(Command::WriteNote {
            ticket: copied,
            note: Some(copy.notes[0].id),
            text: "Changed copy".into()
        }),
        Response::NoteWritten { .. }
    ));
    assert_eq!(
        mesimon_daemon::store::read_note(&h.paths, &original.short_key, original.notes[0].id)
            .unwrap(),
        "# Description\n\nExact **markdown**.\n"
    );
    // A bare last card copies too, still bypassing the column's auto-run.
    let last = duplicate(&mut c, following);
    let board = c.board();
    assert_eq!(board.column_tickets("TODO").last().unwrap().id, last);
    assert!(board.ticket(last).unwrap().notes.is_empty());
    assert!(!board.sessions.iter().any(|s| s.ticket == last));
    // A missing note refuses the entire copy before reserving a key.
    std::fs::remove_file(
        h.paths
            .board_dir
            .join("board/tickets")
            .join(&original.short_key)
            .join("notes")
            .join(format!("{}.md", original.notes[1].id)),
    )
    .unwrap();
    err_containing(c.request(Command::DuplicateTicket { id: source }), "could not copy the note");
    assert_eq!(c.board().tickets.len(), board.tickets.len());
    assert_eq!(c.board().next_key, board.next_key);
    err_containing(c.request(Command::DuplicateTicket { id: ulid::Ulid::nil() }), "no such ticket");
}
