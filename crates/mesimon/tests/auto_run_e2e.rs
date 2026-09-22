//! "Start claude on creation" (T-117) end to end: a ticket a PERSON creates
//! in an `auto_run` column gets claude started on its title, submitted, with
//! the description — written a beat AFTER `Created`, the composer's order —
//! pasted under it; the feed says `auto_run_started`; and the three roads
//! that must NOT fire it (an agent's `create_ticket`, a hand move into the
//! column, an unarchive into it) spawn nothing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;

mod common;
use common::*;

#[test]
fn a_ticket_created_in_an_auto_run_column_starts_claude_on_its_brief() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot("autorun", Some(STUB)) else { return };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("autorun");

    // Flip TODO's switch — the whole struct, off the snapshot.
    let mut s = c.board().column("TODO").unwrap().settings.clone();
    s.auto_run = true;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "TODO".into(), settings: s }),
        Response::Ok
    ));

    // ---- the person's composer: Created says a claude started -------------
    let (ticket, started) = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "auto me".into(),
        workspace: None,
    }) {
        Response::Created { id, started } => (id, started),
        other => panic!("create: {other:?}"),
    };
    assert!(started, "the column starts a claude on creation");
    let board = c.board();
    let rec = board.sessions.iter().find(|s| s.ticket == ticket).expect("a session was spawned");
    assert_eq!(rec.kind, SessionKind::Claude);
    assert!(rec.pending_submit, "the Enter is owed — the composed road");
    let sid = rec.id;
    // The description lands AFTER the mint, as the composer sends it.
    assert!(matches!(
        c.request(Command::WriteNote {
            ticket,
            note: None,
            text: "the brief, mesimon-autorun-77".into()
        }),
        Response::NoteWritten { .. }
    ));

    let pane_alive = |sock: &std::path::Path| {
        tmux(sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false)
    };
    wait_until(Duration::from_secs(15), "the stub agent's pane", || pane_alive(&tmux_sock));
    std::thread::sleep(Duration::from_millis(500));
    hook_send_with(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t-autorun.jsonl","cwd":"/tmp"}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let received = loop {
        let text = std::fs::read_to_string(&got).unwrap_or_default();
        if text.contains("mesimon-autorun-77") {
            break text;
        }
        assert!(Instant::now() < deadline, "the brief never reached the agent: {text:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let lines: Vec<&str> = received.lines().collect();
    let title_at = lines.iter().position(|l| l.trim() == "auto me").expect("the title line");
    let brief_at = lines.iter().position(|l| l.contains("mesimon-autorun-77")).unwrap();
    assert!(title_at < brief_at, "title, then the brief written after the mint: {received:?}");
    hook_send_with(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{}"#);
    wait_until(Duration::from_secs(3), "the ack", || {
        c.board().sessions.iter().any(|s| s.id == sid && !s.pending_submit)
    });

    // The feed names the automation.
    let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
    assert!(feed.contains("\"auto_run_started\""), "{feed}");

    // ---- the roads that must not fire it ----------------------------------
    let sessions = |c: &mut TestClient| c.board().sessions.len();
    let before = sessions(&mut c);
    // An agent's create_ticket into the column: a card, no session.
    match c.send(
        Principal::Agent { session: sid },
        Command::AgentCreateTicket {
            title: "filed by the agent".into(),
            column: Some("TODO".into()),
            description: None,
            tags: vec![],
            idempotency_key: None,
        },
    ) {
        Response::AgentCreated { .. } => {}
        other => panic!("agent create: {other:?}"),
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(sessions(&mut c), before, "an agent's create_ticket starts nothing");
    // A hand move into the column.
    let moved = match c.request(Command::CreateTicket {
        column: "REVIEW".into(),
        title: "moved in".into(),
        workspace: None,
    }) {
        Response::Created { id, started } => {
            assert!(!started, "REVIEW does not auto-run");
            id
        }
        other => panic!("create: {other:?}"),
    };
    assert!(matches!(
        c.request(Command::MoveTicket { id: moved, column: "TODO".into(), before: None }),
        Response::Ok
    ));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(sessions(&mut c), before, "a move into the column starts nothing");
    // An unarchive into it.
    assert!(matches!(c.request(Command::ArchiveTicket { id: moved }), Response::Ok));
    assert!(matches!(c.request(Command::UnarchiveTicket { id: moved }), Response::Ok));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(sessions(&mut c), before, "an unarchive into the column starts nothing");
    // And a second creation while the seat is… a NEW ticket has its own
    // seat, so a second created ticket starts a second claude of its own.
    let again = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "another".into(),
        workspace: None,
    }) {
        Response::Created { started, .. } => started,
        other => panic!("create: {other:?}"),
    };
    assert!(again);
    assert_eq!(sessions(&mut c), before + 1);
}

/// The composer's one-command mint (T-243): the brief rides
/// `CreateTicketWithNote`, so the column's auto-run spawns onto a ticket
/// that already carries it — title, then the brief, in the pane, with no
/// `WriteNote` after `Created`.
#[test]
fn a_ticket_minted_with_its_brief_in_one_command_starts_claude_on_it() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot("autorun1", Some(STUB)) else { return };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("autorun1");

    let mut s = c.board().column("TODO").unwrap().settings.clone();
    s.auto_run = true;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "TODO".into(), settings: s }),
        Response::Ok
    ));

    let (ticket, started) = match c.request(Command::CreateTicketWithNote {
        column: "TODO".into(),
        title: "auto whole".into(),
        workspace: None,
        text: "the brief, mesimon-autorun-78".into(),
        uploads: Vec::new(),
        tags: Vec::new(),
    }) {
        Response::Created { id, started } => (id, started),
        other => panic!("create: {other:?}"),
    };
    assert!(started, "the column starts a claude on creation");
    let board = c.board();
    assert_eq!(board.ticket(ticket).unwrap().notes.len(), 1, "the brief was on the card at spawn");
    let rec = board.sessions.iter().find(|s| s.ticket == ticket).expect("a session was spawned");
    assert_eq!(rec.kind, SessionKind::Claude);
    assert!(rec.pending_submit, "the Enter is owed — the composed road");
    let sid = rec.id;

    let pane_alive = |sock: &std::path::Path| {
        tmux(sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false)
    };
    wait_until(Duration::from_secs(15), "the stub agent's pane", || pane_alive(&tmux_sock));
    std::thread::sleep(Duration::from_millis(500));
    hook_send_with(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t-autorun1.jsonl","cwd":"/tmp"}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let received = loop {
        let text = std::fs::read_to_string(&got).unwrap_or_default();
        if text.contains("mesimon-autorun-78") {
            break text;
        }
        assert!(Instant::now() < deadline, "the brief never reached the agent: {text:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let lines: Vec<&str> = received.lines().collect();
    let title_at = lines.iter().position(|l| l.trim() == "auto whole").expect("the title line");
    let brief_at = lines.iter().position(|l| l.contains("mesimon-autorun-78")).unwrap();
    assert!(title_at < brief_at, "title, then the brief it was minted with: {received:?}");
    hook_send_with(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{}"#);
    wait_until(Duration::from_secs(3), "the ack", || {
        c.board().sessions.iter().any(|s| s.id == sid && !s.pending_submit)
    });
    let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
    assert!(feed.contains("\"create_ticket_with_note\""), "one mint line: {feed}");
    assert!(!feed.contains("\"write_note\""), "no second trip for the brief: {feed}");
}
