//! Daemon client: connect (spawning the daemon if needed), send typed envelopes,
//! receive responses; a reader thread routes async events to a dirty flag.
//! A dead connection (daemon update restart, crash) is survivable: the next
//! request reopens — respawning the daemon when it is gone — instead of
//! killing the TUI.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

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
    /// A known older wire protocol needs an explicit `U` handover.
    fn daemon_upgrade_needed(&self) -> bool {
        false
    }
}

pub struct Client {
    repo_root: PathBuf,
    conn: Option<Conn>,
    /// A restart that did not converge disables the mechanism for this
    /// process. "At most one restart per connect" must not become "one
    /// restart every 2 s" on the app's reconnect cadence.
    restart_suppressed: bool,
    /// A second connection on this board (`Client::connect_observer`), which
    /// never brings a daemon UP: reopening goes through `open_existing`, so
    /// nothing is spawned and nothing waits on the lock. The board's own
    /// client owns the daemon's life — and a `U` reload asks the daemon to
    /// stop and then waits for it to be GONE, a wait a thread respawning it
    /// behind the reload would turn into thirty seconds of nothing.
    observer: bool,
    /// Surfaced once, when a skew could not be settled.
    notice: Option<Notice>,
    legacy_daemon: bool,
}

#[derive(Debug)]
struct ProtocolMismatch(u32);

impl std::fmt::Display for ProtocolMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "daemon speaks protocol {}; this client speaks {PROTOCOL_VERSION}", self.0)
    }
}

impl std::error::Error for ProtocolMismatch {}

fn upgradeable(error: &anyhow::Error) -> bool {
    matches!(error.downcast_ref::<ProtocolMismatch>(), Some(ProtocolMismatch(1)))
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
            // Only an explicit local reload uses the compatible v1 control
            // messages. Never subscribe, fetch a board, or send a mutation
            // through a downgraded connection; never spawn during shutdown.
            None if !self.observer && matches!(command, Command::Shutdown) => {
                shutdown_existing(&Paths::for_repo(&self.repo_root)?.orch_sock())
            }
            None if self.observer => open_existing(&self.repo_root)
                .map(|c| self.conn.insert(c))
                .and_then(|c| c.request(command)),
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
        if let Err(error) = &res {
            // Connection is toast; drop it so the next request reopens. No
            // blind replay — a mutation may have landed before its reply died.
            self.conn = None;
            self.legacy_daemon = !self.observer && upgradeable(error);
        } else {
            self.legacy_daemon = false;
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

    fn daemon_upgrade_needed(&self) -> bool {
        self.legacy_daemon
    }
}

impl Client {
    /// Connect, or come up DISCONNECTED with the reason as a notice.
    ///
    /// A daemon that cannot be reached at launch used to be fatal — the
    /// board never opened and the error was one line on a terminal the user
    /// had just handed over to a `U` reload (dogfood 2026-09-04: the reload
    /// landed during a full parallel e2e run, the handover overran the
    /// client's stopwatch, and the board was gone with twelve sessions live
    /// behind it). The board is a better place to say "daemon unreachable"
    /// than an exit status: the app's reconnect cadence keeps dialling, and
    /// the reopen inside it respawns the daemon when it is truly gone.
    pub fn connect(repo_root: &Path) -> Result<Self> {
        // The path itself is fatal — nothing to dial, ever.
        let _ = Paths::for_repo(repo_root)?;
        match open_current(repo_root, true) {
            Ok((conn, notice)) => Ok(Client {
                repo_root: repo_root.to_path_buf(),
                conn: Some(conn),
                restart_suppressed: notice.is_some(),
                observer: false,
                notice,
                legacy_daemon: false,
            }),
            Err(why) => Ok(Client {
                repo_root: repo_root.to_path_buf(),
                conn: None,
                restart_suppressed: false,
                observer: false,
                legacy_daemon: upgradeable(&why),
                notice: Some(if upgradeable(&why) {
                    Notice::new(
                        "protocol_mismatch",
                        "older daemon — press U to upgrade; sessions keep running",
                    )
                    .with_detail(why.to_string())
                } else {
                    Notice::new("daemon_down", format!("no daemon yet — {why}"))
                        .with_detail("reconnecting on a 2 s cadence; `mesimon doctor` says why a daemon will not start".to_string())
                }),
            }),
        }
    }

    /// A SECOND connection on the same board: the notification thread's
    /// (T-291, `crate::notifier`).
    ///
    /// Two things it deliberately does not do. It never restarts the daemon
    /// over a build skew — the board's own client owns that decision, and two
    /// clients racing to restart one daemon is exactly the loop
    /// `build_skew`'s ordering rule exists to prevent. And it raises no
    /// notice, because a second client has nowhere to put an advisory: the
    /// status line and the advisory row are the board's.
    ///
    /// A daemon that is not there is not fatal, as for [`Client::connect`]:
    /// the caller comes up disconnected and dials again on its own cadence.
    pub fn connect_observer(repo_root: &Path) -> Result<Self> {
        let _ = Paths::for_repo(repo_root)?;
        Ok(Client {
            repo_root: repo_root.to_path_buf(),
            conn: open_existing(repo_root).ok(),
            restart_suppressed: true,
            observer: true,
            notice: None,
            legacy_daemon: false,
        })
    }
}

/// How long a client will wait on a daemon that is HOLDING the lock — the
/// old one finishing its shutdown, or one just spawned still binding —
/// before it calls the handover wedged. Generous on purpose: a shutdown
/// commits every pending settle through automove (worktree flags, git forks
/// per binding) and a busy box stretches all of it; the alternative is a
/// board that exits under the user with its sessions live.
pub const HANDOVER_MAX: Duration = Duration::from_secs(30);

/// Is `daemon.lock` flocked by a daemon right now?
///
/// The daemon takes the lock before it binds `orch.sock` and holds it to
/// process exit, so "socket gone, lock held" is the handover gap. Probed with
/// a SHARED lock, released at once: it never claims the daemon's role, only
/// asks whether someone holds it. A file that cannot be opened reads as free
/// — the caller falls back to the stopwatch it always had.
fn lock_held(lock: &Path) -> bool {
    let Ok(f) =
        std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock)
    else {
        return false;
    };
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(&f);
    // SAFETY: flock on a descriptor this function owns; the file closes on
    // drop, which releases anything the probe took.
    let rc = unsafe { libc::flock(fd, libc::LOCK_SH | libc::LOCK_NB) };
    if rc == 0 {
        unsafe { libc::flock(fd, libc::LOCK_UN) };
        return false;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK)
}

/// Wait for a daemon asked to stop to be GONE: socket unlinked and lock
/// released, up to [`HANDOVER_MAX`]. True when it is; false when the wait ran
/// out (the caller proceeds anyway — a wedged daemon is not improved by a
/// client that will not start).
///
/// The reload (`U`) calls this between `Shutdown` and its exec, because the
/// fresh client it becomes only tolerates a few seconds of no daemon — and
/// a shutdown is not bounded by the client's patience.
pub fn await_daemon_gone(repo_root: &Path) -> bool {
    let Ok(paths) = Paths::for_repo(repo_root) else { return true };
    let sock = paths.orch_sock();
    let lock = paths.lock_file();
    let start = Instant::now();
    let mut said = false;
    loop {
        if !sock.exists() && !lock_held(&lock) {
            return true;
        }
        let waited = start.elapsed();
        if waited >= HANDOVER_MAX {
            return false;
        }
        // The terminal is the user's again while this runs; a second of
        // nothing on it deserves a sentence.
        if !said && waited >= Duration::from_secs(1) {
            eprintln!("mesimon: waiting for the daemon to finish shutting down…");
            said = true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A sentence said only if a wait outlasts `delay`, on stderr, where
/// `await_daemon_gone`'s already goes. Drop it when the wait ends: dropped in
/// time, nothing is said; kept past the delay, it is said once.
///
/// It exists for the connect. From `run`'s connect to its `init_terminal`
/// the PRIMARY screen is what the user sees, and after a `U` that screen
/// holds whatever was last printed there — on a board that has been into a
/// session, tmux's `[detached (from session …)]` line. On an idle box the
/// connect is milliseconds and the line flashes; under two parallel e2e
/// suites it ran for a minute and that line was the only thing on screen
/// (dogfood 2026-09-05). A quick connect still says nothing, a slow one
/// says what it is doing, and a stuck one says the same instead of looking
/// dead.
pub struct LateWord {
    _cancel: Sender<()>,
}

impl LateWord {
    pub fn new(delay: Duration, sentence: &'static str) -> Self {
        Self::with(delay, move || eprintln!("{sentence}"))
    }

    fn with(delay: Duration, say: impl FnOnce() + Send + 'static) -> Self {
        let (tx, rx) = channel::<()>();
        std::thread::spawn(move || {
            // The sender dropping ends the wait: `Disconnected`, not a word.
            if rx.recv_timeout(delay) == Err(RecvTimeoutError::Timeout) {
                say();
            }
        });
        Self { _cancel: tx }
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
/// `budget` is how long a daemon may take to come up once the lock is FREE.
/// While something holds the lock — the old daemon finishing its shutdown,
/// or the one we just spawned between its flock and its bind — nothing is
/// spawned (it would only lose the flock and exit) and the budget does not
/// run; that wait is bounded by [`HANDOVER_MAX`] instead. Before this the
/// stopwatch ran through the handover, and a shutdown that outlasted it left
/// the user with no board and no daemon (dogfood 2026-09-04: a `U` during a
/// parallel e2e run).
///
/// Repeat-spawning stays safe: a spawn that lands in the last millisecond of
/// a handover loses the flock and exits quietly without touching the socket,
/// the lock file, or any state. One extra fork+exec, and the next attempt
/// wins.
fn connect_or_spawn(repo_root: &Path, sock: &Path, budget: Duration) -> Result<UnixStream> {
    let lock = Paths::for_repo(repo_root)?.lock_file();
    let start = Instant::now();
    let mut deadline = start + budget;
    let mut next_spawn = start;
    loop {
        let last = match UnixStream::connect(sock) {
            Ok(s) => return Ok(s),
            Err(e) => e,
        };
        let now = Instant::now();
        if lock_held(&lock) {
            if now >= start + HANDOVER_MAX {
                bail!(
                    "a daemon still holds {} after {} s — it did not finish shutting down",
                    lock.display(),
                    HANDOVER_MAX.as_secs()
                );
            }
            // The budget is the next daemon's, from the moment the lock is
            // its to take.
            deadline = now + budget;
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        if now >= deadline {
            bail!("daemon did not come up: {last}");
        }
        if now >= next_spawn {
            mesimon_daemon::spawn_detached(repo_root)?;
            next_spawn = now + Duration::from_millis(400);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Connect to a daemon that is ALREADY listening, or fail at once.
///
/// The observer's road (T-291). No spawn, no lock wait, no budget: a second
/// connection is a passive reader, and bringing a daemon up — or holding
/// still for thirty seconds while one shuts down — is the board's own
/// client's business, not a background thread's.
fn open_existing(repo_root: &Path) -> Result<Conn> {
    let paths = Paths::for_repo(repo_root)?;
    let sock = paths.orch_sock();
    let stream =
        UnixStream::connect(&sock).with_context(|| format!("no daemon on {}", sock.display()))?;
    handshake(stream)
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
    let mut c = hello(stream, PROTOCOL_VERSION)?;
    c.request(Command::Subscribe)?;
    Ok(c)
}

/// Hello and Shutdown have the same wire shape in protocols 1 and 2. This
/// connection is deliberately short-lived and never enters `Client::conn`.
fn shutdown_existing(sock: &Path) -> Result<Response> {
    let mut c = match hello(UnixStream::connect(sock)?, PROTOCOL_VERSION) {
        Ok(c) => c,
        Err(error) if upgradeable(&error) => hello(UnixStream::connect(sock)?, 1)?,
        Err(error) => return Err(error),
    };
    c.request(Command::Shutdown)
}

fn hello(stream: UnixStream, version: u32) -> Result<Conn> {
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
        version,
        client: format!("mesimon-tui/{}", env!("CARGO_PKG_VERSION")),
    })?;
    match hello {
        Response::Hello { version: received, daemon_pid, build, exe_stamp, detached }
            if received == version =>
        {
            c.daemon = DaemonIdent { pid: daemon_pid, build, exe_stamp, detached };
        }
        Response::Err { message } => {
            if let Some(other) = message
                .strip_prefix(&format!("protocol {version} unsupported; daemon speaks "))
                .and_then(|other| other.parse::<u32>().ok())
            {
                return Err(ProtocolMismatch(other).into());
            }
            bail!("daemon refused: {message}");
        }
        other => bail!("unexpected hello response: {other:?}"),
    }
    Ok(c)
}

impl Drop for Conn {
    fn drop(&mut self) {
        // Wake the reader even when a refused Hello leaves the peer open.
        // Otherwise each reconnect leaks a reader and its socket descriptor.
        let _ = self.write.shutdown(std::net::Shutdown::Both);
    }
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

    fn read_command(reader: &mut BufReader<UnixStream>) -> Command {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let envelope: Envelope = serde_json::from_str(&line).unwrap();
        assert_eq!(envelope.principal, Principal::Local);
        envelope.command
    }

    fn reply(reader: &mut BufReader<UnixStream>, response: Response) {
        writeln!(reader.get_mut(), "{}", serde_json::to_string(&response).unwrap()).unwrap();
    }

    /// Reproduce the old daemon's literal refusal on a socket which stays
    /// open. The normal handshake must neither downgrade nor leave its
    /// reader alive. Only the explicit shutdown road can use protocol 1.
    #[test]
    fn refused_hello_closes_the_connection_without_downgrading() {
        let (client, server) = UnixStream::pair().unwrap();
        server.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let peer = std::thread::spawn(move || {
            let mut reader = BufReader::new(server);
            assert!(matches!(read_command(&mut reader), Command::Hello { version: 2, .. }));
            reply(
                &mut reader,
                Response::Err { message: "protocol 2 unsupported; daemon speaks 1".into() },
            );
            let mut line = String::new();
            assert_eq!(reader.read_line(&mut line).unwrap(), 0, "no fallback or subscription");
        });
        let error = handshake(client).err().expect("v1 is not a board connection");
        assert!(upgradeable(&error));
        peer.join().unwrap();
    }

    #[test]
    fn explicit_shutdown_negotiates_only_the_known_legacy_control_protocol() {
        use std::os::unix::net::UnixListener;
        struct SocketFile(PathBuf);
        impl Drop for SocketFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        for version in [1, 2, 3] {
            // Short enough for macOS sun_path; unique, owned, and removed
            // on unwind. No daemon, tmux, model, or user repository involved.
            let socket =
                SocketFile(PathBuf::from(format!("/tmp/msmn-wire-{}.sock", ulid::Ulid::new())));
            let listener = UnixListener::bind(&socket.0).unwrap();
            listener.set_nonblocking(true).unwrap();
            let peer = std::thread::spawn(move || {
                let accept = || {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    loop {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                stream.set_nonblocking(false).unwrap();
                                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                                return BufReader::new(stream);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(Instant::now() < deadline, "control connection missing");
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            Err(e) => panic!("accept: {e}"),
                        }
                    }
                };
                let mut reader = accept();
                assert!(matches!(read_command(&mut reader), Command::Hello { version: 2, .. }));
                if version != 2 {
                    reply(
                        &mut reader,
                        Response::Err {
                            message: format!("protocol 2 unsupported; daemon speaks {version}"),
                        },
                    );
                    assert_eq!(reader.read_line(&mut String::new()).unwrap(), 0);
                    if version == 3 {
                        return;
                    }
                    reader = accept();
                    assert!(matches!(read_command(&mut reader), Command::Hello { version: 1, .. }));
                }
                reply(
                    &mut reader,
                    Response::Hello {
                        version,
                        daemon_pid: 123,
                        build: "0.0.1".into(),
                        exe_stamp: None,
                        detached: false,
                    },
                );
                assert!(matches!(read_command(&mut reader), Command::Shutdown));
                reply(&mut reader, Response::Ok);
                assert_eq!(reader.read_line(&mut String::new()).unwrap(), 0);
            });
            let result = shutdown_existing(&socket.0);
            if version == 3 {
                assert!(result.is_err(), "never downgrade an unknown protocol");
            } else {
                assert!(matches!(result.unwrap(), Response::Ok));
            }
            peer.join().unwrap();
        }
    }

    #[test]
    fn hello_must_echo_the_requested_version() {
        let (client, server) = UnixStream::pair().unwrap();
        let peer = std::thread::spawn(move || {
            let mut reader = BufReader::new(server);
            let _ = read_command(&mut reader);
            reply(
                &mut reader,
                Response::Hello {
                    version: 99,
                    daemon_pid: 123,
                    build: String::new(),
                    exe_stamp: None,
                    detached: false,
                },
            );
        });
        assert!(handshake(client).is_err());
        peer.join().unwrap();
    }

    /// Dropped inside its delay, a late word is never said; kept past it, it
    /// is said exactly once. Driven by channels, not sleeps, so a loaded box
    /// cannot make the quick one late.
    #[test]
    fn a_late_word_is_said_only_past_its_delay() {
        let (said, heard) = channel::<&'static str>();
        let quick = LateWord::with(Duration::from_secs(30), {
            let said = said.clone();
            move || {
                let _ = said.send("quick");
            }
        });
        drop(quick);
        let slow = LateWord::with(Duration::from_millis(10), move || {
            let _ = said.send("slow");
        });
        assert_eq!(heard.recv_timeout(Duration::from_secs(10)), Ok("slow"));
        drop(slow);
        // Every sender is gone once both threads have returned; the quick
        // one's returned `Disconnected` the moment it was dropped.
        assert!(matches!(
            heard.recv_timeout(Duration::from_secs(10)),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

    /// The probe reads a daemon's exclusive flock as held, and its release
    /// as free — without ever taking the lock itself.
    #[test]
    fn lock_probe_follows_the_holder() {
        let dir = std::env::temp_dir().join(format!("msmn-lock-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join("daemon.lock");
        assert!(!lock_held(&lock), "a lock nobody holds reads free (and the probe creates it)");
        assert!(lock.exists());

        // A stand-in daemon: exclusive flock, held for the scope.
        let holder = std::fs::OpenOptions::new().write(true).open(&lock).unwrap();
        let fd = std::os::unix::io::AsRawFd::as_raw_fd(&holder);
        assert_eq!(unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) }, 0);
        assert!(lock_held(&lock), "an exclusive holder reads held");
        // The probe took nothing: the holder can still re-lock exclusively.
        assert_eq!(unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) }, 0);
        drop(holder);
        // Release is prompt, not instantaneous, and the assertion has to say
        // the first thing rather than the second. Another thread in this
        // binary can be between `fork` and `exec` holding an INHERITED copy
        // of this descriptor — same open file description, so the same flock
        // — which keeps the lock alive past our close until CLOEXEC fires.
        // (`release.rs`'s install test runs a `Command`; at
        // `--test-threads=128` this landed in the window about 2% of runs,
        // and `lsof` found no holder at all by the next probe.) The product
        // never notices: both callers poll, so a stale `held` costs one more
        // 100 ms turn of a loop that was already turning.
        let deadline = Instant::now() + Duration::from_secs(5);
        while lock_held(&lock) {
            assert!(Instant::now() < deadline, "release reads free");
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

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
