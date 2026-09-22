//! The project's terminal (T-273), end to end: `!` attaches a persistent
//! shell in the checkout on the private tmux server — a named session that is
//! no session of any ticket — reuses it while it lives, holds the focus token
//! while attached, and respawns it once its shell has exited.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::Path;
use std::time::Duration;

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};

const TERM: &str = "msmn-term";

/// Every pane on the private server: `(session name, cwd, dead)`.
fn list_panes(sock: &Path) -> Vec<(String, String, bool)> {
    let out = tmux(sock)
        .args(["list-panes", "-a", "-F", "#{session_name}|#{pane_current_path}|#{pane_dead}"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '|');
            Some((it.next()?.to_string(), it.next()?.to_string(), it.next()? == "1"))
        })
        .collect()
}

fn terminals(sock: &Path) -> Vec<(String, String, bool)> {
    list_panes(sock).into_iter().filter(|(name, _, _)| name == TERM).collect()
}

#[test]
fn the_terminal_is_one_persistent_shell_in_the_checkout() {
    // `/bin/sh` on every machine: the pane runs `$SHELL`, and the test types
    // `exit` at it.
    let Some(h) = Harness::boot_with_env("term", None, &[("SHELL", "/bin/sh")]) else {
        return;
    };
    let mut c = h.client("term");
    let sock = h.paths.tmux_sock();
    let checkout = h.paths.repo_root.display().to_string();

    // `!` on the board: the checkout's terminal, attachable, and listed on
    // the private server in the checkout — as its own session name, never a
    // record on the board.
    let argv = match c.request(Command::OpenTerminal { ticket: None }) {
        Response::Attach { argv } => argv,
        other => panic!("expected the terminal's attach argv, got {other:?}"),
    };
    assert!(argv.iter().any(|a| a.contains("tmux")), "{argv:?}");
    assert_eq!(argv.last().map(String::as_str), Some(TERM));
    let terms = terminals(&sock);
    assert_eq!(terms.len(), 1, "{terms:?}");
    assert_eq!(terms[0].1, checkout, "the terminal stands in the checkout");
    assert!(!terms[0].2);
    let board = board_of(c.request(Command::Snapshot));
    assert!(board.sessions.is_empty(), "the terminal is nobody's session");

    // While attached it holds the focus token: a ticket's session cannot be
    // focused over it, and a second `!` finds the same shell.
    let t = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "a ticket".into(),
        workspace: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let sid = match c.request(Command::SpawnSession {
        ticket: t,
        kind: SessionKind::Bash,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    match c.request(Command::FocusStart { session: sid }) {
        Response::Err { message } => assert!(message.contains("focused"), "{message}"),
        other => panic!("the terminal holds the token: {other:?}"),
    }
    assert!(matches!(c.request(Command::OpenTerminal { ticket: None }), Response::Attach { .. }));
    assert_eq!(terminals(&sock).len(), 1, "a second `!` reuses the shell");

    // `TerminalEnd` gives the token back.
    assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));
    assert!(matches!(c.request(Command::FocusStart { session: sid }), Response::Attach { .. }));
    assert!(matches!(c.request(Command::FocusEnd { session: sid }), Response::Ok));
    // And a session holding it keeps the terminal out the same way.
    assert!(matches!(c.request(Command::FocusStart { session: sid }), Response::Attach { .. }));
    assert!(matches!(c.request(Command::OpenTerminal { ticket: None }), Response::Err { .. }));
    assert!(matches!(c.request(Command::FocusEnd { session: sid }), Response::Ok));

    // `exit` typed at the shell leaves a dead pane behind (remain-on-exit);
    // the next `!` replaces it with a live one, and there is still one.
    wait_until(Duration::from_secs(10), "the shell to take keys", || {
        tmux(&sock)
            .args(["send-keys", "-t", TERM, "exit", "Enter"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    });
    wait_until(Duration::from_secs(10), "the pane to die", || {
        terminals(&sock).iter().any(|(_, _, dead)| *dead)
    });
    assert!(matches!(c.request(Command::OpenTerminal { ticket: None }), Response::Attach { .. }));
    let terms = terminals(&sock);
    assert_eq!(terms.len(), 1, "{terms:?}");
    assert!(!terms[0].2, "respawned alive: {terms:?}");
    assert_eq!(terms[0].1, checkout);
    assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));
}
