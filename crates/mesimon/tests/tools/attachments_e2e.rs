//! Pictures travel through the authorized daemon and survive ticket lifecycle changes.
#![allow(clippy::unwrap_used)]
use crate::common;
use common::*;
use mesimon_core::command::{Command, Response};
use serde_json::json;
use std::time::Duration;

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

// A one-page PDF with a blank page and its cross-reference table.
const PDF: &str = concat!(
    "JVBERi0xLjQKMSAwIG9iago8PC9UeXBlL0NhdGFsb2cvUGFnZXMgMiAwIFI+PgplbmRvYmoKMiAwIG9i",
    "ago8PC9UeXBlL1BhZ2VzL0NvdW50IDEvS2lkc1szIDAgUl0+PgplbmRvYmoKMyAwIG9iago8PC9UeXBl",
    "L1BhZ2UvUGFyZW50IDIgMCBSL01lZGlhQm94WzAgMCA3MiA3Ml0+PgplbmRvYmoKeHJlZgowIDQKMDAw",
    "MDAwMDAwMCA2NTUzNSBmIAowMDAwMDAwMDA5IDAwMDAwIG4gCjAwMDAwMDAwNTQgMDAwMDAgbiAKMDAw",
    "MDAwMDEwNSAwMDAwMCBuIAp0cmFpbGVyCjw8L1NpemUgNC9Sb290IDEgMCBSPj4Kc3RhcnR4cmVmCjE2",
    "OAolJUVPRgo=",
);

fn upload(c: &mut TestClient) -> ulid::Ulid {
    match c.request(Command::UploadAttachment {
        file: None,
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
        tags: Vec::new(),
        tier: None,
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
            text: "description changed".into(),
            rev: None,
        }),
        Response::NoteWritten { .. }
    ));
    picture(&mut c, id, attachment);
    c.request(Command::Shutdown);
    wait_until(Duration::from_secs(5), "daemon shutdown", || !paths.orch_sock().exists());
    drop(c);
    // An existing board's picture sidecar predates the attachment kind field.
    let sidecar =
        mesimon_daemon::attachments::directory(&paths, &board.ticket(id).unwrap().short_key)
            .join(format!("{attachment}.json"));
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("kind");
    std::fs::write(sidecar, legacy.to_string()).unwrap();
    let _restarted = fixture.daemon(&repo);
    wait_until(Duration::from_secs(5), "restarted daemon", || paths.orch_sock().exists());
    let mut c = TestClient::connect(&paths.orch_sock());
    picture(&mut c, id, attachment);
    picture(&mut c, copy, attachment);
    // A picture a note links whose bytes are not here (a team board joined
    // without them) is unavailable on this machine; an id no note links is
    // simply not this ticket's (T-609).
    let absent = ulid::Ulid::new();
    assert!(matches!(
        c.request(Command::WriteNote {
            ticket: id,
            note: Some(note),
            text: format!("[Image #1]({})", mesimon_core::attachment::target(absent)),
            rev: None,
        }),
        Response::NoteWritten { .. }
    ));
    err_containing(
        c.request(Command::ReadAttachment { ticket: id, attachment: absent }),
        "image unavailable on this machine",
    );
    err_containing(
        c.request(Command::ReadAttachment { ticket: id, attachment: ulid::Ulid::new() }),
        "no picture with that id on this ticket",
    );
    c.request(Command::Shutdown);
}

#[test]
fn images_are_read_only_mcp_content_bound_to_the_agents_own_ticket() {
    let Some(h) = Harness::boot(
        "attachments-mcp",
        Some("#!/bin/sh\ncat | while IFS= read -r line; do :; done\n"),
    ) else {
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
        tags: Vec::new(),
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let session = match c.request(Command::SpawnSession {
        ticket,
        kind: mesimon_core::board::SessionKind::Claude,
        submit_prompt: false,
        plan: false,
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
        tags: Vec::new(),
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        response => panic!("{response:?}"),
    };
    picture(&mut c, other, foreign);
    let miss = shim.call_err("read_attachment", json!({ "attachment": foreign.to_string() }));
    assert!(miss.contains("no picture with that id on this ticket"), "{miss}");
    // With a key (T-609, T-1089): the other ticket's picture, whole, for
    // any agent, as its notes are; and for a crown as before.
    let key = c.board().ticket(other).unwrap().short_key.clone();
    let keyed = json!({ "attachment": foreign.to_string(), "key": key });
    let result = shim.call("read_attachment", keyed.clone());
    assert_eq!(result["isError"], false, "an uncrowned keyed read: {result}");
    assert_eq!(result["content"][0]["data"], PNG);
    assert!(matches!(c.request(Command::CrownTicket { id: ticket }), Response::Ok));
    let result = shim.call("read_attachment", keyed);
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["content"][0]["data"], PNG);
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
        tier: None,
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

/// A picture with the words to an agent (T-853): the prompt names it, the
/// daemon keeps it with the ticket before the words go, the words reach the
/// agent linked to it, and the agent's own `read_attachment` finds it. A
/// prompt carrying an upload its words do not name is refused and keeps
/// nothing.
#[test]
fn a_prompt_with_a_picture_keeps_it_with_the_ticket_and_links_it_in_the_words() {
    // The stub agent keeps a receipt of every line it reads (prompt_e2e).
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\ncat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot("attachments-prompt", Some(STUB)) else { return };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("pictures");
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "pictured ask".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let session = match c.request(Command::SpawnSession {
        ticket,
        kind: mesimon_core::board::SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("{other:?}"),
    };
    wait_until(Duration::from_secs(15), "the stub's pane", || {
        tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false)
    });
    std::thread::sleep(Duration::from_millis(500));
    let attachment = upload(&mut c);
    let ask = |text: String, uploads: Vec<ulid::Ulid>| Command::PromptSession {
        ticket,
        text,
        queued: false,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads,
    };
    match c.request(ask("look at this".into(), vec![attachment])) {
        Response::Err { message } => assert!(message.contains("do not name"), "{message}"),
        other => panic!("an unnamed upload went through: {other:?}"),
    }
    // Words past the cap with a picture they name are refused for their
    // length (T-896), never as words that name no picture, and keep nothing.
    {
        use mesimon_core::command::PROMPT_MAX_BYTES;
        let link = format!("[Image #1]({})", mesimon_core::attachment::target(attachment));
        let long = format!("look at {link} {}", "x".repeat(PROMPT_MAX_BYTES));
        match c.request(ask(long.clone(), vec![attachment])) {
            Response::Err { message } => {
                assert!(message.contains(&format!("prompt is {} bytes", long.len())), "{message}");
                assert!(!message.contains("do not name"), "{message}");
            }
            other => panic!("a 32 KiB ask with a picture went through: {other:?}"),
        }
    }
    assert!(
        !matches!(
            c.request(Command::ReadAttachment { ticket, attachment }),
            Response::Attachment { .. }
        ),
        "a refused ask kept its picture"
    );
    let text = format!("look at [Image #1]({})", mesimon_core::attachment::target(attachment));
    match c.request(ask(text.clone(), vec![attachment])) {
        Response::Ok => {}
        other => panic!("{other:?}"),
    }
    wait_until(Duration::from_secs(15), "the linked words in the pane", || {
        std::fs::read_to_string(&got).is_ok_and(|s| s.contains(&text))
    });
    picture(&mut c, ticket, attachment);
    let mut shim = Shim::start(&h.paths.orch_sock(), session);
    let result = shim.call("read_attachment", json!({ "attachment": attachment.to_string() }));
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["content"][0]["data"], PNG);
}

#[test]
fn queued_files_survive_restart_and_ticket_lifecycle_with_a_clean_worktree() {
    use mesimon_core::{attachment::FileUpload, board::WorkspaceStrategy};
    let Some(h) = Harness::boot_staged("file-prompt", Some(RECORDING_STUB), &[], |_, repo| {
        init_repo(repo, "readme", "hello");
    }) else {
        return;
    };
    let mut c = h.client("files");
    let ticket = c.create_ticket("data");
    c.request(Command::SetWorkspace { id: ticket, workspace: Some(WorkspaceStrategy::Worktree) });
    c.request(Command::SpawnSession {
        ticket,
        kind: mesimon_core::board::SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    });
    let wt = std::path::PathBuf::from(wait_attached(&mut c, ticket).path.unwrap());
    wait_until(Duration::from_secs(15), "worker", || c.board().live_agent(ticket).is_some());
    let sid = c.board().live_agent(ticket).unwrap().id;
    h.prompt_turn(&mut c, sid);
    let mut upload = |name: &str, data: &str| match c.request(Command::UploadAttachment {
        file: Some(FileUpload { ticket, name: name.into() }),
        upload: None,
        offset: 0,
        data: data.into(),
        complete: true,
    }) {
        Response::AttachmentUploaded { upload } => upload,
        r => panic!("{r:?}"),
    };
    let typed = [
        ("Shares", PDF, "pdf", "application/pdf"),
        ("Snapshot", PNG, "png", "image/png"),
        ("Notes", "csOpc3Vtw6k=", "txt", "text/plain"),
    ]
    .map(|(name, data, ext, mime)| (upload(name, data), name, ext, mime));
    let first = upload("sample.dat", "YWJj/w==");
    let empty = upload("sample.dat", "");
    let mut links = format!("Compare [sample.dat](mesimon-attachment:{first}) [sample.dat #2](mesimon-attachment:{empty})");
    let mut uploads = Vec::new();
    for (id, name, _, _) in typed {
        links.push_str(&format!(" [{name}](mesimon-attachment:{id})"));
        uploads.push(id);
    }
    uploads.extend([first, empty]);
    let ask = Command::PromptSession {
        ticket,
        text: links,
        queued: true,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads,
    };
    let key = c.board().ticket(ticket).unwrap().short_key.clone();
    let dir = mesimon_daemon::attachments::directory(&h.paths, &key);
    std::fs::create_dir_all(dir.join(format!("{empty}.json"))).unwrap();
    err_containing(c.request(ask.clone()), "could not keep");
    for (id, _, ext, _) in typed {
        assert!(!dir.join(format!("{id}.{ext}")).exists());
        assert!(!dir.join(format!("{id}.json")).exists());
    }
    assert!(!dir.join(format!("{first}.bin")).exists());
    assert!(!dir.join(format!("{empty}.bin")).exists());
    std::fs::remove_dir(dir.join(format!("{empty}.json"))).unwrap();
    let r = c.request(ask);
    assert!(matches!(r, Response::Queued { .. }), "{r:?}");
    let pending = pending_of(&mut c, Some(ticket));
    assert!(
        pending.iter().any(|p| p
            .text
            .as_deref()
            .is_some_and(|s| s.contains(&first.to_string()) && s.contains(&empty.to_string()))),
        "the accepted follow-up carries both links"
    );
    // A file committed by the previous version has a .bin payload and no MIME field.
    let sidecar = dir.join(format!("{first}.json"));
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
    legacy["kind"].as_object_mut().unwrap().remove("mime_type").unwrap();
    std::fs::write(sidecar, legacy.to_string()).unwrap();
    drop(c);
    h.restart();
    let mut c = h.client("after restart");
    let mut shim = Shim::start(&h.paths.orch_sock(), sid);
    let read = |shim: &mut Shim, args| {
        let result = shim.call("read_attachment", args);
        assert_eq!(result["isError"], false, "{result}");
        serde_json::from_str::<serde_json::Value>(result["content"][0]["text"].as_str().unwrap())
            .unwrap()
    };
    for (id, name, ext, mime) in typed {
        let meta = read(&mut shim, json!({"attachment":id}));
        let payload = dir.join(format!("{id}.{ext}"));
        assert_eq!(meta["name"], name);
        assert_eq!(meta["mime_type"], mime);
        assert_eq!(meta["path"], json!(payload));
        assert_eq!(meta["bytes"], std::fs::metadata(payload).unwrap().len());
    }
    let meta = read(&mut shim, json!({"attachment":first}));
    assert_eq!(meta["path"], json!(dir.join(format!("{first}.bin"))));
    assert_eq!(meta["mime_type"], "application/octet-stream");
    assert_eq!(meta["name"], "sample.dat");
    assert_eq!(meta["bytes"], 4);
    assert_eq!(std::fs::read(meta["path"].as_str().unwrap()).unwrap(), b"abc\xff");
    let part = read(&mut shim, json!({"attachment":first,"offset":0,"limit":3}));
    assert_eq!(part["part"]["data"], "abc");
    let part = read(&mut shim, json!({"attachment":first,"offset":3}));
    assert_eq!(
        (part["part"]["data"].as_str(), part["part"]["encoding"].as_str()),
        (Some("/w=="), Some("base64"))
    );
    assert_eq!(read(&mut shim, json!({"attachment":empty,"offset":0}))["part"]["next_offset"], 0);
    shim.call_err("read_attachment", json!({"attachment":first,"limit":16385}));
    assert!(git(&wt, &["status", "--porcelain"]).trim().is_empty());
    let copy = match c.request(Command::DuplicateTicket { id: ticket }) {
        Response::Created { id, .. } => id,
        r => panic!("{r:?}"),
    };
    for cmd in [
        Command::ArchiveTicket { id: copy },
        Command::UnarchiveTicket { id: copy },
        Command::DeleteTicket { id: copy, discard_worktree: false },
        Command::RestoreTicket { id: copy },
    ] {
        assert!(matches!(c.request(cmd), Response::Ok));
    }
    let copy_key = c.board().ticket(copy).unwrap().short_key.clone();
    assert_eq!(
        mesimon_daemon::attachments::read(&h.paths, &copy_key, first).unwrap().1,
        b"abc\xff"
    );
}

#[test]
fn queued_start_keeps_its_file_and_words_across_daemon_restart() {
    let Some(h) = Harness::boot_with_env(
        "file-queued-start",
        Some(RECORDING_STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let mut c = h.client("queue a start");
    let holder = c.create_ticket("holder");
    let waiter = c.create_ticket("waiter");
    let session = c.spawn_claude(holder);
    h.prompt_turn(&mut c, session);
    let upload = match c.request(Command::UploadAttachment {
        file: Some(mesimon_core::attachment::FileUpload {
            ticket: waiter,
            name: "report.csv".into(),
        }),
        upload: None,
        offset: 0,
        data: "NDI=".into(),
        complete: true,
    }) {
        Response::AttachmentUploaded { upload } => upload,
        other => panic!("{other:?}"),
    };
    let words = format!("Read [report.csv](mesimon-attachment:{upload})");
    let response = c.request(Command::PromptSession {
        ticket: waiter,
        text: words.clone(),
        queued: true,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: vec![upload],
    });
    assert!(matches!(response, Response::Queued { .. }), "{response:?}");
    drop(c);
    h.restart();
    let mut c = h.client("restored start");
    assert!(pending_of(&mut c, Some(waiter)).iter().any(|p| p.text.as_ref() == Some(&words)));
    h.end_turn(&mut c, session);
    wait_until(Duration::from_secs(10), "restored start gets its own agent", || {
        c.board().live_agent(waiter).is_some()
    });
    let sid = c.board().live_agent(waiter).unwrap().id;
    let mut shim = Shim::start(&h.paths.orch_sock(), sid);
    let result = shim.call("read_attachment", json!({"attachment": upload, "offset": 0}));
    assert_eq!(result["isError"], false, "{result}");
    let file: serde_json::Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(file["part"]["data"], "42");
}
