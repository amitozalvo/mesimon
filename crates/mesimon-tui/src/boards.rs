//! The boards open on this machine (T-733): every daemon under the runtime
//! root, asked who it is, for the Settings › Crown › Reaches boards picker.
//!
//! A board has no registry — the state root holds hundreds of `proj16`
//! directories, none of which names its repository — so the one fact worth
//! listing is which daemons are up right now: each has an `orch.sock` under
//! `/tmp/mesimon-<uid>/<proj16>/`, and its `Hello` says its repository's
//! root and whether a ticket there wears the crown. The walk runs off the
//! draw thread and answers once down a channel; a socket nobody listens on
//! refuses at once, and a daemon that does not answer inside the timeout is
//! left out of the list rather than waited for. Nothing is spawned: a board
//! that is closed is linked by its folder, and opens when the person opens
//! it. Resolved from the machine, so `lib.rs::run` hands `discover` to the
//! app and a test app lists nothing.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use mesimon_core::command::{Command, Envelope, Response, PROTOCOL_VERSION};
use mesimon_core::Principal;

/// One board whose daemon answered: its name (the directory's), its
/// repository's canonical root and the key of its crown, when one is worn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenBoard {
    pub name: String,
    pub path: String,
    pub crown: Option<String>,
}

/// The app's seam to the walk: what `App::board_scanner` holds.
pub type Scanner = fn(PathBuf) -> Receiver<Vec<OpenBoard>>;

/// Ask every daemon on this machine who it is, off the draw thread. The
/// answer comes down the channel once, sorted by name, this board left out.
pub fn discover(me: PathBuf) -> Receiver<Vec<OpenBoard>> {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("boards".into()).spawn(move || {
        let _ = tx.send(scan(&me));
    });
    if spawned.is_err() {
        // No thread: the receiver reads disconnected, which the app takes as
        // an empty list.
        drop(spawned);
    }
    rx
}

fn scan(me: &Path) -> Vec<OpenBoard> {
    let root = mesimon_daemon::paths::runtime_root();
    let Ok(rd) = std::fs::read_dir(&root) else { return Vec::new() };
    let me = me.to_string_lossy();
    let mut out: Vec<OpenBoard> = Vec::new();
    for entry in rd.flatten() {
        let sock = entry.path().join("orch.sock");
        if !sock.exists() {
            continue;
        }
        let Some(b) = hello(&sock) else { continue };
        if b.path.is_empty() || b.path == me || out.iter().any(|o| o.path == b.path) {
            continue;
        }
        out.push(b);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    out
}

/// One `Hello`, as the TUI's own client speaks it, with short timeouts.
fn hello(sock: &Path) -> Option<OpenBoard> {
    let stream = UnixStream::connect(sock).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(800))).ok()?;
    stream.set_write_timeout(Some(Duration::from_millis(300))).ok()?;
    let mut writer = stream.try_clone().ok()?;
    let env = Envelope {
        principal: Principal::Local,
        command: Command::Hello { version: PROTOCOL_VERSION, client: "mesimon-tui boards".into() },
    };
    writeln!(writer, "{}", serde_json::to_string(&env).ok()?).ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    match serde_json::from_str::<Response>(&line).ok()? {
        Response::Hello { repo_root, crown, .. } => Some(OpenBoard {
            name: mesimon_core::boardlink::board_name(&repo_root),
            path: repo_root,
            crown,
        }),
        _ => None,
    }
}
