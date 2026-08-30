//! The Esc-interrupt catch via the transcript (dogfood 2026-08-30): current
//! Claude Code keeps painting the pane for a minute after an interrupt, so the
//! pane-quiet probe reads "working" long past the Esc — but it DOES write a
//! `user` record carrying `interruptedMessageId` at the keypress. A Running
//! session of ours must demote off that record even while the pane never goes
//! quiet. Real tmux, in-process daemon, a stub agent that paints forever.

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
fn interrupt_record_demotes_running_while_pane_still_paints() {
    if !common::require_tmux() {
        return;
    }
    let dir = std::path::PathBuf::from(format!("/tmp/msmn-e2e-intrtail-{}", std::process::id()));
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

    // Post-interrupt Claude Code: the pane paints forever. Quiet never trips.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do echo tick; sleep 0.3; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    std::env::set_var("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    std::env::set_var("MESIMON_CLAUDE_BIN", &stub);
    // Prove the demote comes from the transcript, not byte silence: park the
    // pane-quiet threshold far beyond the test's horizon.
    std::env::set_var("MESIMON_PANE_QUIET_MS", "600000");

    let transcript = dir.join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();

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
        c.request(Command::Hello { version: 1, client: "intrtail".into() }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "intrtail".into() });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;

    let sid = match c.request(Command::SpawnSession { ticket, kind: SessionKind::Claude }) {
        Response::Spawned { id } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let start_body = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"{}","source":"startup"}}"#,
        transcript.display(),
        repo.display()
    );
    hook_send(&hook_sock, &sid.to_string(), "SessionStart", &start_body);
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

    // Let the tail cursor mint (2 s poll cadence) — a cursor starts at the
    // file's end, so the abort record must land after it exists.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(rec(&mut c).state, SessionState::Running, "painting pane must hold Running");

    // The Esc: no hook, the pane keeps painting, only the transcript speaks.
    let abort = r#"{"uuid":"u1","type":"user","interruptedMessageId":"msg_011","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
    use std::fs::OpenOptions;
    let mut f = OpenOptions::new().append(true).open(&transcript).unwrap();
    writeln!(f, "{abort}").unwrap();
    drop(f);

    // Poll cadence (2 s) + settle (1.5 s) + slack.
    let deadline = Instant::now() + Duration::from_secs(12);
    let final_rec = loop {
        let r = rec(&mut c);
        if r.state != SessionState::Running {
            break r;
        }
        assert!(Instant::now() < deadline, "never left Running after the interrupt record");
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(
        final_rec.state,
        SessionState::Idle { stop_reason: StopReason::Interrupted },
        "the transcript's interrupt record must demote a still-painting pane"
    );
    assert_eq!(final_rec.confidence, Confidence::Low, "tier-0 evidence stays Low");

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
    let _ = Proc::new("tmux").arg("-S").arg(&tmux_sock).arg("kill-server").output();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&rt_dir);
}
