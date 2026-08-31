//! Daemon-restart recovery: a restart mid-turn strands our own session at
//! `Unknown{DaemonRestarted}` with no hook due until the next turn boundary
//! (dogfood 2026-08-30: "?" on the card while Claude visibly streams). The
//! transcript tail must re-derive state at Low confidence until hooks
//! re-assert. Real tmux, two in-process daemon generations over one board.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::board::{Board, Confidence, SessionKind, SessionState, UnknownReason};
use mesimon_core::command::{Command, Envelope, Response};
use mesimon_core::Principal;

struct TestClient {
    write: UnixStream,
    read: BufReader<UnixStream>,
}

impl TestClient {
    fn connect(sock: &std::path::Path) -> Self {
        let deadline = Instant::now() + Duration::from_secs(5);
        let stream = loop {
            match UnixStream::connect(sock) {
                Ok(s) => break s,
                Err(e) => {
                    assert!(Instant::now() < deadline, "daemon never accepted: {e}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let read = BufReader::new(stream.try_clone().unwrap());
        Self { write: stream, read }
    }

    fn request(&mut self, command: Command) -> Response {
        let env = Envelope { principal: Principal::Local, command };
        let line = serde_json::to_string(&env).unwrap();
        writeln!(self.write, "{line}").unwrap();
        loop {
            let mut buf = String::new();
            self.read.read_line(&mut buf).expect("read");
            if let Ok(resp) = serde_json::from_str::<Response>(&buf) {
                return resp;
            }
        }
    }
}

fn board_of(resp: Response) -> Board {
    match resp {
        Response::Board { board, .. } => board,
        other => panic!("expected board, got {other:?}"),
    }
}

fn hook_send(sock: &std::path::Path, session: &str, event: &str, body: &str) {
    let mut child = Proc::new(env!("CARGO_BIN_EXE_mesimon"))
        .args(["hook", "--sock"])
        .arg(sock)
        .args(["--session", session, "--event", event])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn hook");
    child.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
}

/// Both tests set the same process-global env vars — serialize them.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn restart_recovers_state_from_the_transcript() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !common::require_tmux() {
        return;
    }
    let dir = std::path::PathBuf::from(format!("/tmp/msmn-e2e-restart-mid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();
    let state_dir = paths.state_dir.clone();
    let rt_dir = paths.rt_dir.clone();
    let tmux_sock = paths.tmux_sock();

    // Paints forever: the pane must read alive across the restart, and the
    // recovered Running must not trip the quiet probe during the assertions.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do echo tick; sleep 0.3; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let transcript = dir.join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();

    std::env::set_var("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    std::env::set_var("MESIMON_CLAUDE_BIN", &stub);

    // --- Generation 1: spawn, reach Running via hooks, die mid-turn.
    let repo1 = repo.clone();
    let daemon1 = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&repo1);
    });
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "restart".into() }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "restart".into() });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        &format!(r#"{{"session_id":"x","transcript_path":"{}"}}"#, transcript.display()),
    );
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    let rec = |c: &mut TestClient| {
        board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .expect("session")
            .clone()
    };
    assert_eq!(rec(&mut c).state, SessionState::Running);
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon1.join().unwrap();

    // --- Generation 2: reconcile must be honest, then the tail must recover.
    let _ = std::fs::remove_file(&sock); // gen 1's socket file lingers
    let repo2 = repo.clone();
    let daemon2 = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&repo2);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "gen-2 socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "restart2".into() }),
        Response::Hello { .. }
    ));
    assert_eq!(
        rec(&mut c).state,
        SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
        "a restart must not trust stale activity claims"
    );

    // Let the tail poller mint its at-EOF cursor, then stream one record —
    // the only evidence a mid-turn session emits between hook boundaries.
    std::thread::sleep(Duration::from_secs(3));
    let mut f = std::fs::OpenOptions::new().append(true).open(&transcript).unwrap();
    writeln!(
        f,
        r#"{{"uuid":"u1","type":"assistant","message":{{"content":[{{"type":"text","text":"still working"}}]}}}}"#
    )
    .unwrap();
    drop(f);
    let deadline = Instant::now() + Duration::from_secs(6);
    let final_rec = loop {
        let r = rec(&mut c);
        if r.state == SessionState::Running {
            break r;
        }
        assert!(
            Instant::now() < deadline,
            "transcript evidence never recovered the session (still {:?})",
            r.state
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(final_rec.confidence, Confidence::Low, "tail evidence is Tier-0");

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon2.join().unwrap();
    let _ = Proc::new("tmux").arg("-S").arg(&tmux_sock).arg("kill-server").output();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&rt_dir);
}

/// The nudge-free case: the turn ENDED before the restart, so the transcript
/// never grows again. The mint-time backfill must read how it rested
/// (`turn_duration` trailing) and seed `Idle{EndTurn}` — no prompt required.
#[test]
fn restart_recovers_done_from_a_resting_transcript() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !common::require_tmux() {
        return;
    }
    let dir =
        std::path::PathBuf::from(format!("/tmp/msmn-e2e-restart-rest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();
    let state_dir = paths.state_dir.clone();
    let rt_dir = paths.rt_dir.clone();
    let tmux_sock = paths.tmux_sock();

    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do echo tick; sleep 0.3; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // The finished turn, at rest on disk before the restart.
    let transcript = dir.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"all done\"}]}}\n\
         {\"uuid\":\"u2\",\"type\":\"system\",\"subtype\":\"turn_duration\"}\n\
         {\"type\":\"last-prompt\"}\n",
    )
    .unwrap();

    std::env::set_var("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    std::env::set_var("MESIMON_CLAUDE_BIN", &stub);

    // --- Generation 1: spawn, register the transcript, die while Running so
    // reconcile has no choice but Unknown.
    let repo1 = repo.clone();
    let daemon1 = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&repo1);
    });
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "rest".into() }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "rest".into() });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        &format!(r#"{{"session_id":"x","transcript_path":"{}"}}"#, transcript.display()),
    );
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    // "die while Running" is the whole premise, and a hook frame's ingestion
    // races this connection's next request: shutting down between SessionStart
    // and UserPromptSubmit persists idle{unknown}, which is a STICKY claim that
    // survives the restart, so generation 2 would read it instead of waiting
    // for the backfill. Wait for the promotion before killing the daemon.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let st = board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .expect("session")
            .state
            .clone();
        if st == SessionState::Running {
            break;
        }
        assert!(Instant::now() < deadline, "gen-1 never reached Running (still {st:?})");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon1.join().unwrap();

    // --- Generation 2: no hook will ever fire, the transcript never grows —
    // the backfill alone must land Idle{EndTurn} at Low.
    let repo2 = repo.clone();
    let daemon2 = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&repo2);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "gen-2 socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "rest2".into() }),
        Response::Hello { .. }
    ));
    let deadline = Instant::now() + Duration::from_secs(8);
    let final_rec = loop {
        let r = board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .expect("session")
            .clone();
        if !matches!(r.state, SessionState::Unknown { .. }) {
            break r;
        }
        assert!(
            Instant::now() < deadline,
            "backfill never recovered the resting session (still {:?})",
            r.state
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(
        final_rec.state,
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn },
        "a resting turn_duration transcript must read done, not ?"
    );
    assert_eq!(final_rec.confidence, Confidence::Low);

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon2.join().unwrap();
    let _ = Proc::new("tmux").arg("-S").arg(&tmux_sock).arg("kill-server").output();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&rt_dir);
}
