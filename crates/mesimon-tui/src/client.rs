//! Daemon client: connect (spawning the daemon if needed), send typed envelopes,
//! receive responses; a reader thread routes async events to a dirty flag.
//! A dead connection (daemon update restart, crash) is survivable: the next
//! request reopens — respawning the daemon when it is gone — instead of
//! killing the TUI.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use mesimon_core::command::{Command, Envelope, ExeStamp, Notice, Response, PROTOCOL_VERSION};
use mesimon_core::Principal;
use mesimon_daemon::Paths;
use semver::Version;

/// The app's seam to the daemon: the real `Client` over `orch.sock`, or a
/// canned fake under test (the TestBackend harness never spawns a daemon).
pub trait Transport {
    fn request(&mut self, command: Command) -> Result<Response>;
    /// Non-blocking: true when an async board-changed event has arrived.
    fn poll_event(&mut self) -> bool;
    /// False when the connection is known dead (the app enters its slow
    /// reconnect cadence). Fakes are always healthy.
    fn healthy(&mut self) -> bool {
        true
    }
    /// A transport-level advisory (a build skew that would not settle),
    /// taken once. Fakes never produce one.
    fn take_notice(&mut self) -> Option<Notice> {
        None
    }
}

pub struct Client {
    repo_root: PathBuf,
    conn: Option<Conn>,
    /// A restart that did not converge disables the mechanism for this
    /// process. "At most one restart per connect" must not become "one
    /// restart every 2 s" on the app's reconnect cadence.
    restart_suppressed: bool,
    /// Surfaced once, when a skew could not be settled.
    notice: Option<Notice>,
}

/// One live connection's worth of channel plumbing — replaced wholesale on
/// reconnect so a stale reader thread can never route into fresh channels.
struct Conn {
    write: UnixStream,
    responses: Receiver<Response>,
    events: Receiver<()>,
    daemon: DaemonIdent,
}

/// What the daemon said about itself in the handshake.
#[derive(Debug, Clone, Default)]
struct DaemonIdent {
    pid: u32,
    build: String,
    exe_stamp: Option<ExeStamp>,
    detached: bool,
}

/// Is this daemon running older code than we are?
///
/// The version string is frozen at `0.1.0-dev` across every dev rebuild, so
/// the exe stamp is the load-bearing signal and `build` is the release-time
/// backstop. Both sides read the stamp through `mesimon_daemon::exe_stamp`,
/// so they cannot disagree about which fields they compare.
fn build_skew(d: &DaemonIdent, mine: Option<ExeStamp>) -> bool {
    // A daemon inside our own process is the in-process one the e2e tests
    // run. Structural guard, not an env seam: no test can trip it.
    if d.pid == std::process::id() {
        return false;
    }
    // Only a daemon mesimon spawned detached is ours to restart. A human's
    // foreground `mesimon daemon --repo` is never killed under them.
    if !d.detached {
        return false;
    }
    if std::env::var_os("MESIMON_NO_DAEMON_RESTART").is_some() {
        return false;
    }
    // Ordered, not merely different. "Different" would make two TUIs of
    // different builds on one repo restart each other forever: A restarts the
    // daemon to its build, B sees a mismatch and restarts it back. Only a
    // client that is strictly NEWER acts; an older one leaves the daemon
    // alone and says so.
    let ours = env!("CARGO_PKG_VERSION");
    if !d.build.is_empty() && d.build != ours {
        return match (Version::parse(&d.build), Version::parse(ours)) {
            (Ok(theirs), Ok(mine)) => mine > theirs,
            // Unparseable on either side: we cannot order them, so we cannot
            // know who is newer. Do nothing (D26 fails closed).
            _ => false,
        };
    }
    // Same version — the dogfood case, where 0.1.0-dev never moves. The exe
    // mtime is the only thing that does, and it orders the same way.
    match (d.exe_stamp, mine) {
        (Some(theirs), Some(ours)) => ours.mtime_ms > theirs.mtime_ms,
        // Unknown on either side is never "changed" (D26 fails closed).
        _ => false,
    }
}

impl Transport for Client {
    fn request(&mut self, command: Command) -> Result<Response> {
        let res = match self.conn.as_mut() {
            Some(c) => c.request(command),
            None => open_current(&self.repo_root, !self.restart_suppressed)
                .map(|(c, n)| {
                    // One failed restart disables the mechanism for this
                    // process — the 2 s reconnect cadence must not turn
                    // "one attempt per connect" into a restart storm.
                    if n.is_some() {
                        self.restart_suppressed = true;
                        self.notice = n;
                    }
                    self.conn.insert(c)
                })
                .and_then(|c| c.request(command)),
        };
        if res.is_err() {
            // Connection is toast; drop it so the next request reopens. No
            // blind replay — a mutation may have landed before its reply died.
            self.conn = None;
        }
        res
    }

    fn poll_event(&mut self) -> bool {
        match self.conn.as_mut().map(|c| c.events.try_recv()) {
            Some(Ok(())) => true,
            Some(Err(TryRecvError::Disconnected)) => {
                // Reader thread exited: the daemon hung up.
                self.conn = None;
                false
            }
            _ => false,
        }
    }

    fn healthy(&mut self) -> bool {
        self.conn.is_some()
    }

    fn take_notice(&mut self) -> Option<Notice> {
        self.notice.take()
    }
}

impl Client {
    pub fn connect(repo_root: &Path) -> Result<Self> {
        let (conn, notice) = open_current(repo_root, true)?;
        Ok(Client {
            repo_root: repo_root.to_path_buf(),
            conn: Some(conn),
            restart_suppressed: notice.is_some(),
            notice,
        })
    }
}

/// `open`, plus the at-most-one build-skew restart.
///
/// Deliberate reversal of "never automatic" (STALE-MAP): a daemon running
/// older code than its client is invisible until something behaves oddly, and
/// the restart itself is provably state-preserving — `Shutdown` touches no
/// session, panes live on the tmux server, and records re-derive from
/// `Unknown{DaemonRestarted}`. The guard rails are in `build_skew`.
fn open_current(repo_root: &Path, allow_restart: bool) -> Result<(Conn, Option<Notice>)> {
    let conn = open(repo_root)?;
    let mine = mesimon_daemon::exe_stamp();
    if !allow_restart || !build_skew(&conn.daemon, mine) {
        return Ok((conn, None));
    }
    let theirs = conn.daemon.clone();
    match restart_daemon(repo_root, conn) {
        Ok(fresh) => Ok((fresh, None)),
        Err(why) => {
            // We already asked the old daemon to stop, so get a working
            // connection back before surfacing anything: a board that opens
            // beats a diagnosis that does not.
            let conn = open(repo_root)?;
            let notice = Notice::new(
                "build_skew",
                format!("the daemon is running an older mesimon than this client — {why}",),
            )
            .with_detail(format!(
                "daemon {} (pid {}), client {}",
                if theirs.build.is_empty() { "unknown build" } else { &theirs.build },
                theirs.pid,
                env!("CARGO_PKG_VERSION"),
            ));
            Ok((conn, Some(notice)))
        }
    }
}

/// Shut the current daemon down and bring a fresh one up on our binary.
/// Single-shot: the caller reopens on failure rather than retrying here.
fn restart_daemon(repo_root: &Path, conn: Conn) -> std::result::Result<Conn, String> {
    let paths = Paths::for_repo(repo_root).map_err(|e| e.to_string())?;
    let sock = paths.orch_sock();
    let old_pid = conn.daemon.pid;

    let mut conn = conn;
    let _ = conn.request(Command::Shutdown);
    drop(conn); // release our fd so the daemon can finish hanging up

    // Wait for the socket to go. Load-bearing: the daemon answers Shutdown
    // BEFORE it breaks its loop, so for a moment the old listener is still
    // accepting and a reconnect would be handed the very daemon we asked to
    // die — which reads as "the restart failed".
    let mut gone = false;
    for _ in 0..60 {
        if !sock.exists() {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !gone {
        // Wedged (a blocked writer, a stuck provisioning join). Do not spawn
        // into that — churn without progress.
        return Err("it did not shut down".into());
    }

    let stream =
        connect_or_spawn(repo_root, &sock, Duration::from_secs(8)).map_err(|e| e.to_string())?;
    let fresh = handshake(stream).map_err(|e| e.to_string())?;
    if fresh.daemon.pid == old_pid {
        return Err("it came back as the same process".into());
    }
    Ok(fresh)
}

/// Connect, re-spawning the daemon on a cadence while we wait.
///
/// Repeat-spawning is safe and is the fix for the flock window: the daemon
/// unlinks its socket BEFORE releasing the lock, so a spawn landing in that
/// gap loses the race and exits quietly without touching the socket, the lock
/// file, or any state. One extra fork+exec, and the next attempt wins.
fn connect_or_spawn(repo_root: &Path, sock: &Path, budget: Duration) -> Result<UnixStream> {
    let deadline = std::time::Instant::now() + budget;
    let mut next_spawn = std::time::Instant::now();
    loop {
        let last = match UnixStream::connect(sock) {
            Ok(s) => return Ok(s),
            Err(e) => e,
        };
        if std::time::Instant::now() >= deadline {
            bail!("daemon did not come up: {last}");
        }
        if std::time::Instant::now() >= next_spawn {
            mesimon_daemon::spawn_detached(repo_root)?;
            next_spawn = std::time::Instant::now() + Duration::from_millis(400);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn open(repo_root: &Path) -> Result<Conn> {
    let paths = Paths::for_repo(repo_root)?;
    let sock = paths.orch_sock();
    let stream = connect_or_spawn(repo_root, &sock, Duration::from_secs(5))?;
    handshake(stream)
}

/// Wire up the reader thread, say Hello, subscribe — and record what the
/// daemon said about itself.
fn handshake(stream: UnixStream) -> Result<Conn> {
    let write = stream.try_clone()?;
    let (rtx, rrx): (Sender<Response>, Receiver<Response>) = channel();
    let (etx, erx) = channel();
    let reader = BufReader::new(stream);
    std::thread::spawn(move || {
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Ok(resp) = serde_json::from_str::<Response>(&line) {
                if rtx.send(resp).is_err() {
                    break;
                }
            } else if serde_json::from_str::<mesimon_core::command::Event>(&line).is_ok() {
                let _ = etx.send(());
            }
        }
    });

    let mut c = Conn { write, responses: rrx, events: erx, daemon: DaemonIdent::default() };
    let hello = c.request(Command::Hello {
        version: PROTOCOL_VERSION,
        client: format!("mesimon-tui/{}", env!("CARGO_PKG_VERSION")),
    })?;
    match hello {
        Response::Hello { daemon_pid, build, exe_stamp, detached, .. } => {
            c.daemon = DaemonIdent { pid: daemon_pid, build, exe_stamp, detached };
        }
        Response::Err { message } => bail!("daemon refused: {message}"),
        other => bail!("unexpected hello response: {other:?}"),
    }
    c.request(Command::Subscribe)?;
    Ok(c)
}

impl Conn {
    fn request(&mut self, command: Command) -> Result<Response> {
        let env = Envelope { principal: Principal::Local, command };
        let json = serde_json::to_string(&env)?;
        writeln!(self.write, "{json}").context("write to daemon")?;
        self.responses.recv_timeout(Duration::from_secs(10)).context("daemon response timeout")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(build: &str, mtime: u64, detached: bool) -> DaemonIdent {
        DaemonIdent {
            // Any pid but our own: the in-process guard is tested separately.
            pid: std::process::id() + 1,
            build: build.into(),
            exe_stamp: Some(ExeStamp { mtime_ms: mtime, len: 100 }),
            detached,
        }
    }

    fn stamp(mtime: u64) -> Option<ExeStamp> {
        Some(ExeStamp { mtime_ms: mtime, len: 100 })
    }

    /// `build_skew` reads `MESIMON_NO_DAEMON_RESTART`, and the seam test sets
    /// it — process-wide, while cargo runs the rest of this module on other
    /// threads. Every test here takes this lock first, or the seam leaks into
    /// whichever positive assertion happens to be mid-flight (seen:
    /// `empty_build_falls_through_to_the_stamp` failing about one workspace
    /// run in three). A poisoned lock is not a failure of the thing under
    /// test, so the guard steps over it.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A daemon living inside our own process is the in-process one the e2e
    /// tests run. Structural guard: no test can trip the restart path.
    #[test]
    fn never_restarts_a_daemon_in_our_own_process() {
        let _env = env_guard();
        let mut d = ident("0.0.1", 1, true);
        d.pid = std::process::id();
        assert!(!build_skew(&d, stamp(2)));
    }

    /// A human's foreground `mesimon daemon --repo` is never killed under
    /// them, however stale it looks.
    #[test]
    fn never_restarts_a_foreground_daemon() {
        let _env = env_guard();
        assert!(!build_skew(&ident("0.0.1", 1, false), stamp(2)));
    }

    /// The release-time signal: an OLDER daemon is skew.
    #[test]
    fn older_build_is_skew() {
        let _env = env_guard();
        assert!(build_skew(&ident("0.0.1", 1, true), stamp(1)));
    }

    /// ...and a NEWER daemon is not. This is the rule that stops two TUIs of
    /// different builds from restarting each other forever.
    #[test]
    fn newer_daemon_is_left_alone() {
        let _env = env_guard();
        assert!(!build_skew(&ident("99.0.0", 1, true), stamp(1)));
    }

    /// A version we cannot order is a version we do not act on.
    #[test]
    fn unparseable_build_is_never_skew() {
        let _env = env_guard();
        assert!(!build_skew(&ident("not-a-version", 1, true), stamp(1)));
    }

    /// The dogfood signal: the version is frozen at 0.1.0-dev across every
    /// rebuild, so the exe stamp is what actually moves.
    #[test]
    fn same_build_newer_exe_is_skew() {
        let _env = env_guard();
        let d = ident(env!("CARGO_PKG_VERSION"), 5, true);
        assert!(build_skew(&d, stamp(6)), "our binary is newer");
        assert!(!build_skew(&d, stamp(5)), "identical stamp is not skew");
        assert!(!build_skew(&d, stamp(4)), "an older client leaves it alone");
    }

    /// Unknown on either side means unknown, never "changed" (D26).
    #[test]
    fn unknown_stamp_is_never_skew() {
        let _env = env_guard();
        let mut d = ident(env!("CARGO_PKG_VERSION"), 1, true);
        assert!(!build_skew(&d, None));
        d.exe_stamp = None;
        assert!(!build_skew(&d, stamp(2)));
    }

    /// A daemon too old to name itself reports an empty build. It is left
    /// alone unless its exe stamp says otherwise — the bootstrap gap.
    #[test]
    fn empty_build_falls_through_to_the_stamp() {
        let _env = env_guard();
        let d = ident("", 5, true);
        assert!(build_skew(&d, stamp(6)));
        assert!(!build_skew(&d, stamp(4)));
    }

    /// The seam that turns the whole mechanism off.
    #[test]
    fn env_seam_disables_restart() {
        let _env = env_guard();
        let d = ident("0.0.1", 1, true);
        // (0.0.1 is older than any real build, so this is genuine skew.)
        assert!(build_skew(&d, stamp(2)), "skew without the seam");
        // SAFETY: `env_guard` holds off every other reader in this module,
        // and no daemon thread reads env here.
        unsafe { std::env::set_var("MESIMON_NO_DAEMON_RESTART", "1") };
        let off = build_skew(&d, stamp(2));
        unsafe { std::env::remove_var("MESIMON_NO_DAEMON_RESTART") };
        assert!(!off);
    }
}
