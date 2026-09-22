//! A claude typed into a ticket's shell, bound to the ticket (T-369), end to
//! end: the shell is adopted as a Bash record; Claude Code's pid file places
//! a session in that record's pane (`tmux: "<sid16>:@w.%p"`); the daemon
//! attaches the session observe-only with the shell as its `host`, the seat
//! is held, Enter lands in the shell's pane; the process leaves and the
//! record parks; the wake road resumes it in a pane of its own.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::Path;
use std::time::Duration;

use mesimon_core::board::{Board, Provenance, SessionKind, SessionRecord, SessionState};
use mesimon_core::command::{Command, Response};

const SID: &str = "5e7a1c2d-3b4f-4a6e-9c8d-0f1e2d3c4b5a";

/// The pid of the pane running under `name` on the private server.
fn pane_pid(sock: &Path, name: &str) -> i32 {
    let out = tmux(sock)
        .args(["list-panes", "-a", "-F", "#{session_name}|#{pane_pid}"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| {
            let (n, pid) = l.split_once('|')?;
            (n == name).then(|| pid.trim().parse().ok()).flatten()
        })
        .unwrap_or_else(|| panic!("no pane named {name}"))
}

fn claude_of(b: &Board) -> Option<&SessionRecord> {
    b.sessions.iter().find(|s| s.kind == SessionKind::Claude)
}

#[test]
fn a_claude_in_the_tickets_shell_is_bound_parked_and_resumed() {
    let stub = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";
    let Some(h) = Harness::boot_with_env("shellclaude", Some(stub), &[("SHELL", "/bin/sh")]) else {
        return;
    };
    let mut c = h.client("shellclaude");
    let sock = h.paths.tmux_sock();
    let claude_home = h.dir.join("claude-home");
    let repo = h.paths.repo_root.display().to_string();

    let t = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "a ticket".into(),
        workspace: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };

    // The ticket's terminal, adopted: a Bash record whose pane wears its sid16.
    assert!(matches!(
        c.request(Command::OpenTerminal { ticket: Some(t) }),
        Response::Attach { .. }
    ));
    assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));
    let shell = match c.request(Command::AdoptTerminal { ticket: t }) {
        Response::Spawned { id, .. } => id,
        other => panic!("adopt: {other:?}"),
    };
    let sid16 = shell.simple().to_string()[..16].to_string();
    let pid = pane_pid(&sock, &sid16);

    // Claude Code's own files: the transcript (written at the first prompt)
    // and the pid file that places the process in the shell's pane. The pid
    // is the pane's shell — alive exactly as long as the pane is.
    let proj = claude_home.join("projects").join("-slug-never-parsed");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join(format!("{SID}.jsonl")),
        format!(
            "{{\"sessionId\":\"{SID}\",\"cwd\":\"{repo}\",\"type\":\"user\",\"uuid\":\"u0\",\"message\":{{}}}}\n\
             {{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"typed in the shell\"}}]}}}}\n"
        ),
    )
    .unwrap();
    let sessions = claude_home.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let pid_file = sessions.join(format!("{pid}.json"));
    std::fs::write(
        &pid_file,
        format!(
            r#"{{"pid":{pid},"sessionId":"{SID}","cwd":"{repo}","tmux":"{sid16}:@0.%0","status":"busy"}}"#
        ),
    )
    .unwrap();

    // Bound on the poll bucket: an observe-only Claude record of the ticket,
    // hosted by the shell, with the transcript's preview; the shell row says
    // `claude` whatever tmux named the process.
    wait_until(Duration::from_secs(15), "the shell's claude to be bound", || {
        claude_of(&board_of(c.request(Command::Snapshot))).is_some_and(|r| r.host == Some(shell))
    });
    let b = board_of(c.request(Command::Snapshot));
    let rec = claude_of(&b).unwrap();
    assert_eq!(rec.ticket, t);
    assert_eq!(rec.provenance, Provenance::Adopted);
    assert!(rec.argv.is_empty(), "{:?}", rec.argv);
    assert_eq!(rec.claude_session_id.map(|s| s.to_string()).as_deref(), Some(SID));
    assert!(rec.transcript_path.as_deref().is_some_and(|p| p.ends_with(".jsonl")));
    assert_eq!(rec.detail.as_deref(), Some("typed in the shell"));
    let hosted = rec.id;
    wait_until(Duration::from_secs(10), "the shell row to say claude", || {
        board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .any(|s| s.id == shell && s.foreground.as_deref() == Some("claude"))
    });
    assert_eq!(b.sessions.len(), 2, "{:?}", b.sessions);

    // The seat is held: a second claude on the ticket is refused, and Enter
    // on the hosted row lands in the shell's pane.
    err_containing(
        c.request(Command::SpawnSession {
            ticket: t,
            kind: SessionKind::Claude,
            submit_prompt: false,
        }),
        "already has an agent",
    );
    match c.request(Command::FocusStart { session: hosted }) {
        Response::Attach { argv } => {
            assert_eq!(argv.last().map(String::as_str), Some(sid16.as_str()), "{argv:?}")
        }
        other => panic!("focus: {other:?}"),
    }
    assert!(matches!(c.request(Command::FocusEnd { session: hosted }), Response::Ok));

    // The binding survives the daemon: `host` is on disk.
    let persisted = std::fs::read_to_string(h.paths.sessions_file()).unwrap();
    assert!(persisted.contains(&format!("\"host\": \"{shell}\"")), "{persisted}");

    // The process leaves (its pid file goes): the record parks where the
    // wake road picks it up, and the shell is a shell again.
    std::fs::remove_file(&pid_file).unwrap();
    wait_until(Duration::from_secs(15), "the hosted record to park", || {
        board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .any(|s| s.id == hosted && s.state == SessionState::Sleeping && s.host.is_none())
    });
    wait_until(Duration::from_secs(10), "the shell row to clear", || {
        board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .any(|s| s.id == shell && s.foreground.is_none())
    });

    // Wake: the same record, resumed in a pane of its own with hooks and MCP
    // — argv carries `--resume <claude session>`.
    match c.request(Command::ResumeSession { id: hosted, confirm: false }) {
        Response::Spawned { id, .. } => assert_eq!(id, hosted),
        other => panic!("resume: {other:?}"),
    }
    let b = board_of(c.request(Command::Snapshot));
    let rec = b.sessions.iter().find(|s| s.id == hosted).unwrap();
    assert_eq!(rec.state, SessionState::Spawning);
    assert!(rec.host.is_none());
    let resume_at = rec.argv.iter().position(|a| a == "--resume").expect("--resume in argv");
    assert_eq!(rec.argv.get(resume_at + 1).map(String::as_str), Some(SID), "{:?}", rec.argv);
}
