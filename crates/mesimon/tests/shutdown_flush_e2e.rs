//! A turn that ended inside the settle window is not lost to a restart.
//!
//! `Stop` → `Idle{EndTurn}` waits out a 1500 ms settle before it commits, and
//! a daemon that exits inside that window used to drop it: the restart
//! re-derived the state from the transcript at Low confidence, which automove
//! refuses, so the ticket sat in IN PROGRESS with its turn over (dogfood
//! 2026-09-01, T-140 — a `Stop` one second before a `U` reload). Now both exit
//! roads flush the pending transition through the ordinary change path first:
//! `Command::Shutdown` (the `U` reload and the build-skew restart) in-process,
//! and SIGTERM (`pkill -f "mesimon daemon"`) against the real binary.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::time::Duration;

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};

/// Drive one session to the edge: TODO → IN PROGRESS by a settled prompt,
/// then a `Stop` whose settle is still pending when we return. Returns the
/// ticket's key, for the file on disk.
fn park_a_stop_in_flight(c: &mut TestClient, hook_sock: &Path) -> String {
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "flush".into() });
    let ticket = c.board().tickets.iter().find(|t| t.title == "flush").unwrap().id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Bash,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sid = sid.to_string();
    // Bash spawns Running already, so give the machine an edge to leave from:
    // a permission ask, then the prompt that answers it settles to Running
    // and automove carries the ticket to IN PROGRESS.
    hook_send_with(
        hook_sock,
        &sid,
        "PermissionRequest",
        None,
        r#"{"tool_name":"Bash","tool_input":{"command":"true"},"prompt_id":"p1"}"#,
    );
    hook_send(hook_sock, &sid, "UserPromptSubmit", r#"{"session_id":"x"}"#);
    wait_until(Duration::from_secs(5), "the ticket to reach IN PROGRESS", || {
        c.board().ticket(ticket).unwrap().column == "IN PROGRESS"
    });
    // The turn ends. Its commit is 1500 ms away — and we do not wait.
    hook_send(hook_sock, &sid, "Stop", r#"{"stop_hook_active":false}"#);
    assert_eq!(
        c.board().ticket(ticket).unwrap().column,
        "IN PROGRESS",
        "the Stop is still settling when the daemon is told to go"
    );
    c.board().ticket(ticket).unwrap().short_key.clone()
}

fn column_on_disk(repo: &Path, key: &str) -> String {
    let toml =
        std::fs::read_to_string(repo.join(".mesimon/board/tickets").join(key).join("ticket.toml"))
            .unwrap();
    let line = toml.lines().find(|l| l.starts_with("column = ")).expect("a column line");
    line.trim_start_matches("column = ").trim_matches('"').to_string()
}

fn session_state_on_disk(state_dir: &Path) -> String {
    let s = std::fs::read_to_string(state_dir.join("sessions.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let recs = v.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default();
    let rec = recs.iter().find(|r| r["state"]["state"] != "sleeping").unwrap_or(&recs[0]);
    format!("{}/{}", rec["state"]["state"], rec["state"]["stop_reason"])
}

#[test]
fn shutdown_commits_a_pending_end_turn() {
    let Some(h) = Harness::boot("flush", None) else { return };
    let mut c = h.client("flush");
    let key = park_a_stop_in_flight(&mut c, &h.paths.hook_sock());

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    let sock = h.paths.orch_sock();
    wait_until(Duration::from_secs(5), "the daemon to exit", || !sock.exists());

    assert_eq!(column_on_disk(&h.repo, &key), "REVIEW", "the flushed EndTurn automoved");
    assert_eq!(session_state_on_disk(&h.paths.state_dir), "\"idle\"/\"end_turn\"");
}

#[test]
fn sigterm_takes_the_shutdown_road() {
    if !require_tmux() {
        return;
    }
    let dir = PathBuf::from(format!("/tmp/msmn-e2e-sigterm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    let _ = std::fs::remove_dir_all(&paths.state_dir);
    let _ = std::fs::remove_dir_all(&paths.rt_dir);

    // The real binary, as its own process: a signal needs a pid of its own.
    let mut daemon = Proc::new(env!("CARGO_BIN_EXE_mesimon"))
        .args(["daemon", "--repo"])
        .arg(&repo)
        .env("SHELL", "/bin/sh")
        .env("MESIMON_CLAUDE_HOME", dir.join("claude-home"))
        .spawn()
        .expect("spawn daemon");
    let sock = paths.orch_sock();
    wait_until(Duration::from_secs(10), "the daemon socket", || sock.exists());

    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "sigterm".into() }),
        Response::Hello { .. }
    ));
    let key = park_a_stop_in_flight(&mut c, &paths.hook_sock());

    let status = Proc::new("kill").arg("-TERM").arg(daemon.id().to_string()).status().unwrap();
    assert!(status.success(), "kill -TERM");
    let exit = daemon.wait().expect("daemon exit");
    assert!(exit.success(), "a TERM is a clean exit, not a signal death: {exit:?}");
    assert!(!sock.exists(), "the shutdown road removes the socket");

    assert_eq!(column_on_disk(&repo, &key), "REVIEW", "the flushed EndTurn automoved");
    assert_eq!(session_state_on_disk(&paths.state_dir), "\"idle\"/\"end_turn\"");

    kill_tmux(&paths.tmux_sock());
    for d in [&dir, &paths.state_dir, &paths.rt_dir] {
        sweep(d);
    }
}
