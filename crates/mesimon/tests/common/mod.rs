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
    Command, Envelope, Event, ExternalItem, GraceItem, Pending, RepoGit, Resources, Response,
    WorktreeItem,
};
use mesimon_core::diff::FileEntry;
use mesimon_core::Principal;
use serde_json::{json, Value};

#[path = "../../../../ci/test_support.rs"]
pub mod support;

// Also used by the paid relay's product tests, compiled from its own repository
// beside this one: there `MESIMON_CORE_DIR` (its `.cargo/config.toml`) names
// this checkout, and those tests build mesimon first.
fn mesimon_binary() -> &'static str {
    static BIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        if let Some(bin) = option_env!("CARGO_BIN_EXE_mesimon") {
            return bin.to_string();
        }
        let core = option_env!("MESIMON_CORE_DIR")
            .unwrap_or(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        format!("{core}/target/debug/mesimon")
    })
}

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
    "MESIMON_CLAUDE_ROAD",
    "MESIMON_COMPOSER_WAIT_MS",
    "MESIMON_DAEMON_BIN",
    "MESIMON_DETACHED",
    "MESIMON_FAKE_BUILD",
    "MESIMON_HOOK_BIN",
    "MESIMON_NO_TAG_SEED",
    "MESIMON_PANE_QUIET_MS",
    "MESIMON_PINGPONG_MS",
    "MESIMON_SERVER_GUARD_TICKS",
    "MESIMON_SLEEP_MIN_AGE_MS",
    "MESIMON_INACTIVITY_PARK_TICKS",
    "MESIMON_INACTIVITY_MINUTE_MS",
    "MESIMON_MOD_BRIDGE_WAIT_MS",
    "MESIMON_RIG_NO_FLAGS",
    "MESIMON_WT_REFRESH_TICKS",
];

/// The road this run's daemons launch Claude on (T-574): `MESIMON_TEST_ROAD`
/// in the TEST process, `hooks` unless it says `mod`. The suite's second pass
/// sets it; the daemon gets `MESIMON_CLAUDE_ROAD` from `TestFixture`, never
/// `auto` (a stub must never be probed as Claude Code), and under `mod`
/// every stub is wrapped so the stand-in engine runs beside it and every
/// `hook_send` sends the mod's twin first.
pub fn test_road() -> &'static str {
    match std::env::var("MESIMON_TEST_ROAD").as_deref() {
        Ok("mod") => "mod",
        _ => "hooks",
    }
}

/// The in-pane half of the stand-in engine for the mod road (T-574).
pub const FAKE_CLAUDE_MOD: &str = include_str!("fake_claude_mod.py");

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
        fixture.set_env("MESIMON_HOOK_BIN", mesimon_binary());
        fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
        fixture.set_env("MESIMON_CLAUDE_HOME", fixture.dir.join("claude-home"));
        fixture.set_env("CODEX_HOME", fixture.dir.join("codex-home"));
        fixture.set_env("MESIMON_CLAUDE_ROAD", test_road());
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
            mesimon_binary().into(),
            "daemon".into(),
            "--repo".into(),
            repo.to_str().unwrap().into(),
        ])
    }

    pub fn spawn(&self, argv: Vec<String>) -> support::TestProcess {
        let mut env = self.env.borrow().clone();
        // Under the mod road a daemon's Claude panes run the stand-in engine
        // beside the stub (T-574). Wrapped on this spawn's copy of the env,
        // so a restart wraps the original once, never a wrapper.
        if argv.get(1).map(String::as_str) == Some("daemon")
            && env.get("MESIMON_CLAUDE_ROAD").map(String::as_str) == Some("mod")
        {
            if let Some(stub) = env.get("MESIMON_CLAUDE_BIN").cloned() {
                env.insert("MESIMON_CLAUDE_BIN".into(), self.wrap_for_mod(&stub));
            }
        }
        self.owner.spawn(argv, env)
    }

    /// A launcher that starts `fake_claude_mod.py` in the background, its
    /// output to a file (never the pane, where it would cover the composer),
    /// and then execs the stub as the pane's process, so the engine's parent
    /// is the stand-in for Claude Code.
    fn wrap_for_mod(&self, stub: &str) -> String {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let engine = self.dir.join("fake_claude_mod.py");
        if !engine.exists() {
            std::fs::write(&engine, FAKE_CLAUDE_MOD).unwrap();
        }
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let wrap = self.dir.join(format!("claude-mod-wrap-{n}.sh"));
        let quote = |p: &str| format!("'{}'", p.replace('\'', "'\\''"));
        // The engine types a `submit` into its own pane (T-575) through the
        // tmux build the server runs: a client of another build is refused.
        let tmux_bin = mesimon_backend_tmux::tmux_bin().display().to_string();
        std::fs::write(
            &wrap,
            format!(
                "#!/bin/sh\nMESIMON_FAKE_TMUX={} python3 {} >>{} 2>&1 </dev/null &\nexec {} \"$@\"\n",
                quote(&tmux_bin),
                quote(&engine.display().to_string()),
                quote(&self.dir.join("fake-mod.log").display().to_string()),
                quote(stub),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrap, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        wrap.display().to_string()
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
    hook_send_from_pane(sock, session, event, reason, None, body);
}

/// `hook_send_with` from a named tmux pane (`--pane <server pid>:%N`): what
/// a frame from inside a pane carries through `TMUX` and `TMUX_PANE`. The
/// test process is outside every pane, so a straggler from the OLD pane is
/// spelled here (T-245).
pub fn hook_send_from_pane(
    sock: &Path,
    session: &str,
    event: &str,
    reason: Option<&str>,
    pane: Option<&str>,
    body: &str,
) {
    // The mod sees every hook-set event before the command hook does, and
    // relays it: the stand-in engine's relay half (T-574). Both are sent on
    // both passes, and the daemon takes one (T-577): the mod's for a pane
    // that reports through it alone, the hook set's for every other. So a
    // test that pins a road, either one, reads the same under both passes.
    if mesimon_core::road::RELAYED_EVENTS.contains(&event) {
        let twin = mod_body(event, body);
        hook_send_road(sock, session, event, reason, pane, &twin, Some("mod"));
    }
    hook_send_road(sock, session, event, reason, pane, body, None);
}

/// What the mod relays for `body`: the payload itself, except `PreToolUse`,
/// which the mod rebuilds from the tool envelope (`register.ts`'s
/// `preToolUseBody`): the tool, its id, its input, and a subagent's id.
pub fn mod_body(event: &str, body: &str) -> String {
    if event != "PreToolUse" {
        return body.to_string();
    }
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let mut rebuilt = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": v.get("tool_name"),
        "tool_use_id": v.get("tool_use_id"),
        "tool_input": v.get("tool_input"),
    });
    if let Some(agent) = v.get("agent_id").filter(|a| a.is_string()) {
        rebuilt["agent_id"] = agent.clone();
    }
    rebuilt.to_string()
}

/// One frame through the real hook binary, on one road (`None` is the hook
/// set's), and nothing else: no twin.
pub fn hook_send_road(
    sock: &Path,
    session: &str,
    event: &str,
    reason: Option<&str>,
    pane: Option<&str>,
    body: &str,
    road: Option<&str>,
) {
    let mut cmd = Proc::new(mesimon_binary());
    cmd.args(["hook", "--sock"]).arg(sock).args(["--session", session, "--event", event]);
    if let Some(r) = reason {
        cmd.args(["--reason", r]);
    }
    if let Some(p) = pane {
        cmd.args(["--pane", p]);
    }
    if let Some(r) = road {
        cmd.args(["--road", r]);
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
/// Ask for the external census and wait for it to land (T-437): the reply
/// carries the *previous* answer with `external_scanning` set, and the
/// walk's result rides the next snapshots.
pub fn rescan_external(c: &mut TestClient) -> Vec<ExternalItem> {
    match c.request(Command::RescanExternal) {
        Response::Board { .. } => {}
        other => panic!("rescan: {other:?}"),
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match c.request(Command::Snapshot) {
            Response::Board { external, external_scanning: false, .. } => return external,
            Response::Board { .. } => {}
            other => panic!("snapshot after rescan: {other:?}"),
        }
        assert!(Instant::now() < deadline, "the external census never landed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

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

/// The command line a session's pane was started with: the launcher, its
/// `--set` variables and the agent's argv.
pub fn pane_start(tmux_sock: &Path, rec: &mesimon_core::board::SessionRecord) -> String {
    let out = tmux(tmux_sock)
        .args(["list-panes", "-t", &rec.sid16(), "-F", "#{pane_start_command}"])
        .output()
        .expect("tmux list-panes");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The tier of tools a Claude launch hands its session (T-117, T-577): on
/// the hook set's road the `--tools` word on the inline `--mcp-config`
/// blob's shim argv; on the mod road the `MESIMON_MOD_TOOLS` its pane
/// carries, which the mod registers (and no blob, no allow rule ride argv).
/// `None` when it hands none.
pub fn launch_tools(tmux_sock: &Path, rec: &mesimon_core::board::SessionRecord) -> Option<String> {
    let blob = rec.argv.iter().position(|a| a == "--mcp-config");
    if rec.road == mesimon_core::road::Road::Mod {
        assert!(blob.is_none(), "the mod road names no MCP server: {:?}", rec.argv);
        assert!(!rec.argv.iter().any(|a| a == "--allowedTools"), "{:?}", rec.argv);
        let start = pane_start(tmux_sock, rec);
        let at = start.find("MESIMON_MOD_TOOLS=")?;
        let word = start[at + "MESIMON_MOD_TOOLS=".len()..].split_whitespace().next()?;
        return Some(word.trim_matches('\'').to_string());
    }
    let blob: Value = serde_json::from_str(&rec.argv[blob? + 1]).unwrap();
    let args = blob["mcpServers"]["mesimon"]["args"].as_array().unwrap();
    let i = args.iter().position(|a| a == "--tools").expect("--tools on the shim's argv");
    Some(args[i + 1].as_str().unwrap().to_string())
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

/// The feed rows naming `ticket` whose command is exactly `cmd`.
pub fn feed_count(h: &Harness, cmd: &str, ticket: ulid::Ulid) -> usize {
    let needle = format!("\"cmd\":\"{cmd}\"");
    let id = ticket.to_string();
    std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(&needle) && l.contains(&id))
        .count()
}

/// Claude Code's composer as a stub paints it (T-570): a rule, the `❯ `
/// row, a rule, and a footer, at the bottom of the pane — then the cursor
/// goes home, so whatever the stub echoes or prints starts on row 0 as it
/// did before. The daemon pastes a launch's words only into a pane showing
/// one (`agents::claude::composer`), so every stand-in for Claude paints it.
pub const COMPOSER: &str = "printf '\\033[999;1H\\033[3A────────────────────\\n❯ \\n\
                            ────────────────────\\n  ? for shortcuts\\033[H'\n";

/// A shell stub for Claude that paints [`COMPOSER`] before its own body.
/// A stub in another language is left as written: it paints its own, or
/// takes no launch road that needs one.
pub fn claude_stub(body: &str) -> String {
    match body.split_once('\n') {
        Some((shebang, rest)) if shebang.starts_with("#!") && shebang.ends_with("sh") => {
            format!("{shebang}\n{COMPOSER}{rest}")
        }
        _ => body.to_string(),
    }
}

impl Harness {
    pub fn boot(name: &str, stub: Option<&str>) -> Option<Self> {
        Self::boot_with_env(name, stub, &[])
    }

    /// The stub paints Claude's composer first ([`claude_stub`]).
    pub fn boot_with_env(name: &str, stub: Option<&str>, env: &[(&str, &str)]) -> Option<Self> {
        Self::boot_bare(name, stub.map(claude_stub).as_deref(), env)
    }

    /// The stub exactly as written: one that paints its own composer, or
    /// none (T-570).
    pub fn boot_bare(name: &str, stub: Option<&str>, env: &[(&str, &str)]) -> Option<Self> {
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

    /// Stop the daemon cleanly and boot a fresh one on the same fixture:
    /// the state dir, the private tmux and its panes all survive, exactly as
    /// a `U` reload or a `pkill` does for a real board. Returns once the new
    /// socket answers.
    pub fn restart(&self) {
        if let Some(mut c) =
            TestClient::try_connect(&self.paths.orch_sock(), Duration::from_millis(500))
        {
            let _ = c.request(Command::Shutdown);
        }
        wait_until(Duration::from_secs(10), "the old daemon to leave", || {
            !self.paths.orch_sock().exists()
        });
        let _daemon = self.fixture.daemon(&self.repo);
        wait_until(Duration::from_secs(10), "the replacement daemon socket", || {
            self.paths.orch_sock().exists()
        });
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

/// The snapshot's worktree rows.
pub fn worktrees_of(c: &mut TestClient) -> Vec<WorktreeItem> {
    match c.request(Command::Snapshot) {
        Response::Board { worktrees, .. } => worktrees,
        other => panic!("not a board: {other:?}"),
    }
}

/// One ticket's worktree row, if it has one.
pub fn wt_of(c: &mut TestClient, ticket: ulid::Ulid) -> Option<WorktreeItem> {
    worktrees_of(c).into_iter().find(|w| w.ticket == ticket)
}

/// Poll until the ticket's binding reads `attached` (a lazy provision lands
/// off the writer thread), and return the row.
pub fn wait_attached(c: &mut TestClient, ticket: ulid::Ulid) -> WorktreeItem {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let item = wt_of(c, ticket);
        if let Some(w) = &item {
            if w.status == "attached" {
                return w.clone();
            }
        }
        assert!(Instant::now() < deadline, "binding never attached; last: {item:?}");
        std::thread::sleep(Duration::from_millis(150));
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
/// Claude Code speaks to a stdio MCP server. Under the mod road's pass
/// (T-577) a `tools/call` goes instead the way the mod makes it: one
/// `mesimon mcp --call <tool>` per call, the arguments on stdin, the result
/// on stdout, so every test's tool calls hold on both roads.
pub struct Shim {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
    next_id: i64,
    argv: Vec<String>,
}

impl Shim {
    pub fn start(sock: &Path, session: uuid::Uuid) -> Self {
        Self::start_with(sock, session, &[])
    }

    /// The shim with extra argv — `--tools <tier>`, the way the daemon's
    /// blob starts it (T-117).
    pub fn start_with(sock: &Path, session: uuid::Uuid, extra: &[&str]) -> Self {
        let mut argv: Vec<String> = vec!["mcp".into(), "--sock".into(), sock.display().to_string()];
        argv.extend(["--session".into(), session.to_string()]);
        argv.extend(extra.iter().map(|a| a.to_string()));
        let mut child = Proc::new(mesimon_binary())
            .args(&argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mcp shim");
        let out = BufReader::new(child.stdout.take().unwrap());
        Self { child, out, next_id: 1, argv }
    }

    /// One call as the mod makes it (T-577): `mesimon mcp --call`.
    fn call_as_the_mod(&self, params: &Value) -> Value {
        let mut argv = self.argv.clone();
        argv.extend(["--call".into(), params["name"].as_str().unwrap_or_default().into()]);
        if let Some(id) = params["_meta"]["claudecode/toolUseId"].as_str() {
            argv.extend(["--tool-use-id".into(), id.into()]);
        }
        let mut child = Proc::new(mesimon_binary())
            .args(&argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mesimon mcp --call");
        let args = params.get("arguments").cloned().unwrap_or(Value::Null);
        child.stdin.take().unwrap().write_all(args.to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().expect("mesimon mcp --call");
        serde_json::from_slice(&out.stdout).expect("mesimon mcp --call prints json")
    }

    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        if method == "tools/call" && test_road() == "mod" {
            return json!({"jsonrpc": "2.0", "id": id, "result": self.call_as_the_mod(&params)});
        }
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
