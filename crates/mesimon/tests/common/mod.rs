//! Shared e2e harness: the tmux precondition, the line-protocol client every
//! test speaks to a supervised daemon with, and the `mesimon hook` sender.
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
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, Stdio};
use std::time::{Duration, Instant};

use mesimon_core::board::{Board, SessionState};
use mesimon_core::command::{
    Command, Envelope, Event, GraceItem, Pending, RepoGit, Resources, Response,
};
use mesimon_core::diff::FileEntry;
use mesimon_core::Principal;
use serde_json::{json, Value};

#[path = "../../../../ci/test_support.rs"]
pub mod support;

/// The seams the daemon reads and an e2e sets. The child's environment is
/// built by `set_env` / `Harness::boot_with_env`, and every `MESIMON_*` in the
/// TEST PROCESS is dropped on the way — so `TestFixture::new` refuses to start
/// while one of these is set there: a test that set it with
/// `std::env::set_var` (the recipe until 2026-09-05) would otherwise run its
/// daemon on default timings and pass, or fail, for the wrong reason.
pub const DAEMON_SEAMS: &[&str] = &[
    "MESIMON_ARCHIVE_SUGGEST_MS",
    "MESIMON_CLAUDE_BIN",
    "MESIMON_CLAUDE_HOME",
    "MESIMON_DAEMON_BIN",
    "MESIMON_DETACHED",
    "MESIMON_FAKE_BUILD",
    "MESIMON_HOOK_BIN",
    "MESIMON_NO_TAG_SEED",
    "MESIMON_PANE_QUIET_MS",
    "MESIMON_PINGPONG_MS",
    "MESIMON_SERVER_GUARD_TICKS",
    "MESIMON_SLEEP_MIN_AGE_MS",
    "MESIMON_WT_REFRESH_TICKS",
];

/// Test configuration is per child, never process-global. In particular a
/// missing stub must not launch the developer's installed, authenticated agent.
pub struct TestFixture {
    pub dir: PathBuf,
    owner: support::Fixture,
    env: std::cell::RefCell<std::collections::BTreeMap<String, String>>,
}

impl TestFixture {
    pub fn new(name: &str) -> Self {
        let stray: Vec<&str> =
            DAEMON_SEAMS.iter().copied().filter(|k| std::env::var_os(k).is_some()).collect();
        assert!(
            stray.is_empty(),
            "{stray:?} is set in the test process, where the daemon (a child now) would never \
             see it: pass it through Harness::boot_with_env or fixture.set_env instead"
        );
        let owner =
            support::Fixture::new(name, &mesimon_backend_tmux::tmux_bin().to_string_lossy());
        let dir = owner.dir.clone();
        let env = std::env::vars()
            .filter(|(key, _)| {
                !key.starts_with("MESIMON_")
                    || matches!(
                        key.as_str(),
                        "MESIMON_TMUX_BIN" | "MESIMON_CI" | "MESIMON_TEST_RUN"
                    )
            })
            .collect();
        let fixture = Self { dir, owner, env: std::cell::RefCell::new(env) };
        let stub = fixture.dir.join("default-agent.sh");
        std::fs::write(&stub, "#!/bin/sh\nexec sleep 120\n").unwrap();
        std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let shell_home = fixture.dir.join("home");
        std::fs::create_dir(&shell_home).unwrap();
        fixture.set_env("HOME", &shell_home);
        fixture.set_env("SHELL", "/bin/sh");
        fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
        fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
        fixture.set_env("MESIMON_CLAUDE_HOME", fixture.dir.join("claude-home"));
        fixture.set_env("CODEX_HOME", fixture.dir.join("codex-home"));
        fixture
    }

    pub fn set_env(&self, key: &str, value: impl AsRef<std::ffi::OsStr>) {
        self.env
            .borrow_mut()
            .insert(key.into(), value.as_ref().to_str().expect("test env is UTF-8").into());
    }

    pub fn paths(&self, repo: &Path) -> mesimon_daemon::Paths {
        let mut paths = mesimon_daemon::Paths::for_repo(repo).unwrap();
        paths.state_dir = PathBuf::from(&self.env.borrow()["HOME"])
            .join(".local/state/mesimon")
            .join(&paths.proj16);
        self.owner.register(repo, Some(&paths.state_dir), Some(&paths.rt_dir), &paths.tmux_sock());
        paths
    }

    pub fn daemon(&self, repo: &Path) -> support::TestProcess {
        self.spawn(vec![
            env!("CARGO_BIN_EXE_mesimon").into(),
            "daemon".into(),
            "--repo".into(),
            repo.to_str().unwrap().into(),
        ])
    }

    pub fn spawn(&self, argv: Vec<String>) -> support::TestProcess {
        self.owner.spawn(argv, self.env.borrow().clone())
    }

    pub fn remove_env(&self, key: &str) {
        self.env.borrow_mut().remove(key);
    }
}

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
        let name = command.wire_name();
        self.send_result(principal, command)
            .unwrap_or_else(|error| panic!("daemon response to {name}: {error}"))
    }

    /// `send` that reports a dead connection instead of panicking. Every
    /// pasted copy of this loop spun forever on EOF — the daemon's listener
    /// thread outlives its shutdown, so a late connect succeeds and then
    /// closes — which is why the harness's teardown uses this one.
    pub fn try_send(&mut self, principal: Principal, command: Command) -> Option<Response> {
        self.send_result(principal, command).ok()
    }

    fn send_result(&mut self, principal: Principal, command: Command) -> std::io::Result<Response> {
        let env = Envelope { principal, command };
        writeln!(self.write, "{}", serde_json::to_string(&env).unwrap())?;
        loop {
            let mut buf = String::new();
            if self.read.read_line(&mut buf)? == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed before response",
                ));
            }
            match serde_json::from_str::<Response>(&buf) {
                Ok(response) => return Ok(response),
                Err(_) if serde_json::from_str::<Event>(&buf).is_ok() => {}
                Err(error) => {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
                }
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

/// A subprocess daemon with an owner established before fallible startup.
pub struct Harness {
    pub dir: PathBuf,
    pub repo: PathBuf,
    pub paths: mesimon_daemon::Paths,
    pub stub: Option<PathBuf>,
    fixture: TestFixture,
}

impl Harness {
    pub fn boot(name: &str, stub: Option<&str>) -> Option<Self> {
        Self::boot_with_env(name, stub, &[])
    }

    pub fn boot_with_env(name: &str, stub: Option<&str>, env: &[(&str, &str)]) -> Option<Self> {
        if !require_tmux() {
            return None;
        }
        let fixture = TestFixture::new(name);
        let dir = fixture.dir.clone();
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for (key, value) in env {
            fixture.set_env(key, value);
        }
        let stub = stub.map(|body| {
            let p = dir.join("claude-stub.sh");
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            fixture.set_env("MESIMON_CLAUDE_BIN", &p);
            p
        });
        let paths = fixture.paths(&repo);
        let _daemon = fixture.daemon(&repo);
        wait_until(Duration::from_secs(5), "the daemon socket", || paths.orch_sock().exists());
        Some(Self { dir, repo, paths, stub, fixture })
    }

    pub fn client(&self, name: &str) -> TestClient {
        let mut c = TestClient::connect(&self.paths.orch_sock());
        assert!(matches!(
            c.request(Command::Hello {
                version: mesimon_core::command::PROTOCOL_VERSION,
                client: name.into()
            }),
            Response::Hello { .. }
        ));
        c
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(mut c) =
            TestClient::try_connect(&self.paths.orch_sock(), Duration::from_millis(100))
        {
            let _ = c.write.set_write_timeout(Some(Duration::from_millis(200)));
            let env = Envelope { principal: Principal::Local, command: Command::Shutdown };
            let _ = writeln!(c.write, "{}", serde_json::to_string(&env).unwrap());
        }
        // The supervisor owns bounded termination and checked resource removal.
        // It remains responsible if this process panics or is forcibly killed.
        let _ = &self.fixture;
    }
}

// ------------------------------------------------------------- git, on disk

/// One `git -C <repo> …`, asserted to succeed; stdout as text.
pub fn git(repo: &Path, args: &[&str]) -> String {
    let out = Proc::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A fresh repository on `main` with one committed file and an e2e identity.
pub fn init_repo(dir: &Path, file: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "e2e@t"]);
    git(dir, &["config", "user.name", "e2e"]);
    std::fs::write(dir.join(file), body).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "init"]);
}

/// The snapshot's git clause.
pub fn git_of(resp: Response) -> RepoGit {
    match resp {
        Response::Board { git, .. } => git,
        other => panic!("expected board, got {other:?}"),
    }
}

/// A diff list's rows.
pub fn files_of(resp: &Response) -> Vec<FileEntry> {
    match resp {
        Response::DiffList { files, .. } => files.clone(),
        other => panic!("expected DiffList, got {other:?}"),
    }
}

/// What the snapshot says mesimon owes — every ticket, or one.
pub fn pending_of(c: &mut TestClient, ticket: Option<ulid::Ulid>) -> Vec<Pending> {
    match c.request(Command::Snapshot) {
        Response::Board { pending, .. } => {
            pending.into_iter().filter(|p| ticket.is_none_or(|t| p.ticket == t)).collect()
        }
        other => panic!("not a board: {other:?}"),
    }
}

// ------------------------------------------- the shim, driven as Claude does

/// The real `mesimon mcp` process, spoken to over stdin/stdout exactly the way
/// Claude Code speaks to a stdio MCP server.
pub struct Shim {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Shim {
    pub fn start(sock: &Path, session: uuid::Uuid) -> Self {
        Self::start_with(sock, session, &[])
    }

    /// The shim with extra argv — `--tools <tier>`, the way the daemon's
    /// blob starts it (T-117).
    pub fn start_with(sock: &Path, session: uuid::Uuid, extra: &[&str]) -> Self {
        let mut child = Proc::new(env!("CARGO_BIN_EXE_mesimon"))
            .args(["mcp", "--sock"])
            .arg(sock)
            .args(["--session", &session.to_string()])
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mcp shim");
        let out = BufReader::new(child.stdout.take().unwrap());
        Self { child, out, next_id: 1 }
    }

    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).expect("shim reply");
        let v: Value = serde_json::from_str(&line).expect("shim reply is json");
        assert_eq!(v["id"], json!(id), "reply id must match the request");
        v
    }

    pub fn notify(&mut self, method: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc":"2.0","method":method})).unwrap();
        stdin.flush().unwrap();
    }

    /// A tool call's raw result object, error or not.
    pub fn call(&mut self, name: &str, args: Value) -> Value {
        self.rpc("tools/call", json!({"name": name, "arguments": args}))["result"].clone()
    }

    /// A tool call's text, asserting it was not an error — for a tool whose
    /// answer is prose (a note's body), not JSON.
    pub fn call_ok_text(&mut self, name: &str, args: Value) -> String {
        let r = self.call(name, args);
        assert_eq!(r["isError"], false, "{name} failed: {r}");
        r["content"][0]["text"].as_str().unwrap().to_string()
    }

    /// A tool call's parsed result body, asserting it was not an error.
    pub fn call_ok(&mut self, name: &str, args: Value) -> Value {
        serde_json::from_str(&self.call_ok_text(name, args)).unwrap()
    }

    /// A tool call that is refused. Returns the message the model sees.
    pub fn call_err(&mut self, name: &str, args: Value) -> String {
        let r = self.call(name, args);
        assert_eq!(r["isError"], true, "{name} unexpectedly succeeded: {r}");
        r["content"][0]["text"].as_str().unwrap().to_string()
    }

    pub fn call_with_meta(&mut self, name: &str, args: Value, tool_use_id: &str) -> Value {
        let r = self.rpc(
            "tools/call",
            json!({"name": name, "arguments": args,
                   "_meta": {"claudecode/toolUseId": tool_use_id}}),
        );
        assert_eq!(r["result"]["isError"], false, "{name} failed: {r}");
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    }
}

impl Drop for Shim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
