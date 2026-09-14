//! T-378: the board's Shift+Enter on a COLUMN HEADER asks every agent seated
//! in the column, end to end. One column, three tickets: one claude awake in
//! a pane, one parked, one ticket with no agent at all. Sent now, the pane
//! is pasted into, the parked claude is woken with the words held for its
//! first tick, and the bare ticket is skipped and counted — never started.
//! Queued on a shared checkout, every seat parks and the queue drains in
//! board order as the checkout goes quiet.
//!
//! The stub agent appends every line it reads to one file beside itself —
//! every pane shares it, and every probe is unique, so what is asserted is
//! which words landed and how many times.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

#[test]
fn a_column_ask_reaches_every_seated_agent_and_starts_none() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot_with_env(
        "colask",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("colask");
    let text = || std::fs::read_to_string(&got).unwrap_or_default();
    let count = |probe: &str| text().lines().filter(|l| l.contains(probe)).count();

    // `holder` is the agent that will hold the checkout from OUTSIDE the
    // column in the queued case: a working agent inside it would be carried
    // to IN PROGRESS by the column's own automation and leave the column.
    for title in ["holder", "awake", "sleeper", "bare"] {
        let _ = c.request(Command::CreateTicket {
            column: "TODO".into(),
            title: title.into(),
            workspace: None,
        });
    }
    let board = c.board();
    let id = |title: &str| board.tickets.iter().find(|t| t.title == title).expect(title).id;
    let (holder, a, s, bare) = (id("holder"), id("awake"), id("sleeper"), id("bare"));

    let spawn = |c: &mut TestClient, ticket| match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let (sh, sa, ss) = (spawn(&mut c, holder), spawn(&mut c, a), spawn(&mut c, s));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let panes = tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0);
        if panes >= 3 {
            break;
        }
        assert!(Instant::now() < deadline, "the stub agents never got their panes");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500));

    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    // A stub emits no `SessionStart`, so both records sit at `Spawning` —
    // WORKING — until the hooks say otherwise. Walk each through a turn so
    // the checkout starts quiet, then park one.
    for sid in [sh, sa, ss] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }
    assert!(matches!(c.request(Command::SleepSession { id: ss }), Response::Ok));
    c.await_state(ss, "sleeping", |s| *s == SessionState::Sleeping);
    // Automove carried the worked cards along (IN PROGRESS → REVIEW on
    // idle); the column under test is where `awake` and `sleeper` landed,
    // with `bare` brought after them and `holder` put back in TODO.
    let column = c.board().ticket(a).unwrap().column.clone();
    assert!(matches!(
        c.request(Command::MoveTicket { id: bare, column: column.clone(), before: None }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::MoveTicket { id: holder, column: "TODO".into(), before: None }),
        Response::Ok
    ));
    assert_eq!(c.board().column_tickets(&column).len(), 3, "one column, three tickets");

    // The refusals first: a column that is not there, and nothing to say.
    err_containing(
        c.request(Command::PromptColumn {
            column: "NOWHERE".into(),
            text: "x".into(),
            queued: false,
        }),
        "no such column",
    );
    err_containing(
        c.request(Command::PromptColumn {
            column: column.clone(),
            text: "  ".into(),
            queued: false,
        }),
        "nothing to send",
    );

    // (1) Sent now: the pane is pasted into, the sleeper is woken with the
    // words held, the bare ticket is skipped — and nothing was started.
    match c.request(Command::PromptColumn {
        column: column.clone(),
        text: "mesimon-probe-81 commit what you have".into(),
        queued: false,
    }) {
        Response::Asked { sent, woke, queued, skipped, failed } => {
            assert_eq!((sent, woke, queued, skipped, failed), (1, 1, 0, 1, 0));
        }
        other => panic!("expected the column's receipt: {other:?}"),
    }
    wait_until(Duration::from_secs(10), "the pane's paste to land", || {
        count("mesimon-probe-81") >= 1
    });
    let rec = c.board().sessions.into_iter().find(|s| s.id == ss).expect("the sleeper's record");
    assert!(rec.state.has_pane(), "woken: {:?}", rec.state);
    assert!(rec.pending_submit, "the words are owed to the woken pane");
    assert!(
        c.board().sessions.iter().all(|s| s.ticket != bare),
        "a column ask never starts a session"
    );
    // The woken pane reads on its `SessionStart` edge, and the same words
    // land a second time — once per seat, never more.
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(count("mesimon-probe-81"), 1, "nothing reaches a pane being born: {:?}", text());
    hook_send_with(&hook_sock, &ss.to_string(), "SessionStart", Some("resume"), "{}");
    wait_until(Duration::from_secs(15), "the parked words to reach the woken agent", || {
        count("mesimon-probe-81") == 2
    });
    // Both agents ack and settle, so the checkout is quiet again.
    for sid in [sa, ss] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }

    // (2) Queued on a shared checkout while an agent outside the column
    // holds it: every seat parks, in board order, and the bare ticket is
    // skipped as before. The holder's settle sends the first; nothing lands
    // before that.
    start(&mut c, sh);
    match c.request(Command::PromptColumn {
        column: column.clone(),
        text: "mesimon-probe-82 rebase onto main".into(),
        queued: true,
    }) {
        Response::Asked { sent, woke, queued, skipped, failed } => {
            assert_eq!((sent, woke, queued, skipped, failed), (0, 0, 2, 1, 0));
        }
        other => panic!("expected the column's receipt: {other:?}"),
    }
    let p = pending_of(&mut c, None);
    assert_eq!(p.len(), 2, "{p:?}");
    assert!(p.iter().all(|p| p.action == "ask" && !p.in_flight), "{p:?}");
    assert!(p.iter().all(|p| p.text.as_deref() == Some("mesimon-probe-82 rebase onto main")));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(count("mesimon-probe-82"), 0, "parked words must not land: {:?}", text());
    stop(&mut c, sh);
    wait_until(Duration::from_secs(10), "the first queued ask to land", || {
        count("mesimon-probe-82") == 1
    });
    // The rest wait on that one's ack and settle, then go.
    let first = pending_of(&mut c, None).into_iter().find(|p| p.in_flight).expect("in flight");
    let first_sid = if first.ticket == a { sa } else { ss };
    start(&mut c, first_sid);
    stop(&mut c, first_sid);
    wait_until(Duration::from_secs(10), "the second queued ask to land", || {
        count("mesimon-probe-82") == 2
    });
    let second_sid = if first_sid == sa { ss } else { sa };
    start(&mut c, second_sid);
    wait_until(Duration::from_secs(5), "the ack to clear the last entry", || {
        pending_of(&mut c, None).is_empty()
    });
    stop(&mut c, second_sid);

    let _ = c.request(Command::Shutdown);
}
