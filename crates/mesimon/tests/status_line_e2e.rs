//! The status line's side (T-264), end to end: `SetStatusLine` before any
//! tmux server exists lands in the conf, so the first server comes up with
//! the bar at the top; the same command against that live server moves it,
//! because a running server never re-reads its conf and the daemon has to
//! say it again as a command. The snapshot reports the side the daemon
//! holds either way — what the TUI reconciles its preference against.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::Path;
use std::time::Duration;

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};

fn status_position(sock: &Path) -> String {
    let out = tmux(sock).args(["show-option", "-gv", "status-position"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn status_top_of(c: &mut TestClient) -> bool {
    match c.request(Command::Snapshot) {
        Response::Board { status_top, .. } => status_top,
        other => panic!("not a board: {other:?}"),
    }
}

#[test]
fn the_status_line_moves_on_a_fresh_server_and_a_live_one() {
    const STUB: &str = "#!/bin/sh\nsleep 300\n";
    let Some(h) = Harness::boot("statusline", Some(STUB)) else { return };
    let sock = h.paths.tmux_sock();
    let mut c = h.client("statusline");

    // Bottom until told: tmux's own default, and what an older prefs file
    // reads as.
    assert!(!status_top_of(&mut c));

    // No server yet — the word goes into the conf and is held.
    assert!(matches!(c.request(Command::SetStatusLine { top: true }), Response::Ok));
    assert!(status_top_of(&mut c), "the snapshot says what was asked, server or not");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "where is the bar".into(),
        workspace: None,
    });
    let ticket = c.board().tickets.first().expect("ticket").id;
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { .. } => {}
        other => panic!("spawn failed: {other:?}"),
    }
    // The first server read the re-rendered conf.
    wait_until(Duration::from_secs(5), "a server on top", || status_position(&sock) == "top");

    // And a live one is told, not left to a conf it will never read again.
    assert!(matches!(c.request(Command::SetStatusLine { top: false }), Response::Ok));
    assert!(!status_top_of(&mut c));
    wait_until(Duration::from_secs(5), "the live server moved down", || {
        status_position(&sock) == "bottom"
    });
}
