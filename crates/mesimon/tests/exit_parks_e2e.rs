//! Leaving a Claude session parks it (2026-09-01). Ctrl+C-out, `/exit` and
//! Ctrl+D end the process, never the conversation — so the record lands in
//! `Sleeping`, exactly where `x` had put it, and `x` brings it back. Real
//! tmux, real pane-died, in-process daemon, stub agents.
//!
//! The three gates are the test: a Claude session with a resumable
//! conversation parks, one without it stays a corpse (parking it would mint
//! a sleeper that can never wake), and a shell is never parked at all — its
//! pane IS its record and there is nothing to resume.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::board::{Board, ExitReason, SessionKind, SessionState};
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

    fn board(&mut self) -> Board {
        match self.request(Command::Snapshot) {
            Response::Board { board, .. } => board,
            other => panic!("expected board, got {other:?}"),
        }
    }

    /// Poll until `id` reaches a state the predicate accepts, or give up.
    fn await_state(
        &mut self,
        id: uuid::Uuid,
        what: &str,
        ok: impl Fn(&SessionState) -> bool,
    ) -> SessionState {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut last = SessionState::unknown();
        while Instant::now() < deadline {
            last = self
                .board()
                .sessions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.state.clone())
                .expect("session record");
            if ok(&last) {
                return last;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        panic!("session never reached {what}; stuck at {last:?}");
    }
}

fn hook_send(sock: &std::path::Path, session: &str, event: &str, reason: &str, body: &str) {
    let mut child = Proc::new(env!("CARGO_BIN_EXE_mesimon"))
        .args(["hook", "--sock"])
        .arg(sock)
        .args(["--session", session, "--event", event, "--reason", reason])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn hook");
    child.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
}

#[test]
fn leaving_claude_parks_the_session() {
    if !common::require_tmux() {
        return;
    }
    let dir = std::path::PathBuf::from(format!("/tmp/msmn-e2e-exitpark-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    let projects = dir.join("claude-home").join("projects").join("msmn");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&projects).unwrap();

    let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();
    let state_dir = paths.state_dir.clone();
    let rt_dir = paths.rt_dir.clone();
    let tmux_sock = paths.tmux_sock();

    // The user's own exit, as tmux sees it: the process leaves with status 0
    // and `pane-died` is the frame that reaches the daemon. Nothing here
    // sends a SessionEnd — this is deliberately the road a Ctrl+C takes when
    // the hook loses the race, and it must park on its own.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\nsleep 1\nexit 0\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    std::env::set_var("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    std::env::set_var("MESIMON_CLAUDE_BIN", &stub);
    std::env::set_var("MESIMON_CLAUDE_HOME", dir.join("claude-home"));

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
        c.request(Command::Hello { version: 1, client: "exitpark".into() }),
        Response::Hello { .. }
    ));

    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "left it".into() });
    let ticket = c.board().tickets.first().expect("ticket").id;

    let spawn = |c: &mut TestClient, kind| match c.request(Command::SpawnSession {
        ticket,
        kind,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };

    // ---- 1. a conversation to come back to → the exit is a park ----------
    let kept = spawn(&mut c, SessionKind::Claude);
    // Claude's own store is what `--resume` reads, so that file existing is
    // the whole difference between a session that can wake and one that
    // cannot. Write one for this session and not for the next.
    std::fs::write(
        projects.join(format!("{kept}.jsonl")),
        "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n",
    )
    .unwrap();

    // ---- 2. no conversation → the exit is an exit ------------------------
    let lost = spawn(&mut c, SessionKind::Claude);

    // ---- 3. a shell is its pane; there is nothing to resume --------------
    let shell = spawn(&mut c, SessionKind::Bash);

    let parked = c.await_state(kept, "sleeping", |s| !matches!(s, SessionState::Spawning));
    assert_eq!(parked, SessionState::Sleeping, "a resumable claude exit parks");

    let corpse = c.await_state(lost, "exited", |s| {
        matches!(s, SessionState::Exited { .. } | SessionState::Sleeping)
    });
    assert_eq!(
        corpse,
        SessionState::Exited { reason: ExitReason::UserQuit },
        "no transcript means no wake — parking it would strand the record"
    );

    // ...and it is not a dead end. `resume` on a record with no conversation
    // used to refuse forever ("no transcript to resume"), which left the row
    // offering `enter resume` and nothing behind it. Starting fresh loses
    // nothing, because there was nothing.
    match c.request(Command::ResumeSession { id: lost, confirm: false }) {
        Response::Spawned { fresh, .. } => assert!(fresh, "a resume with no transcript is fresh"),
        other => panic!("resume refused: {other:?}"),
    }
    let hosting = c
        .board()
        .sessions
        .iter()
        .find(|s| s.id == lost)
        .and_then(|s| s.claude_session_id)
        .expect("the record points at the conversation it now hosts");
    assert_ne!(hosting, lost, "a newly minted id, never the one that had no transcript");

    // The shell never travels this road, so drive it by hand: the same
    // clean-exit signal, and the kind gate is the only thing refusing it.
    hook_send(&hook_sock, &shell.to_string(), "SessionEnd", "prompt_input_exit", "{}");
    let dead_shell = c.await_state(shell, "exited", |s| !matches!(s, SessionState::Running));
    assert_eq!(
        dead_shell,
        SessionState::Exited { reason: ExitReason::UserQuit },
        "a shell's pane is its record; a parked one would wake into a new shell"
    );

    // ---- and `x` brings the parked one back ------------------------------
    match c.request(Command::WakeSession { id: kept }) {
        Response::Spawned { .. } => {}
        other => panic!("wake refused: {other:?}"),
    }
    let woken = c.board().sessions.iter().find(|s| s.id == kept).unwrap().state.clone();
    assert_eq!(woken, SessionState::Spawning, "wake resumes the conversation it parked");

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
    let _ = Proc::new("tmux").arg("-S").arg(&tmux_sock).arg("kill-server").output();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&rt_dir);
}
