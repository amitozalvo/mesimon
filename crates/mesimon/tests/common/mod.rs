//! Shared e2e harness: the tmux precondition, the line-protocol client every
//! test speaks to the in-process daemon with, and the `mesimon hook` sender.
//!
//! Each e2e used to carry its own copy of the client (fifteen of them, byte
//! identical structs, method sets that had drifted apart). A new e2e now
//! starts at `mod common; use common::*;` instead of sixty pasted lines.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
// Every test binary compiles this module and none uses all of it.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command as Proc, Stdio};
use std::time::{Duration, Instant};

use std::path::PathBuf;

use mesimon_core::board::{Board, SessionState};
use mesimon_core::command::{Command, Envelope, Event, GraceItem, Resources, Response};
use mesimon_core::Principal;

/// True when the test may proceed.
///
/// Every e2e used to carry its own copy of "tmux missing? print and return",
/// which meant a machine without tmux ran almost nothing and still reported a
/// fully green suite — a release gate that certifies nothing. Locally the skip
/// is still the right behaviour; in CI, `MESIMON_REQUIRE_TMUX=1` turns it into
/// a hard failure so a green run means the tests actually ran.
pub fn require_tmux() -> bool {
    if Proc::new(mesimon_backend_tmux::tmux_bin()).arg("-V").output().is_ok() {
        return true;
    }
    assert!(
        std::env::var_os("MESIMON_REQUIRE_TMUX").is_none(),
        "tmux is required (MESIMON_REQUIRE_TMUX=1) but is not installed",
    );
    eprintln!("tmux not installed; skipping");
    false
}

/// One connection to `orch.sock`, speaking the newline-delimited JSON protocol.
pub struct TestClient {
    pub write: UnixStream,
    pub read: BufReader<UnixStream>,
}

impl TestClient {
    /// Retries for up to 5 s: the socket file exists from `bind` a moment
    /// before `listen` is accepting on it, and a test that restarts the daemon
    /// mid-run connects into that window on a slower runner.
    pub fn connect(sock: &Path) -> Self {
        Self::try_connect(sock, Duration::from_secs(5)).expect("daemon never accepted")
    }

    /// `connect` that gives up quietly: for teardown, where the daemon may
    /// already be gone.
    pub fn try_connect(sock: &Path, timeout: Duration) -> Option<Self> {
        let deadline = Instant::now() + timeout;
        let stream = loop {
            match UnixStream::connect(sock) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => return None,
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let read = BufReader::new(stream.try_clone().unwrap());
        Some(Self { write: stream, read })
    }

    /// Send as a person (`Principal::Local`) and wait for the reply.
    pub fn request(&mut self, command: Command) -> Response {
        self.send(Principal::Local, command)
    }

    /// Send as any principal and wait for the reply. Events interleave on a
    /// subscribed connection; they are skipped here.
    pub fn send(&mut self, principal: Principal, command: Command) -> Response {
        self.try_send(principal, command).expect("the daemon answered")
    }

    /// `send` that reports a dead connection instead of panicking. Every
    /// pasted copy of this loop spun forever on EOF — the daemon's listener
    /// thread outlives its shutdown, so a late connect succeeds and then
    /// closes — which is why the harness's teardown uses this one.
    pub fn try_send(&mut self, principal: Principal, command: Command) -> Option<Response> {
        let env = Envelope { principal, command };
        writeln!(self.write, "{}", serde_json::to_string(&env).unwrap()).ok()?;
        loop {
            let mut buf = String::new();
            match self.read.read_line(&mut buf) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
            if let Ok(resp) = serde_json::from_str::<Response>(&buf) {
                return Some(resp);
            }
        }
    }

    /// A fresh snapshot's board.
    pub fn board(&mut self) -> Board {
        board_of(self.request(Command::Snapshot))
    }

    /// Swallow any already-queued pushes so the next wait sees only new ones.
    pub fn drain_events(&mut self) {
        while self.next_event(Duration::from_millis(300)).is_some() {}
    }

    /// Wait for one pushed event (no request outstanding).
    pub fn next_event(&mut self, timeout: Duration) -> Option<Event> {
        let deadline = Instant::now() + timeout;
        self.write.set_nonblocking(false).unwrap();
        self.read.get_ref().set_read_timeout(Some(timeout)).unwrap();
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

    /// Poll until `id` reaches a state the predicate accepts, or give up.
    pub fn await_state(
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

pub fn board_of(resp: Response) -> Board {
    match resp {
        Response::Board { board, .. } => board,
        other => panic!("expected board, got {other:?}"),
    }
}

pub fn board_and_grace_of(resp: Response) -> (Board, Vec<GraceItem>) {
    match resp {
        Response::Board { board, grace, .. } => (board, grace),
        other => panic!("expected board, got {other:?}"),
    }
}

pub fn snapshot_of(resp: Response) -> (Board, Resources) {
    match resp {
        Response::Board { board, resources, .. } => (board, resources),
        other => panic!("expected board, got {other:?}"),
    }
}

pub fn err_containing(resp: Response, needle: &str) {
    match resp {
        Response::Err { message } => {
            assert!(message.contains(needle), "expected {needle:?} in {message:?}")
        }
        other => panic!("expected refusal containing {needle:?}, got {other:?}"),
    }
}

/// Run the real `mesimon hook` binary against `sock` as Claude Code would.
pub fn hook_send(sock: &Path, session: &str, event: &str, body: &str) {
    hook_send_with(sock, session, event, None, body);
}

/// `hook_send` with a `--reason`. Asserts the observer's two invariants on
/// every call: exit 0, and nothing on stdout (stdout lands in the agent's
/// context).
pub fn hook_send_with(sock: &Path, session: &str, event: &str, reason: Option<&str>, body: &str) {
    let mut cmd = Proc::new(env!("CARGO_BIN_EXE_mesimon"));
    cmd.args(["hook", "--sock"]).arg(sock).args(["--session", session, "--event", event]);
    if let Some(r) = reason {
        cmd.args(["--reason", r]);
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

/// Poll `f` until it holds, or panic naming `what`.
pub fn wait_until(timeout: Duration, what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A tmux client for the private server at `sock` — the SAME build the daemon
/// runs, via `tmux_bin()`. The release gate runs this suite under
/// `MESIMON_TMUX_BIN`, and a client from another build refuses the server
/// over protocol version, so a bare `tmux` here leaked one server per test.
pub fn tmux(sock: &Path) -> Proc {
    let mut cmd = Proc::new(mesimon_backend_tmux::tmux_bin());
    cmd.arg("-S").arg(sock);
    cmd
}

pub fn kill_tmux(sock: &Path) {
    let _ = tmux(sock).arg("kill-server").output();
}

/// `remove_dir_all` that outlasts a pane still closing: under the parallel
/// suite a plain remove raced the reaper and leaked the directory.
pub fn sweep(dir: &Path) {
    for _ in 0..20 {
        if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// One in-process daemon on a fresh repo, with the test seams set, torn
/// down on drop (shutdown, tmux kill-server, sweep). A new e2e is
/// `Harness::boot`, `client`, assertions.
pub struct Harness {
    /// `/tmp/msmn-e2e-<name>-<pid>`: the repo, the stub, anything the test writes.
    pub dir: PathBuf,
    pub repo: PathBuf,
    pub paths: mesimon_daemon::Paths,
    /// The agent stub `MESIMON_CLAUDE_BIN` names, when one was given.
    pub stub: Option<PathBuf>,
    daemon: Option<std::thread::JoinHandle<()>>,
}

impl Harness {
    /// `None` means tmux is missing and the test should return (a hard
    /// failure under `MESIMON_REQUIRE_TMUX`). `stub` is the body of the agent
    /// script; it lands at `<dir>/claude-stub.sh`, so a stub can keep files
    /// beside itself with `$(dirname "$0")`. Any other `MESIMON_*` seam is
    /// set by the test BEFORE booting — the daemon reads them once.
    pub fn boot(name: &str, stub: Option<&str>) -> Option<Self> {
        if !require_tmux() {
            return None;
        }
        let dir = PathBuf::from(format!("/tmp/msmn-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&paths.state_dir);
        let _ = std::fs::remove_dir_all(&paths.rt_dir);

        std::env::set_var("SHELL", "/bin/sh");
        std::env::set_var("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
        std::env::set_var("MESIMON_CLAUDE_HOME", dir.join("claude-home"));
        let stub = stub.map(|body| {
            let p = dir.join("claude-stub.sh");
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            std::env::set_var("MESIMON_CLAUDE_BIN", &p);
            p
        });

        let daemon_repo = repo.clone();
        let daemon = std::thread::spawn(move || {
            let _ = mesimon_daemon::run_foreground(&daemon_repo);
        });
        let sock = paths.orch_sock();
        wait_until(Duration::from_secs(5), "the daemon socket", || sock.exists());
        Some(Self { dir, repo, paths, stub, daemon: Some(daemon) })
    }

    /// A connected client with the `Hello` done.
    pub fn client(&self, name: &str) -> TestClient {
        let mut c = TestClient::connect(&self.paths.orch_sock());
        assert!(matches!(
            c.request(Command::Hello { version: 1, client: name.into() }),
            Response::Hello { .. }
        ));
        c
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(mut c) =
            TestClient::try_connect(&self.paths.orch_sock(), Duration::from_millis(300))
        {
            let _ = c.try_send(Principal::Local, Command::Shutdown);
        }
        // A wedged daemon must fail the test, not hang it: no join under a panic.
        if let Some(d) = self.daemon.take() {
            if !std::thread::panicking() {
                let _ = d.join();
            }
        }
        kill_tmux(&self.paths.tmux_sock());
        for d in [&self.dir, &self.paths.state_dir, &self.paths.rt_dir] {
            sweep(d);
        }
    }
}
