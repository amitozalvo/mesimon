//! Notes end to end (2026-09-02): a markdown file under the ticket, its
//! metadata in `ticket.toml`, the agent's two tools through the real shim,
//! and the "tell claude" paste landing in a live pane.
//!
//! The stub is `prompt_e2e`'s read loop: a line in `got.txt` proves both
//! delivery and the separate Enter.

#![allow(clippy::unwrap_used)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;
use serde_json::{json, Value};

#[test]
fn notes_are_files_with_authors_and_the_agent_reads_and_writes_them() {
    const STUB: &str = "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" \
                        >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot("notes", Some(STUB)) else { return };
    let got = h.dir.join("got.txt");
    let sock = h.paths.orch_sock();
    let mut c = h.client("notes");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "noted".into(),
        workspace: None,
        tier: None,
    });
    let board = c.board();
    let ticket = board.tickets[0].id;
    let key = board.tickets[0].short_key.clone();
    let tdir = h.repo.join(".mesimon/board/tickets").join(&key);

    // ---- a person writes the description ----------------------------------
    let desc = match c.request(Command::WriteNote {
        ticket,
        note: None,
        text: "# Why this\n\nBecause the peek\tlied.\u{202e}\n".into(),
    }) {
        Response::NoteWritten { note: Some(id) } => id,
        other => panic!("write failed: {other:?}"),
    };
    let file = tdir.join("notes").join(format!("{desc}.md"));
    let body = std::fs::read_to_string(&file).expect("the note is a file");
    // Sanitized by subtraction: the tab became a space, the bidi mark went.
    assert_eq!(body, "# Why this\n\nBecause the peek lied.\n");
    let toml = std::fs::read_to_string(tdir.join("ticket.toml")).unwrap();
    // The stamp is whatever this build writes (it was 2 when notes landed and
    // 3 since the snooze), never a number a later bump has to chase.
    let stamp = format!("schema_version = {}", mesimon_daemon::store::TICKET_SCHEMA);
    assert!(toml.contains(&stamp), "{toml}");
    assert!(toml.contains("[[notes]]"), "{toml}");
    assert!(toml.contains("created_by = \"local\""), "{toml}");
    assert!(toml.contains("name = \"Why this\""), "{toml}");
    let t = c.board();
    let meta = t.tickets[0].description().expect("notes[0] is the description").clone();
    assert_eq!(meta.id, desc);
    assert_eq!(meta.rev, 1);
    assert_eq!(meta.edited_by, "local");

    // Reading it back over the wire, and a foreign id is nothing.
    match c.request(Command::ReadNote { ticket, note: desc }) {
        Response::Note { text, meta } => {
            assert_eq!(text, body);
            assert_eq!(meta.name, "Why this");
        }
        other => panic!("{other:?}"),
    }
    err_containing(
        c.request(Command::ReadNote { ticket, note: ulid::Ulid::nil() }),
        "no such note",
    );
    // Blank on a fresh note is nothing to save; the agent forms need an
    // agent.
    err_containing(
        c.request(Command::WriteNote { ticket, note: None, text: "  \n".into() }),
        "nothing to save",
    );
    err_containing(
        c.request(Command::AgentWriteNote { note: None, text: "x".into(), key: None }),
        "agent principal",
    );

    // ---- no agent yet: telling one is refused, not swallowed -------------
    err_containing(c.request(Command::NoteToAgent { ticket, note: desc }), "no live agent");

    // ---- the agent, through the real shim --------------------------------
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let mut shim = Shim::start(&sock, sid);
    shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    let tools = shim.rpc("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"read_note") && names.contains(&"write_note"), "{names:?}");

    // get_ticket carries the description and lists the note.
    let t: Value = serde_json::from_str(&shim.call_ok_text("get_ticket", json!({}))).unwrap();
    assert_eq!(t["description"], body);
    assert_eq!(t["notes"][0]["id"], desc.to_string());
    assert_eq!(t["notes"][0]["name"], "Why this");
    assert_eq!(t["notes"][0]["by"], "local");

    // read_note is the body itself, not JSON around it.
    assert_eq!(shim.call_ok_text("read_note", json!({"note": desc.to_string()})), body);
    let refused = shim.call_err("read_note", json!({"note": ulid::Ulid::nil().to_string()}));
    assert!(refused.contains("no such note"), "{refused}");

    // write_note creates, stamped with the session.
    let r: Value = serde_json::from_str(
        &shim.call_ok_text("write_note", json!({"text": "## Plan\n\n1. look"})),
    )
    .unwrap();
    let second: ulid::Ulid = r["note"].as_str().unwrap().parse().unwrap();
    let b = c.board();
    let notes = &b.tickets[0].notes;
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[1].id, second);
    assert_eq!(notes[1].name, "Plan");
    assert_eq!(notes[1].created_by, format!("agent:{sid}"));
    assert!(tdir.join("notes").join(format!("{second}.md")).is_file());

    // …and replaces, bumping the revision and the author.
    shim.call_ok_text(
        "write_note",
        json!({"note": desc.to_string(), "text": "# Why this, really\n\nnew"}),
    );
    let b = c.board();
    let d = b.tickets[0].description().unwrap();
    assert_eq!(d.id, desc, "the description keeps its id");
    assert_eq!(d.rev, 2);
    assert_eq!(d.name, "Why this, really");
    assert_eq!(d.edited_by, format!("agent:{sid}"));
    assert_eq!(d.created_by, "local", "creation is not rewritten");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "# Why this, really\n\nnew");

    // Empty text deletes; the file goes with the meta; the second note is
    // now the description.
    let r: Value = serde_json::from_str(
        &shim.call_ok_text("write_note", json!({"note": desc.to_string(), "text": ""})),
    )
    .unwrap();
    assert_eq!(r["deleted"], true);
    assert!(!file.exists());
    let b = c.board();
    assert_eq!(b.tickets[0].notes.len(), 1);
    assert_eq!(b.tickets[0].description().unwrap().id, second);
    // A note the agent cannot see is refused by name, not found by id.
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "other".into(),
        workspace: None,
        tier: None,
    });
    let other = c.board().tickets.iter().find(|t| t.title == "other").unwrap().id;
    let foreign =
        match c.request(Command::WriteNote { ticket: other, note: None, text: "mine".into() }) {
            Response::NoteWritten { note: Some(id) } => id,
            other => panic!("{other:?}"),
        };
    let refused =
        shim.call_err("write_note", json!({"note": foreign.to_string(), "text": "stolen"}));
    assert!(refused.contains("no such note"), "{refused}");
    // The feed flushes on the 250 ms wheel; both writers are named, never
    // their text.
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    wait_until(Duration::from_secs(5), "the feed to name the writes", || {
        let feed = std::fs::read_to_string(&feed_path).unwrap_or_default();
        feed.lines().any(|l| l.contains("\"actor\":\"agent\"") && l.contains("write_note"))
            && feed.lines().any(|l| l.contains("\"actor\":\"local\"") && l.contains("write_note"))
    });
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("Plan") && !feed.contains("Why"), "never the text: {feed}");

    // ---- tell claude ------------------------------------------------------
    // Wait for the stub to be reading before pasting.
    let tmux_sock = h.paths.tmux_sock();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let alive = tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);
        if alive {
            break;
        }
        assert!(Instant::now() < deadline, "the stub agent never got a pane");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500));
    match c.request(Command::NoteToAgent { ticket, note: second }) {
        Response::Ok => {}
        other => panic!("tell claude failed: {other:?}"),
    }
    wait_until(Duration::from_secs(10), "the note sentence to land", || {
        std::fs::read_to_string(&got).unwrap_or_default().contains(&second.to_string())
    });
    let line = std::fs::read_to_string(&got).unwrap();
    assert!(line.contains("Note \"Plan\""), "{line}");
    assert!(line.contains("read_note"), "{line}");
    // An agent may not send it.
    err_containing(
        c.send(Principal::Agent { session: sid }, Command::NoteToAgent { ticket, note: second }),
        "not available",
    );
}

/// An approved plan is the agent's note on the ticket (2026-09-03). The
/// `PostToolUse` frame `ExitPlanMode` fires once the user approves carries the
/// plan whole, through the real hook binary, and the daemon writes it as a
/// note stamped with the session — one per session, replaced on a re-plan,
/// minted afresh after the user deletes it. The frames before the approval,
/// and another tool's `plan` key, write nothing.
#[test]
fn an_approved_plan_is_the_agents_note_on_the_ticket() {
    const STUB: &str = "#!/bin/sh\nexec sleep 60\n";
    let Some(h) = Harness::boot("plan-note", Some(STUB)) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("plan-note");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "planned".into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets[0].id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let session = sid.to_string();
    let frame = |plan: &str| {
        json!({
            "tool_name": "ExitPlanMode",
            "tool_input": { "plan": plan },
            "tool_response": "User has approved your plan. You can now start coding.",
        })
        .to_string()
    };
    let plan_a = "# Plan A\n\n1. look\n2. leap\n";

    // The approval dialog's frames come BEFORE the approval: nothing yet.
    hook_send(&hook_sock, &session, "PreToolUse", &frame(plan_a));
    hook_send(&hook_sock, &session, "PermissionRequest", &frame(plan_a));
    // ...and the approval itself is the note.
    hook_send(&hook_sock, &session, "PostToolUse", &frame(plan_a));
    wait_until(Duration::from_secs(10), "the plan to land as a note", || {
        c.board().tickets[0].notes.len() == 1
    });
    let notes = c.board().tickets[0].notes.clone();
    let first = notes[0].clone();
    assert_eq!(first.name, "Plan A");
    assert_eq!(first.created_by, format!("agent:{sid}"));
    assert_eq!(first.rev, 1);
    match c.request(Command::ReadNote { ticket, note: first.id }) {
        Response::Note { text, .. } => assert_eq!(text, plan_a),
        other => panic!("{other:?}"),
    }
    // The record remembers which note is the plan's, and it is persisted.
    let sessions = h.paths.state_dir.join("sessions.json");
    wait_until(Duration::from_secs(5), "the plan note id to persist", || {
        std::fs::read_to_string(&sessions).unwrap_or_default().contains(&first.id.to_string())
    });

    // A re-plan is a revision of the same note, not a second note; another
    // tool carrying a `plan` key is not a plan.
    hook_send(
        &hook_sock,
        &session,
        "PostToolUse",
        &json!({"tool_name": "Bash", "tool_input": {"plan": "# Not a plan"}}).to_string(),
    );
    hook_send(&hook_sock, &session, "PostToolUse", &frame("# Plan B\n\nagain\n"));
    wait_until(Duration::from_secs(10), "the re-plan to revise the note", || {
        c.board().tickets[0].notes.first().is_some_and(|n| n.rev == 2)
    });
    let notes = c.board().tickets[0].notes.clone();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].id, first.id);
    assert_eq!(notes[0].name, "Plan B");
    assert_eq!(notes[0].edited_by, format!("agent:{sid}"));

    // The user deletes the note; the next approval mints a fresh one rather
    // than resurrecting the old id.
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: Some(first.id), text: String::new() }),
        Response::NoteWritten { note: None }
    ));
    assert!(c.board().tickets[0].notes.is_empty());
    hook_send(&hook_sock, &session, "PostToolUse", &frame("# Plan C\n"));
    wait_until(Duration::from_secs(10), "the third plan to land", || {
        c.board().tickets[0].notes.len() == 1
    });
    let notes = c.board().tickets[0].notes.clone();
    assert_ne!(notes[0].id, first.id);
    assert_eq!(notes[0].name, "Plan C");
    assert_eq!(notes[0].rev, 1);

    // The feed names the write with the agent as actor, never the plan.
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    wait_until(Duration::from_secs(5), "the feed to name the plan note", || {
        let feed = std::fs::read_to_string(&feed_path).unwrap_or_default();
        feed.lines().any(|l| l.contains("\"actor\":\"agent\"") && l.contains("plan_note"))
    });
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("Plan A") && !feed.contains("leap"), "never the text: {feed}");
}

/// T-328: a note past 32 KiB is refused, never cut. Exactly the limit is
/// stored whole; one byte over is refused with both numbers in the words and
/// leaves the board as it was — no new file on a create, the old text and
/// revision on a replace — and the boundary is bytes, so a two-byte script
/// is refused at half the characters.
#[test]
fn a_note_past_the_limit_is_refused_and_the_existing_note_stays_whole() {
    use mesimon_core::board::NOTE_MAX_BYTES;
    let Some(h) = Harness::boot("notesize", None) else { return };
    let mut c = h.client("notesize");
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "sized".into(),
        workspace: None,
        tier: None,
    });
    let board = c.board();
    let ticket = board.tickets[0].id;
    let key = board.tickets[0].short_key.clone();
    let notes_dir = h.repo.join(".mesimon/board/tickets").join(&key).join("notes");

    // Exactly the limit fits, and lands whole.
    let exact = format!("# Exact\n{}", "x".repeat(NOTE_MAX_BYTES - "# Exact\n".len()));
    assert_eq!(exact.len(), NOTE_MAX_BYTES);
    let id = match c.request(Command::WriteNote { ticket, note: None, text: exact.clone() }) {
        Response::NoteWritten { note: Some(id) } => id,
        other => panic!("exactly the limit is accepted: {other:?}"),
    };
    let file = notes_dir.join(format!("{id}.md"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), exact);

    // One byte over: refused, with the submitted size and the limit named,
    // and no second file appears.
    let over = format!("{exact}y");
    let Response::Err { message } =
        c.request(Command::WriteNote { ticket, note: None, text: over.clone() })
    else {
        panic!("one byte over is refused")
    };
    assert!(message.contains(&format!("note is {} bytes", NOTE_MAX_BYTES + 1)), "{message}");
    assert!(message.contains(&format!("limit is {NOTE_MAX_BYTES} bytes")), "{message}");
    assert_eq!(std::fs::read_dir(&notes_dir).unwrap().count(), 1);
    assert_eq!(c.board().tickets[0].notes.len(), 1);

    // A refused replacement leaves the existing note whole, revision and all.
    let before = c.board().tickets[0].notes[0].clone();
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: Some(id), text: over }),
        Response::Err { .. }
    ));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), exact);
    let after = c.board().tickets[0].notes[0].clone();
    assert_eq!(after.rev, before.rev);
    assert_eq!(after.edited_at, before.edited_at);

    // Bytes, not characters: Hebrew is two bytes a letter.
    let hebrew = "ש".repeat(NOTE_MAX_BYTES / 2 + 1);
    let Response::Err { message } =
        c.request(Command::WriteNote { ticket, note: None, text: hebrew })
    else {
        panic!("a multi-byte note over the limit is refused")
    };
    assert!(message.contains(&format!("note is {} bytes", NOTE_MAX_BYTES + 2)), "{message}");
    assert_eq!(c.board().tickets[0].notes.len(), 1);
}
