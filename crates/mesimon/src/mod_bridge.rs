//! `mesimon mod-bridge` — the daemon's commands to one session's mod (T-574).
//!
//! Spawned once per session by the mod mesimon lays (`$.process.spawn` at
//! `session.start`; its stdin is closed and its stdout streams to the mod).
//! It long-polls `orch.sock` as the session (`ModNext`), prints every frame
//! the daemon addressed to the session as one JSON line, and acks it with
//! the next poll. stdout carries frames and nothing else; anything to say
//! goes to stderr.
//!
//! It outlives nothing: it exits when its parent (Claude Code) is gone —
//! nothing else kills a spawned child of a SIGKILLed host — when the mod
//! stops reading (stdout closed), and with `BRIDGE_REFUSED_EXIT` when the
//! daemon refuses it for good (the session is unknown or gone, the pane is
//! not its own, another bridge took the seat), which the mod does not
//! respawn. A daemon that went away is waited for: a restart, a `U`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use mesimon_core::command::{Command, Envelope, Response};
use mesimon_core::road::BRIDGE_REFUSED_EXIT;
use mesimon_core::Principal;

const BACKOFF_FIRST: Duration = Duration::from_millis(100);
const BACKOFF_MAX: Duration = Duration::from_secs(2);

enum Ended {
    /// The connection went (or never came): try again.
    Disconnected,
    /// The daemon said no, for good.
    Refused(String),
    /// The mod stopped reading.
    StdoutGone,
}

pub fn run(args: &[String]) -> ! {
    let sock = val(args, "--sock");
    let session = val(args, "--session").and_then(|s| s.parse::<uuid::Uuid>().ok());
    let (Some(sock), Some(session)) = (sock, session) else {
        eprintln!("usage: mesimon mod-bridge --sock <orch.sock> --session <uuid>");
        std::process::exit(2);
    };
    watch_parent();
    let pane = crate::hook::pane_key_from_env();
    let mut ack: Option<String> = None;
    let mut backoff = BACKOFF_FIRST;
    loop {
        match poll(sock, session, pane.as_deref(), &mut ack, &mut backoff) {
            Ended::Disconnected => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
            Ended::Refused(message) => {
                eprintln!("mesimon mod-bridge: {message}");
                std::process::exit(BRIDGE_REFUSED_EXIT);
            }
            Ended::StdoutGone => std::process::exit(0),
        }
    }
}

/// One connection's life: poll, print, poll again with the ack.
fn poll(
    sock: &str,
    session: uuid::Uuid,
    pane: Option<&str>,
    ack: &mut Option<String>,
    backoff: &mut Duration,
) -> Ended {
    let Ok(stream) = UnixStream::connect(sock) else { return Ended::Disconnected };
    let Ok(mut writer) = stream.try_clone() else { return Ended::Disconnected };
    let mut reader = BufReader::new(stream);
    let mut out = std::io::stdout().lock();
    loop {
        let envelope = Envelope {
            principal: Principal::Agent { session },
            command: Command::ModNext { ack: ack.clone(), pane: pane.map(str::to_string) },
        };
        let Ok(mut line) = serde_json::to_vec(&envelope) else { return Ended::Disconnected };
        line.push(b'\n');
        if writer.write_all(&line).and_then(|()| writer.flush()).is_err() {
            return Ended::Disconnected;
        }
        let mut reply = String::new();
        match reader.read_line(&mut reply) {
            Ok(0) | Err(_) => return Ended::Disconnected,
            Ok(_) => {}
        }
        // Connected and answered: the next outage starts its backoff over.
        *backoff = BACKOFF_FIRST;
        match serde_json::from_str::<Response>(&reply) {
            Ok(Response::ModFrames { frames }) => {
                for frame in frames {
                    let Ok(text) = serde_json::to_string(&frame) else { continue };
                    if writeln!(out, "{text}").and_then(|()| out.flush()).is_err() {
                        return Ended::StdoutGone;
                    }
                    if let Some(id) = frame.get("id").and_then(|v| v.as_str()) {
                        *ack = Some(id.to_string());
                    }
                }
            }
            // The writer thread ended under a parked poll: a daemon stopping.
            Ok(Response::Err { message }) if message == "daemon gone" => {
                return Ended::Disconnected
            }
            Ok(Response::Err { message }) => return Ended::Refused(message),
            // An answer this build does not know: from a newer daemon. Ask
            // again, not at once.
            Ok(_) | Err(_) => std::thread::sleep(BACKOFF_MAX),
        }
    }
}

/// Exit when the parent goes: a Claude Code killed outright leaves its
/// spawned children running, and a bridge must not poll for a dead session.
fn watch_parent() {
    let parent = std::os::unix::process::parent_id();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        if std::os::unix::process::parent_id() != parent {
            std::process::exit(0);
        }
    });
}

fn val<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).map(String::as_str)
}
