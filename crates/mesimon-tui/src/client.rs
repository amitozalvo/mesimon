//! Daemon client: connect (spawning the daemon if needed), send typed envelopes,
//! receive responses; a reader thread routes async events to a dirty flag.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
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
}

pub struct Client {
    write: UnixStream,
    responses: Receiver<Response>,
    pub events: Receiver<()>,
}

impl Transport for Client {
    fn request(&mut self, command: Command) -> Result<Response> {
        Client::request(self, command)
    }

    fn poll_event(&mut self) -> bool {
        self.events.try_recv().is_ok()
    }
}

impl Client {
    pub fn connect(repo_root: &Path) -> Result<Self> {
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

        let mut c = Client { write, responses: rrx, events: erx };
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

    pub fn request(&mut self, command: Command) -> Result<Response> {
        let env = Envelope { principal: Principal::Local, command };
        let json = serde_json::to_string(&env)?;
        writeln!(self.write, "{json}").context("write to daemon")?;
        self.responses
            .recv_timeout(Duration::from_secs(10))
            .context("daemon response timeout")
    }
}
