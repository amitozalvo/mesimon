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

use crate::common;
use common::*;

use std::path::Path;
use std::process::Command as Proc;
use std::time::Duration;

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};

/// Drive one session to the edge: TODO → IN PROGRESS by a settled prompt,
/// then a `Stop` whose settle is still pending when we return. Returns the
/// ticket's key, for the file on disk.
fn park_a_stop_in_flight(c: &mut TestClient, hook_sock: &Path) -> String {
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "flush".into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.iter().find(|t| t.title == "flush").unwrap().id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Bash,
        submit_prompt: false,
        plan: false,
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
    let fixture = common::TestFixture::new("sigterm");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = fixture.paths(&repo);

    // The real binary, as its own process: a signal needs a pid of its own.
    let daemon = fixture.daemon(&repo);
    let sock = paths.orch_sock();
    wait_until(Duration::from_secs(10), "the daemon socket", || sock.exists());

    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "sigterm".into()
        }),
        Response::Hello { .. }
    ));
    let key = park_a_stop_in_flight(&mut c, &paths.hook_sock());

    let status = Proc::new("kill").arg("-TERM").arg(daemon.id().to_string()).status().unwrap();
    assert!(status.success(), "kill -TERM");
    let exit = daemon.wait().expect("daemon exit");
    assert_eq!(exit, 0, "a TERM is a clean exit, not a signal death");
    assert!(!sock.exists(), "the shutdown road removes the socket");
    // The journal names the reason and the end (2026-09-05): a clean exit
    // that said nothing is what a four-minute daemon absence looked like.
    let journal = std::fs::read_to_string(paths.daemon_log()).unwrap_or_default();
    assert!(journal.contains(" started pid "), "journal has the start: {journal}");
    assert!(journal.contains(" stopping: SIGTERM"), "journal names the reason: {journal}");
    assert!(journal.contains(" stopped ∙ shutdown took "), "journal has the end: {journal}");

    assert_eq!(column_on_disk(&repo, &key), "REVIEW", "the flushed EndTurn automoved");
    assert_eq!(session_state_on_disk(&paths.state_dir), "\"idle\"/\"end_turn\"");
}

/// The worker is inside a changed save. A second client can already see the
/// new seat, but the spawning client gets its receipt only after the sync.
#[test]
fn snapshot_during_a_held_save_and_acknowledged_sessions_survive_a_crash() {
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    if !require_tmux() {
        return;
    }
    let fixture = TestFixture::new("held-save");
    let repo = fixture.dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = fixture.paths(&repo);
    let gate = paths.rt_dir.join("save-gate.sock");
    fixture.set_env("MESIMON_TEST_SESSION_SAVE_GATE", &gate);
    let daemon = fixture.daemon(&repo);
    let mut reader = TestClient::connect(&paths.orch_sock());
    let mut spawner = TestClient::connect(&paths.orch_sock());
    let ticket = reader.create_ticket("held save");
    let listener = UnixListener::bind(&gate).unwrap();
    listener.set_nonblocking(true).unwrap();
    let spawn = std::thread::spawn(move || {
        spawner.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Bash,
            submit_prompt: false,
            plan: false,
        })
    });
    let accept_save = |listener: &UnixListener| {
        let mut held = None;
        wait_until(Duration::from_secs(8), "worker to reach a changed save", || {
            held = listener.accept().ok();
            held.is_some()
        });
        held.unwrap().0
    };
    let mut held = accept_save(&listener);
    let snapshot = reader.board();
    let id = snapshot.sessions.iter().find(|s| s.ticket == ticket).unwrap().id;
    assert!(!spawn.is_finished(), "the spawn was acknowledged before its save");
    std::fs::remove_file(&gate).unwrap();
    held.write_all(&[1]).unwrap();
    assert!(matches!(spawn.join().unwrap(), Response::Spawned { id: got, .. } if got == id));

    // No shutdown flush can rescue an early acknowledgment.
    assert!(Proc::new("kill")
        .args(["-KILL", &daemon.id().to_string()])
        .status()
        .unwrap()
        .success());
    assert_ne!(daemon.wait().unwrap(), 0);
    let next = fixture.daemon(&repo);
    let mut reader = TestClient::connect(&paths.orch_sock());
    assert!(reader.board().sessions.iter().any(|s| s.id == id && s.ticket == ticket));
    // Hold a second change through a clean handover. Shutdown has begun
    // when the journal says so, and another connection can still read.
    let second = reader.create_ticket("held through shutdown");
    let listener = UnixListener::bind(&gate).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut writer = TestClient::connect(&paths.orch_sock());
    let spawn = std::thread::spawn(move || {
        writer.request(Command::SpawnSession {
            ticket: second,
            kind: SessionKind::Bash,
            submit_prompt: false,
            plan: false,
        })
    });
    let mut held = accept_save(&listener);
    let mut stopper = TestClient::connect(&paths.orch_sock());
    let stop = std::thread::spawn(move || stopper.request(Command::Shutdown));
    wait_until(Duration::from_secs(5), "shutdown to start", || {
        std::fs::read_to_string(paths.daemon_log()).unwrap().contains(" stopping: shutdown asked")
    });
    assert!(reader.board().sessions.iter().any(|s| s.ticket == second));
    assert!(!stop.is_finished(), "shutdown replied before the held write finished");
    std::fs::remove_file(&gate).unwrap();
    held.write_all(&[1]).unwrap();
    assert!(matches!(spawn.join().unwrap(), Response::Spawned { .. }));
    assert!(matches!(stop.join().unwrap(), Response::Ok));
    next.join().unwrap();
}
