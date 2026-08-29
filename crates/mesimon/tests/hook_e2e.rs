//! M2 attention e2e: real `mesimon hook` binary → daemon ingest → state
//! machine → subscriber push. Uses an in-process daemon (like the M1 e2e) and
//! the real built binary for the hook side (CARGO_BIN_EXE lives here).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command as Proc, Stdio};
use std::time::{Duration, Instant};

use mesimon_core::board::{Board, Reason, SessionKind, SessionState};
use mesimon_core::command::{Command, Envelope, Event, GraceItem, Response};
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
            // Events interleave on a subscribed connection; skip them here.
        }
    }

    /// Swallow any already-queued pushes so the next wait sees only new ones.
    fn drain_events(&mut self) {
        while self.next_event(Duration::from_millis(300)).is_some() {}
    }

    /// Wait for one pushed event (no request outstanding).
    fn next_event(&mut self, timeout: Duration) -> Option<Event> {
        let deadline = Instant::now() + timeout;
        self.write.set_nonblocking(false).unwrap();
        self.read
            .get_ref()
            .set_read_timeout(Some(timeout))
            .unwrap();
        while Instant::now() < deadline {
            let mut buf = String::new();
            match self.read.read_line(&mut buf) {
                Ok(0) => return None,
                Ok(_) => {
                    if let Ok(ev) = serde_json::from_str::<Event>(&buf) {
                        return Some(ev);
                    }
                }
                Err(_) => return None,
            }
        }
        None
    }
}

fn board_of(resp: Response) -> (Board, Vec<GraceItem>) {
    match resp {
        Response::Board { board, grace } => (board, grace),
        other => panic!("expected board, got {other:?}"),
    }
}

fn hook_send(sock: &std::path::Path, session: &str, event: &str, reason: Option<&str>, body: &str) {
    let bin = env!("CARGO_BIN_EXE_mesimon");
    let mut cmd = Proc::new(bin);
    cmd.arg("hook")
        .arg("--sock")
        .arg(sock)
        .arg("--session")
        .arg(session)
        .arg("--event")
        .arg(event);
    if let Some(r) = reason {
        cmd.arg("--reason").arg(r);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn hook");
    child.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
    let out = child.wait_with_output().expect("hook exit");
    assert!(out.status.success(), "hook must exit 0");
    assert!(out.stdout.is_empty(), "hook must never write stdout");
}

#[test]
fn m2_attention_headless() {
    if Proc::new("tmux").arg("-V").output().is_err() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let dir = std::path::PathBuf::from(format!("/tmp/msmn-e2e-hook-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.clone();

    let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();
    let state_dir = paths.state_dir.clone();
    let rt_dir = paths.rt_dir.clone();
    let tmux_sock = paths.tmux_sock();

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
        c.request(Command::Hello { version: 1, client: "e2e".into() }),
        Response::Hello { .. }
    ));

    // A second, subscribed connection watches for pushes.
    let mut watcher = TestClient::connect(&sock);
    assert!(matches!(
        watcher.request(Command::Hello { version: 1, client: "watch".into() }),
        Response::Hello { .. }
    ));
    assert!(matches!(watcher.request(Command::Subscribe), Response::Ok));

    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "attn".into() });
    let (board, _) = board_of(c.request(Command::Snapshot));
    let ticket = board.tickets[0].id;

    // A bash session stands in for the agent pane; hook frames come from us.
    let sid = match c.request(Command::SpawnSession { ticket, kind: SessionKind::Bash }) {
        Response::Spawned { id } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    // Drain the spawn's own BoardChanged push.
    assert!(watcher.next_event(Duration::from_secs(2)).is_some());

    // SessionStart → running (bash spawns Running already; frame is harmless),
    // then PermissionRequest → requires_action{permission}, pushed unprompted.
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}"#,
    );
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "PermissionRequest",
        None,
        r#"{"tool_name":"Bash","tool_input":{"command":"npm test"},"prompt_id":"p1"}"#,
    );
    assert!(
        watcher.next_event(Duration::from_secs(2)).is_some(),
        "attention transition must push without any client request"
    );

    let (board, _) = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == sid).expect("session");
    assert_eq!(rec.state, SessionState::RequiresAction { reason: Reason::Permission });
    assert!(rec.waiting_since.is_some(), "attention items carry waiting_since");
    assert_eq!(rec.detail.as_deref(), Some("Bash(npm test)"));
    assert_eq!(rec.transcript_path.as_deref(), Some("/tmp/t.jsonl"));

    // The human-deny path: nothing fires but the next prompt; settle clears.
    watcher.drain_events();
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{"session_id":"x"}"#);
    assert!(
        watcher.next_event(Duration::from_secs(3)).is_some(),
        "settled leave must push from the tick wheel"
    );
    let (board, _) = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == sid).unwrap();
    assert_eq!(rec.state, SessionState::Running);
    assert!(rec.waiting_since.is_none());
    assert!(rec.detail.is_none());

    // Hook binary cost sanity (debug build — the 5 ms p99 budget is a release
    // number; this catches order-of-magnitude regressions only).
    let mut worst = Duration::ZERO;
    for _ in 0..10 {
        let t0 = Instant::now();
        hook_send(&hook_sock, &sid.to_string(), "Stop", None, r#"{"stop_hook_active":true}"#);
        worst = worst.max(t0.elapsed());
    }
    assert!(worst < Duration::from_millis(150), "hook exec took {worst:?}");

    // Absent socket must be silently fine (rule 7) — daemon-down is invisible.
    hook_send(
        &std::path::PathBuf::from("/tmp/msmn-no-such.sock"),
        &sid.to_string(),
        "Stop",
        None,
        "{}",
    );

    // Claude spawn: enters Spawning with a generated 0600 settings file.
    // The stub ignores its args and stays alive so reconcile sees a live pane.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\nsleep 60\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::env::set_var("MESIMON_CLAUDE_BIN", &stub);
    let claude_sid = match c.request(Command::SpawnSession { ticket, kind: SessionKind::Claude }) {
        Response::Spawned { id } => id,
        other => panic!("claude spawn failed: {other:?}"),
    };
    let (board, _) = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == claude_sid).unwrap();
    assert_eq!(rec.state, SessionState::Spawning);
    assert!(rec.argv.iter().any(|a| a == "--settings"));
    let settings = state_dir.join("hooks").join(format!("{claude_sid}.json"));
    assert!(settings.is_file(), "settings file written");
    let mode = std::os::unix::fs::MetadataExt::mode(&settings.metadata().unwrap()) & 0o777;
    assert_eq!(mode, 0o600, "settings must be 0600");
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    let n: usize =
        parsed["hooks"].as_object().unwrap().values().map(|a| a.as_array().unwrap().len()).sum();
    assert_eq!(n, 30, "the registered set is 30 entries");
    let _ = c.request(Command::KillSession { id: claude_sid });

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();

    let _ = Proc::new("tmux").arg("-S").arg(&tmux_sock).arg("kill-server").output();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&rt_dir);
}
