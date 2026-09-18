//! Pictures travel through the authorized daemon and survive ticket lifecycle changes.
#![allow(clippy::unwrap_used)]
mod common;
use common::*;
use mesimon_core::command::{Command, Response};
use serde_json::json;
use std::time::Duration;

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

fn upload(c: &mut TestClient) -> ulid::Ulid {
    match c.request(Command::UploadAttachment {
        upload: None,
        offset: 0,
        data: PNG.into(),
        complete: true,
    }) {
        Response::AttachmentUploaded { upload } => upload,
        other => panic!("upload failed: {other:?}"),
    }
}

fn picture(c: &mut TestClient, ticket: ulid::Ulid, attachment: ulid::Ulid) {
    match c.request(Command::ReadAttachment { ticket, attachment }) {
        Response::Attachment { meta, data } => {
            assert_eq!(data, PNG);
            assert_eq!((meta.width, meta.height), (1, 1));
        }
        other => panic!("picture missing: {other:?}"),
    }
}

#[test]
fn pictures_survive_duplicate_delete_undo_archive_and_daemon_restart() {
    if !require_tmux() {
        return;
    }
    let fixture = TestFixture::new("attachments-lifecycle");
    let repo = fixture.dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = fixture.paths(&repo);
    let _daemon = fixture.daemon(&repo);
    wait_until(Duration::from_secs(5), "daemon socket", || paths.orch_sock().exists());
    let mut c = TestClient::connect(&paths.orch_sock());
    let attachment = upload(&mut c);
    let text =
        format!("The screenshot\n[Image #1]({})", mesimon_core::attachment::target(attachment));
    let id = match c.request(Command::CreateTicketWithNote {
        column: "TODO".into(),
        title: "pictured".into(),
        workspace: None,
        text: text.clone(),
        uploads: vec![attachment],
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create failed: {other:?}"),
    };
    let board = c.board();
    let note = board.ticket(id).unwrap().notes[0].id;
    assert!(
        matches!(c.request(Command::ReadNote { ticket: id, note }), Response::Note { text: body, .. } if body == text)
    );
    picture(&mut c, id, attachment);
    let copy = match c.request(Command::DuplicateTicket { id }) {
        Response::Created { id, .. } => id,
        other => panic!("copy failed: {other:?}"),
    };
    picture(&mut c, copy, attachment);
    assert!(matches!(
        c.request(Command::DeleteTicket { id: copy, discard_worktree: false }),
        Response::Ok
    ));
    assert!(matches!(c.request(Command::RestoreTicket { id: copy }), Response::Ok));
    picture(&mut c, copy, attachment);
    assert!(matches!(c.request(Command::ArchiveTicket { id: copy }), Response::Ok));
    picture(&mut c, copy, attachment);
    assert!(matches!(c.request(Command::UnarchiveTicket { id: copy }), Response::Ok));
    // Removing a reference does not destroy a saved picture.
    assert!(matches!(
        c.request(Command::WriteNote {
            ticket: id,
            note: Some(note),
            text: "description changed".into()
        }),
        Response::NoteWritten { .. }
    ));
    picture(&mut c, id, attachment);
    c.request(Command::Shutdown);
    wait_until(Duration::from_secs(5), "daemon shutdown", || !paths.orch_sock().exists());
    drop(c);
    let _restarted = fixture.daemon(&repo);
    wait_until(Duration::from_secs(5), "restarted daemon", || paths.orch_sock().exists());
    let mut c = TestClient::connect(&paths.orch_sock());
    picture(&mut c, id, attachment);
    picture(&mut c, copy, attachment);
    // Plain shared-note references are readable even when their picture isn't here.
    let absent = ulid::Ulid::new();
    err_containing(
        c.request(Command::ReadAttachment { ticket: id, attachment: absent }),
        "image unavailable on this machine",
    );
    c.request(Command::Shutdown);
}

#[test]
fn images_are_read_only_mcp_content_bound_to_the_agents_own_ticket() {
    let Some(h) =
        Harness::boot("attachments-mcp", Some("#!/bin/sh\nwhile IFS= read -r line; do :; done\n"))
    else {
        return;
    };
    let mut c = h.client("pictures");
    let attachment = upload(&mut c);
    let text = format!("[Image #1]({})", mesimon_core::attachment::target(attachment));
    let ticket = match c.request(Command::CreateTicketWithNote {
        column: "TODO".into(),
        title: "pictured".into(),
        workspace: None,
        text,
        uploads: vec![attachment],
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let session = match c.request(Command::SpawnSession {
        ticket,
        kind: mesimon_core::board::SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let mut shim = Shim::start(&h.paths.orch_sock(), session);
    let result = shim.call("read_attachment", json!({ "attachment": attachment.to_string() }));
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["content"][0]["type"], "image");
    assert_eq!(result["content"][0]["mimeType"], "image/png");
    assert_eq!(result["content"][0]["data"], PNG);
    let foreign = upload(&mut c);
    let other = match c.request(Command::CreateTicketWithNote {
        column: "TODO".into(),
        title: "other".into(),
        workspace: None,
        text: format!("[Image #1]({})", mesimon_core::attachment::target(foreign)),
        uploads: vec![foreign],
    }) {
        Response::Created { id, .. } => id,
        response => panic!("{response:?}"),
    };
    picture(&mut c, other, foreign);
    assert!(shim
        .call_err("read_attachment", json!({ "attachment": foreign.to_string() }))
        .contains("image unavailable"));
    assert!(shim
        .call_err("read_attachment", json!({ "attachment": "../../secret" }))
        .contains("not an attachment id"));
    // Upload handles are local to the TUI connection, even among local users.
    let staged = upload(&mut c);
    let mut other_client = h.client("other connection");
    err_containing(
        other_client.request(Command::SaveNoteWithAttachments {
            ticket,
            note: None,
            text: format!("[Image #2]({})", mesimon_core::attachment::target(staged)),
            uploads: vec![staged],
        }),
        "another connection",
    );
    c.request(Command::DiscardAttachmentUploads { uploads: vec![staged] });
}

#[test]
fn failed_note_save_rolls_back_new_files_and_can_be_retried() {
    let Some(h) = Harness::boot("attachments-retry", None) else { return };
    let mut c = h.client("picture retry");
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "retry".into(),
        workspace: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let board = c.board();
    let key = &board.ticket(ticket).unwrap().short_key;
    let notes = h.repo.join(".mesimon/board/tickets").join(key).join("notes");
    std::fs::write(&notes, "a file blocks the notes directory").unwrap();
    let attachment = upload(&mut c);
    let save = Command::SaveNoteWithAttachments {
        ticket,
        note: None,
        text: format!("[Image #1]({})", mesimon_core::attachment::target(attachment)),
        uploads: vec![attachment],
    };
    err_containing(c.request(save.clone()), "could not write the note");
    assert!(c.board().ticket(ticket).unwrap().notes.is_empty());
    assert!(mesimon_daemon::attachments::read(&h.paths, key, attachment).is_err());
    std::fs::remove_file(notes).unwrap();
    assert!(matches!(c.request(save), Response::NoteWritten { note: Some(_) }));
    picture(&mut c, ticket, attachment);
}
