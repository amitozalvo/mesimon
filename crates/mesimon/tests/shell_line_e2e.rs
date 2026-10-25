//! A `!` line sent to a Claude agent (T-876), end to end: the words a person
//! types in the board's ask field or on the phone that begin with `!` are a
//! shell line for Claude Code's prompt box, not a prompt for the model. They
//! take the paste road on every Claude session — a bracketed paste, then a
//! separate Enter, which the box reads into its bash mode (measured on
//! 2.1.306) — and never a mod's `submit`, which would hand them to the model
//! as a sentence (the author's `! deploy/ship.sh all`, 2026-10-19). No hook
//! fires for a shell line (T-707), so the receipt is the screen: the box
//! reading empty after the Enter. A line into a box holding a draft is
//! refused, since `!` switches the mode only from an empty box; and one sent
//! to a parked agent wakes it, goes in at its composer and is never kept
//! unsent.
//!
//! Under the mod pass (`MESIMON_TEST_ROAD=mod`) the stand-in engine runs
//! beside the stub and records every frame it is sent: a shell line must
//! leave it none.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

/// A line reader with a receipt, as `prompt_e2e`'s: the harness paints
/// Claude's composer ahead of it, and what the tty echoes of a paste lands
/// at the top of the pane, never in the box, so the box reads empty once
/// the line is read — the receipt a real shell line leaves.
const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                    printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

fn feed(h: &Harness) -> String {
    std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap_or_default()
}

fn got(h: &Harness) -> String {
    std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default()
}

/// Every frame the session's stand-in mod read, of `kind` — none off the
/// mod pass, where no stand-in runs.
fn frames(h: &Harness, sid: uuid::Uuid, kind: &str) -> usize {
    std::fs::read_to_string(h.dir.join(format!("mod-{sid}.ndjson")))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|f| f["kind"] == kind)
        .count()
}

fn prompt(c: &mut TestClient, ticket: ulid::Ulid, text: &str) -> Response {
    c.request(Command::PromptSession {
        ticket,
        text: text.into(),
        queued: false,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: Vec::new(),
    })
}

fn wait_line(h: &Harness, needle: &str) {
    let what = format!("the stub to read {needle:?}");
    wait_until(Duration::from_secs(15), &what, || got(h).contains(needle));
}

fn wait_feed(h: &Harness, cmd: &str, ticket: ulid::Ulid, n: usize) {
    let what = format!("{n} feed line(s) {cmd}");
    wait_until(Duration::from_secs(10), &what, || feed_count(h, cmd, ticket) >= n);
}

#[test]
fn a_shell_line_is_pasted_and_acked_off_the_screen_on_every_road() {
    let Some(h) = Harness::boot("shell-line", Some(STUB)) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("shell-line");
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "run it".into(),
        workspace: None,
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
    let sid_s = sid.to_string();
    hook_send_with(&hook_sock, &sid_s, "SessionStart", Some("startup"), "{}");
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    // On the mod pass, the words would go down the mod as a submit were
    // they a prompt: wait for its bridge so a frame could be sent at all.
    if test_road() == "mod" {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(Duration::from_secs(10), "the mod's bridge", || pid.exists());
        std::thread::sleep(Duration::from_millis(300));
    }
    let submits_before = frames(&h, sid, "submit");

    // The ask field's `!echo …`: typed into the pane, whole, and submitted
    // — one line at the stub, the Enter separate from the paste.
    assert!(matches!(prompt(&mut c, ticket, "!echo mesimon-876-one"), Response::Ok));
    wait_line(&h, "!echo mesimon-876-one");
    assert_eq!(
        got(&h).lines().filter(|l| l.contains("mesimon-876-one")).count(),
        1,
        "one paste, one Enter: {:?}",
        got(&h)
    );
    assert_eq!(frames(&h, sid, "submit"), submits_before, "a shell line is never a mod submit");
    wait_feed(&h, "shell_sent", ticket, 1);

    // No hook is sent for it, and the ack still comes: the box reads empty
    // on a sample after the Enter. The card never showed a launch, and the
    // entry did not have to time out.
    wait_feed(&h, "prompt_submitted", ticket, 1);
    let rec = c.board().sessions.into_iter().find(|s| s.id == sid).expect("record");
    assert!(!rec.pending_submit);
    assert!(rec.unsent.is_none(), "{:?}", rec.unsent);
    assert_eq!(feed_count(&h, "paste_unacked", ticket), 0, "{}", feed(&h));

    // The same from Remote Control's road is `paste_to_ticket` too; the
    // board's queued ask takes it on the next idle. Queued here at an idle
    // agent, it goes now, as any queued ask does.
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "! echo mesimon-876-two".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Ok
    ));
    wait_line(&h, "! echo mesimon-876-two");
    assert_eq!(frames(&h, sid, "submit"), submits_before);
    wait_feed(&h, "shell_sent", ticket, 2);
    wait_feed(&h, "queued_ask_delivered", ticket, 1);
    assert_eq!(feed_count(&h, "paste_unacked", ticket), 0, "{}", feed(&h));

    // A shell line to a PARKED agent wakes it, and the line waits for the
    // pane's `SessionStart` and its composer, as any launch's words do.
    // Pasted there, it is acked off the screen as well: the presses stop,
    // and the seat never reads `not sent`.
    assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
    c.await_state(sid, "sleeping", |s| *s == SessionState::Sleeping);
    match prompt(&mut c, ticket, "!echo mesimon-876-wake") {
        Response::Spawned { id, .. } => assert_eq!(id, sid, "the wake re-enters the record"),
        other => panic!("a shell line at a sleeping claude wakes it: {other:?}"),
    }
    assert!(c.board().sessions.iter().any(|s| s.id == sid && s.pending_submit));
    hook_send_with(&hook_sock, &sid_s, "SessionStart", Some("resume"), "{}");
    wait_line(&h, "!echo mesimon-876-wake");
    assert_eq!(frames(&h, sid, "submit"), submits_before, "pasted at the wake, not submitted");
    wait_feed(&h, "shell_sent", ticket, 3);
    wait_feed(&h, "prompt_submitted", ticket, 2);
    let rec = c.board().sessions.into_iter().find(|s| s.id == sid).expect("record");
    assert!(!rec.pending_submit, "the screen's ack ended the presses");
    assert!(rec.unsent.is_none(), "a shell line in the box is never kept unsent: {:?}", rec.unsent);
    assert_eq!(feed_count(&h, "prompt_submit_gave_up", ticket), 0, "{}", feed(&h));
    assert_eq!(feed_count(&h, "paste_unacked", ticket), 0, "{}", feed(&h));
    // Exactly one line per send reached the pane: the owed presses after
    // the paste were Enters into an empty box, and the ack stopped them.
    assert_eq!(got(&h).lines().filter(|l| l.contains("mesimon-876-wake")).count(), 1);

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
}

/// A box holding a draft refuses a shell line: `!` enters bash mode only
/// from an empty box, and pasted after a draft the line would ride to the
/// model as the draft's last words — the failure this road exists to close.
#[test]
fn a_shell_line_into_a_box_holding_a_draft_is_refused() {
    // The stub paints a composer with a draft typed in it (the cursor after
    // the draft, as a typed one leaves it) and then reads lines.
    const DRAFT: &str = "#!/bin/sh\n\
        printf '\\033[999;1H\\033[3A────────────────────\\n❯ a draft the person is typing\\n\
        ────────────────────\\n  ? for shortcuts\\033[2A\\033[33G'\n\
        stty -icanon 2>/dev/null\n\
        while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; \
        done\n";
    let Some(h) = Harness::boot_bare("shell-draft", Some(DRAFT), &[]) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("shell-draft");
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "draft".into(),
        workspace: None,
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
    hook_send_with(&hook_sock, &sid.to_string(), "SessionStart", Some("startup"), "{}");
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    // The stub's composer is on screen before anything is sent at it.
    let sid16: String = sid.simple().to_string().chars().take(16).collect();
    let tmux_sock = h.paths.tmux_sock();
    wait_until(Duration::from_secs(15), "the stub's composer", || {
        tmux(&tmux_sock)
            .args(["capture-pane", "-p", "-t", &sid16])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("a draft the person"))
            .unwrap_or(false)
    });
    match prompt(&mut c, ticket, "!echo mesimon-876-draft") {
        Response::Err { message } => {
            assert!(message.contains("holds text"), "{message}");
        }
        other => panic!("a shell line into a draft must be refused: {other:?}"),
    }
    std::thread::sleep(Duration::from_millis(800));
    assert!(got(&h).is_empty(), "nothing was pasted: {:?}", got(&h));
    assert_eq!(feed_count(&h, "shell_sent", ticket), 0);
    // A prompt is not held to it: words ride a draft as they always have.
    assert!(matches!(prompt(&mut c, ticket, "mesimon-876-words"), Response::Ok));
    wait_line(&h, "mesimon-876-words");
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
}

/// A shell line is a person's alone: the crown's `ask_agent` with one is
/// refused before anything is queued or pasted, and so is a title beginning
/// with `!` from an agent's `create_ticket` or `rename_ticket`, since a start
/// types the title into the box. In a worker's pane the line would run with
/// no permission check at all, the crown's own included.
#[test]
fn an_agent_s_shell_line_and_shell_title_are_refused() {
    use mesimon_core::command::{AgentTicketView, Deliver};
    use mesimon_core::Principal;
    let Some(h) = Harness::boot("shell-agent", Some(STUB)) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("shell-agent");
    let mut ticket = |title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let a = ticket("the crown");
    let b = ticket("the worker");
    let spawn = |c: &mut TestClient, t: ulid::Ulid| match c.request(Command::SpawnSession {
        ticket: t,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    let sa = spawn(&mut c, a);
    let sb = spawn(&mut c, b);
    for s in [sa, sb] {
        hook_send_with(&hook_sock, &s.to_string(), "SessionStart", Some("startup"), "{}");
        c.await_state(s, "idle", |st| matches!(st, SessionState::Idle { .. }));
        // On the mod pass a launch is still coming up until its bridge
        // polls, and the queue holds the checkout for it.
        if test_road() == "mod" {
            let pid = h.dir.join(format!("mod-bridge-{s}.pid"));
            wait_until(Duration::from_secs(10), "the mod's bridge", || pid.exists());
        }
    }
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    let kb = c.board().ticket(b).unwrap().short_key.clone();
    let seen = match c
        .send(Principal::Agent { session: sa }, Command::AgentReadTicket { key: kb.clone() })
    {
        Response::AgentTicket { ticket: AgentTicketView { seen, .. } } => seen,
        other => panic!("get_ticket: {other:?}"),
    };
    for deliver in [Deliver::Idle, Deliver::Now] {
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kb.clone(),
                text: "!echo mesimon-876-crown".into(),
                seen: seen.clone(),
                plan: false,
                deliver,
                expect_secs: None,
            },
        ) {
            Response::Err { message } => {
                assert!(message.contains("begins with `!`"), "{message}");
                assert!(message.contains("only a person"), "{message}");
            }
            other => panic!("the crown's shell line must be refused ({deliver:?}): {other:?}"),
        }
    }
    // The queue is the one road every asker shares: a person's shell line
    // parks there as any ask does.
    match c.request(Command::PromptSession {
        ticket: b,
        text: "!echo mesimon-876-person".into(),
        queued: true,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: Vec::new(),
    }) {
        // Taken: delivered at the idle, or parked for the checkout's quiet.
        Response::Ok | Response::Queued { .. } => {}
        other => panic!("a person's queued shell line: {other:?}"),
    }
    wait_line(&h, "!echo mesimon-876-person");
    assert!(!got(&h).contains("mesimon-876-crown"), "{:?}", got(&h));
    // The feed line is the ack, read off the screen a sample after the
    // Enter, so it can trail the stub's read.
    wait_feed(&h, "shell_sent", b, 1);
    assert_eq!(feed_count(&h, "shell_sent", b), 1);

    // A title the agent sets may not begin with `!` either way — judged as
    // the board stores it: the zero-width space here goes in the scrub, so
    // a check on the raw title would pass what the start then types.
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentCreateTicket {
            title: "\u{200B}!rm -rf mesimon-876".into(),
            column: Some("TODO".into()),
            description: None,
            tags: Vec::new(),
            idempotency_key: None,
            tier: None,
            workspace: Some("worktree".into()),
        },
    ) {
        Response::Err { message } => {
            assert!(message.contains("title that begins with `!`"), "{message}")
        }
        other => panic!("a shell title filed by the crown must be refused: {other:?}"),
    }
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentRenameTicket {
            key: kb.clone(),
            title: "\u{200B} !rm -rf mesimon-876".into(),
            seen,
        },
    ) {
        Response::Err { message } => {
            assert!(message.contains("title that begins with `!`"), "{message}")
        }
        other => panic!("a shell title renamed by the crown must be refused: {other:?}"),
    }
    assert_eq!(c.board().ticket(b).unwrap().title, "the worker");
    assert!(c.board().tickets.iter().all(|t| !t.title.starts_with('!')));

    // A person's own `!` title is theirs to file, and the backstop is the
    // start: a Claude agent is refused on it, since the start would type the
    // title into the box and run the rest.
    let urgent = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "!!! urgent mesimon-876".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("a person's title: {other:?}"),
    };
    match c.request(Command::SpawnSession {
        ticket: urgent,
        kind: SessionKind::Claude,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Err { message } => assert!(message.contains("begins with `!`"), "{message}"),
        other => panic!("a start on a `!` title must be refused: {other:?}"),
    }
    assert!(c.board().sessions.iter().all(|s| s.ticket != urgent));
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
}
