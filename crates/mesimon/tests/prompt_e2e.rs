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

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};

#[test]
fn a_prompt_typed_on_the_board_reaches_the_agent_and_is_submitted() {
    // The stub agent: a line reader that keeps a receipt. `read` blocks until
    // a newline, so anything in this file was both delivered and submitted.
    // The receipt sits beside the stub rather than riding the environment
    // because `shellenv`'s denylist strips `MESIMON_*` on the way to a pane,
    // and a test that leans on a variable the daemon deliberately withholds
    // would be testing a hole.
    const STUB: &str = "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" \
                        >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot("prompt", Some(STUB)) else { return };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("prompt");

    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "ask me".into() });
    let ticket = c.board().tickets.first().expect("ticket").id;

    // Before any agent exists the key has nowhere to send, and the daemon
    // says so rather than swallowing the press.
    match c.request(Command::PromptSession { ticket, text: "too early".into() }) {
        Response::Err { message } => {
            assert!(message.contains("no live claude"), "wrong refusal: {message}");
        }
        other => panic!("a ticket with no agent must refuse: {other:?}"),
    }

    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
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
    match c.request(Command::PromptSession { ticket, text: "   ".into() }) {
        Response::Err { message } => assert!(message.contains("nothing to send"), "{message}"),
        other => panic!("a blank prompt must refuse: {other:?}"),
    }

    assert!(matches!(
        c.request(Command::PromptSession { ticket, text: "mesimon-probe-42 run the tests".into() }),
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

    // No pane, no prompt. That is the line `Ctx::ticket_promptable` keeps the
    // KEY away from — `is_live()` would have said yes here, because a parked
    // session is live — and the daemon holds it independently for a client
    // that asks anyway. Dismissing is how this test reaches a paneless record
    // (the stub never gets far enough into a turn to be sleepable).
    let _ = c.request(Command::KillSession { id: sid });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match c.request(Command::PromptSession { ticket, text: "still there?".into() }) {
            Response::Err { message } => {
                assert!(message.contains("no live claude"), "wrong refusal: {message}");
                break;
            }
            other => {
                assert!(Instant::now() < deadline, "a parked agent kept accepting: {other:?}");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
}
