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

struct BoardTransport(TestClient);
impl mesimon_tui::client::Transport for BoardTransport {
    fn request(&mut self, command: Command) -> anyhow::Result<Response> {
        Ok(self.0.request(command))
    }
    fn poll_event(&mut self) -> bool {
        false
    }
}

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
    err_containing(c.request(Command::AdoptTerminal { ticket: t, pane_key: None }), "no terminal");

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
    err_containing(c.request(Command::AdoptTerminal { ticket: t, pane_key: None }), "attached");
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
    let sid = match c.request(Command::AdoptTerminal { ticket: t, pane_key: None }) {
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
    let exact_key = tmux(&sock)
        .args([
            "display-message",
            "-p",
            "-t",
            &format!("={}:0.0", rec.sid16()),
            "#{pid}:#{pane_id}",
        ])
        .output()
        .unwrap();
    assert_eq!(
        rec.pane_key.as_deref(),
        Some(String::from_utf8_lossy(&exact_key.stdout).trim()),
        "exact pane target: {:?}",
        exact_key
    );
    assert!(terminals_of(c.request(Command::Snapshot)).is_empty());
    let panes = list_panes(&sock);
    assert!(panes.iter().any(|(n, _, dead)| *n == sid16 && !dead), "{panes:?}");
    assert!(!panes.iter().any(|(n, _, _)| *n == term), "{panes:?}");
    err_containing(c.request(Command::AdoptTerminal { ticket: t, pane_key: None }), "no terminal");
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

#[test]
fn shell_output_is_shared_scoped_revocable_and_close_is_human_confirmed() {
    use mesimon_core::Principal;
    let Some(h) = Harness::boot_with_env("shell-sharing", None, &[("SHELL", "/bin/sh")]) else {
        return;
    };
    let mut c = h.client("shell-sharing");
    let owner = c.create_ticket("shell owner");
    let crown = c.create_ticket("crown");
    let outsider = c.create_ticket("outsider");
    let worker = c.spawn_claude(owner);
    let coordinator = c.spawn_claude(crown);
    let stranger = c.spawn_claude(outsider);
    let owner_key = c.board().ticket(owner).unwrap().short_key.clone();
    assert!(matches!(c.request(Command::CrownTicket { id: crown }), Response::Ok));
    let view = c.send(Principal::Agent { session: worker }, Command::AgentGetTicket);
    let Response::AgentTicket { ticket: view } = view else { panic!("get_ticket: {view:?}") };
    let json = serde_json::to_value(view).unwrap();
    let ticket = &json;
    for field in ["shells", "shells_about", "terminal_open"] {
        assert!(ticket.get(field).is_none(), "unused shell feature leaked {field}: {json}");
    }
    assert!(matches!(
        c.request(Command::OpenTerminal { ticket: Some(owner) }),
        Response::Attach { .. }
    ));
    assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));
    for session in [worker, coordinator] {
        assert!(matches!(
            c.send(Principal::Agent { session }, Command::TerminalCapture { ticket: owner }),
            Response::Err { .. }
        ));
    }
    // Shift+Enter previews without adopting; Esc leaves both the terminal and board untouched.
    let preview = mesimon_tui::test_support::capture_then_cancel(
        Box::new(BoardTransport(h.client("capture-unadopted"))),
        h.repo.clone(),
        owner,
        None,
    )
    .unwrap();
    assert!(preview.starts_with("Terminal output"));
    assert!(!c.board().sessions.iter().any(|s| s.ticket == owner && s.kind == SessionKind::Bash));
    assert!(terminals_of(c.request(Command::Snapshot)).iter().any(|t| t.ticket == Some(owner)));
    err_containing(
        c.send(
            Principal::Agent { session: worker },
            Command::AgentReadShell { shell: uuid::Uuid::nil(), key: None },
        ),
        "no shared shell",
    );
    // Sending the next capture adopts that terminal, still granting no live reads.
    mesimon_tui::test_support::capture_then_send(
        Box::new(BoardTransport(h.client("capture-send-unadopted"))),
        h.repo.clone(),
        owner,
        None,
    )
    .unwrap();
    let shell = c
        .board()
        .sessions
        .iter()
        .find(|s| s.ticket == owner && s.kind == SessionKind::Bash)
        .unwrap()
        .id;
    let sid = shell.simple().to_string()[..16].to_owned();
    let read = |key| Command::AgentReadShell { shell, key };
    let as_worker = Principal::Agent { session: worker };
    let as_crown = Principal::Agent { session: coordinator };
    let as_stranger = Principal::Agent { session: stranger };
    err_containing(c.send(as_worker.clone(), read(None)), "no shared shell");
    // The adopted row takes the same actual keys, with no sharing command.
    mesimon_tui::test_support::capture_then_cancel(
        Box::new(BoardTransport(h.client("capture-adopted"))),
        h.repo.clone(),
        owner,
        Some(shell),
    )
    .unwrap();
    assert!(!c.board().sessions.iter().find(|s| s.id == shell).unwrap().shell_shared);
    err_containing(c.send(as_worker.clone(), read(None)), "no shared shell");
    err_containing(c.send(as_crown.clone(), read(Some(owner_key.clone()))), "no shared shell");

    assert!(matches!(
        c.request(Command::ShareShell { session: shell, shared: true }),
        Response::Ok
    ));
    send_keys(
        &h.paths.tmux_sock(),
        &sid,
        &["printf '%s%s\\n\\n%s\\n' shell- share-marker '/exit'", "Enter"],
    );
    wait_until(
        Duration::from_secs(10),
        "shared output",
        || matches!(c.send(as_worker.clone(), read(None)), Response::ShellOutput { output } if output.text.contains("shell-share-marker\n\n/exit")),
    );
    assert!(matches!(
        c.send(as_crown.clone(), read(Some(owner_key.clone()))),
        Response::ShellOutput { .. }
    ));
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    wait_until(Duration::from_secs(5), "metadata-only shell read on the feed", || {
        let feed = std::fs::read_to_string(&feed_path).unwrap_or_default();
        assert!(!feed.contains("shell-share-marker"));
        feed.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()).any(
            |line| {
                line["cmd"] == "agent_read_shell"
                    && line["session"] == worker.to_string()
                    && line["shell"] == shell.to_string()
                    && line["outcome"] == "ok"
            },
        )
    });
    err_containing(c.send(as_stranger.clone(), read(None)), "no shared shell");
    assert!(matches!(c.send(as_stranger, read(Some(owner_key.clone()))), Response::Err { .. }));
    for command in [
        Command::PaneTail { session: shell, lines: 20 },
        Command::ShellCapture { session: shell },
        Command::ShareShell { session: shell, shared: true },
        Command::SleepSession { id: shell },
    ] {
        assert!(matches!(c.send(as_crown.clone(), command), Response::Err { .. }));
    }
    assert!(matches!(
        c.request(Command::ShareShell { session: shell, shared: false }),
        Response::Ok
    ));
    err_containing(c.send(as_worker.clone(), read(None)), "no shared shell");
    err_containing(c.send(as_crown.clone(), read(Some(owner_key.clone()))), "no shared shell");
    assert!(matches!(
        c.send(as_worker, Command::AgentRequestCloseShell { shell, key: owner_key.clone() }),
        Response::Err { .. }
    ));
    let request = Command::AgentRequestCloseShell { shell, key: owner_key.clone() };
    wait_until(Duration::from_secs(10), "shell close request", || {
        matches!(c.send(as_crown.clone(), request.clone()), Response::AgentTicket { .. })
    });
    let q = c
        .board()
        .sessions
        .iter()
        .find(|s| s.id == shell)
        .unwrap()
        .shell_close_request
        .clone()
        .unwrap();
    assert!(matches!(c.send(as_crown.clone(), request.clone()), Response::AgentTicket { .. }));
    assert_eq!(
        c.board()
            .sessions
            .iter()
            .find(|s| s.id == shell)
            .unwrap()
            .shell_close_request
            .as_ref()
            .unwrap()
            .id,
        q.id,
        "retry keeps approval identity"
    );
    assert!(matches!(c.request(Command::FocusStart { session: shell }), Response::Attach { .. }));
    err_containing(c.send(as_crown.clone(), request.clone()), "attached");
    assert!(matches!(c.request(Command::FocusEnd { session: shell }), Response::Ok));
    let confirm = Command::ConfirmCloseShell { session: shell, request: q.id, close: true };
    assert!(matches!(c.send(as_crown.clone(), confirm.clone()), Response::Err { .. }));
    assert!(matches!(
        c.request(Command::ConfirmCloseShell {
            session: shell,
            request: uuid::Uuid::new_v4(),
            close: true
        }),
        Response::Err { .. }
    ));
    send_keys(&h.paths.tmux_sock(), &sid, &["sleep 30", "Enter"]);
    wait_until(Duration::from_secs(10), "shell busy", || {
        c.board().sessions.iter().any(|s| s.id == shell && s.foreground.is_some())
    });
    err_containing(c.request(confirm.clone()), "busy");
    let busy = c.send(as_crown.clone(), request.clone());
    match busy {
        Response::Err { message } => {
            assert!(message.contains("busy"));
            assert!(!message.contains("sleep"));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        c.board().sessions.iter().any(|s| s.id == shell),
        "failed confirmation leaves shell alive"
    );
    send_keys(&h.paths.tmux_sock(), &sid, &["C-c"]);
    wait_until(Duration::from_secs(10), "shell idle", || {
        c.board().sessions.iter().any(|s| s.id == shell && s.foreground.is_none())
    });
    assert!(matches!(
        c.request(Command::ConfirmCloseShell { session: shell, request: q.id, close: false }),
        Response::Ok
    ));
    assert!(matches!(c.request(confirm), Response::Err { .. }), "denial invalidates approval");
    assert!(matches!(c.send(as_crown.clone(), request), Response::AgentTicket { .. }));
    let q = c
        .board()
        .sessions
        .iter()
        .find(|s| s.id == shell)
        .unwrap()
        .shell_close_request
        .clone()
        .unwrap();
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(matches!(c.request(Command::CrownTicket { id: crown }), Response::Ok));
    err_containing(
        c.request(Command::ConfirmCloseShell { session: shell, request: q.id, close: true }),
        "request changed",
    );
    assert!(matches!(
        c.send(as_crown.clone(), Command::AgentRequestCloseShell { shell, key: owner_key.clone() }),
        Response::AgentTicket { .. }
    ));
    let q = c
        .board()
        .sessions
        .iter()
        .find(|s| s.id == shell)
        .unwrap()
        .shell_close_request
        .clone()
        .unwrap();
    assert!(matches!(
        c.request(Command::ConfirmCloseShell { session: shell, request: q.id, close: true }),
        Response::Ok
    ));
    assert!(!c.board().sessions.iter().any(|s| s.id == shell));
    assert!(matches!(c.send(as_crown, read(Some(owner_key))), Response::Err { .. }));
}

#[test]
fn lifecycle_actions_leave_adopted_shells_for_the_person_to_close() {
    let Some(h) = Harness::boot_with_env("shell-lifecycle", None, &[("SHELL", "/bin/sh")]) else {
        return;
    };
    let mut c = h.client("shell-lifecycle");
    let sock = h.paths.tmux_sock();
    for adopted in [false, true] {
        for archive in [false, true] {
            let ticket = c.create_ticket("idle terminal");
            assert!(matches!(
                c.request(Command::OpenTerminal { ticket: Some(ticket) }),
                Response::Attach { .. }
            ));
            assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));
            if adopted {
                assert!(matches!(
                    c.request(Command::AdoptTerminal { ticket, pane_key: None }),
                    Response::Spawned { .. }
                ));
                let shell = c.board().sessions.iter().find(|s| s.ticket == ticket).unwrap().clone();
                // Both actions retain the ordinary live-child refusal and leave the pane standing.
                send_keys(&sock, &shell.sid16(), &["sleep 30", "Enter"]);
                wait_until(Duration::from_secs(10), "shell's live child", || {
                    c.board().sessions.iter().any(|s| s.id == shell.id && s.foreground.is_some())
                });
                let cmd = if archive {
                    Command::ArchiveTicket { id: ticket }
                } else {
                    Command::SnoozeTicket {
                        id: ticket,
                        until: mesimon_core::clock::now_secs() + 600,
                        needs_you: false,
                    }
                };
                err_containing(
                    c.request(cmd),
                    if archive {
                        "sessions still awake — sleep them first"
                    } else {
                        "live children"
                    },
                );
                assert!(!c.board().ticket(ticket).unwrap().is_archived());
                send_keys(&sock, &shell.sid16(), &["C-c"]);
                wait_until(Duration::from_secs(10), "idle shell", || {
                    c.board().sessions.iter().any(|s| s.id == shell.id && s.foreground.is_none())
                });
            }
            let cmd = if archive {
                Command::ArchiveTicket { id: ticket }
            } else {
                Command::SnoozeTicket {
                    id: ticket,
                    until: mesimon_core::clock::now_secs() + 600,
                    needs_you: false,
                }
            };
            let response = c.request(cmd);
            if adopted {
                let expected = if archive {
                    "sessions still awake — sleep them first"
                } else {
                    "shell still awake — close the shell first (x)"
                };
                assert!(
                    matches!(response, Response::Err { ref message } if message == expected),
                    "{response:?}"
                );
                assert!(c
                    .board()
                    .sessions
                    .iter()
                    .any(|s| s.ticket == ticket && s.state.has_pane()));
                err_containing(
                    c.request(Command::RestartServer),
                    "shell cannot park yet — close the shell first (x); nothing restarted",
                );
                continue;
            }
            assert!(
                matches!(response, Response::Ok),
                "adopted={adopted}, archive={archive}: {response:?}"
            );
            assert!(!c.board().sessions.iter().any(|s| s.ticket == ticket));
        }
    }
}

#[test]
fn crown_archive_and_the_bulk_offer_leave_idle_adopted_shells_alive() {
    use mesimon_core::Principal;
    let Some(h) = Harness::boot_with_env(
        "shell-archive-refusal",
        None,
        &[("SHELL", "/bin/sh"), ("MESIMON_ARCHIVE_SUGGEST_MS", "1")],
    ) else {
        return;
    };
    let mut c = h.client("shell-archive-refusal");
    let crown = c.create_ticket("crown");
    let coordinator = c.spawn_claude(crown);
    assert!(matches!(c.request(Command::CrownTicket { id: crown }), Response::Ok));
    assert!(matches!(c.request(Command::SetCrownArchives { on: true }), Response::Ok));
    let owner = c.create_ticket("idle shells belong to the person");
    let key = c.board().ticket(owner).unwrap().short_key.clone();
    let mut shells = Vec::new();
    for _ in 0..2 {
        assert!(matches!(
            c.request(Command::OpenTerminal { ticket: Some(owner) }),
            Response::Attach { .. }
        ));
        assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));
        let Response::Spawned { id, .. } =
            c.request(Command::AdoptTerminal { ticket: owner, pane_key: None })
        else {
            panic!("adopt")
        };
        shells.push(id);
    }
    let mut settings = c.board().column("TODO").unwrap().settings.clone();
    settings.offers = Some(mesimon_core::board::ColumnOffers::Archive);
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "TODO".into(), settings }),
        Response::Ok
    ));
    // A separate eligible ticket makes a stale zero offer unable to pass this check.
    let empty = c.create_ticket("archive candidate");
    wait_until(
        Duration::from_secs(10),
        "only the empty ticket in the archive offer",
        || matches!(c.request(Command::Snapshot), Response::Board { resources, .. } if resources.archive_tickets == 1),
    );
    for principal in [Principal::Local, Principal::Agent { session: coordinator }] {
        let command = if matches!(principal, Principal::Local) {
            Command::ArchiveTicket { id: owner }
        } else {
            let Response::AgentTicket { ticket: view } =
                c.send(principal.clone(), Command::AgentReadTicket { key: key.clone() })
            else {
                panic!("crown reads target first")
            };
            Command::AgentArchiveTicket { key: key.clone(), restore: false, seen: view.seen }
        };
        let response = c.send(principal, command);
        assert!(
            matches!(response, Response::Err { ref message } if message == "sessions still awake — sleep them first"),
            "{response:?}"
        );
    }
    assert!(matches!(
        c.request(Command::ArchiveAll),
        Response::Archived { archived: 1, skipped: 0 }
    ));
    assert!(c.is_archived(empty));
    assert!(!c.board().ticket(owner).unwrap().is_archived());
    for id in shells {
        let shell = c.board().sessions.iter().find(|s| s.id == id).unwrap().clone();
        assert!(shell.state.has_pane());
        assert!(list_panes(&h.paths.tmux_sock())
            .iter()
            .any(|(name, _, dead)| name == &shell.sid16() && !dead));
        // A live round-trip proves the pane is usable after both archive roads refused it.
        send_keys(&h.paths.tmux_sock(), &shell.sid16(), &["printf 'still-alive\\n'", "Enter"]);
        wait_until(
            Duration::from_secs(5),
            "shell alive after archive refusal",
            || matches!(c.request(Command::PaneTail { session: id, lines: 10 }), Response::PaneTail { lines, .. } if lines.iter().any(|line| line.trim() == "still-alive")),
        );
    }
}
