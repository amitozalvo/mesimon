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
use mesimon_core::command::{Command, Envelope, Response, PROTOCOL_VERSION};
use mesimon_core::Principal;
use mesimon_daemon::Paths;

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
}

pub struct Client {
    repo_root: PathBuf,
    conn: Option<Conn>,
}

/// One live connection's worth of channel plumbing — replaced wholesale on
/// reconnect so a stale reader thread can never route into fresh channels.
struct Conn {
    write: UnixStream,
    responses: Receiver<Response>,
    events: Receiver<()>,
}

impl Transport for Client {
    fn request(&mut self, command: Command) -> Result<Response> {
        let res = match self.conn.as_mut() {
            Some(c) => c.request(command),
            None => open(&self.repo_root)
                .map(|c| self.conn.insert(c))
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
}

impl Client {
    pub fn connect(repo_root: &Path) -> Result<Self> {
        let conn = open(repo_root)?;
        Ok(Client { repo_root: repo_root.to_path_buf(), conn: Some(conn) })
    }
}

fn open(repo_root: &Path) -> Result<Conn> {
    let paths = Paths::for_repo(repo_root)?;
    let sock = paths.orch_sock();

    let stream = match UnixStream::connect(&sock) {
        Ok(s) => s,
        Err(_) => {
            mesimon_daemon::spawn_detached(repo_root)?;
            // Retry-connect while the daemon comes up (02 §4).
            let mut last = None;
            let mut ok = None;
            for _ in 0..50 {
                std::thread::sleep(Duration::from_millis(100));
                match UnixStream::connect(&sock) {
                    Ok(s) => {
                        ok = Some(s);
                        break;
                    }
                    Err(e) => last = Some(e),
                }
            }
            match ok {
                Some(s) => s,
                None => bail!("daemon did not come up: {last:?}"),
            }
        }
    };

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

    let mut c = Conn { write, responses: rrx, events: erx };
    let hello = c.request(Command::Hello {
        version: PROTOCOL_VERSION,
        client: format!("mesimon-tui/{}", env!("CARGO_PKG_VERSION")),
    })?;
    match hello {
        Response::Hello { .. } => {}
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
        self.responses
            .recv_timeout(Duration::from_secs(10))
            .context("daemon response timeout")
    }
}
