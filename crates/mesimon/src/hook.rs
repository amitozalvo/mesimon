//! `mesimon hook` — the pure observer (11 §11.2.2, the seven rules).
//!
//! Invoked by Claude Code's hook engine (exec form, never a shell) and by the
//! tmux `pane-died` hook. Reads stdin to EOF, forwards one frame to the
//! daemon's hook socket, exits 0. It never writes to stdout (stdout on
//! `SessionStart`/`UserPromptSubmit` is injected into the agent's context),
//! never returns a decision, and a missing daemon is invisible to the agent
//! (D26: fail open for pure display).
//!
//! Frame: one JSON header line + `\n` + the stdin payload verbatim
//! (SOCK_STREAM one-shot; EOF is the frame delimiter — macOS caps unix
//! datagrams at 2 KB, which real payloads exceed).

use mesimon_daemon::ingest::HOOK_FRAME_MAX_BYTES;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Hard self-abort: `async: true` disables Claude Code's own timeout
/// enforcement, so a hook blocked on a wedged daemon is otherwise unbounded.
const ABORT_MS: u64 = 500;
const WRITE_TIMEOUT_MS: u64 = 250;

pub fn run(args: &[String]) -> ! {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(ABORT_MS));
        std::process::exit(0);
    });
    forward(args);
    std::process::exit(0);
}

fn forward(args: &[String]) {
    // Rule 2: read stdin to EOF before ANY early return — a short read EPIPEs
    // the agent on a large payload. Keep only what the daemon will look at;
    // drain the rest.
    let mut body = Vec::new();
    let mut stdin = std::io::stdin().lock();
    let _ = (&mut stdin).take(HOOK_FRAME_MAX_BYTES).read_to_end(&mut body);
    let _ = std::io::copy(&mut stdin, &mut std::io::sink());

    let Some(sock) = val(args, "--sock") else { return };
    let Some(session) = val(args, "--session") else { return };
    let Some(event) = val(args, "--event") else { return };
    let reason = val(args, "--reason");

    // Rule 7: socket absent or refusing → silent success.
    let Ok(mut stream) = UnixStream::connect(sock) else { return };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(WRITE_TIMEOUT_MS)));

    let header = serde_json::json!({
        "v": 1u32,
        "session": session,
        "event": event,
        "reason": reason,
    });
    let Ok(mut buf) = serde_json::to_vec(&header) else { return };
    buf.push(b'\n');
    buf.extend_from_slice(&body);
    // Rule 6: one connect, one write, exit.
    let _ = stream.write_all(&buf);
}

fn val<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).map(String::as_str)
}
