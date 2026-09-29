//! A ticket filed from a paired browser is picked up at the desk (T-497):
//! the person opening its page, or an agent starting on it, marks it once,
//! and the browser that filed it turns its ticks teal. A person's own ticket
//! is never marked.
//!
//! A phone's ticket can only be minted through Mesophon, which needs a
//! relay; so the test files two tickets as a person, makes them a phone's on
//! disk (`created_by = "device:…"`, what `Principal::Paired` writes), and
//! restarts the daemon onto them.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, PICKED_AT_DESK, PICKED_BY_AGENT};
use mesimon_core::command::{Command, Response};

#[test]
fn a_phone_s_ticket_is_picked_up_by_its_page_or_an_agent_and_only_once() {
    let Some(h) = Harness::boot("pickup", Some("#!/bin/sh\nexec sleep 600\n")) else { return };
    let mut c = h.client("pickup");
    for title in ["read me", "start me", "mine"] {
        let _ = c.request(Command::CreateTicket {
            column: "TODO".into(),
            title: title.into(),
            workspace: None,
            tier: None,
        });
    }
    let board = c.board();
    let key = |title: &str| {
        let t = board.tickets.iter().find(|t| t.title == title).expect(title);
        (t.id, t.short_key.clone())
    };
    let ((read, read_key), (start, start_key), (mine, _)) =
        (key("read me"), key("start me"), key("mine"));
    drop(c);
    for short_key in [&read_key, &start_key] {
        let file = h.repo.join(".mesimon/board/tickets").join(short_key).join("ticket.toml");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("created_by = \"local\""), "{text}");
        let phone = text.replace("created_by = \"local\"", "created_by = \"device:ab12\"");
        std::fs::write(&file, phone).unwrap();
    }
    h.restart();
    let mut c = h.client("pickup");
    let picked = |c: &mut TestClient, id| {
        c.board().tickets.into_iter().find(|t| t.id == id).expect("ticket").picked
    };
    assert!(picked(&mut c, read).is_none() && picked(&mut c, start).is_none());

    // Opening the page picks it up at the desk; opening it again changes
    // nothing, and neither does opening a person's own ticket.
    assert!(matches!(c.request(Command::OpenedTicket { id: read }), Response::Ok));
    let first = picked(&mut c, read).expect("picked at the desk");
    assert_eq!(first.by, PICKED_AT_DESK);
    std::thread::sleep(Duration::from_millis(1100));
    assert!(matches!(c.request(Command::OpenedTicket { id: read }), Response::Ok));
    assert_eq!(picked(&mut c, read), Some(first.clone()), "once, the first time");
    assert!(matches!(c.request(Command::OpenedTicket { id: mine }), Response::Ok));
    assert!(picked(&mut c, mine).is_none(), "a person's own ticket is nobody's news");

    // An agent starting on the other one picks it up too.
    match c.request(Command::SpawnSession {
        ticket: start,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { .. } => {}
        other => panic!("spawn failed: {other:?}"),
    }
    assert_eq!(picked(&mut c, start).expect("picked by the agent").by, PICKED_BY_AGENT);

    // It is the ticket's own fact: it survives a restart.
    drop(c);
    h.restart();
    let mut c = h.client("pickup");
    assert_eq!(picked(&mut c, read), Some(first));
    assert_eq!(picked(&mut c, start).map(|p| p.by), Some(PICKED_BY_AGENT.to_string()));
}
