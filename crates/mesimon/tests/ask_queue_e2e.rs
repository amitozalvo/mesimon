//! The board's ask can WAIT for a quiet checkout (2026-09-04), end to end:
//! two claudes in the shared checkout, one mid-turn; an ask queued at the
//! other is parked, lands the moment the first one's turn settles, and is
//! dropped when the user talks to the agent ahead of it.
//!
//! The stub agent appends every line it reads to one file beside itself —
//! both panes share it, and every probe is unique, so what is asserted is
//! which words landed and when, never which pane. `read` only returns on a
//! newline, so a line proves delivery AND the separate Enter (T-5).

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Pending, Response};

fn pending_of(c: &mut TestClient) -> Vec<Pending> {
    match c.request(Command::Snapshot) {
        Response::Board { pending, .. } => pending,
        other => panic!("not a board: {other:?}"),
    }
}

#[test]
fn a_queued_ask_waits_for_the_checkout_and_is_dropped_when_the_user_talks_first() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    // The quiet probe must not free the checkout under the test, and the
    // sleep floor must not refuse the sleep case.
    let Some(h) = Harness::boot_with_env("askq", Some(STUB), &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")]) else { return };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("askq");
    let text = || std::fs::read_to_string(&got).unwrap_or_default();

    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "holder".into() });
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "waiter".into() });
    let board = c.board();
    let a = board.tickets.iter().find(|t| t.title == "holder").expect("a").id;
    let b = board.tickets.iter().find(|t| t.title == "waiter").expect("b").id;
    let a_key = board.ticket(a).unwrap().short_key.clone();

    // A queued ask needs an awake pane: refused before any agent exists.
    err_containing(
        c.request(Command::PromptSession { ticket: b, text: "early".into(), queued: true }),
        "awake claude",
    );

    let spawn = |c: &mut TestClient, ticket| match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sa = spawn(&mut c, a);
    let sb = spawn(&mut c, b);
    // Both stubs reading before anything is pasted.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let panes = tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0);
        if panes >= 2 {
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
    // which is WORKING — until the hooks say otherwise. Walk each through a
    // turn first so the checkout starts quiet.
    for sid in [sa, sb] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }

    // (1) A holds the checkout; an ask at B waits, then lands on A's settle.
    start(&mut c, sa);
    match c.request(Command::PromptSession {
        ticket: b,
        text: "mesimon-probe-51 commit what you have".into(),
        queued: true,
    }) {
        Response::Queued { behind } => assert_eq!(behind, vec![a_key.clone()]),
        other => panic!("expected the ask to be parked: {other:?}"),
    }
    let p = pending_of(&mut c);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].ticket, b);
    assert_eq!(p[0].action, "ask");
    assert_eq!(p[0].waits_on, vec![a_key.clone()]);
    assert_eq!(p[0].text.as_deref(), Some("mesimon-probe-51 commit what you have"));
    assert!(!p[0].in_flight);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!text().contains("mesimon-probe-51"), "parked words must not land: {:?}", text());
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "the queued ask to land", || {
        text().contains("mesimon-probe-51 commit what you have")
    });
    assert_eq!(text().lines().filter(|l| l.contains("mesimon-probe-51")).count(), 1);
    // In flight until B's agent acks it; the ack clears the entry.
    let p = pending_of(&mut c);
    assert!(p.iter().any(|p| p.ticket == b && p.in_flight), "{p:?}");
    start(&mut c, sb);
    wait_until(Duration::from_secs(5), "the ack to clear the entry", || {
        pending_of(&mut c).is_empty()
    });
    stop(&mut c, sb);

    // (2) The user talks to B ahead of the ask: the ask is dropped.
    start(&mut c, sa);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-52 never".into(),
            queued: true
        }),
        Response::Queued { .. }
    ));
    assert_eq!(pending_of(&mut c).len(), 1);
    start(&mut c, sb);
    wait_until(Duration::from_secs(5), "the hand prompt to drop the ask", || {
        pending_of(&mut c).is_empty()
    });
    stop(&mut c, sa);
    stop(&mut c, sb);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!text().contains("mesimon-probe-52"), "a dropped ask must never land: {:?}", text());

    // (3) A second queued ask on the same ticket replaces the first.
    start(&mut c, sa);
    for probe in ["mesimon-probe-53 first", "mesimon-probe-54 second"] {
        assert!(matches!(
            c.request(Command::PromptSession { ticket: b, text: probe.into(), queued: true }),
            Response::Queued { .. }
        ));
    }
    let p = pending_of(&mut c);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].text.as_deref(), Some("mesimon-probe-54 second"));
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "the replaced ask to land", || {
        text().contains("mesimon-probe-54")
    });
    assert!(!text().contains("mesimon-probe-53"), "{:?}", text());
    start(&mut c, sb);
    stop(&mut c, sb);

    // (4) A quiet checkout sends at once: `Ok`, not `Queued`.
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-55 now".into(),
            queued: true
        }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(10), "the immediate ask to land", || {
        text().contains("mesimon-probe-55")
    });
    start(&mut c, sb);
    stop(&mut c, sb);

    // (5) The person drops it: once is `Ok`, twice is nothing to drop.
    start(&mut c, sa);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-56 dropped".into(),
            queued: true
        }),
        Response::Queued { .. }
    ));
    assert!(matches!(c.request(Command::DropQueuedAsk { ticket: b }), Response::Ok));
    err_containing(c.request(Command::DropQueuedAsk { ticket: b }), "nothing queued");
    assert!(pending_of(&mut c).is_empty());
    stop(&mut c, sa);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!text().contains("mesimon-probe-56"), "{:?}", text());

    // (6) Sleeping the waiting agent drops its ask: a parked pane is not one
    // the words can be pasted into later without waking it against the
    // user's gesture.
    start(&mut c, sa);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-57 asleep".into(),
            queued: true
        }),
        Response::Queued { .. }
    ));
    assert!(matches!(c.request(Command::SleepSession { id: sb }), Response::Ok));
    c.await_state(sb, "sleeping", |s| *s == SessionState::Sleeping);
    assert!(pending_of(&mut c).is_empty());
    // And a queued ask at a sleeping claude is refused, not parked.
    err_containing(
        c.request(Command::PromptSession { ticket: b, text: "later".into(), queued: true }),
        "awake claude",
    );
    stop(&mut c, sa);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!text().contains("mesimon-probe-57"), "{:?}", text());

    let _ = c.request(Command::Shutdown);
}
