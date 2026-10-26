//! The brief travels with the title (T-224, 2026-09-05): a composed spawn
//! (`submit_prompt`) on a ticket WITH a description pastes that description
//! under the typed title before the owed Enter, so the agent's first prompt is
//! the whole brief — and the record says so (`ticket_read`). A `get_ticket`
//! from the agent stamps the same flag; a ticket without a description
//! submits the title alone, as before. And the agent brief (`brief::TEXT`)
//! rides the spawn's argv exactly while the board's switch is on.
//!
//! The stub agent appends every line it reads to a file (prompt_e2e's shape):
//! `read` only returns on a newline, so a line in that file proves the text
//! arrived AND that the Enter was pressed separately afterwards.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;

const BRIEF: &str = "## Brief\n\nmesimon-brief-77 the description travels under the title\n\n\
                     - and so does its second paragraph, mesimon-brief-78";

/// A description of about `bytes`: numbered paragraphs of plain prose, and
/// the one fact the test looks for in its LAST paragraph (T-896).
fn long_brief(bytes: usize, fact: &str) -> String {
    let mut body =
        String::from("## Brief\n\nmesimon-long-brief: the fact is in the last paragraph.\n\n");
    let mut n = 0;
    while body.len() < bytes {
        n += 1;
        body.push_str(&format!(
            "{n}. A paragraph of filler for the long-brief test, with \"quotes\", an \
             apostrophe's turn, a `backtick` and the number {n} so no two lines read alike.\n\n"
        ));
    }
    body.push_str(fact);
    body
}

/// A long description reaches the agent whole (T-896): a 12 KiB brief with
/// its one fact in the last paragraph is the agent's whole first prompt,
/// under the title, on the paste road here and on the mod's `submit` in the
/// suite's mod pass (the stand-in engine pastes what the mod was handed).
/// `get_ticket` carries the same 12 KiB whole and says nothing of a cut;
/// a brief past `mcp::DESCRIPTION_MAX_BYTES` rides as a head ending in `…`
/// with `description_cut` saying how long the whole is and which note
/// `read_note` reads — never a bare `…` a reader takes for the end.
#[test]
fn a_long_description_reaches_the_agent_whole_as_its_first_prompt() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\ncat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot("brief_long", Some(STUB)) else { return };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("brief_long");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "brief me at length".into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.first().expect("ticket").id;
    let body = long_brief(12 * 1024, "The code word is persimmon-9081.");
    assert!(body.len() > 12 * 1024 && body.len() < mesimon_core::mcp::DESCRIPTION_MAX_BYTES);
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: None, text: body.clone(), rev: None }),
        Response::NoteWritten { .. }
    ));

    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
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
        r#"{"session_id":"x","transcript_path":"/tmp/t-brief-long.jsonl","cwd":"/tmp"}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let received = loop {
        let text = std::fs::read_to_string(&got).unwrap_or_default();
        if text.contains("persimmon-9081") {
            break text;
        }
        assert!(Instant::now() < deadline, "the last paragraph never reached the agent: {text:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    // Every line of the body arrived, in order, under the title: the whole
    // brief, not its first 4 KiB.
    let title_at = received.find("brief me at length").expect("the title line");
    let mut from = title_at;
    for line in body.lines().filter(|l| !l.is_empty()) {
        let at = received[from..]
            .find(line)
            .unwrap_or_else(|| panic!("missing or out of order: {line:?}\n{received}"));
        from += at + line.len();
    }
    hook_send_with(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{}"#);
    wait_until(Duration::from_secs(3), "the ack", || {
        c.board().sessions.iter().any(|s| s.id == sid && !s.pending_submit)
    });
    let _ = c.request(Command::KillSession { id: sid });

    // ---- get_ticket: the same 12 KiB whole, and a longer one said cut ------
    let plain = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("plain spawn failed: {other:?}"),
    };
    let view = |c: &mut TestClient| match c
        .send(Principal::Agent { session: plain }, Command::AgentGetTicket { full: false })
    {
        Response::AgentTicket { ticket: view } => view,
        other => panic!("get_ticket failed: {other:?}"),
    };
    let v = view(&mut c);
    assert_eq!(v.description.as_deref(), Some(body.as_str()), "12 KiB rides get_ticket whole");
    assert_eq!(v.description_cut, None);

    let note = v.notes[0].id;
    let longer = long_brief(20 * 1024, "The code word is quince-4410.");
    assert!(longer.len() > mesimon_core::mcp::DESCRIPTION_MAX_BYTES);
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: Some(note), text: longer.clone(), rev: None }),
        Response::NoteWritten { .. }
    ));
    let v = view(&mut c);
    let head = v.description.expect("a head");
    assert!(head.ends_with('…'), "{}", &head[head.len() - 20..]);
    assert_eq!(head.trim_end_matches('…').len(), mesimon_core::mcp::DESCRIPTION_MAX_BYTES);
    assert!(longer.starts_with(head.trim_end_matches('…')), "the head is the brief's own start");
    let cut = v.description_cut.expect("said");
    assert!(cut.contains(&format!("is {} bytes", longer.len())), "{cut}");
    assert!(cut.contains(&format!("read_note with note {note}")), "{cut}");
    // And read_note returns the whole of it, fact and all.
    match c.send(Principal::Agent { session: plain }, Command::AgentReadNote { note, key: None }) {
        Response::Note { text, .. } => assert_eq!(text, longer),
        other => panic!("read_note failed: {other:?}"),
    }
    let _ = c.request(Command::KillSession { id: plain });
}

#[test]
fn a_composed_spawn_submits_the_description_under_the_title() {
    // `stty -icanon`: the paste is longer than a canonical tty's 1 KiB line
    // would be in real use, and Claude sets raw mode; the stub stands in.
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\ncat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot("brief", Some(STUB)) else { return };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("brief");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "brief me".into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.first().expect("ticket").id;
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: None, text: BRIEF.into(), rev: None }),
        Response::NoteWritten { .. }
    ));

    // ---- the composed road: title typed, description parked ---------------
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let rec = c.board().sessions.into_iter().find(|s| s.id == sid).unwrap();
    assert!(rec.pending_submit, "the Enter is owed");
    assert!(!rec.ticket_read, "nothing has been read before the pane exists");

    // Wait for the stub to be reading, as prompt_e2e does, so the paste lands
    // on a live reader and not in the pty buffer ahead of the exec.
    let pane_alive = |sock: &std::path::Path| {
        tmux(sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false)
    };
    wait_until(Duration::from_secs(15), "the stub agent's pane", || pane_alive(&tmux_sock));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        std::fs::read_to_string(&got).unwrap_or_default().is_empty(),
        "nothing may be submitted before SessionStart"
    );

    // The edge starts the clock; the first tick pastes the brief and presses.
    hook_send_with(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t-brief.jsonl","cwd":"/tmp"}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let received = loop {
        let text = std::fs::read_to_string(&got).unwrap_or_default();
        if text.contains("mesimon-brief-78") {
            break text;
        }
        assert!(Instant::now() < deadline, "the description never reached the agent: {text:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    // Title first, then the description, whole — the paste is one bracketed
    // paste of the body under the title that was already in the box.
    let lines: Vec<&str> = received.lines().collect();
    let title_at = lines.iter().position(|l| l.trim() == "brief me").expect("the title line");
    let brief_at = lines.iter().position(|l| l.contains("mesimon-brief-77")).expect("the brief");
    assert!(title_at < brief_at, "the title leads the brief: {received:?}");
    assert!(received.contains("## Brief"), "the body travelled whole: {received:?}");
    assert!(received.contains("mesimon-brief-78"), "the body travelled whole: {received:?}");

    // The record says the ticket was read — by the paste, no tool involved.
    wait_until(Duration::from_secs(3), "ticket_read after the paste", || {
        c.board().sessions.iter().any(|s| s.id == sid && s.ticket_read)
    });
    // The ack stops the pressing (the stub never acks on its own).
    hook_send_with(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{}"#);
    wait_until(Duration::from_secs(3), "the ack", || {
        c.board().sessions.iter().any(|s| s.id == sid && !s.pending_submit)
    });
    let _ = c.request(Command::KillSession { id: sid });

    // ---- the tool road: a plain spawn reads nothing until get_ticket -------
    let plain = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("plain spawn failed: {other:?}"),
    };
    let rec = c.board().sessions.into_iter().find(|s| s.id == plain).unwrap();
    assert!(!rec.pending_submit && !rec.ticket_read, "a plain spawn owes and reads nothing");
    match c.send(Principal::Agent { session: plain }, Command::AgentGetTicket { full: false }) {
        Response::AgentTicket { ticket: view } => {
            assert_eq!(view.description.as_deref(), Some(BRIEF), "get_ticket carries the brief");
        }
        other => panic!("get_ticket failed: {other:?}"),
    }
    let rec = c.board().sessions.into_iter().find(|s| s.id == plain).unwrap();
    assert!(rec.ticket_read, "get_ticket stamps the record");
    let _ = c.request(Command::KillSession { id: plain });

    // ---- no description: the title alone, as before -----------------------
    // With the agent brief switched on for this spawn: the flag and the text
    // ride the argv, verbatim, right after the tools the text names.
    assert!(matches!(c.request(Command::SetSystemPrompt { on: true }), Response::Ok));
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "bare title".into(),
        workspace: None,
        tier: None,
    });
    let bare = c.board().tickets.into_iter().find(|t| t.title == "bare title").unwrap().id;
    let bsid = match c.request(Command::SpawnSession {
        ticket: bare,
        kind: SessionKind::Claude,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("bare spawn failed: {other:?}"),
    };
    {
        use mesimon_core::brief;
        let rec = c.board().sessions.into_iter().find(|s| s.id == bsid).unwrap();
        let at = rec.argv.iter().position(|a| a == brief::FLAG).expect("the brief's flag");
        assert_eq!(rec.argv[at + 1], brief::TEXT, "the text is the argv value, verbatim");
        // The brief follows the tools it names: the blob on the hook set's
        // road; on the mod's the mod registers them (T-577) and the brief
        // still rides argv.
        if test_road() == "hooks" {
            let tools = rec.argv.iter().position(|a| a == "--mcp-config").expect("--mcp-config");
            assert!(tools < at, "the brief follows the tools it names: {:?}", rec.argv);
        } else {
            assert!(!rec.argv.iter().any(|a| a == "--mcp-config"), "{:?}", rec.argv);
        }
        assert!(!rec.argv.iter().any(|a| a == "--system-prompt"), "append, never replace");
    }
    std::thread::sleep(Duration::from_millis(700));
    hook_send_with(
        &hook_sock,
        &bsid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"y","transcript_path":"/tmp/t-bare.jsonl","cwd":"/tmp"}"#,
    );
    wait_until(Duration::from_secs(15), "the bare title", || {
        std::fs::read_to_string(&got).unwrap_or_default().contains("bare title")
    });
    let rec = c.board().sessions.into_iter().find(|s| s.id == bsid).unwrap();
    assert!(!rec.ticket_read, "no description means nothing was read");
    hook_send_with(&hook_sock, &bsid.to_string(), "UserPromptSubmit", None, r#"{}"#);
    let _ = c.request(Command::KillSession { id: bsid });

    // ---- the brief is inert beside no tools, and gone when switched off ----
    assert!(matches!(c.request(Command::SetMcpTools { on: false }), Response::Ok));
    let no_tools = match c.request(Command::SpawnSession {
        ticket: bare,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let rec = c.board().sessions.into_iter().find(|s| s.id == no_tools).unwrap();
    assert!(
        !rec.argv.iter().any(|a| a == mesimon_core::brief::FLAG),
        "no tools, no brief: the text names get_ticket. {:?}",
        rec.argv
    );
    let _ = c.request(Command::KillSession { id: no_tools });
    assert!(matches!(c.request(Command::SetMcpTools { on: true }), Response::Ok));
    assert!(matches!(c.request(Command::SetSystemPrompt { on: false }), Response::Ok));
    let off = match c.request(Command::SpawnSession {
        ticket: bare,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let rec = c.board().sessions.into_iter().find(|s| s.id == off).unwrap();
    assert!(!rec.argv.iter().any(|a| a == mesimon_core::brief::FLAG), "{:?}", rec.argv);
    let _ = c.request(Command::KillSession { id: off });
}
