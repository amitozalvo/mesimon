//! Subscription quota (T-327): reading what each provider says is left of the
//! plan, off the writer thread, and keeping one reading per MACHINE.
//!
//! A quota belongs to a sign-in, not to a repo, so every board's daemon on the
//! machine shares one file, `~/.local/state/mesimon/usage.json`, and one lock
//! beside it: the daemon that holds the lock reads, the others pick its answer
//! up from the file. Nothing here runs unless a board asked
//! (`Command::SetUsageWants`, held per connection like the merge train), so a
//! daemon with no board open never probes.
//!
//! The probes are the providers' own CLIs, launched the way a pane is
//! (`mesimon exec --env …`: the same binary and the same sign-in):
//! - Claude: `claude -p` in stream-json mode answers the SDK's `get_usage`
//!   control request with the rows its `/usage` screen draws. No prompt and
//!   no tokens; no transcript (`--no-session-persistence`), none of the
//!   user's hooks (`disableAllHooks`) and none of their MCP servers.
//! - Codex: a short-lived `codex app-server` answers `account/rateLimits/read`.
//!   A Codex session mesimon runs reports the same snapshot after every turn
//!   (`account/rateLimits/updated`, carried in its runtime's observation
//!   snapshot), which spares the probe while one is working.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use mesimon_core::usage::{
    parse_claude, parse_codex, Cadence, Problem, Provider, ProviderUsage, Reading, Usage, Wants,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The shared file's own schema, like every state file's.
pub const USAGE_SCHEMA: u32 = 1;

/// How long a probe may take before it is called failed and killed. Claude's
/// cold start syncs plugins before it answers.
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(30);
const CODEX_TIMEOUT: Duration = Duration::from_secs(20);

/// How often the shared file is looked at for another daemon's answer.
const FILE_POLL_MS: u64 = 10_000;

/// The Claude probe's flags, after the binary: print mode in stream-json,
/// nothing persisted, no hook of the user's, no MCP server of theirs.
pub fn claude_args() -> Vec<String> {
    [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--settings",
        r#"{"disableAllHooks":true}"#,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn codex_args() -> Vec<String> {
    vec!["app-server".into()]
}

/// What a probe came back with.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Read(Reading),
    Problem(Problem),
    /// Another board's daemon holds the lock and is reading right now; its
    /// answer arrives through the file.
    Busy,
}

#[derive(Serialize, Deserialize, Default)]
struct FileBody {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    claude: ProviderUsage,
    #[serde(default)]
    codex: ProviderUsage,
}

/// The writer's account: the view the snapshot carries, each provider's
/// schedule, and what each connection asked for.
pub struct UsageState {
    view: Usage,
    claude: Cadence,
    codex: Cadence,
    in_flight: [bool; 2],
    /// Unix ms of this daemon's last probe that read: a mod's merged windows
    /// (T-581) refresh the reading's age but not the probe's, which alone
    /// sees a model's week.
    probed_at: [Option<u64>; 2],
    wants_by: HashMap<usize, Wants>,
    file: Option<PathBuf>,
    /// The file's (mtime, len) as last read or written: a change means another
    /// daemon wrote it.
    seen: Option<(SystemTime, u64)>,
    /// A newer mesimon wrote the file: read nothing from it, write nothing.
    barred: bool,
    polled_at_ms: u64,
}

fn slot(p: Provider) -> usize {
    match p {
        Provider::Claude => 0,
        Provider::Codex => 1,
    }
}

impl UsageState {
    /// `file` is the shared `usage.json`; `None` (no `HOME`) keeps the account
    /// in memory only.
    pub fn new(file: Option<PathBuf>) -> Self {
        let mut s = UsageState {
            view: Usage::default(),
            claude: Cadence::default(),
            codex: Cadence::default(),
            in_flight: [false; 2],
            probed_at: [None; 2],
            wants_by: HashMap::new(),
            file,
            seen: None,
            barred: false,
            polled_at_ms: 0,
        };
        s.reload();
        s
    }

    fn cadence(&mut self, p: Provider) -> &mut Cadence {
        match p {
            Provider::Claude => &mut self.claude,
            Provider::Codex => &mut self.codex,
        }
    }

    /// The snapshot's copy.
    pub fn view(&self) -> Usage {
        let mut v = self.view.clone();
        v.reading = Provider::ALL.into_iter().filter(|p| self.in_flight[slot(*p)]).collect();
        v
    }

    /// One connection's wants (`SetUsageWants`); the daemon reads for the
    /// union. True when the union moved.
    pub fn set_wants(&mut self, conn: usize, wants: Wants) -> bool {
        self.wants_by.insert(conn, wants);
        self.recompute()
    }

    /// A connection closed: what it asked for goes with it.
    pub fn drop_conn(&mut self, conn: usize) -> bool {
        self.wants_by.remove(&conn).is_some() && self.recompute()
    }

    fn recompute(&mut self) -> bool {
        let union = self.wants_by.values().fold(Wants::default(), |a, w| a.union(*w));
        let moved = union != self.view.wants;
        self.view.wants = union;
        moved
    }

    /// A turn ended in one of this provider's sessions: the quota moved.
    pub fn turn_ended(&mut self, p: Provider) {
        self.cadence(p).turn_ended = true;
    }

    /// A session stopped on a rate limit: read at once.
    pub fn limited(&mut self, p: Provider) {
        self.cadence(p).limited = true;
    }

    /// A person asked (`RefreshUsage`): read these now, wanted or not.
    pub fn ask(&mut self, which: Wants) {
        for p in Provider::ALL {
            if which.get(p) {
                self.cadence(p).asked = true;
            }
        }
    }

    /// The providers to read now. Each one returned is marked in flight; its
    /// [`UsageState::landed`] must follow.
    pub fn due(&mut self, now_ms: u64) -> Vec<Provider> {
        let mut out = Vec::new();
        for p in Provider::ALL {
            let wanted = self.view.wants.get(p);
            let usage = self.view.get(p);
            let read_at = self.probed_at[slot(p)].or(usage.reading.as_ref().map(|r| r.read_at_ms));
            let next_reset = usage.reading.as_ref().and_then(Reading::next_reset_ms);
            let c = self.cadence(p).clone();
            if self.in_flight[slot(p)]
                || !(wanted || c.asked)
                || !c.due(now_ms, read_at, next_reset)
            {
                continue;
            }
            self.cadence(p).start(now_ms);
            self.in_flight[slot(p)] = true;
            out.push(p);
        }
        out
    }

    /// A probe came back. True when the view changed.
    pub fn landed(&mut self, p: Provider, outcome: Outcome, now_ms: u64) -> bool {
        self.in_flight[slot(p)] = false;
        match outcome {
            Outcome::Busy => {
                // Nothing to say: the other daemon's answer comes through the
                // file, and the floor keeps this one from asking again first.
                self.polled_at_ms = 0;
                return true;
            }
            Outcome::Read(reading) => {
                self.probed_at[slot(p)] = Some(reading.read_at_ms);
                self.cadence(p).succeeded();
                self.view.get_mut(p).read(reading);
            }
            Outcome::Problem(problem) => {
                self.cadence(p).failed(&problem);
                self.view.get_mut(p).failed(problem, now_ms);
            }
        }
        self.save();
        true
    }

    /// A Codex session's own report (`account/rateLimits/updated`). Taken when
    /// it is newer than what is held; true when it was.
    pub fn passive(&mut self, p: Provider, reading: Reading) -> bool {
        let held = self.view.get(p).reading.as_ref().map_or(0, |r| r.read_at_ms);
        if reading.read_at_ms <= held {
            return false;
        }
        self.cadence(p).succeeded();
        self.view.get_mut(p).read(reading);
        self.save();
        true
    }

    /// The windows a Claude session's mod read at a turn's end (T-581),
    /// merged into the held reading (`usage::merge_mod`) and shared through
    /// the file like a probe's. True when the view changed. The probe keeps
    /// its own schedule: only it reads the windows the mod cannot see.
    pub fn merge_mod(&mut self, limits: &Value, now_ms: u64) -> bool {
        let held = self.view.claude.reading.as_ref();
        let Some(reading) = mesimon_core::usage::merge_mod(held, limits, now_ms) else {
            return false;
        };
        if held.is_some_and(|h| h.read_at_ms > now_ms) {
            return false;
        }
        self.view.claude.read(reading);
        self.save();
        true
    }

    /// Pick up another daemon's answer, at most every [`FILE_POLL_MS`]. True
    /// when the view changed.
    pub fn poll_file(&mut self, now_ms: u64) -> bool {
        if now_ms.saturating_sub(self.polled_at_ms) < FILE_POLL_MS {
            return false;
        }
        self.polled_at_ms = now_ms;
        let before = (self.view.claude.clone(), self.view.codex.clone());
        self.reload();
        before != (self.view.claude.clone(), self.view.codex.clone())
    }

    fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
        let m = std::fs::metadata(path).ok()?;
        Some((m.modified().ok()?, m.len()))
    }

    /// Read the shared file when it changed since we last saw it, keeping the
    /// newer account of each provider.
    fn reload(&mut self) {
        let Some(path) = self.file.clone() else { return };
        let stamp = Self::stamp(&path);
        if stamp.is_none() || stamp == self.seen {
            return;
        }
        self.seen = stamp;
        let Some(body) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<FileBody>(&t).ok())
        else {
            return;
        };
        if body.schema_version > USAGE_SCHEMA {
            self.barred = true;
            return;
        }
        self.view.claude = std::mem::take(&mut self.view.claude).newer(body.claude);
        self.view.codex = std::mem::take(&mut self.view.codex).newer(body.codex);
    }

    /// Write both accounts, merged with what another daemon may have written
    /// since: per provider, the later attempt wins.
    fn save(&mut self) {
        let Some(path) = self.file.clone() else { return };
        if self.barred {
            return;
        }
        self.seen = None;
        self.reload();
        if self.barred {
            return;
        }
        let body = FileBody {
            schema_version: USAGE_SCHEMA,
            claude: self.view.claude.clone(),
            codex: self.view.codex.clone(),
        };
        let Ok(text) = serde_json::to_string(&body) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if crate::store::write_atomic(&path, &(text + "\n"), crate::store::PRIVATE).is_ok() {
            self.seen = Self::stamp(&path);
        }
    }
}

/// The shared file: `~/.local/state/mesimon/usage.json`.
pub fn shared_file() -> Option<PathBuf> {
    crate::paths::state_root().ok().map(|r| r.join("usage.json"))
}

/// The machine's reading as the shared file holds it, with no daemon and no
/// probe — what `mesimon doctor` says. Empty when there is no file yet, or a
/// newer mesimon wrote it.
pub fn read_shared() -> Usage {
    UsageState::new(shared_file()).view()
}

/// Read one provider: take the machine's lock, run its CLI, parse the answer.
/// Runs on a worker. `argv` is the whole command line, launcher included.
pub fn probe(
    p: Provider,
    argv: &[String],
    cwd: &Path,
    lock: Option<&Path>,
    now_ms: u64,
) -> Outcome {
    let _held = match lock.map(try_lock) {
        Some(Some(file)) => Some(file),
        Some(None) => return Outcome::Busy,
        None => None,
    };
    match p {
        Provider::Claude => claude(argv, cwd, now_ms),
        Provider::Codex => codex(argv, cwd, now_ms),
    }
}

/// The lock beside the shared file, held for the probe's life. `None` when
/// another daemon holds it.
fn try_lock(path: &Path) -> Option<std::fs::File> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file =
        std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).ok()?;
    // SAFETY: flock on a descriptor this function owns; the lock lives as long
    // as the returned File.
    let rc = unsafe {
        libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&file), libc::LOCK_EX | libc::LOCK_NB)
    };
    (rc == 0).then_some(file)
}

/// A child with its stdout lines on a channel and its stderr in a buffer, in a
/// process group of its own so the whole of it can be stopped.
struct Probe {
    child: Child,
    lines: mpsc::Receiver<String>,
    stderr: std::sync::Arc<std::sync::Mutex<String>>,
    /// The stderr reader saw end-of-file: everything the CLI said is in.
    stderr_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Probe {
    fn spawn(argv: &[String], cwd: &Path) -> Result<Probe, Problem> {
        let (bin, args) = argv.split_first().ok_or_else(|| Problem::Failed("no command".into()))?;
        let mut child = Command::new(bin)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Problem::Missing
                } else {
                    Problem::Failed(format!("could not start it: {e}"))
                }
            })?;
        let (tx, lines) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
        }
        let stderr = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let stderr_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Some(mut err) = child.stderr.take() {
            let sink = stderr.clone();
            let done = stderr_done.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = err.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut s) = sink.lock() {
                        if s.len() < 4096 {
                            s.push_str(&String::from_utf8_lossy(&buf[..n]));
                        }
                    }
                }
                done.store(true, std::sync::atomic::Ordering::Release);
            });
        } else {
            stderr_done.store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(Probe { child, lines, stderr, stderr_done })
    }

    fn send(&mut self, v: &Value) {
        if let Some(stdin) = self.child.stdin.as_mut() {
            let _ = writeln!(stdin, "{v}");
            let _ = stdin.flush();
        }
    }

    /// The first stdout line `want` accepts, or why there was none.
    fn wait_for(
        &mut self,
        deadline: Instant,
        want: impl Fn(&Value) -> bool,
    ) -> Result<Value, Problem> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Problem::Failed("it did not answer in time".into()));
            }
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        if want(&v) {
                            return Ok(v);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(Problem::Failed("it did not answer in time".into()));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // It exited without answering: its last words say why,
                    // once the process is gone and its stderr has drained.
                    let settle = Instant::now() + Duration::from_secs(1);
                    let mut status = None;
                    while Instant::now() < settle {
                        status = status.or_else(|| self.child.try_wait().ok().flatten());
                        if status.is_some()
                            && self.stderr_done.load(std::sync::atomic::Ordering::Acquire)
                        {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    let err = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
                    return Err(exited(status.and_then(|s| s.code()), &err));
                }
            }
        }
    }
}

impl Drop for Probe {
    /// Stop the whole group: TERM, a moment, then KILL; always reaped.
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let pgid = self.child.id() as libc::pid_t;
        // SAFETY: signalling the process group this probe created.
        unsafe { libc::killpg(pgid, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: as above.
        unsafe { libc::killpg(pgid, libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

/// A CLI that exited before answering, classified from its exit code and its
/// stderr.
fn exited(code: Option<i32>, stderr: &str) -> Problem {
    let low = stderr.to_ascii_lowercase();
    if code == Some(127) || low.contains("no such file or directory") {
        return Problem::Missing;
    }
    if low.contains("unknown option") || low.contains("unexpected argument") {
        return Problem::Unsupported;
    }
    classify(stderr)
}

/// An error message, sorted into the problems the dialog can explain.
fn classify(message: &str) -> Problem {
    let low = message.to_ascii_lowercase();
    if [
        "401",
        "token_expired",
        "unauthorized",
        "not logged in",
        "log in",
        "login",
        "sign in",
        "oauth",
    ]
    .iter()
    .any(|w| low.contains(w))
    {
        return Problem::SignedOut;
    }
    if low.contains("api key") || low.contains("apikey") {
        return Problem::NoPlan;
    }
    if ["unknown", "unsupported", "not supported", "method not found", "-32601"]
        .iter()
        .any(|w| low.contains(w))
    {
        return Problem::Unsupported;
    }
    let first = message
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("it answered with an error");
    Problem::Failed(first.chars().take(80).collect())
}

fn claude(argv: &[String], cwd: &Path, now_ms: u64) -> Outcome {
    let mut probe = match Probe::spawn(argv, cwd) {
        Ok(p) => p,
        Err(problem) => return Outcome::Problem(problem),
    };
    probe.send(&json!({
        "type": "control_request",
        "request_id": "mesimon-usage",
        "request": {"subtype": "get_usage", "skip_behaviors": true}
    }));
    let answer = probe.wait_for(Instant::now() + CLAUDE_TIMEOUT, |v| {
        v["type"] == "control_response" && v["response"]["request_id"] == "mesimon-usage"
    });
    match answer {
        Ok(v) => {
            let r = &v["response"];
            if r["subtype"] == "success" {
                match parse_claude(&r["response"], now_ms) {
                    Ok(reading) => Outcome::Read(reading),
                    Err(problem) => Outcome::Problem(problem),
                }
            } else {
                Outcome::Problem(classify(r["error"].as_str().unwrap_or("")))
            }
        }
        Err(problem) => Outcome::Problem(problem),
    }
}

fn codex(argv: &[String], cwd: &Path, now_ms: u64) -> Outcome {
    let mut probe = match Probe::spawn(argv, cwd) {
        Ok(p) => p,
        Err(problem) => return Outcome::Problem(problem),
    };
    probe.send(&json!({
        "method": "initialize",
        "id": 0,
        "params": {"clientInfo": {"name": "mesimon", "title": "mesimon", "version": env!("CARGO_PKG_VERSION")}}
    }));
    probe.send(&json!({"method": "initialized"}));
    probe.send(&json!({"method": "account/rateLimits/read", "id": 1}));
    match probe.wait_for(Instant::now() + CODEX_TIMEOUT, |v| v["id"] == 1) {
        Ok(v) => {
            if let Some(err) = v.get("error") {
                let message = err["message"].as_str().unwrap_or("");
                if err["code"] == -32601 {
                    return Outcome::Problem(Problem::Unsupported);
                }
                return Outcome::Problem(classify(message));
            }
            let reading = parse_codex(&v["result"], now_ms).unwrap_or(Reading {
                read_at_ms: now_ms,
                plan: None,
                windows: Vec::new(),
            });
            Outcome::Read(reading)
        }
        Err(problem) => Outcome::Problem(problem),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::usage::Severity;

    const NOW: u64 = 1_790_870_640_000;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("msmn-usage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stand-in CLI: a shell script that answers one line.
    fn script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[test]
    fn the_claude_probe_parses_a_control_response() {
        let dir = scratch("claude");
        let answer = r#"{"type":"control_response","response":{"subtype":"success","request_id":"mesimon-usage","response":{"subscription_type":"max","rate_limits_available":true,"rate_limits":{"limits":[{"kind":"session","group":"session","percent":91,"resets_at":"2026-10-01T17:50:00Z","severity":"warning","is_active":true}]}}}}"#;
        let bin = script(
            &dir,
            "claude",
            &format!("read line\necho '{{\"type\":\"system\"}}'\necho '{answer}'\nsleep 5"),
        );
        let Outcome::Read(r) = probe(Provider::Claude, &[bin], &dir, None, NOW) else {
            panic!("no reading")
        };
        assert_eq!(r.windows[0].severity, Severity::Warning);
        assert_eq!(r.plan.as_deref(), Some("max"));
    }

    #[test]
    fn the_codex_probe_reads_its_answer_and_names_an_expired_sign_in() {
        let dir = scratch("codex");
        let ok = r#"{"id":1,"result":{"rateLimits":{"limitId":"codex","primary":{"usedPercent":12,"windowDurationMins":300,"resetsAt":1790880000}}}}"#;
        let bin = script(
            &dir,
            "codex",
            &format!("read a; read b; read c\necho '{{\"id\":0,\"result\":{{}}}}'\necho '{ok}'"),
        );
        let Outcome::Read(r) = probe(Provider::Codex, &[bin], &dir, None, NOW) else {
            panic!("no reading")
        };
        assert_eq!(r.windows[0].label, "5h");
        let expired = r#"{"id":1,"error":{"code":-32603,"message":"failed to fetch codex rate limits: 401 Unauthorized; token_expired"}}"#;
        let bin = script(&dir, "codex2", &format!("read a; read b; read c\necho '{expired}'"));
        assert_eq!(
            probe(Provider::Codex, &[bin], &dir, None, NOW),
            Outcome::Problem(Problem::SignedOut)
        );
    }

    #[test]
    fn a_missing_cli_an_old_one_and_a_held_lock() {
        let dir = scratch("missing");
        let gone = dir.join("nope").display().to_string();
        assert_eq!(
            probe(Provider::Claude, &[gone], &dir, None, NOW),
            Outcome::Problem(Problem::Missing)
        );
        let old = script(
            &dir,
            "old",
            "echo 'error: unknown option --no-session-persistence' >&2\nexit 1",
        );
        assert_eq!(
            probe(Provider::Claude, &[old], &dir, None, NOW),
            Outcome::Problem(Problem::Unsupported)
        );
        let lock = dir.join("usage.lock");
        let _held = try_lock(&lock).unwrap();
        let bin = script(&dir, "any", "exit 0");
        assert_eq!(probe(Provider::Claude, &[bin], &dir, Some(&lock), NOW), Outcome::Busy);
    }

    #[test]
    fn two_daemons_share_one_reading_through_the_file() {
        let dir = scratch("shared");
        let file = dir.join("usage.json");
        let mut a = UsageState::new(Some(file.clone()));
        let mut b = UsageState::new(Some(file.clone()));
        assert!(a.set_wants(1, Wants { claude: true, codex: false }));
        assert_eq!(a.due(NOW), vec![Provider::Claude]);
        assert!(a.due(NOW).is_empty(), "in flight");
        let reading = Reading { read_at_ms: NOW, plan: Some("max".into()), windows: Vec::new() };
        assert!(a.landed(Provider::Claude, Outcome::Read(reading.clone()), NOW));
        // B sees A's answer and does not read for itself.
        assert!(b.poll_file(NOW + FILE_POLL_MS));
        assert_eq!(b.view().claude.reading, Some(reading));
        b.set_wants(7, Wants { claude: true, codex: false });
        assert!(b.due(NOW + 20_000).is_empty(), "a minute's floor over A's reading");
        // Wants are per connection, and leave with it.
        assert!(a.drop_conn(1));
        assert_eq!(a.view().wants, Wants::default());
        assert!(a.due(NOW + 3_600_000).is_empty(), "nobody asked");
        // A person's ask reads even with nothing wanted.
        a.ask(Wants { claude: true, codex: false });
        assert_eq!(a.due(NOW + 3_600_000), vec![Provider::Claude]);
        // A passive Codex report is taken only when newer.
        let codex = Reading { read_at_ms: NOW + 5, ..Default::default() };
        assert!(a.passive(Provider::Codex, codex.clone()));
        assert!(!a.passive(Provider::Codex, codex));
        // A file from a newer mesimon is neither read nor written over.
        std::fs::write(&file, r#"{"schema_version":99}"#).unwrap();
        let mut c = UsageState::new(Some(file.clone()));
        c.landed(Provider::Claude, Outcome::Problem(Problem::SignedOut), NOW);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), r#"{"schema_version":99}"#);
    }
}
