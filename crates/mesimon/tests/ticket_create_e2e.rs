//! `mesimon ticket create` (T-693), end to end: a script on the machine
//! files a ticket on a board through the real binary, and the card it
//! prints the key of carries the column, the title, the description and
//! the tag it asked for. mesimon's half of a chat command or a launcher is
//! this subcommand alone, so this is the whole contract such a script has.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::io::Write;
use std::process::Stdio;

use mesimon_core::command::{Command, Response};

/// Run the subcommand; `(exit ok, stdout, stderr)`.
fn create(h: &Harness, args: &[&str], stdin: Option<&str>) -> (bool, String, String) {
    let mut cmd = h.command(&[&["ticket", "create"], args].concat());
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    if let Some(text) = stdin {
        child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_script_files_a_ticket_and_reads_its_key_back() {
    let Some(h) = Harness::boot("ticket_create", None) else { return };
    let mut c = h.client("ticket-create-e2e");
    let link = "https://example.slack.com/archives/C024BE91L/p1700000000000100";

    // The whole shape, from the repo as cwd: column (case aside), title, a
    // description and a tag the board's starter vocabulary carries.
    let (ok, out, err) = create(
        &h,
        &["--column", "todo", "--title", "  Flaky login test ", "--note", link, "--tag", "feature"],
        None,
    );
    assert!(ok, "create failed: {err}");
    let key = out.trim().to_string();
    let board = c.board();
    let t = board.tickets.iter().find(|t| t.short_key == key).expect("the printed key is a card");
    assert_eq!(t.column, "TODO", "the board's own spelling of the column");
    assert_eq!(t.title, "Flaky login test");
    assert_eq!(t.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["FEATURE"]);
    let note = t.notes.first().expect("the note is the description").id;
    match c.request(Command::ReadNote { ticket: t.id, note }) {
        Response::Note { text, .. } => assert_eq!(text, link),
        other => panic!("{other:?}"),
    }
    assert!(err.is_empty(), "nothing on stderr for a clean create: {err}");

    // `--note -` is stdin, and `--repo` names the board from anywhere.
    let elsewhere = h.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let mut cmd = h.command(&[
        "ticket",
        "create",
        "--repo",
        h.repo.to_str().unwrap(),
        "--column",
        "TODO",
        "--title",
        "From stdin",
        "--note",
        "-",
    ]);
    cmd.current_dir(&elsewhere).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"line one\nline two\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let board = c.board();
    let t = board.tickets.iter().find(|t| t.short_key == key).unwrap();
    assert_eq!(t.title, "From stdin");
    let note = t.notes.first().unwrap().id;
    match c.request(Command::ReadNote { ticket: t.id, note }) {
        Response::Note { text, .. } => assert_eq!(text, "line one\nline two\n"),
        other => panic!("{other:?}"),
    }
    assert_eq!(board.tickets.len(), 2);

    // Refusals file nothing and say what the board has.
    let (ok, out, err) = create(&h, &["--column", "LATER", "--title", "x"], None);
    assert!(!ok && out.is_empty());
    assert!(err.contains("no such column: LATER") && err.contains("TODO"), "{err}");
    let (ok, out, err) = create(&h, &["--column", "TODO", "--title", "x", "--tag", "URGENT"], None);
    assert!(!ok && out.is_empty());
    assert!(err.contains("no such tag: URGENT") && err.contains("FEATURE"), "{err}");
    let (ok, out, err) = create(&h, &["--title", "x"], None);
    assert!(!ok && out.is_empty());
    assert!(err.contains("--column is needed") && err.contains("usage:"), "{err}");
    assert_eq!(c.board().tickets.len(), 2, "a refusal files nothing");
}
