//! A tmux-owned supervisor for the native Codex UI and its dedicated server.
//!
//! JSON messages cross the relay unchanged. Observation never acknowledges an
//! approval, starts a turn, or modifies the user's conversation.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use mesimon_core::board::{SessionState, UnknownReason};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tungstenite::protocol::{Message, WebSocketConfig};
use tungstenite::WebSocket;

use super::observation::{Ledger, Update};
use super::{write_json, RuntimeConfig, Snapshot};
use crate::agents::{AgentActivity, AgentPreview};

const MAX_MESSAGE: usize = 32 * 1024 * 1024;
const MAX_BUFFERED: usize = 256;
const START_TIMEOUT: Duration = Duration::from_secs(20);
const HEARTBEAT: Duration = Duration::from_millis(250);
static TERMINATED: AtomicBool = AtomicBool::new(false);

extern "C" fn terminate(_: libc::c_int) {
    TERMINATED.store(true, Ordering::Relaxed);
}

struct Signals {
    term: libc::sighandler_t,
    hup: libc::sighandler_t,
    interrupt: libc::sighandler_t,
}
impl Signals {
    fn install() -> Self {
        TERMINATED.store(false, Ordering::Relaxed);
        // This subcommand has its own process; it never installs handlers in
        // the board daemon. The handler performs only an atomic store.
        unsafe {
            Self {
                term: libc::signal(libc::SIGTERM, terminate as *const () as libc::sighandler_t),
                hup: libc::signal(libc::SIGHUP, terminate as *const () as libc::sighandler_t),
                interrupt: libc::signal(libc::SIGINT, terminate as *const () as libc::sighandler_t),
            }
        }
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGTERM, self.term);
            libc::signal(libc::SIGHUP, self.hup);
            libc::signal(libc::SIGINT, self.interrupt);
        }
    }
}

/// PID reuse cannot turn an unrelated process into a cleanup target. Parent
/// and group can change during exec/detach; the kernel start identity cannot.
#[derive(Clone, Debug)]
struct ProcessIdentity {
    pid: libc::pid_t,
    parent: libc::pid_t,
    group: libc::pid_t,
    started: (u64, u64),
    zombie: bool,
    stopped: bool,
}
impl ProcessIdentity {
    fn same(&self, other: &Self) -> bool {
        self.pid == other.pid && self.started == other.started
    }
}

#[cfg(target_os = "macos")]
fn process_identity(pid: libc::pid_t) -> Result<Option<ProcessIdentity>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size)
    };
    if read != size {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ENOENT)) {
            return Ok(None);
        }
        return Err(anyhow!("cannot identify process {pid}: {error}"));
    }
    let info = unsafe { info.assume_init() };
    Ok(Some(ProcessIdentity {
        pid,
        parent: info.pbi_ppid as i32,
        group: info.pbi_pgid as i32,
        started: (info.pbi_start_tvsec, info.pbi_start_tvusec),
        zombie: info.pbi_status == libc::SZOMB,
        stopped: info.pbi_status == libc::SSTOP,
    }))
}

#[cfg(target_os = "linux")]
fn process_identity(pid: libc::pid_t) -> Result<Option<ProcessIdentity>> {
    let text = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    // comm is parenthesized and may itself contain spaces/parentheses.
    let tail = text.rsplit_once(')').context("malformed process stat")?.1;
    let fields: Vec<_> = tail.split_whitespace().collect();
    if fields.len() < 20 {
        bail!("truncated process stat");
    }
    Ok(Some(ProcessIdentity {
        pid,
        parent: fields[1].parse()?,
        group: fields[2].parse()?,
        started: (fields[19].parse()?, 0),
        zombie: fields[0] == "Z",
        stopped: matches!(fields[0], "T" | "t"),
    }))
}

fn process_table() -> Result<BTreeMap<libc::pid_t, ProcessIdentity>> {
    #[cfg(target_os = "macos")]
    let pids = {
        let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 || count > 65_280 {
            bail!("process enumeration unavailable or exceeds bound");
        }
        let mut pids = vec![0i32; count as usize + 256];
        let length = unsafe {
            libc::proc_listallpids(
                pids.as_mut_ptr().cast(),
                (pids.len() * std::mem::size_of::<i32>()) as libc::c_int,
            )
        };
        if length <= 0 || length as usize >= pids.len() {
            bail!("process enumeration was incomplete");
        }
        pids.truncate(length as usize);
        pids
    };
    #[cfg(target_os = "linux")]
    let pids = {
        let mut pids = Vec::new();
        for entry in std::fs::read_dir("/proc")? {
            if let Ok(pid) = entry?.file_name().to_string_lossy().parse::<i32>() {
                pids.push(pid);
            }
            if pids.len() > 65_536 {
                bail!("process enumeration exceeds bound");
            }
        }
        pids
    };
    let mut table = BTreeMap::new();
    for pid in pids {
        if pid <= 0 {
            continue;
        }
        // Other users' metadata may be private. A known owned PID is checked
        // separately below; inability to read it is never evidence of exit.
        if let Ok(Some(identity)) = process_identity(pid) {
            table.insert(pid, identity);
        }
    }
    Ok(table)
}

#[derive(Default)]
struct ProcessTree {
    known: BTreeMap<libc::pid_t, ProcessIdentity>,
    server: Option<ProcessIdentity>,
    uncertain: bool,
    last_sample: Option<Instant>,
}
impl ProcessTree {
    fn root(&mut self, pid: u32, server: bool) -> Result<()> {
        let identity = process_identity(pid as i32).inspect_err(|_| {
            self.uncertain = true;
        })?;
        let Some(identity) = identity else {
            self.uncertain = true;
            bail!("owned process vanished before identification");
        };
        if server {
            self.server = Some(identity.clone());
        }
        self.known.insert(pid as i32, identity);
        Ok(())
    }

    fn extend(&mut self, table: &BTreeMap<libc::pid_t, ProcessIdentity>) -> Result<()> {
        loop {
            let children: Vec<_> = table
                .values()
                .filter(|entry| {
                    !entry.zombie
                        && !self.known.contains_key(&entry.pid)
                        && self.known.get(&entry.parent).is_some_and(|parent| {
                            table.get(&parent.pid).is_some_and(|current| parent.same(current))
                        })
                })
                .cloned()
                .collect();
            if children.is_empty() {
                break;
            }
            if self.known.len() + children.len() > 4096 {
                self.uncertain = true;
                bail!("owned descendant bound exceeded");
            }
            self.known.extend(children.into_iter().map(|entry| (entry.pid, entry)));
        }
        Ok(())
    }

    fn refresh(&mut self, closing: bool) -> Result<()> {
        let mut table = process_table()?;
        for identity in self.known.values() {
            // Table omission could be a permission failure, not an exit.
            if let std::collections::btree_map::Entry::Vacant(entry) = table.entry(identity.pid) {
                if let Some(current) = process_identity(identity.pid)? {
                    entry.insert(current);
                }
            }
        }
        if !closing
            && self.server.as_ref().is_some_and(|server| {
                !table
                    .get(&server.pid)
                    .is_some_and(|current| server.same(current) && !current.zombie)
            })
        {
            // An unexpectedly lost ancestor may have reparented a new child
            // before a sample. Never claim a complete cleanup audit in that gap.
            self.uncertain = true;
        }
        self.known.retain(|pid, identity| {
            table.get(pid).is_some_and(|current| identity.same(current) && !current.zombie)
        });
        for (pid, before) in &mut self.known {
            if let Some(current) = table.get(pid) {
                *before = current.clone();
            }
        }
        self.extend(&table)?;
        self.last_sample = Some(Instant::now());
        Ok(())
    }

    fn sample(&mut self) -> Result<()> {
        if self.last_sample.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
            if let Err(error) = self.refresh(false) {
                self.uncertain = true;
                return Err(error);
            }
        }
        Ok(())
    }

    fn signal(&self, signal: libc::c_int) -> Result<()> {
        for before in self.known.values() {
            if let Some(current) = process_identity(before.pid)? {
                if before.same(&current) && !current.zombie {
                    signal_owned(before.pid, signal)?;
                }
            }
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        // Capture and freeze ancestry before the server can exit/reparent its
        // tools. Repeat until every descendant, including its own groups, is
        // frozen. This also catches forks racing the first snapshot.
        self.refresh(false)?;
        let freeze_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            self.signal(libc::SIGSTOP)?;
            let before: Vec<_> =
                self.known.values().map(|entry| (entry.pid, entry.started)).collect();
            self.refresh(true)?;
            let after: Vec<_> =
                self.known.values().map(|entry| (entry.pid, entry.started)).collect();
            if after == before && self.known.values().all(|entry| entry.stopped) {
                break;
            }
            if Instant::now() >= freeze_deadline {
                self.uncertain = true;
                break;
            }
        }
        // SIGCONT lets queued SIGTERM handlers run. All later signals recheck
        // start identities, including descendants now reparented to init.
        for signal in [libc::SIGTERM, libc::SIGKILL] {
            self.signal(signal)?;
            self.signal(libc::SIGCONT)?;
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                self.refresh(true)?;
                if self.known.is_empty() {
                    if self.uncertain {
                        bail!("descendant ownership had an observation gap");
                    }
                    return Ok(());
                }
                // Newly discovered descendants receive the same rung.
                self.signal(signal)?;
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        bail!(
            "owned descendants survived cleanup: {:?}",
            self.known.values().map(|entry| (entry.pid, entry.group)).collect::<Vec<_>>()
        )
    }
}

/// Own only processes and paths created by this supervisor. Cleanup follows
/// exact descendant ancestry even when a tool creates a separate process group.
#[derive(Default)]
struct Owned {
    server: Option<Child>,
    native: Option<Child>,
    sockets: Vec<PathBuf>,
    tree: ProcessTree,
}
impl Owned {
    fn cleanup(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        if self.server.is_some() || self.native.is_some() || !self.tree.known.is_empty() {
            if let Err(error) = self.tree.stop() {
                // Never strand a process we froze if inspection fails.
                let _ = self.tree.signal(libc::SIGCONT);
                failures.push(format!("descendant cleanup: {error}"));
            }
        }
        // Stop checkout-mutating tools before the display process. Each
        // successful stop consumes its handle so Drop cannot signal a reused
        // PID after cleanup has already been acknowledged.
        if let Some(child) = &mut self.server {
            match stop(child) {
                Ok(()) => {
                    self.server.take();
                }
                Err(error) => failures.push(format!("app-server cleanup: {error}")),
            }
        }
        if let Some(child) = &mut self.native {
            match stop(child) {
                Ok(()) => {
                    self.native.take();
                }
                Err(error) => failures.push(format!("native terminal cleanup: {error}")),
            }
        }
        self.sockets.retain(|path| match std::fs::remove_file(path) {
            Ok(()) => false,
            Err(error) if error.kind() == ErrorKind::NotFound => false,
            Err(error) => {
                failures.push(format!("socket cleanup {}: {error}", path.display()));
                true
            }
        });
        if !failures.is_empty() {
            bail!("{}", failures.join("; "));
        }
        Ok(())
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // A panic or early return still tries bounded cleanup, but only the
        // explicit successful cleanup path may emit a stopped acknowledgement.
        if let Err(error) = self.cleanup() {
            eprintln!("Codex runtime cleanup failed: {error}");
        }
    }
}

fn process_exists(target: libc::pid_t) -> Result<bool> {
    if unsafe { libc::kill(target, 0) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error.into())
    }
}

fn signal_owned(target: libc::pid_t, signal: libc::c_int) -> Result<()> {
    if unsafe { libc::kill(target, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error.into())
    }
}

fn stop(child: &mut Child) -> Result<()> {
    let pid = child.id() as libc::pid_t;
    let target = pid;
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    signal_owned(target, libc::SIGTERM)?;
    for (signal, grace) in
        [(libc::SIGTERM, Duration::from_secs(2)), (libc::SIGKILL, Duration::from_secs(2))]
    {
        if signal == libc::SIGKILL && process_exists(target)? {
            eprintln!("Codex runtime: force-stopping owned child {target}");
            signal_owned(target, signal)?;
        }
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            let reaped = child.try_wait()?.is_some();
            // Descendants were checked separately; this reaps only the
            // exact direct child whose handle this supervisor owns.
            if reaped {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    bail!("owned child {target} still exists after cleanup deadline")
}

pub fn run(config_path: &Path) -> Result<()> {
    let mut bytes = Vec::new();
    File::open(config_path)?.take(65_537).read_to_end(&mut bytes)?;
    if bytes.len() > 65_536 {
        bail!("Codex runtime configuration exceeds 64 KiB");
    }
    let config: RuntimeConfig = serde_json::from_slice(&bytes)?;
    let _signals = Signals::install();
    let mut observation = Observation::new(&config);
    observation.publish(&config)?;
    let result = supervise(&config, &mut observation);
    observation.snapshot.observation_hold = true;
    observation.snapshot.state = SessionState::Unknown { reason: UnknownReason::ObservationLost };
    observation.preview.activity = None;
    // A terminal pane exit is interpreted by the daemon's normal pane reaper.
    // Loss of the observation stream must never manufacture a completion.
    let publish_result = observation.publish(&config);
    let result = result.and(publish_result);
    if let Err(error) = &result {
        // The pane may disappear before a person can inspect its stderr.
        // Preserve a bounded diagnostic beside the owned app-server log.
        use std::io::Write;
        let path = config.snapshot_path.with_extension("app-server.log");
        if let Ok(mut log) = OpenOptions::new().create(true).append(true).mode(0o600).open(path) {
            let diagnostic = bound_text(format!("{error:#}"), 4096);
            let _ = writeln!(log, "Mesimon Codex runtime failed: {diagnostic}");
        }
    }
    result
}

fn socket_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE))
}

fn stream_deadlines(stream: &UnixStream, handshake: bool) -> Result<()> {
    // On macOS accept inherits the listener's O_NONBLOCK flag. Deadlines
    // alone do not clear it: a native startup burst would make send/flush
    // fail with EAGAIN instead of waiting for the terminal reader.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(if handshake {
        Duration::from_secs(2)
    } else {
        Duration::from_millis(2)
    }))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(())
}

fn supervise(config: &RuntimeConfig, observation: &mut Observation) -> Result<()> {
    let mut owned = Owned::default();
    let result = relay(config, observation, &mut owned);
    let cleanup = owned.cleanup();
    observation.snapshot.stopped = cleanup.is_ok();
    if let Err(error) = &cleanup {
        eprintln!("Codex runtime cleanup failed: {error}");
    }
    result.and(cleanup)
}

fn relay(config: &RuntimeConfig, observation: &mut Observation, owned: &mut Owned) -> Result<()> {
    let listener = UnixListener::bind(&config.proxy_socket).context("bind native Codex relay")?;
    owned.sockets.push(config.proxy_socket.clone());
    listener.set_nonblocking(true)?;
    if config.upstream_socket.exists() {
        bail!("Codex upstream socket already exists: {}", config.upstream_socket.display());
    }
    let log_path = config.snapshot_path.with_extension("app-server.log");
    let log = OpenOptions::new().create(true).append(true).mode(0o600).open(&log_path)?;
    let mut server = Command::new(&config.executable);
    server
        .args(&config.config_flags)
        .args(["app-server", "--listen"])
        .arg(format!("unix://{}", config.upstream_socket.display()))
        .current_dir(&config.cwd)
        .envs(config.env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0);
    owned.server = Some(server.spawn().context("start Codex app-server")?);
    owned.tree.root(owned.server.as_ref().expect("spawned server").id(), true)?;
    owned.sockets.push(config.upstream_socket.clone());
    let deadline = Instant::now() + START_TIMEOUT;
    while !config.upstream_socket.exists() {
        owned.tree.sample()?;
        check_children(owned)?;
        if TERMINATED.load(Ordering::Relaxed) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("Codex app-server startup timed out; see {}", log_path.display());
        }
        observation.publish(config)?;
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut native = Command::new(&config.executable);
    native
        .arg("--remote")
        .arg(format!("unix://{}", config.proxy_socket.display()))
        .arg("-C")
        .arg(&config.cwd)
        .args(&config.config_flags)
        .current_dir(&config.cwd)
        .envs(config.env.iter().cloned())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(thread) = &config.resume {
        native.args(["resume", thread]);
    }
    owned.native = Some(native.spawn().context("start native Codex terminal")?);
    owned.tree.root(owned.native.as_ref().expect("spawned terminal").id(), false)?;
    let stream = loop {
        owned.tree.sample()?;
        check_children(owned)?;
        if TERMINATED.load(Ordering::Relaxed) {
            return Ok(());
        }
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        if Instant::now() >= deadline {
            bail!("native Codex connection timed out");
        }
        observation.publish(config)?;
        std::thread::sleep(Duration::from_millis(20));
    };
    stream_deadlines(&stream, true)?;
    let mut native = tungstenite::accept_with_config(stream, Some(socket_config()))
        .map_err(|e| anyhow!("native Codex handshake: {e}"))?;
    let upstream = UnixStream::connect(&config.upstream_socket)?;
    stream_deadlines(&upstream, true)?;
    let (mut upstream, _) =
        tungstenite::client::client_with_config("ws://localhost/", upstream, Some(socket_config()))
            .map_err(|e| anyhow!("Codex app-server handshake: {e}"))?;
    stream_deadlines(native.get_ref(), false)?;
    stream_deadlines(upstream.get_ref(), false)?;
    let mut heartbeat = Instant::now();
    loop {
        if TERMINATED.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Some(child) = &mut owned.native {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    bail!("native Codex exited ({status})");
                }
                return Ok(());
            }
        }
        if let Some(child) = &mut owned.server {
            if let Some(status) = child.try_wait()? {
                bail!("Codex app-server exited ({status})");
            }
        }
        // Small fair batches keep UI traffic and heartbeats moving even during
        // streamed replies. No additional initialize/subscription is injected.
        for _ in 0..32 {
            let Some(message) = receive(&mut native).context("read native Codex")? else { break };
            if let Message::Text(text) = &message {
                observation.outgoing(&serde_json::from_str::<Value>(text)?)?;
            }
            let closed = matches!(message, Message::Close(_));
            upstream.send(message).context("forward native Codex message")?;
            if closed {
                return Ok(());
            }
        }
        for _ in 0..32 {
            let Some(message) = receive(&mut upstream).context("read Codex app-server")? else {
                break;
            };
            if let Message::Text(text) = &message {
                observation.incoming(config, serde_json::from_str::<Value>(text)?)?;
            }
            let closed = matches!(message, Message::Close(_));
            native.send(message).context("forward Codex app-server message")?;
            if closed {
                bail!("Codex app-server disconnected");
            }
        }
        observation.poll_audit(config);
        if heartbeat.elapsed() >= HEARTBEAT {
            owned.tree.sample()?;
            observation.publish(config)?;
            heartbeat = Instant::now();
        }
    }
}

fn check_children(owned: &mut Owned) -> Result<()> {
    if let Some(child) = &mut owned.server {
        if let Some(status) = child.try_wait()? {
            bail!("Codex app-server exited ({status})");
        }
    }
    if let Some(child) = &mut owned.native {
        if let Some(status) = child.try_wait()? {
            bail!("native Codex exited during startup ({status})");
        }
    }
    Ok(())
}

fn receive(socket: &mut WebSocket<UnixStream>) -> Result<Option<Message>> {
    match socket.read() {
        Ok(Message::Ping(_) | Message::Pong(_)) => {
            socket.flush()?;
            Ok(None)
        }
        Ok(message @ (Message::Text(_) | Message::Close(_))) => Ok(Some(message)),
        Ok(_) => bail!("unsupported native Codex WebSocket message"),
        Err(tungstenite::Error::Io(error))
            if matches!(
                error.kind(),
                ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug)]
struct Selection {
    method: String,
    thread: Option<String>,
}
struct Audit {
    revision: u64,
    thread: Value,
}
type StateEvidence =
    (Option<String>, Option<String>, SessionState, bool, Option<String>, bool, Option<String>);

struct Observation {
    published_state: Option<StateEvidence>,
    snapshot: Snapshot,
    ledger: Option<Ledger>,
    pending: BTreeMap<String, Selection>,
    buffered: Vec<Value>,
    buffered_bytes: usize,
    preview: AgentPreview,
    published_preview: Option<AgentPreview>,
    revision: u64,
    needs_audit: bool,
    audit: Option<Receiver<Result<Audit>>>,
    next_audit: Instant,
}
impl Observation {
    fn new(config: &RuntimeConfig) -> Self {
        Self {
            published_state: None,
            snapshot: Snapshot {
                session: config.session,
                generation: config.generation,
                sequence: 0,
                heartbeat_ms: 0,
                thread_id: None,
                turn_id: None,
                state: SessionState::Spawning,
                observation_hold: true,
                history_path: None,
                plan: None,
                plan_key: None,
                stopped: false,
            },
            ledger: None,
            pending: BTreeMap::new(),
            buffered: Vec::new(),
            buffered_bytes: 0,
            preview: AgentPreview::default(),
            published_preview: None,
            revision: 0,
            needs_audit: false,
            audit: None,
            next_audit: Instant::now(),
        }
    }

    fn outgoing(&mut self, frame: &Value) -> Result<()> {
        let Some(method) = frame["method"].as_str() else { return Ok(()) };
        if !matches!(method, "thread/start" | "thread/resume" | "thread/fork")
            || frame["params"]["ephemeral"] == true
        {
            return Ok(());
        }
        let Some(id) = frame.get("id") else { return Ok(()) };
        if self.pending.len() >= 128 {
            bail!("too many pending Codex thread selections");
        }
        self.pending.insert(
            id.to_string(),
            Selection {
                method: method.into(),
                thread: frame["params"]["threadId"].as_str().map(str::to_owned),
            },
        );
        Ok(())
    }

    fn incoming(&mut self, config: &RuntimeConfig, frame: Value) -> Result<()> {
        if frame.get("method").is_none() {
            if let Some(selection) =
                frame.get("id").and_then(|id| self.pending.remove(&id.to_string()))
            {
                if frame.get("error").is_none() {
                    let thread = &frame["result"]["thread"];
                    if thread["ephemeral"] != true && thread["threadSource"] != "system" {
                        let id = thread["id"]
                            .as_str()
                            .context("Codex thread response lacks identity")?;
                        if selection.method == "thread/resume"
                            && selection.thread.as_deref() != Some(id)
                        {
                            bail!("Codex resumed a different conversation than requested");
                        }
                        if self.snapshot.thread_id.is_none()
                            && config.resume.as_deref().is_some_and(|expected| expected != id)
                        {
                            bail!("native Codex did not resume the captured conversation");
                        }
                        self.snapshot.thread_id = Some(id.into());
                        self.snapshot.history_path = thread["path"].as_str().map(str::to_owned);
                        let mut ledger = Ledger::new(id.into(), config.generation);
                        self.needs_audit = selection.method != "thread/start";
                        let update = ledger.reconcile(config.generation, thread, !self.needs_audit);
                        self.apply(update);
                        self.ledger = Some(ledger);
                        let buffered = std::mem::take(&mut self.buffered);
                        self.buffered_bytes = 0;
                        for event in buffered {
                            self.observe(config, &event);
                        }
                    }
                }
            }
        }
        if self.ledger.is_some() {
            self.observe(config, &frame);
        } else if frame["params"].get("threadId").is_some() {
            self.buffered_bytes += serde_json::to_vec(&frame)?.len();
            if self.buffered.len() >= MAX_BUFFERED || self.buffered_bytes > MAX_MESSAGE {
                bail!("Codex observation exceeded the pre-thread event bound");
            }
            self.buffered.push(frame);
        }
        Ok(())
    }

    fn observe(&mut self, config: &RuntimeConfig, frame: &Value) {
        if frame.get("method").is_some() {
            // Conservatively invalidate an in-flight audit on every event,
            // including descendant changes which may lack the parent ID.
            self.revision = self.revision.wrapping_add(1);
        }
        if let Some(ledger) = &mut self.ledger {
            let update = ledger.observe(config.generation, frame);
            self.apply(update);
        }
    }

    fn apply(&mut self, update: Update) {
        self.snapshot.state = update.state;
        self.snapshot.observation_hold = update.observation_hold;
        self.snapshot.turn_id = update.turn_id;
        if let Some(plan) = &update.plan {
            let text = bound_text(plan.clone(), 8000);
            self.snapshot.plan_key = Some(format!("{:x}", Sha256::digest(text.as_bytes())));
            self.snapshot.plan = Some(text);
        }
        if let Some(text) = update.reply.or(update.plan) {
            self.preview.text = Some(bound_text(text, 40_000));
            self.preview.reply_key = update.reply_key.map(|id| {
                let hash = Sha256::digest(id.as_bytes());
                u64::from_be_bytes(hash[..8].try_into().expect("eight hash bytes"))
            });
        }
        if let Some(activity) = update.activity {
            self.preview.activity = Some(AgentActivity::Tool(bound_text(activity, 512)));
        } else if !matches!(self.snapshot.state, SessionState::Running) {
            self.preview.activity = None;
        } else if self.preview.activity.is_none() {
            self.preview.activity = Some(AgentActivity::Thinking);
        }
    }

    fn poll_audit(&mut self, config: &RuntimeConfig) {
        if let Some(receiver) = &self.audit {
            match receiver.try_recv() {
                Ok(result) => {
                    self.audit = None;
                    self.next_audit = Instant::now() + Duration::from_secs(2);
                    if let Ok(audit) = result {
                        if audit.revision == self.revision {
                            if let Some(ledger) = &mut self.ledger {
                                if ledger.can_reconcile_idle() {
                                    let update =
                                        ledger.reconcile(config.generation, &audit.thread, true);
                                    self.apply(update);
                                    self.needs_audit = false;
                                }
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.audit = None;
                    self.next_audit = Instant::now() + Duration::from_secs(2);
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.needs_audit || self.audit.is_some() || Instant::now() < self.next_audit {
            return;
        }
        let Some(thread) = self.snapshot.thread_id.clone() else { return };
        let socket = config.upstream_socket.clone();
        let revision = self.revision;
        let (sender, receiver) = mpsc::sync_channel(1);
        self.audit = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(audit_idle(&socket, &thread, revision));
        });
    }

    fn advance_evidence(&mut self) {
        let evidence = (
            self.snapshot.thread_id.clone(),
            self.snapshot.turn_id.clone(),
            self.snapshot.state.clone(),
            self.snapshot.observation_hold,
            self.snapshot.history_path.clone(),
            self.snapshot.stopped,
            self.snapshot.plan_key.clone(),
        );
        if self.published_state.as_ref() != Some(&evidence) {
            self.snapshot.sequence = self.snapshot.sequence.saturating_add(1);
            self.published_state = Some(evidence);
        }
    }

    fn publish(&mut self, config: &RuntimeConfig) -> Result<()> {
        self.advance_evidence();
        self.snapshot.heartbeat_ms =
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?;
        write_json(&config.snapshot_path, &self.snapshot)?;
        if self.published_preview.as_ref() != Some(&self.preview) {
            // JSON escaping may expand text; bound the serialized artifact too.
            while serde_json::to_vec(&self.preview)?.len() > 65_536 {
                if let Some(text) = &mut self.preview.text {
                    let limit = text.len() / 2;
                    *text = bound_text(std::mem::take(text), limit);
                } else {
                    bail!("Codex preview exceeds its bound");
                }
            }
            write_json(&config.preview_path, &self.preview)?;
            self.published_preview = Some(self.preview.clone());
        }
        Ok(())
    }
}

fn bound_text(mut text: String, bytes: usize) -> String {
    let mut boundary = bytes.min(text.len());
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text.truncate(boundary);
    text
}

/// No subscription is created: this connection cannot claim native approvals.
/// The relay remains active on the calling thread throughout these reads.
fn audit_idle(socket: &Path, thread_id: &str, revision: u64) -> Result<Audit> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut client = super::rpc::Client::connect(socket)?;
    let mut unexpected_request = false;
    let mut observe = |frame: Value| {
        if frame.get("method").is_some() && frame.get("id").is_some() {
            unexpected_request = true;
        }
    };
    let loaded = client.call("thread/loaded/list", json!({"limit": 1000}), &mut observe)?;
    if !complete_page(&loaded) {
        bail!("loaded Codex threads exceed audit bound");
    }
    let loaded_ids = loaded["data"].as_array().context("invalid loaded Codex thread list")?;
    if !loaded_ids.iter().any(|id| id.as_str() == Some(thread_id)) {
        bail!("Codex parent is not loaded");
    }
    let descendants = client.call("thread/list", json!({"ancestorThreadId": thread_id, "limit": 100,
        "sourceKinds": ["cli", "vscode", "exec", "appServer", "subAgent", "subAgentReview", "subAgentCompact", "subAgentThreadSpawn", "subAgentOther", "unknown"]}), &mut observe)?;
    if !complete_page(&descendants) {
        bail!("Codex descendants exceed audit bound");
    }
    // The persistent ancestor index may lag. Inspect *every* loaded thread
    // in this dedicated server; absence from the descendant list is no proof
    // that it cannot still mutate the checkout.
    for loaded_id in loaded_ids {
        if Instant::now() >= deadline {
            bail!("Codex reconciliation audit timed out");
        }
        let id = loaded_id.as_str().context("loaded Codex thread lacks identity")?;
        if id == thread_id {
            continue;
        }
        let current = client.call(
            "thread/read",
            json!({"threadId": id, "includeTurns": false}),
            &mut observe,
        )?;
        let thread = &current["thread"];
        if thread["id"] != id {
            bail!("Codex audit returned the wrong loaded thread");
        }
        // Native title generation was measured as ephemeral system work; it
        // does not own a ticket turn or a checkout seat.
        let title_thread = thread["ephemeral"] == true && thread["threadSource"] == "system";
        if !title_thread && thread["status"]["type"] != "idle" {
            bail!("a loaded Codex thread is not idle");
        }
    }
    if Instant::now() >= deadline {
        bail!("Codex reconciliation audit timed out");
    }
    let loaded_after = client.call("thread/loaded/list", json!({"limit": 1000}), &mut observe)?;
    if loaded_after != loaded {
        bail!("loaded Codex threads changed during reconciliation");
    }
    let parent = client.call(
        "thread/read",
        json!({"threadId": thread_id, "includeTurns": false}),
        &mut observe,
    )?;
    if unexpected_request {
        bail!("read-only Codex audit received a server request");
    }
    let thread = parent.get("thread").context("Codex audit lacks thread")?;
    if thread["id"] != thread_id || thread["status"]["type"] != "idle" {
        bail!("Codex parent is not idle");
    }
    Ok(Audit { revision, thread: thread.clone() })
}

fn complete_page(page: &Value) -> bool {
    page.get("nextCursor").is_some_and(Value::is_null) && page["data"].is_array()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> RuntimeConfig {
        RuntimeConfig {
            session: uuid::Uuid::nil(),
            generation: 1,
            cwd: PathBuf::new(),
            executable: String::new(),
            upstream_socket: PathBuf::new(),
            proxy_socket: PathBuf::new(),
            snapshot_path: PathBuf::new(),
            preview_path: PathBuf::new(),
            resume: None,
            config_flags: vec![],
            env: vec![],
        }
    }

    #[test]
    fn descendant_ancestry_ignores_group_changes_but_rejects_reused_parent_pids() {
        let process = |pid, parent, group, start| ProcessIdentity {
            pid,
            parent,
            group,
            started: (start, 0),
            zombie: false,
            stopped: false,
        };
        let root = process(10, 1, 10, 1);
        let detached = process(20, 10, 20, 2);
        let grandchild = process(30, 20, 30, 3);
        let unrelated = process(40, 1, 40, 4);
        let table = [root.clone(), detached, grandchild, unrelated]
            .into_iter()
            .map(|entry| (entry.pid, entry))
            .collect();
        let mut tree = ProcessTree::default();
        tree.known.insert(root.pid, root.clone());
        tree.extend(&table).unwrap();
        assert_eq!(tree.known.keys().copied().collect::<Vec<_>>(), vec![10, 20, 30]);
        let mut recycled = ProcessTree::default();
        recycled.known.insert(root.pid, process(10, 1, 10, 99));
        recycled.extend(&table).unwrap();
        assert_eq!(recycled.known.len(), 1, "a reused PID is not an ancestor we own");
    }

    #[test]
    fn supervised_cleanup_reaps_a_tool_in_its_own_process_group() {
        use std::io::{BufRead, BufReader, Write};
        struct Guard {
            child: Child,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                // EOF invokes the existing guard's exact-resource cleanup on
                // assertion failures too; its own 15-second deadline bounds it.
                self.child.stdin.take();
                let _ = self.child.wait();
            }
        }
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ci/test_guard.py");
        let mut guard = Guard {
            child: Command::new("python3")
                .arg("-B")
                .arg(script)
                .args(["--name", "codex-descendants", "--tmux"])
                .arg(mesimon_backend_tmux::tmux_bin())
                .args(["--timeout", "15"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        };
        let mut output = BufReader::new(guard.child.stdout.take().unwrap());
        let reply = |output: &mut BufReader<std::process::ChildStdout>| -> Value {
            let mut line = String::new();
            assert!(output.read_line(&mut line).unwrap() > 0, "fixture guard exited");
            let value: Value = serde_json::from_str(&line).unwrap();
            assert!(value.get("error").is_none(), "fixture guard refused: {value}");
            value["ok"].clone()
        };
        let root = PathBuf::from(reply(&mut output).as_str().unwrap());
        let ready = root.join("tool.pid");
        let code = "import pathlib, subprocess, sys, time\nchild=subprocess.Popen([sys.executable, '-c', 'import os,time;os.setsid();time.sleep(60)'])\npathlib.Path(sys.argv[1]).write_text(str(child.pid))\ntime.sleep(60)\n";
        let request = json!({"op":"spawn", "argv":["python3", "-c", code, ready], "env":{}});
        writeln!(guard.child.stdin.as_mut().unwrap(), "{request}").unwrap();
        guard.child.stdin.as_mut().unwrap().flush().unwrap();
        let server = reply(&mut output).as_u64().unwrap() as u32;
        let mut tree = ProcessTree::default();
        tree.root(server, true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let tool = loop {
            assert!(Instant::now() < deadline, "fixture tool did not detach");
            if let Ok(text) = std::fs::read_to_string(&ready) {
                if let Ok(pid) = text.parse::<i32>() {
                    if process_identity(pid).unwrap().is_some_and(|entry| entry.group == pid) {
                        break pid;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        tree.refresh(false).unwrap();
        assert!(tree.known.contains_key(&tool));
        tree.stop().unwrap();
        assert!(
            process_identity(tool).unwrap().is_none_or(|entry| entry.zombie),
            "detached tool survived runtime cleanup"
        );
        writeln!(guard.child.stdin.as_mut().unwrap(), "{}", json!({"op":"finish"})).unwrap();
        guard.child.stdin.as_mut().unwrap().flush().unwrap();
        guard.child.stdin.take();
        assert!(guard.child.wait().unwrap().success(), "supervised fixture cleanup failed");
    }

    #[test]
    fn utf8_preview_and_page_bounds_are_conservative() {
        assert_eq!(bound_text("a😀b".into(), 4), "a");
        assert_eq!(bound_text("a😀b".into(), 5), "a😀");
        assert!(complete_page(&json!({"data": [], "nextCursor": null})));
        assert!(!complete_page(&json!({"data": []})));
        assert!(!complete_page(&json!({"data": [], "nextCursor": "more"})));
    }

    #[test]
    fn relay_deadlines_clear_inherited_nonblocking_mode() {
        use std::os::fd::AsRawFd;
        let (stream, _peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        stream_deadlines(&stream, true).unwrap();
        let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(flags & libc::O_NONBLOCK, 0);
    }

    #[test]
    fn native_json_is_forwarded_without_reserialization_or_approval_answers() {
        use tungstenite::protocol::Role;
        let (native_end, relay_in) = UnixStream::pair().unwrap();
        let (relay_out, server_end) = UnixStream::pair().unwrap();
        for stream in [&native_end, &relay_in, &relay_out, &server_end] {
            stream_deadlines(stream, false).unwrap();
        }
        let mut native =
            WebSocket::from_raw_socket(native_end, Role::Client, Some(socket_config()));
        let mut incoming =
            WebSocket::from_raw_socket(relay_in, Role::Server, Some(socket_config()));
        let mut outgoing =
            WebSocket::from_raw_socket(relay_out, Role::Client, Some(socket_config()));
        let mut server =
            WebSocket::from_raw_socket(server_end, Role::Server, Some(socket_config()));
        let original =
            r#"{ "id" : "approval-7", "result": { "decision": "cancel" }, "unicode": "✓" }"#;
        native.send(Message::Text(original.into())).unwrap();
        outgoing.send(receive(&mut incoming).unwrap().unwrap()).unwrap();
        assert_eq!(receive(&mut server).unwrap(), Some(Message::Text(original.into())));
        assert!(receive(&mut server).unwrap().is_none());
    }

    #[test]
    fn ephemeral_native_thread_requests_do_not_select_foreground() {
        let config = config();
        let mut observer = Observation::new(&config);
        observer
            .outgoing(&json!({"id": 1, "method":"thread/start", "params":{"ephemeral":true}}))
            .unwrap();
        observer.outgoing(&json!({"id": 2, "method":"thread/start", "params":{}})).unwrap();
        assert_eq!(observer.pending.len(), 1);
        observer
            .incoming(&config, json!({"id":1,"result":{"thread":{"id":"system","ephemeral":true}}}))
            .unwrap();
        assert!(observer.snapshot.thread_id.is_none());
        observer.incoming(&config, json!({"id":2,"result":{"thread":{"id":"main","status":{"type":"idle"},"turns":[]}}})).unwrap();
        assert_eq!(observer.snapshot.thread_id.as_deref(), Some("main"));
        assert!(!observer.snapshot.observation_hold);
    }
    #[test]
    fn heartbeat_and_preview_do_not_repeat_terminal_evidence() {
        let mut observation = Observation::new(&config());
        observation.advance_evidence();
        assert_eq!(observation.snapshot.sequence, 1);
        observation.snapshot.heartbeat_ms = 123;
        observation.preview.text = Some("finished".into());
        observation.advance_evidence();
        assert_eq!(observation.snapshot.sequence, 1);
        observation.snapshot.state =
            SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn };
        observation.advance_evidence();
        assert_eq!(observation.snapshot.sequence, 2);
        observation.advance_evidence();
        assert_eq!(observation.snapshot.sequence, 2);
    }

    #[test]
    fn exact_resume_rejects_different_thread_and_never_clears_hold_from_idle_alone() {
        let mut config = config();
        config.resume = Some("expected".into());
        let mut observation = Observation::new(&config);
        observation
            .outgoing(&json!({"id":1,"method":"thread/resume","params":{"threadId":"expected"}}))
            .unwrap();
        assert!(observation.incoming(&config,json!({"id":1,"result":{"thread":{"id":"wrong","status":{"type":"idle"},"turns":[]}}})).is_err());
        assert!(observation.snapshot.thread_id.is_none());
        observation
            .outgoing(&json!({"id":2,"method":"thread/resume","params":{"threadId":"expected"}}))
            .unwrap();
        observation.incoming(&config,json!({"id":2,"result":{"thread":{"id":"expected","status":{"type":"idle"},"turns":[]}}})).unwrap();
        assert_eq!(observation.snapshot.thread_id.as_deref(), Some("expected"));
        assert!(observation.snapshot.observation_hold);
        assert!(observation.needs_audit);
    }
}
