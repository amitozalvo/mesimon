//! A tmux-owned supervisor for the native Codex UI and its dedicated server.
//!
//! JSON messages cross the relay unchanged. Observation never acknowledges an
//! approval, starts a turn, or modifies the user's conversation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
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
use super::{write_json, LaunchPhase, RuntimeConfig, Snapshot};
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

    fn relay_ended(&mut self, result: &Result<()>) {
        if result.is_err() && self.server.is_some() {
            // The launched CLI can be a live Node wrapper around the native
            // server. A broken relay may mean that an inner ancestor already
            // exited and reparented children before the next process sample.
            // Cleaning every remembered PID cannot close that observation gap.
            self.uncertain = true;
        }
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
    owned.tree.relay_ended(&result);
    let cleanup = owned.cleanup();
    observation.snapshot.stopped = cleanup.is_ok() && !owned.tree.uncertain;
    if let Err(error) = &cleanup {
        eprintln!("Codex runtime cleanup failed: {error}");
    }
    result.and(cleanup)
}

/// Native success/Close can be a reaction to an inner server crash while its
/// CLI wrapper is still alive. Require a live private listener before accepting
/// that as an intentional quit. This nonblocking connect sends no RPC or data.
fn verify_upstream_listener(path: &Path) -> Result<()> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        bail!("Codex upstream endpoint cannot be verified on native quit");
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = std::mem::size_of_val(&address) as u8;
    }
    for (target, source) in address.sun_path.iter_mut().zip(bytes) {
        *target = *source as libc::c_char;
    }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("create Codex quit liveness probe");
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error()).context("bound Codex quit liveness probe");
    }
    if unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error())
            .context("Codex app-server listener is unavailable or unverified on native quit");
    }
    Ok(())
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
        // Server loss can also make the native client exit successfully.
        // Preserve the upstream failure instead of misclassifying that as quit.
        if let Some(child) = &mut owned.server {
            if let Some(status) = child.try_wait()? {
                bail!("Codex app-server exited ({status})");
            }
        }
        if let Some(child) = &mut owned.native {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    bail!("native Codex exited ({status})");
                }
                verify_upstream_listener(&config.upstream_socket)?;
                return Ok(());
            }
        }
        // Small fair batches keep UI traffic and heartbeats moving even during
        // streamed replies. No additional initialize/subscription is injected.
        let mut native_drained = false;
        for _ in 0..32 {
            let Some(message) = receive(&mut native).context("read native Codex")? else {
                native_drained = true;
                break;
            };
            if let Message::Text(text) = &message {
                observation.before_native_forward(config, &serde_json::from_str::<Value>(text)?)?;
            }
            let closed = matches!(message, Message::Close(_));
            upstream.send(message).context("forward native Codex message")?;
            if closed {
                if let Some(child) = &mut owned.server {
                    if let Some(status) = child.try_wait()? {
                        bail!("Codex app-server exited while native client closed ({status})");
                    }
                }
                verify_upstream_listener(&config.upstream_socket)?;
                return Ok(());
            }
        }
        let mut upstream_drained = false;
        for _ in 0..32 {
            let Some(message) = receive(&mut upstream).context("read Codex app-server")? else {
                upstream_drained = true;
                break;
            };
            if let Message::Text(text) = &message {
                observation.before_server_forward(config, serde_json::from_str::<Value>(text)?)?;
            }
            let closed = matches!(message, Message::Close(_));
            native.send(message).context("forward Codex app-server message")?;
            if closed {
                bail!("Codex app-server disconnected");
            }
        }
        if native_drained && upstream_drained {
            // A fair-batch boundary is not proof that the observation queue
            // is empty. Do not apply an audit ahead of already queued work.
            observation.poll_audit(config);
        }
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
    threads: BTreeMap<String, Value>,
}
type StateEvidence = (
    Option<String>,
    Option<String>,
    SessionState,
    bool,
    Option<String>,
    bool,
    Option<String>,
    Option<String>,
    LaunchPhase,
);

struct Observation {
    published_state: Option<StateEvidence>,
    published_completions: BTreeSet<(String, String)>,
    revoked_completions: BTreeSet<(String, String)>,
    snapshot: Snapshot,
    ledger: Option<Ledger>,
    retired: BTreeMap<String, Ledger>,
    owned_threads: BTreeSet<String>,
    pending: BTreeMap<String, Selection>,
    native_requests: BTreeMap<String, bool>,
    system_threads: BTreeSet<String>,
    mutation_unobserved: bool,
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
            published_completions: BTreeSet::new(),
            revoked_completions: BTreeSet::new(),
            snapshot: Snapshot {
                session: config.session,
                generation: config.generation,
                sequence: 0,
                heartbeat_ms: 0,
                thread_id: None,
                launch_phase: if config.resume.is_none() {
                    LaunchPhase::BeforeSelection
                } else {
                    LaunchPhase::SelectionPending
                },
                turn_id: None,
                state: SessionState::Spawning,
                observation_hold: true,
                history_path: None,
                title: None,
                plan: None,
                plan_key: None,
                stopped: false,
            },
            ledger: None,
            retired: BTreeMap::new(),
            owned_threads: BTreeSet::new(),
            pending: BTreeMap::new(),
            native_requests: BTreeMap::new(),
            system_threads: BTreeSet::new(),
            mutation_unobserved: false,
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

    fn before_native_forward(&mut self, config: &RuntimeConfig, frame: &Value) -> Result<()> {
        let before = self.snapshot.launch_phase;
        self.outgoing(frame)?;
        if before != self.snapshot.launch_phase {
            self.publish(config)?;
        }
        Ok(())
    }

    fn before_server_forward(&mut self, config: &RuntimeConfig, frame: Value) -> Result<()> {
        let before = (self.snapshot.launch_phase, self.snapshot.thread_id.clone());
        self.incoming(config, frame)?;
        if before != (self.snapshot.launch_phase, self.snapshot.thread_id.clone()) {
            self.publish(config)?;
        }
        Ok(())
    }

    fn outgoing(&mut self, frame: &Value) -> Result<()> {
        // Native input is part of continuity too. An audit which raced a
        // forwarded turn/approval must not release the checkout before its
        // server-side effects become observable.
        self.revision = self.revision.wrapping_add(1);
        let Some(method) = frame["method"].as_str() else { return Ok(()) };
        if let Some(id) = frame.get("id") {
            if self.native_requests.len() >= 1024 {
                bail!("too many pending native Codex requests");
            }
            let system = frame["params"]["ephemeral"] == true
                || frame["params"]["threadId"]
                    .as_str()
                    .is_some_and(|id| self.system_threads.contains(id));
            let mutation = !system
                && (method.starts_with("turn/")
                    || matches!(
                        method,
                        "thread/start"
                            | "thread/resume"
                            | "thread/fork"
                            | "thread/compact/start"
                            | "review/start"
                    ));
            self.native_requests.insert(id.to_string(), mutation);
            self.mutation_unobserved |= mutation;
        }
        if !matches!(method, "thread/start" | "thread/resume" | "thread/fork")
            || frame["params"]["ephemeral"] == true
        {
            return Ok(());
        }
        // This phase is published by the relay before forwarding this native
        // selection. A later missing response cannot be mistaken for proof
        // that no conversation was ever created.
        self.snapshot.launch_phase = LaunchPhase::SelectionPending;
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
            if let Some(id) = frame.get("id") {
                self.native_requests.remove(&id.to_string());
            }
            let returned_thread = &frame["result"]["thread"];
            if returned_thread["ephemeral"] == true && returned_thread["threadSource"] == "system" {
                if let Some(id) = returned_thread["id"].as_str() {
                    if self.system_threads.len() >= 1024 {
                        bail!("too many native Codex system threads");
                    }
                    self.system_threads.insert(id.into());
                }
            }
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
                        if self.snapshot.thread_id.as_deref() != Some(id) {
                            if let (Some(previous), Some(ledger)) =
                                (self.snapshot.thread_id.clone(), self.ledger.take())
                            {
                                if self.retired.len() >= 128 {
                                    bail!("too many owned Codex foreground conversations");
                                }
                                self.retired.insert(previous, ledger);
                            }
                            self.preview = AgentPreview::default();
                            self.snapshot.plan = None;
                            self.snapshot.plan_key = None;
                        }
                        self.snapshot.thread_id = Some(id.into());
                        self.snapshot.launch_phase = LaunchPhase::Selected;
                        self.owned_threads.insert(id.into());
                        self.snapshot.history_path = thread["path"].as_str().map(str::to_owned);
                        self.snapshot.title =
                            thread["name"].as_str().map(|name| bound_text(name.into(), 512));
                        let existing = self.ledger.take().or_else(|| self.retired.remove(id));
                        let retained = existing.is_some();
                        let mut ledger =
                            existing.unwrap_or_else(|| Ledger::new(id.into(), config.generation));
                        self.needs_audit = retained
                            || selection.method != "thread/start"
                            || !self.retired.is_empty();
                        self.mutation_unobserved = false;
                        let update = if retained {
                            // Re-selecting a conversation cannot erase locally
                            // observed requests or unfinished descendant work.
                            ledger.observe(config.generation, &json!({}))
                        } else {
                            ledger.reconcile(config.generation, thread, !self.needs_audit)
                        };
                        self.ledger = Some(ledger);
                        self.apply(update);
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
        if let Some(thread) = frame["params"]["threadId"].as_str() {
            if self.owned_threads.contains(thread)
                && self.snapshot.thread_id.as_deref() != Some(thread)
                && !self.retired.contains_key(thread)
                && frame["method"].as_str().is_some_and(|method| {
                    method.starts_with("turn/")
                        || method.starts_with("item/")
                        || method.starts_with("hook/")
                        || method == "serverRequest/resolved"
                        || method == "thread/status/changed"
                })
            {
                // An audited idle conversation can become active again after
                // native foreground selection changes. Keep its identity,
                // even after discarding its quiescent historical ledger.
                self.retired.insert(thread.into(), Ledger::new(thread.into(), config.generation));
                self.needs_audit = true;
            }
        }
        if frame["method"] == "thread/name/updated"
            && frame["params"]["threadId"].as_str() == self.snapshot.thread_id.as_deref()
        {
            if let Some(name) = frame["params"]["threadName"].as_str() {
                self.snapshot.title = Some(bound_text(name.into(), 512));
            }
        }
        if frame.get("method").is_some() {
            // Conservatively invalidate an in-flight audit on every event,
            // including descendant changes which may lack the parent ID.
            self.revision = self.revision.wrapping_add(1);
        }
        if frame["method"] == "turn/started"
            && frame["params"]["threadId"].as_str() == self.snapshot.thread_id.as_deref()
        {
            self.mutation_unobserved = false;
        }
        for ledger in self.retired.values_mut() {
            ledger.observe(config.generation, frame);
        }
        if let Some(ledger) = &mut self.ledger {
            let update = ledger.observe(config.generation, frame);
            self.owned_threads.extend(ledger.known_children());
            self.apply(update);
        }
        for ledger in self.retired.values() {
            self.owned_threads.extend(ledger.known_children());
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
        if self.snapshot.state
            == (SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn })
        {
            if let (Some(thread), Some(turn)) = (&self.snapshot.thread_id, &self.snapshot.turn_id) {
                let identity = (thread.clone(), turn.clone());
                if self.transport_held() && self.published_completions.contains(&identity) {
                    self.revoked_completions.insert(identity.clone());
                }
                if self.revoked_completions.contains(&identity) {
                    self.snapshot.state = SessionState::Idle {
                        stop_reason: mesimon_core::board::StopReason::Unknown,
                    };
                } else if self.transport_held() {
                    if let Some(ledger) = &mut self.ledger {
                        ledger.defer_terminal_projection();
                    }
                }
            }
        }
        self.apply_transport_hold();
    }

    fn transport_held(&self) -> bool {
        self.needs_audit
            || self.mutation_unobserved
            || !self.retired.is_empty()
            || self.native_requests.values().any(|mutation| *mutation)
    }

    fn apply_transport_hold(&mut self) {
        if self.transport_held() {
            self.snapshot.observation_hold = true;
            if matches!(
                self.snapshot.state,
                SessionState::Idle { .. }
                    | SessionState::RequiresAction { reason: mesimon_core::board::Reason::Plan }
            ) {
                self.snapshot.state = SessionState::Running;
            }
        }
    }

    fn poll_audit(&mut self, config: &RuntimeConfig) {
        if let Some(receiver) = &self.audit {
            match receiver.try_recv() {
                Ok(result) => {
                    self.audit = None;
                    self.next_audit = Instant::now() + Duration::from_secs(2);
                    if let Ok(audit) = result {
                        self.apply_audit(config, audit);
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.audit = None;
                    self.next_audit = Instant::now() + Duration::from_secs(2);
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(ledger) = &self.ledger else { return };
        let required = self.needs_audit
            || self.mutation_unobserved
            || !self.retired.is_empty()
            || ledger.needs_work_audit();
        if !required
            || self.audit.is_some()
            || Instant::now() < self.next_audit
            || !self.native_requests.is_empty()
            || !ledger.can_reconcile_work()
            || !self.retired.values().all(Ledger::can_reconcile_work)
        {
            return;
        }
        let Some(thread) = self.snapshot.thread_id.clone() else { return };
        let mut known: BTreeSet<String> = ledger.known_children().into_iter().collect();
        for (id, retired) in &self.retired {
            known.insert(id.clone());
            known.extend(retired.known_children());
        }
        let socket = config.upstream_socket.clone();
        let revision = self.revision;
        let (sender, receiver) = mpsc::sync_channel(1);
        self.audit = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(audit_idle(&socket, &thread, &known, revision));
        });
    }

    fn apply_audit(&mut self, config: &RuntimeConfig, audit: Audit) -> bool {
        if audit.revision != self.revision
            || !self.native_requests.is_empty()
            || !self.retired.values().all(Ledger::can_reconcile_work)
        {
            return false;
        }
        let Some(ledger) = &mut self.ledger else { return false };
        if !ledger.can_reconcile_work() {
            return false;
        }
        let Some(thread) = self.snapshot.thread_id.as_ref().and_then(|id| audit.threads.get(id))
        else {
            return false;
        };
        let covered = ledger.known_children().into_iter().chain(
            self.retired
                .iter()
                .flat_map(|(id, old)| std::iter::once(id.clone()).chain(old.known_children())),
        );
        if covered.into_iter().any(|id| !audit.threads.contains_key(&id)) {
            return false;
        }
        let update = ledger.reconcile_work(config.generation, thread);
        self.retired.clear();
        self.mutation_unobserved = false;
        self.needs_audit = false;
        self.apply(update);
        true
    }

    fn advance_evidence(&mut self) {
        if !self.snapshot.observation_hold
            && self.snapshot.state
                == (SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn })
        {
            if let (Some(thread), Some(turn)) = (&self.snapshot.thread_id, &self.snapshot.turn_id) {
                self.published_completions.insert((thread.clone(), turn.clone()));
            }
        }
        let evidence = (
            self.snapshot.thread_id.clone(),
            self.snapshot.turn_id.clone(),
            self.snapshot.state.clone(),
            self.snapshot.observation_hold,
            self.snapshot.history_path.clone(),
            self.snapshot.stopped,
            self.snapshot.plan_key.clone(),
            self.snapshot.title.clone(),
            self.snapshot.launch_phase,
        );
        if self.published_state.as_ref() != Some(&evidence) {
            self.snapshot.sequence = self.snapshot.sequence.saturating_add(1);
            self.published_state = Some(evidence);
        }
    }

    fn publish(&mut self, config: &RuntimeConfig) -> Result<()> {
        if self.owned_threads.len() > 1024
            || self.retired.len() > 1024
            || self.published_completions.len() > 1024
        {
            bail!("owned Codex conversation tracking exceeded its bound");
        }
        self.apply_transport_hold();
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
fn audit_idle(
    socket: &Path,
    thread_id: &str,
    known: &BTreeSet<String>,
    revision: u64,
) -> Result<Audit> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut client = super::rpc::Client::connect(socket)?;
    let mut unexpected_request = false;
    let mut activity_changed = false;
    let mut observe = |frame: Value| {
        if frame.get("method").is_some() && frame.get("id").is_some() {
            unexpected_request = true;
        }
        if matches!(
            frame["method"].as_str(),
            Some("thread/status/changed" | "thread/started" | "thread/closed")
        ) {
            activity_changed = true;
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
    let descendants = client.call("thread/list", json!({"ancestorThreadId": thread_id, "limit": 1000,
        "sourceKinds": ["cli", "vscode", "exec", "appServer", "subAgent", "subAgentReview", "subAgentCompact", "subAgentThreadSpawn", "subAgentOther", "unknown"]}), &mut observe)?;
    if !complete_page(&descendants) {
        bail!("Codex descendants exceed audit bound");
    }
    // The persistent ancestor index may lag. Inspect *every* loaded thread
    // in this dedicated server; absence from the descendant list is no proof
    // that it cannot still mutate the checkout.
    let mut ids = known.clone();
    for id in loaded_ids {
        ids.insert(id.as_str().context("loaded Codex thread lacks identity")?.into());
    }
    for child in descendants["data"].as_array().context("invalid Codex descendants")? {
        ids.insert(child["id"].as_str().context("Codex descendant lacks identity")?.into());
    }
    if ids.len() > 1024 {
        bail!("owned Codex threads exceed audit bound");
    }
    let mut threads = BTreeMap::new();
    for id in ids {
        if Instant::now() >= deadline {
            bail!("Codex reconciliation audit timed out");
        }
        let loaded_here = loaded_ids.iter().any(|entry| entry.as_str() == Some(id.as_str()));
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
        let closed_here = !loaded_here && thread["status"]["type"] == "notLoaded";
        if !title_thread && thread["status"]["type"] != "idle" && !closed_here {
            bail!("a loaded Codex thread is not idle");
        }
        threads.insert(id, thread.clone());
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
    // Status notifications are global on the initialized read-only client.
    // Drain ones queued around the last read; an active child changing while
    // the metadata walk runs invalidates the walk even if its native UI was
    // not subscribed to that child's item stream.
    let mut drained = false;
    for _ in 0..64 {
        match client.receive(Duration::from_millis(1))? {
            Some(frame) => observe(frame),
            None => {
                drained = true;
                break;
            }
        }
    }
    if unexpected_request {
        bail!("read-only Codex audit received a server request");
    }
    if activity_changed || !drained {
        bail!("Codex activity changed during reconciliation");
    }
    let thread = parent.get("thread").context("Codex audit lacks thread")?;
    if thread["id"] != thread_id || thread["status"]["type"] != "idle" {
        bail!("Codex parent is not idle");
    }
    threads.insert(thread_id.into(), thread.clone());
    Ok(Audit { revision, threads })
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

    fn selected() -> Observation {
        let mut observer = Observation::new(&config());
        observer.outgoing(&json!({"id":1,"method":"thread/start","params":{}})).unwrap();
        observer
            .incoming(
                &config(),
                json!({"id":1,"result":{"thread":{
            "id":"main","status":{"type":"idle"},"turns":[]}}}),
            )
            .unwrap();
        observer
    }

    fn event(observer: &mut Observation, thread: &str, method: &str, mut params: Value) {
        params["threadId"] = json!(thread);
        observer.incoming(&config(), json!({"method":method,"params":params})).unwrap();
    }

    fn idle_audit(observer: &Observation, ids: &[&str]) -> Audit {
        Audit {
            revision: observer.revision,
            threads: ids
                .iter()
                .map(|id| ((*id).into(), json!({"id":id,"status":{"type":"idle"},"turns":[]})))
                .collect(),
        }
    }

    #[test]
    fn pending_native_mutation_and_later_input_reject_an_idle_audit() {
        let mut observer = selected();
        let stale = idle_audit(&observer, &["main"]);
        observer
            .outgoing(&json!({"id":2,"method":"turn/start","params":{"threadId":"main"}}))
            .unwrap();
        observer.apply_transport_hold();
        assert!(observer.snapshot.observation_hold);
        assert_eq!(observer.snapshot.state, SessionState::Running);
        assert!(!observer.apply_audit(&config(), stale));
        assert!(!observer.apply_audit(&config(), idle_audit(&observer, &["main"])));
        observer
            .incoming(&config(), json!({"id":2,"error":{"message":"fixture refusal"}}))
            .unwrap();
        assert!(observer.snapshot.observation_hold, "an RPC response alone is not a work audit");
        let audit = idle_audit(&observer, &["main"]);
        observer.outgoing(&json!({"id":"approval","result":{"decision":"cancel"}})).unwrap();
        assert!(
            !observer.apply_audit(&config(), audit),
            "approval input also invalidates an audit"
        );
        assert!(observer.apply_audit(&config(), idle_audit(&observer, &["main"])));
        assert!(!observer.snapshot.observation_hold);
        assert_eq!(
            observer.snapshot.state,
            SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Unknown }
        );
    }

    #[test]
    fn foreground_switch_retains_old_tools_and_the_unpublished_new_completion() {
        let mut observer = selected();
        event(&mut observer, "main", "turn/started", json!({"turn":{"id":"old-turn"}}));
        event(
            &mut observer,
            "main",
            "item/started",
            json!({"turnId":"old-turn",
            "item":{"id":"old-tool","type":"commandExecution","status":"inProgress"}}),
        );
        observer.outgoing(&json!({"id":2,"method":"thread/start","params":{}})).unwrap();
        observer
            .incoming(
                &config(),
                json!({"id":2,"result":{"thread":{
            "id":"new","status":{"type":"idle"},"turns":[]}}}),
            )
            .unwrap();
        assert!(observer.retired.contains_key("main"));
        assert!(observer.snapshot.observation_hold);
        assert!(!observer.apply_audit(&config(), idle_audit(&observer, &["main", "new"])));
        event(&mut observer, "new", "turn/started", json!({"turn":{"id":"new-turn"}}));
        event(
            &mut observer,
            "new",
            "turn/completed",
            json!({"turn":{"id":"new-turn","status":"completed","items":[]}}),
        );
        assert!(observer.snapshot.observation_hold);
        event(
            &mut observer,
            "main",
            "item/completed",
            json!({"turnId":"old-turn",
            "item":{"id":"old-tool","type":"commandExecution","status":"completed"}}),
        );
        event(
            &mut observer,
            "main",
            "turn/completed",
            json!({"turn":{"id":"old-turn","status":"completed","items":[]}}),
        );
        assert!(
            !observer.apply_audit(&config(), idle_audit(&observer, &["new"])),
            "audit must cover old foreground"
        );
        assert!(observer.apply_audit(&config(), idle_audit(&observer, &["main", "new"])));
        assert!(!observer.snapshot.observation_hold);
        assert_eq!(
            observer.snapshot.state,
            SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn }
        );
        assert!(
            observer.owned_threads.contains("main"),
            "retain identity after discarding idle history"
        );
    }

    #[test]
    fn reselecting_the_same_thread_does_not_erase_a_native_request() {
        let mut observer = selected();
        observer
            .incoming(
                &config(),
                json!({"id":"approval","method":"item/commandExecution/requestApproval",
            "params":{"threadId":"main","turnId":"turn"}}),
            )
            .unwrap();
        observer
            .outgoing(&json!({"id":2,"method":"thread/resume","params":{"threadId":"main"}}))
            .unwrap();
        observer
            .incoming(
                &config(),
                json!({"id":2,"result":{"thread":{
            "id":"main","status":{"type":"idle"},"turns":[]}}}),
            )
            .unwrap();
        assert!(!observer.apply_audit(&config(), idle_audit(&observer, &["main"])));
        event(&mut observer, "main", "serverRequest/resolved", json!({"requestId":"approval"}));
        assert!(observer.apply_audit(&config(), idle_audit(&observer, &["main"])));
        assert!(!observer.snapshot.observation_hold);
    }

    #[test]
    fn transport_work_cannot_be_hidden_by_a_dismissible_plan_state() {
        let mut observer = selected();
        event(&mut observer, "main", "turn/started", json!({"turn":{"id":"turn"}}));
        event(
            &mut observer,
            "main",
            "item/completed",
            json!({"turnId":"turn",
            "item":{"id":"plan","type":"plan","text":"fixture plan"}}),
        );
        event(
            &mut observer,
            "main",
            "turn/completed",
            json!({"turn":{"id":"turn","status":"completed","items":[]}}),
        );
        assert!(matches!(
            observer.snapshot.state,
            SessionState::RequiresAction { reason: mesimon_core::board::Reason::Plan }
        ));
        observer
            .outgoing(&json!({"id":2,"method":"thread/compact/start","params":{"threadId":"main"}}))
            .unwrap();
        observer.apply_transport_hold();
        assert_eq!(observer.snapshot.state, SessionState::Running);
        assert!(observer.snapshot.observation_hold);
    }

    #[test]
    fn transport_audit_releases_hidden_completion_once_but_not_published_completion() {
        for published in [false, true] {
            let mut observer = selected();
            event(&mut observer, "main", "turn/started", json!({"turn":{"id":"turn"}}));
            if !published {
                observer.mutation_unobserved = true;
            }
            event(
                &mut observer,
                "main",
                "turn/completed",
                json!({"turn":{"id":"turn","status":"completed","items":[]}}),
            );
            if published {
                observer.advance_evidence();
                observer.owned_threads.insert("old".into());
                event(&mut observer, "old", "turn/started", json!({"turn":{"id":"old-turn"}}));
                event(
                    &mut observer,
                    "old",
                    "turn/completed",
                    json!({"turn":{"id":"old-turn","status":"completed","items":[]}}),
                );
            }
            assert!(observer.snapshot.observation_hold);
            assert!(observer.apply_audit(&config(), idle_audit(&observer, &["main", "old"])));
            assert_eq!(
                observer.snapshot.state,
                SessionState::Idle {
                    stop_reason: if published {
                        mesimon_core::board::StopReason::Unknown
                    } else {
                        mesimon_core::board::StopReason::EndTurn
                    }
                }
            );
            if published {
                event(&mut observer, "main", "thread/name/updated", json!({"threadName":"title"}));
                assert_eq!(
                    observer.snapshot.state,
                    SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Unknown }
                );
            }
        }
    }

    #[test]
    fn interrupted_native_compaction_releases_after_the_interrupt_audit() {
        use mesimon_core::board::StopReason;
        let mut observer = selected();
        observer
            .outgoing(&json!({"id":2,"method":"thread/compact/start",
            "params":{"threadId":"main"}}))
            .unwrap();
        observer.incoming(&config(), json!({"id":2,"result":{}})).unwrap();
        event(&mut observer, "main", "turn/started", json!({"turn":{"id":"compact"}}));
        event(
            &mut observer,
            "main",
            "item/started",
            json!({"turnId":"compact",
            "item":{"id":"compact-item","type":"contextCompaction"}}),
        );
        observer
            .outgoing(&json!({"id":3,"method":"turn/interrupt",
            "params":{"threadId":"main","turnId":"compact"}}))
            .unwrap();
        observer.incoming(&config(), json!({"id":3,"result":{}})).unwrap();
        // Cancellation does not send item/completed for contextCompaction.
        event(
            &mut observer,
            "main",
            "turn/completed",
            json!({"turn":{"id":"compact","status":"interrupted","items":[]}}),
        );
        event(&mut observer, "main", "thread/status/changed", json!({"status":{"type":"idle"}}));
        assert!(observer.snapshot.observation_hold, "interrupt still requires the transport audit");
        assert!(observer.apply_audit(&config(), idle_audit(&observer, &["main"])));
        observer.advance_evidence();
        assert_eq!(
            observer.snapshot.state,
            SessionState::Idle { stop_reason: StopReason::Interrupted }
        );
        assert!(!observer.snapshot.observation_hold);
        assert!(
            observer.published_completions.is_empty(),
            "interruption cannot complete the ticket"
        );
    }

    #[test]
    fn native_compact_maintenance_never_publishes_a_ticket_completion() {
        use mesimon_core::board::StopReason;
        let mut observer = selected();
        event(&mut observer, "main", "turn/started", json!({"turn":{"id":"task"}}));
        event(
            &mut observer,
            "main",
            "turn/completed",
            json!({"turn":{"id":"task","status":"completed","items":[]}}),
        );
        observer.advance_evidence();
        assert!(observer.published_completions.contains(&("main".into(), "task".into())));
        observer
            .outgoing(&json!({"id":2,"method":"thread/compact/start","params":{"threadId":"main"}}))
            .unwrap();
        observer.incoming(&config(), json!({"id":2,"result":{}})).unwrap();
        event(&mut observer, "main", "turn/started", json!({"turn":{"id":"compact"}}));
        event(
            &mut observer,
            "main",
            "item/completed",
            json!({"turnId":"compact","item":{"id":"compact-item","type":"contextCompaction"}}),
        );
        event(
            &mut observer,
            "main",
            "turn/completed",
            json!({"turn":{"id":"compact","status":"completed","items":[]}}),
        );
        observer.advance_evidence();
        assert_eq!(
            observer.snapshot.state,
            SessionState::Idle { stop_reason: StopReason::Unknown }
        );
        assert!(!observer.snapshot.observation_hold);
        assert!(!observer.published_completions.contains(&("main".into(), "compact".into())));
        assert!(observer.apply_audit(&config(), idle_audit(&observer, &["main"])));
        observer.advance_evidence();
        assert_eq!(
            observer.snapshot.state,
            SessionState::Idle { stop_reason: StopReason::Unknown }
        );
        assert!(!observer.published_completions.contains(&("main".into(), "compact".into())));
        event(&mut observer, "main", "turn/started", json!({"turn":{"id":"next-task"}}));
        event(
            &mut observer,
            "main",
            "item/completed",
            json!({"turnId":"next-task","item":{"id":"user","type":"userMessage"}}),
        );
        event(
            &mut observer,
            "main",
            "item/completed",
            json!({"turnId":"next-task","item":{"id":"auto-compact","type":"contextCompaction"}}),
        );
        event(
            &mut observer,
            "main",
            "turn/completed",
            json!({"turn":{"id":"next-task","status":"completed","items":[]}}),
        );
        observer.advance_evidence();
        assert_eq!(
            observer.snapshot.state,
            SessionState::Idle { stop_reason: StopReason::EndTurn }
        );
        assert!(observer.published_completions.contains(&("main".into(), "next-task".into())));
    }

    fn mock_audit(
        child_status: &str,
        unrelated_active: bool,
        inject: Option<&str>,
    ) -> (Result<Audit>, Vec<Value>) {
        struct SocketDir(PathBuf);
        impl Drop for SocketDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join("rpc.sock"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let directory = SocketDir(PathBuf::from(format!(
            "/tmp/msmn-cdx-audit-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        )));
        std::fs::create_dir(&directory.0).unwrap();
        let path = directory.0.join("rpc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let status = child_status.to_string();
        let inject = inject.map(str::to_owned);
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let stream = loop {
                assert!(Instant::now() < deadline, "audit client did not connect");
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("audit listener: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let mut requests = Vec::new();
            let mut main_reads = 0;
            while Instant::now() < deadline {
                let text = match socket.read() {
                    Ok(Message::Text(text)) => text,
                    Ok(Message::Close(_)) => break,
                    Err(tungstenite::Error::Io(error))
                        if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) =>
                    {
                        continue
                    }
                    Err(_) => break,
                    other => panic!("unexpected audit frame: {other:?}"),
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                requests.push(request.clone());
                let method =
                    request["method"].as_str().expect("audit must not answer server requests");
                let result = match method {
                    "initialized" => continue,
                    "initialize" => json!({}),
                    "thread/loaded/list" => {
                        let mut ids = vec!["main"];
                        if status != "notLoaded" {
                            ids.push("child");
                        }
                        if unrelated_active {
                            ids.push("unrelated");
                        }
                        json!({"data":ids,"nextCursor":null})
                    }
                    "thread/list" => {
                        json!({"data":[{"id":"child","parentThreadId":"main"}],"nextCursor":null})
                    }
                    "thread/read" => {
                        assert_eq!(request["params"]["includeTurns"], false);
                        let id = request["params"]["threadId"].as_str().unwrap();
                        if id == "main" {
                            main_reads += 1;
                        }
                        if id == "main" && main_reads == 2 {
                            let notification = match inject.as_deref() {
                                Some("activity") => Some(
                                    json!({"method":"thread/status/changed","params":{"threadId":"child","status":{"type":"active","activeFlags":[]}}}),
                                ),
                                Some("request") => Some(
                                    json!({"id":"approval","method":"item/commandExecution/requestApproval","params":{"threadId":"child"}}),
                                ),
                                _ => None,
                            };
                            if let Some(frame) = notification {
                                socket.send(Message::Text(frame.to_string().into())).unwrap();
                            }
                        }
                        let state = if id == "child" {
                            status.as_str()
                        } else if id == "unrelated" {
                            "active"
                        } else {
                            "idle"
                        };
                        json!({"thread":{"id":id,"status":{"type":state,"activeFlags":[]},"turns":[]}})
                    }
                    other => panic!("audit attempted mutating/subscribing RPC: {other}"),
                };
                if socket
                    .send(Message::Text(
                        json!({"id":request["id"],"result":result}).to_string().into(),
                    ))
                    .is_err()
                {
                    break;
                }
            }
            requests
        });
        let result = audit_idle(&path, "main", &BTreeSet::from(["child".into()]), 7);
        (result, server.join().unwrap())
    }

    #[test]
    fn read_only_audit_reads_closed_known_children_and_rejects_other_owned_active_threads() {
        let (closed, requests) = mock_audit("notLoaded", false, None);
        assert_eq!(closed.unwrap().threads["child"]["status"]["type"], "notLoaded");
        assert!(requests
            .iter()
            .any(|r| r["method"] == "thread/read" && r["params"]["threadId"] == "child"));
        assert!(mock_audit("active", false, None).0.is_err());
        assert!(mock_audit("idle", true, None).0.is_err());
    }

    #[test]
    fn read_only_audit_rejects_late_child_activity_and_never_answers_native_approvals() {
        for injected in ["activity", "request"] {
            let (result, requests) = mock_audit("idle", false, Some(injected));
            assert!(result.is_err());
            assert!(requests.iter().all(|r| r.get("method").is_some()));
            assert!(!requests
                .iter()
                .any(|r| matches!(r["method"].as_str(), Some("thread/resume" | "turn/start"))));
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
    fn launch_evidence_is_saved_before_native_selection_is_forwarded() {
        struct Directory(PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join("snapshot.json"));
                let _ = std::fs::remove_file(self.0.join("preview.json"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let directory = Directory(PathBuf::from(format!(
            "/tmp/msmn-cdx-launch-{}",
            uuid::Uuid::new_v4().simple()
        )));
        let mut config = config();
        config.snapshot_path = directory.0.join("snapshot.json");
        config.preview_path = directory.0.join("preview.json");
        let mut observer = Observation::new(&config);
        observer.publish(&config).unwrap();
        let saved = || {
            serde_json::from_slice::<Snapshot>(&std::fs::read(&config.snapshot_path).unwrap())
                .unwrap()
        };
        assert_eq!(saved().launch_phase, LaunchPhase::BeforeSelection);
        let initial_sequence = saved().sequence;
        observer
            .before_native_forward(&config, &json!({"id": 1, "method":"thread/start", "params":{}}))
            .unwrap();
        assert_eq!(saved().launch_phase, LaunchPhase::SelectionPending);
        assert!(saved().sequence > initial_sequence);
        assert!(saved().thread_id.is_none());
        observer.before_server_forward(&config, json!({"id":1,"result":{"thread":{"id":"generated-history", "status":{"type":"idle"}, "turns":[]}}})).unwrap();
        assert_eq!(saved().launch_phase, LaunchPhase::Selected);
        assert_eq!(saved().thread_id.as_deref(), Some("generated-history"));
        observer
            .before_native_forward(
                &config,
                &json!({"id":2,"method":"thread/fork","params":{"threadId":"generated-history"}}),
            )
            .unwrap();
        assert_eq!(saved().launch_phase, LaunchPhase::SelectionPending);
        assert_eq!(saved().thread_id.as_deref(), Some("generated-history"));
        config.resume = Some("existing-history".into());
        assert_eq!(Observation::new(&config).snapshot.launch_phase, LaunchPhase::SelectionPending);
    }

    #[test]
    fn lost_inner_server_cannot_be_audited_by_its_surviving_cli_wrapper() {
        let wrapper = ProcessIdentity {
            pid: 10,
            parent: 1,
            group: 10,
            started: (1, 0),
            zombie: false,
            stopped: false,
        };
        let inner = ProcessIdentity {
            pid: 20,
            parent: 10,
            group: 10,
            started: (2, 0),
            zombie: false,
            stopped: false,
        };
        let mut tree = ProcessTree {
            server: Some(wrapper.clone()),
            known: [(10, wrapper.clone())].into(),
            ..Default::default()
        };
        tree.extend(&[(10, wrapper), (20, inner)].into()).unwrap();
        // Between samples the inner server dies and a newly created tool can
        // reparent without ever entering known. Root-wrapper liveness remains
        // true, just as in captured native app-server SIGKILL 15fa686e.
        tree.known.remove(&20);
        assert!(tree.known.contains_key(&tree.server.as_ref().unwrap().pid));
        tree.relay_ended(&Err(anyhow!("app-server connection reset without closing handshake")));
        assert!(tree.uncertain, "wrapper liveness cannot prove descendant cleanup");
        tree.known.clear();
        tree.relay_ended(&Ok(()));
        assert!(tree.uncertain, "later cleanup or quit cannot erase the ownership gap");

        let mut graceful = ProcessTree { server: tree.server.clone(), ..Default::default() };
        graceful.relay_ended(&Ok(()));
        assert!(
            !graceful.uncertain,
            "verified native quit and controlled TERM retain normal cleanup"
        );
        let mut unlaunched = ProcessTree::default();
        unlaunched.relay_ended(&Err(anyhow!("configuration refused before any child launch")));
        assert!(!unlaunched.uncertain, "no spawned owner means no lost descendants");
    }

    #[test]
    fn native_quit_requires_live_listener_not_a_stale_wrapper_socket() {
        struct SocketPath(PathBuf);
        impl Drop for SocketPath {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let socket = SocketPath(PathBuf::from(format!(
            "/tmp/msmn-cdx-quit-{}.sock",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        )));
        let path = &socket.0;
        let listener = UnixListener::bind(path).unwrap();
        assert!(verify_upstream_listener(path).is_ok());
        drop(listener);
        assert!(path.exists(), "a stale socket alone is not evidence of a live server");
        // Measured 2026-09-12 (T-357): for a few hundred microseconds after
        // `close()` returns, XNU still routes a `connect()` on the path into
        // the closed listener's backlog and reports success. Under the load
        // of the whole `agents::codex` group that window is hit reliably. The
        // probe is right to call that "live" — a false live errs towards not
        // trusting a native quit — so the test waits the kernel out, bounded.
        let deadline = Instant::now() + Duration::from_secs(1);
        while verify_upstream_listener(path).is_ok() {
            assert!(Instant::now() < deadline, "a closed listener kept accepting connections");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(verify_upstream_listener(path).is_err());
        std::fs::remove_file(path).unwrap();
        assert!(verify_upstream_listener(path).is_err());
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
        // CPython on Linux cannot resolve sys.executable with an empty
        // PATH; the grandchild would fail before exercising process cleanup.
        let request = json!({"op":"spawn", "argv":["python3", "-c", code, ready],
            "env":{"PATH":std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into())}});
        writeln!(guard.child.stdin.as_mut().unwrap(), "{request}").unwrap();
        guard.child.stdin.as_mut().unwrap().flush().unwrap();
        let server = reply(&mut output).as_u64().unwrap() as u32;
        let mut tree = ProcessTree::default();
        tree.root(server, true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let tool = loop {
            assert!(
                Instant::now() < deadline,
                "fixture tool did not detach: {}",
                std::fs::read_to_string(root.join("child-0.log")).unwrap_or_default()
            );
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
    fn native_title_tracks_only_selected_thread_and_advances_metadata_sequence() {
        let config = config();
        let mut observation = Observation::new(&config);
        observation.outgoing(&json!({"id":1,"method":"thread/start","params":{}})).unwrap();
        // A notification can precede the selecting response on the native wire.
        observation
            .incoming(
                &config,
                json!({"method":"thread/name/updated",
            "params":{"threadId":"main","threadName":"Buffered native title"}}),
            )
            .unwrap();
        observation
            .incoming(
                &config,
                json!({"id":1,"result":{"thread":{
            "id":"main","name":null,"status":{"type":"idle"},"turns":[]}}}),
            )
            .unwrap();
        assert_eq!(observation.snapshot.title.as_deref(), Some("Buffered native title"));
        observation.advance_evidence();
        let sequence = observation.snapshot.sequence;
        let state = observation.snapshot.state.clone();
        observation
            .incoming(
                &config,
                json!({"method":"thread/name/updated",
            "params":{"threadId":"system","threadName":"Unrelated title generator"}}),
            )
            .unwrap();
        observation.advance_evidence();
        assert_eq!(observation.snapshot.sequence, sequence);
        assert_eq!(observation.snapshot.title.as_deref(), Some("Buffered native title"));
        let name = "🙂".repeat(200);
        observation
            .incoming(
                &config,
                json!({"method":"thread/name/updated",
            "params":{"threadId":"main","threadName":name}}),
            )
            .unwrap();
        observation.advance_evidence();
        assert_eq!(observation.snapshot.sequence, sequence + 1);
        assert_eq!(observation.snapshot.title.as_ref().unwrap().len(), 512);
        assert_eq!(observation.snapshot.state, state);
        observation
            .incoming(
                &config,
                json!({"method":"thread/name/updated",
            "params":{"threadId":"main","threadName":name}}),
            )
            .unwrap();
        observation.advance_evidence();
        assert_eq!(observation.snapshot.sequence, sequence + 1);
        observation.preview.text = Some("Previous conversation reply".into());
        observation.snapshot.plan = Some("Previous plan".into());
        observation.snapshot.plan_key = Some("previous-plan-key".into());
        observation
            .outgoing(&json!({"id":2,"method":"thread/resume","params":{"threadId":"another"}}))
            .unwrap();
        observation
            .incoming(
                &config,
                json!({"id":2,"result":{"thread":{
            "id":"another","name":"Resumed name","status":{"type":"idle"},"turns":[]}}}),
            )
            .unwrap();
        assert_eq!(observation.snapshot.title.as_deref(), Some("Resumed name"));
        assert_eq!(observation.preview, AgentPreview::default());
        assert!(observation.snapshot.plan.is_none());
        assert!(observation.snapshot.plan_key.is_none());
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

    #[test]
    fn plan_and_reply_artifacts_preserve_markdown_links_and_stable_item_identity() {
        let config = config();
        let mut observation = Observation::new(&config);
        observation.outgoing(&json!({"id":1,"method":"thread/start","params":{}})).unwrap();
        observation
            .incoming(
                &config,
                json!({"id":1,"result":{"thread":{
            "id":"main","status":{"type":"idle"},"turns":[]}}}),
            )
            .unwrap();
        observation
            .incoming(
                &config,
                json!({"method":"turn/started","params":{
            "threadId":"main","turn":{"id":"turn","status":"inProgress","items":[]}}}),
            )
            .unwrap();
        let plan = "1. Review [the fixture](https://example.com/fixture).\n2. Verify.";
        observation.incoming(&config, json!({"method":"item/completed","params":{
            "threadId":"main","turnId":"turn","item":{"id":"opaque-plan","type":"plan","text":plan}}})).unwrap();
        assert_eq!(observation.snapshot.plan.as_deref(), Some(plan));
        let plan_key = observation.snapshot.plan_key.clone();
        assert!(plan_key.is_some());
        let reply = "Verified [the fixture](https://example.com/fixture).";
        let frame = json!({"method":"item/completed","params":{
            "threadId":"main","turnId":"turn","item":{"id":"opaque-reply","type":"agentMessage","text":reply}}});
        observation.incoming(&config, frame.clone()).unwrap();
        assert_eq!(observation.preview.text.as_deref(), Some(reply));
        let reply_key = observation.preview.reply_key;
        assert!(reply_key.is_some());
        observation.incoming(&config, frame).unwrap();
        assert_eq!(observation.preview.reply_key, reply_key);
        assert_eq!(observation.snapshot.plan_key, plan_key);
        assert_eq!(observation.snapshot.plan.as_deref(), Some(plan));
    }
}
