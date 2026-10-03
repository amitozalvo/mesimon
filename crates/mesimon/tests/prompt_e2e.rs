//! The board's Shift+Enter, end to end: a prompt the user typed reaches a
//! live agent's stdin and is SUBMITTED, with nobody attaching to the pane.
//!
//! The stub agent appends every line it reads to a file. That is the whole
//! assertion, and it is two claims in one: `read` only returns on a newline,
//! so a line in that file proves the text arrived AND that the Enter was
//! pressed separately afterwards — which is the shape T-5 measured and the
//! reason `paste_text` forks three times instead of once.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

#[test]
fn a_prompt_typed_on_the_board_reaches_the_agent_and_is_submitted() {
    // The stub agent: a line reader that keeps a receipt. `read` blocks until
    // a newline, so anything in this file was both delivered and submitted.
    // The receipt sits beside the stub rather than riding the environment
    // because `shellenv`'s denylist strips `MESIMON_*` on the way to a pane,
    // and a test that leans on a variable the daemon deliberately withholds
    // would be testing a hole.
    //
    // `stty -icanon`, because the sleeping-claude case below pastes 2.5 KB:
    // a tty in canonical mode assembles the line itself and keeps 1 KiB of
    // it (`MAX_CANON`), dropping the rest AND the newline that would have
    // ended `read` — measured here first, 2026-09-04. Claude sets raw mode
    // and has no such cap; the stub has to opt out to stand in for it.
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot("prompt", Some(STUB)) else { return };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("prompt");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "ask me".into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.first().expect("ticket").id;

    // (Before any agent exists the same command STARTS one — T-294, the last
    // clause of this test, where the seat is empty again for the right
    // reason rather than because nothing has happened yet.)

    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };

    // The spawn types the ticket TITLE into the box and stops there (zero
    // token injection). Wait for the stub to be reading before prompting, or
    // the paste races the exec and lands in the pty buffer ahead of it —
    // which works, but proves nothing about delivery to a live reader.
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

    // Blank in, nothing out: an empty prompt must never press Enter on a turn
    // the user did not write. The title is still sitting unsubmitted in the
    // box, so a stray Enter here would submit somebody else's words.
    match c.request(Command::PromptSession {
        ticket,
        text: "   ".into(),
        queued: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    }) {
        Response::Err { message } => assert!(message.contains("nothing to send"), "{message}"),
        other => panic!("a blank prompt must refuse: {other:?}"),
    }

    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "mesimon-probe-42 run the tests".into(),
            queued: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Ok
    ));

    let deadline = Instant::now() + Duration::from_secs(15);
    let received = loop {
        let text = std::fs::read_to_string(&got).unwrap_or_default();
        if text.contains("mesimon-probe-42") {
            break text;
        }
        assert!(Instant::now() < deadline, "the prompt never reached the agent: {text:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    // The whole sentence travelled, not a truncated first word — the delivery
    // is a bracketed paste, and a single `send-keys` truncated 3696 bytes to
    // 630 the last time anyone tried it that way (T-5).
    assert!(
        received.contains("mesimon-probe-42 run the tests"),
        "the prompt arrived in pieces: {received:?}"
    );
    // And exactly one turn was submitted. A second line would mean the Enter
    // rode along with the text and something else pressed one too.
    assert_eq!(
        received.lines().filter(|l| l.contains("mesimon-probe-42")).count(),
        1,
        "one press, one turn: {received:?}"
    );

    // A prompt with lines (T-380, the ask room) keeps them through the
    // daemon and the paste: both lines arrive, in order, the `\r` of a
    // CRLF gone. (A stub reads a line at a time, so it cannot tell one
    // bracketed paste from two; claude and codex were measured to keep the
    // lines in one box — `docs/STALE-MAP.md`, T-380.)
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "mesimon-probe-44 first line\r\nmesimon-probe-44 second line".into(),
            queued: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Ok
    ));
    let deadline = Instant::now() + Duration::from_secs(15);
    let received = loop {
        let text = std::fs::read_to_string(&got).unwrap_or_default();
        if text.contains("mesimon-probe-44 second line") {
            break text;
        }
        assert!(Instant::now() < deadline, "the second line never arrived: {text:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let lines: Vec<&str> = received.lines().filter(|l| l.contains("mesimon-probe-44")).collect();
    assert_eq!(
        lines,
        vec!["mesimon-probe-44 first line", "mesimon-probe-44 second line"],
        "both lines, in order, no CR: {received:?}"
    );

    // A PARKED claude is woken by the ask (2026-09-04). Drive the stub to
    // Idle through the hooks — the stub emits none of its own — and sleep it
    // the way `x` does; then the same command that refused a paneless
    // record before answers `Spawned` instead: the wake is `resume_session`'s
    // (fresh, since a stub writes no transcript) and the words are parked
    // until the pane reads. The prompt is deliberately LONG — past the 1 KiB a
    // pty in canonical mode would keep of anything typed ahead of the process
    // — because the delivery must be the paste a live pane takes, made on
    // the first cadence after the `SessionStart` edge, never a type-ahead.
    let hook_sock = h.paths.hook_sock();
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
    c.await_state(sid, "sleeping", |s| *s == SessionState::Sleeping);

    let long = format!("mesimon-probe-43 {}", "carry on where you left off ".repeat(90));
    assert!(long.len() > 2048, "{}", long.len());
    match c.request(Command::PromptSession {
        ticket,
        text: long.clone(),
        queued: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    }) {
        Response::Spawned { id, fresh } => {
            assert_eq!(id, sid, "the wake re-enters the record — never a second claude");
            assert!(fresh, "a stub has no transcript, so the wake starts fresh");
        }
        other => panic!("an ask at a sleeping claude must wake it: {other:?}"),
    }
    let rec = c.board().sessions.into_iter().find(|s| s.id == sid).expect("record");
    assert!(rec.pending_submit, "the owed Enter is on the record — the card shows the launch");
    assert!(rec.state.has_pane(), "woken: {:?}", rec.state);

    // Nothing may reach the pane before its `SessionStart`: the pane is
    // being born, and the words wait on the edge the way the composer's
    // Enter does.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !std::fs::read_to_string(&got).unwrap_or_default().contains("mesimon-probe-43"),
        "the prompt went in before the pane said it was reading"
    );
    hook_send_with(&hook_sock, &sid.to_string(), "SessionStart", Some("resume"), "{}");

    let deadline = Instant::now() + Duration::from_secs(15);
    let received = loop {
        let text = std::fs::read_to_string(&got).unwrap_or_default();
        if text.contains("mesimon-probe-43") {
            break text;
        }
        assert!(Instant::now() < deadline, "the parked prompt never reached the woken agent");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(received.contains(long.trim_end()), "the prompt arrived in pieces: {received:?}");
    assert_eq!(
        received.lines().filter(|l| l.contains("mesimon-probe-43")).count(),
        1,
        "one paste, one turn: {received:?}"
    );
    // The ack clears the offer, as for the composer's.
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(5), "the ack to clear the owed Enter", || {
        c.board().sessions.iter().any(|s| s.id == sid && !s.pending_submit)
    });

    // No pane, and no PARKED claude either: a dismissed record is `Exited`,
    // which is neither live nor sleeping, so the seat is EMPTY — and since
    // T-294 that is a seat this command fills rather than refuses. The words
    // may even be blank there: the prompt is the ticket's own title, which
    // the spawn types as it always has. The board's ask field is the only
    // caller, and a blank field on an empty seat is what its Enter means.
    let _ = c.request(Command::KillSession { id: sid });
    wait_until(Duration::from_secs(10), "the killed record to leave the seat", || {
        c.board().sessions.iter().all(|s| s.id != sid || !s.state.is_live())
    });
    match c.request(Command::PromptSession {
        ticket,
        text: "   ".into(),
        queued: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    }) {
        Response::Spawned { id, .. } => {
            assert_ne!(id, sid, "a new session, never the corpse");
            assert!(
                c.board().sessions.iter().any(|s| s.id == id && s.pending_submit),
                "the composed spawn owes its Enter"
            );
        }
        other => panic!("an empty seat starts a claude: {other:?}"),
    }

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
}

/// T-603. A plain start (`c`, `+ agent session`) types the title and owes
/// nothing: the person submits it. One that comes up and is never asked sits
/// awake at its composer, `idle` with no stop reason, and Claude Code has
/// written no transcript. A blank ask there is the ticket's brief — title and
/// description — where it was refused as `nothing to send` (the TUI did not
/// even send it). A seat that has conversed still refuses; a person's wake of
/// one that never did comes up fresh and gets the title typed again, as the
/// start typed it; and a blank ask at it while parked wakes it on the brief.
#[test]
fn a_blank_ask_at_a_seat_that_never_took_a_prompt_sends_the_brief() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot("prompt-brief", Some(STUB)) else { return };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("prompt-brief");
    let _ = c.request(Command::CreateTicketWithNote {
        column: "TODO".into(),
        title: "fresh seat".into(),
        workspace: None,
        text: "mesimon-probe-603 the brief's own words".into(),
        uploads: Vec::new(),
        tags: Vec::new(),
        tier: None,
    });
    let ticket = c.board().tickets.first().expect("ticket").id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sid16: String = sid.simple().to_string().chars().take(16).collect();
    let pane_text = || -> String {
        tmux(&tmux_sock)
            .args(["capture-pane", "-p", "-t", &sid16])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    wait_until(Duration::from_secs(15), "the plain start to type the title", || {
        pane_text().contains("fresh seat")
    });

    // Claude names the transcript it will write; there is no file yet.
    let transcript = h.dir.join(format!("{sid}.jsonl"));
    let start = format!(r#"{{"transcript_path":"{}"}}"#, transcript.display());
    hook_send_with(&hook_sock, &sid.to_string(), "SessionStart", Some("startup"), &start);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(Duration::from_secs(5), "the snapshot to read the seat unprompted", || {
        c.board().sessions.iter().any(|s| s.id == sid && s.unprompted)
    });
    let blank = || Command::PromptSession {
        ticket,
        text: "   ".into(),
        queued: true,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    };
    // At `queued` too: the brief has no turn of this seat's to wait for.
    match c.request(blank()) {
        Response::Ok => {}
        other => panic!("a blank ask at a fresh seat sends its brief: {other:?}"),
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let received = loop {
        let text = std::fs::read_to_string(&got).unwrap_or_default();
        if text.contains("mesimon-probe-603") {
            break text;
        }
        assert!(Instant::now() < deadline, "the brief never reached the agent: {text:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(received.contains("fresh seat"), "the title leads the brief: {received:?}");
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(5), "the ack to clear the owed brief", || {
        c.board().sessions.iter().any(|s| s.id == sid && !s.pending_submit)
    });
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));

    // Conversed: Claude wrote the transcript, and a blank ask is nothing.
    std::fs::write(&transcript, "{}\n").unwrap();
    match c.request(blank()) {
        Response::Err { message } => assert!(message.contains("nothing to send"), "{message}"),
        other => panic!("a conversed seat refuses a blank ask: {other:?}"),
    }

    // A seat that never conversed (the file gone again stands in for one),
    // parked and woken by a person: no conversation to resume, so the wake
    // is fresh and its empty composer gets the title again.
    std::fs::remove_file(&transcript).unwrap();
    let _ = std::fs::remove_file(&got);
    assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
    c.await_state(sid, "sleeping", |s| *s == SessionState::Sleeping);
    match c.request(Command::ResumeSession { id: sid, confirm: false }) {
        Response::Spawned { fresh, .. } => assert!(fresh, "nothing to resume"),
        other => panic!("the wake: {other:?}"),
    }
    wait_until(Duration::from_secs(15), "the wake to type the title again", || {
        pane_text().contains("fresh seat")
    });
    let fresh_id = c
        .board()
        .sessions
        .into_iter()
        .find(|s| s.id == sid)
        .and_then(|s| s.claude_session_id)
        .expect("a fresh wake mints the conversation's id");
    let transcript = h.dir.join(format!("{fresh_id}.jsonl"));
    let start = format!(r#"{{"transcript_path":"{}"}}"#, transcript.display());
    hook_send_with(&hook_sock, &sid.to_string(), "SessionStart", Some("startup"), &start);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));

    // And parked again, a blank ask wakes it on the brief.
    assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
    c.await_state(sid, "sleeping", |s| *s == SessionState::Sleeping);
    wait_until(Duration::from_secs(5), "a parked fresh seat to read unprompted", || {
        c.board().sessions.iter().any(|s| s.id == sid && s.unprompted)
    });
    match c.request(blank()) {
        Response::Spawned { id, .. } => assert_eq!(id, sid, "the wake re-enters the record"),
        other => panic!("a blank ask at a parked fresh seat wakes it: {other:?}"),
    }
    hook_send_with(&hook_sock, &sid.to_string(), "SessionStart", Some("startup"), "{}");
    let deadline = Instant::now() + Duration::from_secs(15);
    let received = loop {
        let text = std::fs::read_to_string(&got).unwrap_or_default();
        if text.contains("mesimon-probe-603") {
            break text;
        }
        assert!(Instant::now() < deadline, "the woken seat never got its brief: {text:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(received.contains("fresh seat"), "the title leads it: {received:?}");

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
}
