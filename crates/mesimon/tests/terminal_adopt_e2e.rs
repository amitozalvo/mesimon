//! The ticket's terminal, adopted (T-366), end to end: `!` on a ticket page
//! opens THAT ticket's terminal, the board lists it as a terminal with the
//! command it is running, the preview reads its pane before adoption, and
//! `AdoptTerminal` turns the same pane into a shell session of the ticket —
//! the record's `sid16` is the pane's new name, and every session road
//! (`PaneTail`, sleep, wake) reaches it from then on.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::Path;
use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response, TerminalItem};

/// Every pane on the private server: `(session name, current command, dead)`.
fn list_panes(sock: &Path) -> Vec<(String, String, bool)> {
    let out = tmux(sock)
        .args(["list-panes", "-a", "-F", "#{session_name}|#{pane_current_command}|#{pane_dead}"])
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

fn terminals_of(resp: Response) -> Vec<TerminalItem> {
    match resp {
        Response::Board { terminals, .. } => terminals,
        other => panic!("expected a board, got {other:?}"),
    }
}

fn send_keys(sock: &Path, target: &str, keys: &[&str]) {
    wait_until(Duration::from_secs(10), "the pane to take keys", || {
        tmux(sock)
            .args(["send-keys", "-t", target])
            .args(keys)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    });
}

#[test]
fn the_tickets_terminal_is_listed_previewed_and_adopted() {
    // `/bin/sh` on every machine: the pane runs `$SHELL`, and the daemon
    // tells a command from the shell by that name.
    let Some(h) = Harness::boot_with_env("adopt", None, &[("SHELL", "/bin/sh")]) else {
        return;
    };
    let mut c = h.client("adopt");
    let sock = h.paths.tmux_sock();

    let t = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "a ticket".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let term = format!("msmn-term-{t}");

    // Nothing to preview or adopt before `!`.
    err_containing(c.request(Command::TerminalTail { ticket: Some(t), lines: 20 }), "no terminal");
    err_containing(c.request(Command::AdoptTerminal { ticket: t }), "no terminal");

    // `!` on the ticket page: THIS ticket's terminal, worktree or not, and
    // listed on the board the moment it opens — no record, a terminal.
    match c.request(Command::OpenTerminal { ticket: Some(t) }) {
        Response::Attach { argv } => assert_eq!(argv.last().map(String::as_str), Some(&*term)),
        other => panic!("expected the terminal's attach argv, got {other:?}"),
    }
    let listed = terminals_of(c.request(Command::Snapshot));
    assert_eq!(listed, [TerminalItem { ticket: Some(t), foreground: None }], "{listed:?}");
    assert!(board_of(c.request(Command::Snapshot)).sessions.is_empty());
    // Adopting under the user's feet is refused while the token is held.
    err_containing(c.request(Command::AdoptTerminal { ticket: t }), "attached");
    assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));

    // A command running in it is its foreground, read off the pane on the
    // poll bucket; the preview shows the pane itself.
    send_keys(&sock, &term, &["echo adopt-probe-7; sleep 30", "Enter"]);
    wait_until(Duration::from_secs(10), "the foreground to be read", || {
        terminals_of(c.request(Command::Snapshot))
            .iter()
            .any(|i| i.ticket == Some(t) && i.foreground.as_deref() == Some("sleep"))
    });
    let lines = match c.request(Command::TerminalTail { ticket: Some(t), lines: 20 }) {
        Response::PaneTail { lines, .. } => lines,
        other => panic!("terminal tail: {other:?}"),
    };
    assert!(lines.iter().any(|l| l.trim() == "adopt-probe-7"), "{lines:?}");

    // Adopt: a Bash record of the ticket, `Running`, with the foreground
    // carried over; the pane wears the record's sid16 and the terminal's
    // name is gone from the server and from the board's list.
    let sid = match c.request(Command::AdoptTerminal { ticket: t }) {
        Response::Spawned { id, .. } => id,
        other => panic!("adopt: {other:?}"),
    };
    let sid16 = sid.simple().to_string()[..16].to_string();
    let b = board_of(c.request(Command::Snapshot));
    assert_eq!(b.sessions.len(), 1, "{:?}", b.sessions);
    let rec = &b.sessions[0];
    assert_eq!((rec.id, rec.kind, rec.ticket), (sid, SessionKind::Bash, t));
    assert_eq!(rec.state, SessionState::Running);
    assert_eq!(rec.foreground.as_deref(), Some("sleep"));
    assert_eq!(rec.argv, ["/bin/sh"]);
    assert!(terminals_of(c.request(Command::Snapshot)).is_empty());
    let panes = list_panes(&sock);
    assert!(panes.iter().any(|(n, _, dead)| *n == sid16 && !dead), "{panes:?}");
    assert!(!panes.iter().any(|(n, _, _)| *n == term), "{panes:?}");
    err_containing(c.request(Command::AdoptTerminal { ticket: t }), "no terminal");
    err_containing(c.request(Command::TerminalTail { ticket: Some(t), lines: 20 }), "no terminal");

    // The same pane, by the session's own road: the preview keeps its
    // history, and sleep is refused while the command runs.
    let lines = match c.request(Command::PaneTail { session: sid, lines: 20 }) {
        Response::PaneTail { lines, .. } => lines,
        other => panic!("pane tail: {other:?}"),
    };
    assert!(lines.iter().any(|l| l.trim() == "adopt-probe-7"), "{lines:?}");
    err_containing(c.request(Command::SleepSession { id: sid }), "live children");

    // The foreground is never written to disk: the persisted record has none.
    let persisted = std::fs::read_to_string(h.paths.sessions_file()).unwrap();
    assert!(!persisted.contains("foreground"), "{persisted}");

    // The command ends and the foreground clears. A shell's sleep is its
    // CLOSE: the pane goes and so does the record — nothing is parked,
    // because a woken shell would be a different shell wearing the row.
    send_keys(&sock, &sid16, &["C-c"]);
    wait_until(Duration::from_secs(10), "the foreground to clear", || {
        board_of(c.request(Command::Snapshot)).sessions[0].foreground.is_none()
    });
    match c.request(Command::SleepSession { id: sid }) {
        Response::Ok => {}
        other => panic!("sleep: {other:?}"),
    }
    assert!(board_of(c.request(Command::Snapshot)).sessions.is_empty(), "the record is gone");
    wait_until(Duration::from_secs(10), "the closed pane to go", || {
        !list_panes(&sock).iter().any(|(n, _, dead)| *n == sid16 && !dead)
    });
    err_containing(c.request(Command::WakeSession { id: sid }), "no such session");
    let persisted = std::fs::read_to_string(h.paths.sessions_file()).unwrap();
    assert!(!persisted.contains(&sid.to_string()), "{persisted}");

    // `!` again on the ticket page opens a fresh terminal on the ticket,
    // adoptable in turn; it is listed as a terminal again, and the closed
    // shell stays gone.
    assert!(matches!(
        c.request(Command::OpenTerminal { ticket: Some(t) }),
        Response::Attach { .. }
    ));
    assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));
    assert_eq!(terminals_of(c.request(Command::Snapshot)).len(), 1);
    assert!(board_of(c.request(Command::Snapshot)).sessions.is_empty());
}
