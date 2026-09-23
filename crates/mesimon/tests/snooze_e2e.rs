//! Snooze (T-74), end to end: an archive with a deadline leaves the board,
//! the daemon's tick wheel brings it back at the top of its column with its
//! age restarted and — when asked — lit, `SeenTicket` puts the light out,
//! the past-deadline refusal holds, a working claude holds the ticket on
//! the board, and an idle one is put to sleep by the snooze. Real tmux,
//! in-process daemon, stub agent.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[test]
fn a_snoozed_ticket_leaves_and_comes_back_lit_at_the_top() {
    const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";
    let Some(h) = Harness::boot("snooze", Some(STUB)) else { return };
    let mut c = h.client("snooze");

    // Two cards in TODO, the napper UNDER the other: the wake must put it on top.
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "stays".into(),
        workspace: None,
        tier: None,
    });
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "napper".into(),
        workspace: None,
        tier: None,
    });
    let board = c.board();
    let stays = board.tickets.iter().find(|t| t.title == "stays").unwrap().id;
    let napper = board.tickets.iter().find(|t| t.title == "napper").unwrap().id;
    let key = board.ticket(napper).unwrap().short_key.clone();
    let ticket_toml = h.repo.join(".mesimon/board/tickets").join(&key).join("ticket.toml");
    assert_eq!(
        board.column_tickets("TODO").iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![stays, napper]
    );

    // A deadline already behind us is refused, not honoured on the next tick.
    err_containing(
        c.request(Command::SnoozeTicket { id: napper, until: now_secs() - 5, needs_you: true }),
        "past",
    );

    // Two seconds out: gone from the board, listed as archived with the
    // deadline, and on disk with it.
    let until = now_secs() + 2;
    assert!(matches!(
        c.request(Command::SnoozeTicket { id: napper, until, needs_you: true }),
        Response::Ok
    ));
    let board = c.board();
    let t = board.ticket(napper).unwrap();
    assert_eq!(t.snooze_until_secs(), Some(until));
    assert!(t.archived.as_ref().is_some_and(|a| a.needs_you));
    assert!(board.column_tickets("TODO").iter().all(|t| t.id != napper));
    let on_disk = std::fs::read_to_string(&ticket_toml).unwrap();
    assert!(on_disk.contains(&format!("until = \"@{until}\"")), "{on_disk}");
    assert!(on_disk.contains("needs_you = true"), "{on_disk}");
    // The current schema, whatever it is: the snooze rode the bump to 3 and
    // every later field rides its own (T-227's `manual_merge` is 4).
    assert!(
        on_disk.contains(&format!("schema_version = {}", mesimon_daemon::store::TICKET_SCHEMA)),
        "{on_disk}"
    );

    // The tick wheel wakes it: back, first in TODO, lit, age restarted.
    wait_until(Duration::from_secs(8), "the snooze to wake", || {
        !c.board().ticket(napper).unwrap().is_archived()
    });
    let board = c.board();
    let t = board.ticket(napper).unwrap();
    assert!(t.is_woke(), "asked to be lit");
    // The 1 s tick records the ACTUAL wake time, not the scheduled deadline.
    // Scheduling/IPC may cross a second boundary, especially in a container.
    let entered = t
        .entered_at
        .as_deref()
        .and_then(mesimon_core::board::stamp_secs)
        .expect("the wake restarted its age");
    assert!((until..=now_secs()).contains(&entered), "wake {entered} must follow deadline {until}");
    assert_eq!(t.entered_at, t.woke_at, "age and attention refer to the same wake");
    assert_eq!(
        board.column_tickets("TODO").iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![napper, stays],
        "a woken ticket lands on top"
    );
    assert_eq!(board.needs_you_count(), 1);
    let on_disk = std::fs::read_to_string(&ticket_toml).unwrap();
    assert!(on_disk.contains("woke_at"), "{on_disk}");
    assert!(!on_disk.contains("[archived]"), "{on_disk}");

    // Seen: the light goes out, and again is a no-op.
    assert!(matches!(c.request(Command::SeenTicket { id: napper }), Response::Ok));
    assert!(!c.board().ticket(napper).unwrap().is_woke());
    assert!(matches!(c.request(Command::SeenTicket { id: napper }), Response::Ok));
    assert_eq!(c.board().needs_you_count(), 0);

    // A quiet snooze wakes unlit.
    let until = now_secs() + 1;
    assert!(matches!(
        c.request(Command::SnoozeTicket { id: stays, until, needs_you: false }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(8), "the quiet snooze to wake", || {
        !c.board().ticket(stays).unwrap().is_archived()
    });
    assert!(!c.board().ticket(stays).unwrap().is_woke(), "quiet means quiet");

    // A session still working holds the ticket on the board, naming itself —
    // and an idle one is put to sleep by the snooze, so the ticket goes.
    let Response::Spawned { id: sid, .. } = c.request(Command::SpawnSession {
        ticket: napper,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) else {
        panic!("spawn");
    };
    let hook_sock = h.paths.hook_sock();
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    err_containing(
        c.request(Command::SnoozeTicket { id: napper, until: now_secs() + 60, needs_you: true }),
        "agent still awake",
    );
    assert!(!c.board().ticket(napper).unwrap().is_archived());
    assert!(c.board().sessions.iter().any(|s| s.id == sid && s.state == SessionState::Running));

    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(matches!(
        c.request(Command::SnoozeTicket { id: napper, until: now_secs() + 60, needs_you: true }),
        Response::Ok
    ));
    let board = c.board();
    assert!(board.ticket(napper).unwrap().is_archived());
    assert!(board.sessions.iter().any(|s| s.id == sid && s.state == SessionState::Sleeping));
    assert_eq!(board.ticket_awake_sessions(napper), 0);
    drop(h);
}
