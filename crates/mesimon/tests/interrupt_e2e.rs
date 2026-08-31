//! The Esc-interrupt catch, end to end (spike S-E): a user interrupt fires no
//! hook and may write nothing to the transcript, so the pane-activity probe is
//! the only thing standing between the board and "working" forever. Real tmux,
//! in-process daemon, a stub agent that paints for a while and then goes
//! silent — exactly the byte signature of a turn that was interrupted.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::board::{Board, Confidence, SessionKind, SessionState, StopReason};
use mesimon_core::command::{Command, Envelope, Response};
use mesimon_core::Principal;

struct TestClient {
    write: UnixStream,
    read: BufReader<UnixStream>,
}

impl TestClient {
    fn connect(sock: &std::path::Path) -> Self {
        let stream = UnixStream::connect(sock).expect("connect");
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
    let bin = env!("CARGO_BIN_EXE_mesimon");
    let mut child = Proc::new(bin)
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

#[test]
fn interrupted_turn_demotes_to_idle_without_any_hook() {
    if !common::require_tmux() {
        return;
    }
    let dir = std::path::PathBuf::from(format!("/tmp/msmn-e2e-intr-{}", std::process::id()));
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

    // "Working, then interrupted": paint sub-second for ~6 s, then silence.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ntrap 'exit 0' TERM\ni=0\nwhile [ $i -lt 20 ]; do echo tick; i=$((i+1)); sleep 0.3; done\nwhile true; do sleep 1; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    std::env::set_var("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    std::env::set_var("MESIMON_CLAUDE_BIN", &stub);
    // Real threshold is 8 s; the e2e can't spend that per verdict.
    std::env::set_var("MESIMON_PANE_QUIET_MS", "1500");

    let daemon_repo = repo.clone();
    let daemon = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&daemon_repo);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "intr".into() }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "intr".into() });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;

    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    // The hooks a real session would fire: started, then a submitted prompt.
    hook_send(&hook_sock, &sid.to_string(), "SessionStart", r#"{"session_id":"x"}"#);
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

    // While the pane paints, quiet never trips: still Running 3 s in.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(rec(&mut c).state, SessionState::Running, "painting pane must hold Running");

    // The paint stops (the "Esc"); no hook will ever fire. The probe must
    // demote within quiet (1.5 s seam) + settle (1.5 s) + cadence slack.
    let deadline = Instant::now() + Duration::from_secs(12);
    let final_rec = loop {
        let r = rec(&mut c);
        if r.state != SessionState::Running {
            break r;
        }
        assert!(Instant::now() < deadline, "never left Running after the pane went quiet");
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(
        final_rec.state,
        SessionState::Idle { stop_reason: StopReason::Interrupted },
        "quiet Running pane must read idle, not working"
    );
    assert_eq!(final_rec.confidence, Confidence::Medium, "byte silence is inference");

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
    let _ = Proc::new("tmux").arg("-S").arg(&tmux_sock).arg("kill-server").output();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&rt_dir);
}
