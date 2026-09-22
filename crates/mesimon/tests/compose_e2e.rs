//! The composer's mint (T-243) end to end against a real daemon: one
//! `CreateTicketWithNote` carries the title, the tags and the description,
//! and the ticket exists with all of it or not at all — two tags on one
//! group, a note past its limit, are refused and leave no ticket behind; a
//! blank text is a ticket with no note; a name the registry does not hold
//! is registered on the fly, the way the picker's `SetTag` does.
//!
//! No tmux and no agent — the mint is pure board state, so this one runs
//! everywhere.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use mesimon_core::board::{TagRef, NOTE_MAX_BYTES};
use mesimon_core::command::{Command, Response};

mod common;
use common::*;

fn tag(group: u8, name: &str) -> TagRef {
    TagRef { group, name: name.into() }
}

fn mint(title: &str, text: &str, tags: Vec<TagRef>) -> Command {
    Command::CreateTicketWithNote {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        text: text.into(),
        uploads: Vec::new(),
        tags,
    }
}

#[test]
fn the_composer_mints_a_ticket_whole_or_not_at_all() {
    let fixture = common::TestFixture::new("compose");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    fixture.set_env("MESIMON_NO_TAG_SEED", "1");
    let daemon = fixture.daemon(&repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "compose".into()
        }),
        Response::Hello { .. }
    ));

    // ---- everything at once -------------------------------------------------
    let id = match c.request(mint(
        "all of it",
        "the brief\n\nwith a second line",
        vec![tag(1, "BUG"), tag(2, "STAGING")],
    )) {
        Response::Created { id, started } => {
            assert!(!started, "TODO does not auto-run");
            id
        }
        other => panic!("mint: {other:?}"),
    };
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(id).expect("the ticket");
    assert_eq!(t.title, "all of it");
    assert_eq!(t.tag_in(1).unwrap().name, "BUG");
    assert_eq!(t.tag_in(2).unwrap().name, "STAGING");
    assert_eq!(t.notes.len(), 1, "the description is notes[0]: {:?}", t.notes);
    let note = &t.notes[0];
    assert_eq!(note.created_by, "local");
    assert_eq!(note.rev, 1);
    match c.request(Command::ReadNote { ticket: id, note: note.id }) {
        Response::Note { text, .. } => assert_eq!(text, "the brief\n\nwith a second line"),
        other => panic!("read: {other:?}"),
    }
    // The names were registered on the fly: the vocabulary offers them now.
    assert_eq!(board.group_tags(1), vec!["BUG"]);
    assert_eq!(board.group_tags(2), vec!["STAGING"]);
    // On disk, as the composer's three trips used to leave it.
    let key = t.short_key.clone();
    let ticket_dir = repo.join(".mesimon/board/tickets").join(&key);
    assert!(ticket_dir.join("ticket.toml").exists());
    assert!(ticket_dir.join("notes").join(format!("{}.md", note.id)).exists());

    // ---- refusals leave nothing --------------------------------------------
    let tickets_before = board.tickets.len();
    let next_key = board.next_key;
    err_containing(
        c.request(mint("two on one axis", "", vec![tag(1, "BUG"), tag(1, "REGR")])),
        "one tag per group",
    );
    let long = "x".repeat(NOTE_MAX_BYTES + 1);
    err_containing(c.request(mint("too long", &long, vec![])), "the limit is");
    err_containing(c.request(mint("   ", "a brief", vec![])), "needs a title");
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.tickets.len(), tickets_before, "no half-made ticket: {:?}", board.tickets);
    assert_eq!(board.next_key, next_key, "no key spent on a refusal");
    // Registration is the picker's own gesture and outlives the refused mint,
    // the way `RegisterTag` then a cancelled composer would leave it.
    assert_eq!(board.group_tags(1), vec!["BUG", "REGR"]);
    let entries: Vec<_> = std::fs::read_dir(repo.join(".mesimon/board/tickets"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(entries, vec![key.clone()], "no ticket directory for a refused mint");

    // ---- a blank text is a ticket with no note ------------------------------
    let bare = match c.request(mint("title only", "  \n", vec![])) {
        Response::Created { id, .. } => id,
        other => panic!("mint: {other:?}"),
    };
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(bare).expect("the ticket");
    assert!(t.notes.is_empty(), "blank means no note: {:?}", t.notes);
    assert!(t.tags.is_empty());

    // ---- the thin form still mints a title alone -----------------------------
    assert!(matches!(
        c.request(Command::CreateTicket {
            column: "TODO".into(),
            title: "thin".into(),
            workspace: None,
        }),
        Response::Created { .. }
    ));

    drop(c);
    drop(daemon);
}
