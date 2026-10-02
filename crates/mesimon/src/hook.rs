//! `mesimon hook` — the pure observer (11 §11.2.2, the seven rules).
//!
//! Invoked by Claude Code's hook engine (exec form, never a shell) and by the
//! tmux `pane-died` hook. The header names the pane the frame comes from
//! (`--pane`, else `TMUX_PANE`), so the daemon can tell a reused session
//! name from a reused pane (T-245). Reads stdin to EOF, forwards one frame to the
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
    // The pane this frame comes from, as `<server pid>:<pane id>`: the tmux
    // `pane-died` hook spells it in argv (`--pane "#{pid}:#{pane_id}"`); a
    // Claude hook inherits tmux's own `TMUX` (`socket,server pid,index`)
    // and `TMUX_PANE` through claude, because `mesimon exec` layers the env
    // file over the inherited environment and never clears it. A wake
    // reuses the session name, never the pane — so a death frame naming
    // another pane is the previous tenant's and the daemon drops it. The
    // server pid is part of the key because a pane id is only unique per
    // server: a wake that took the server's last session down restarts it,
    // and the fresh server hands out `%0` again (wake_straggler_e2e).
    let pane = val(args, "--pane").map(str::to_string).or_else(pane_key_from_env);

    // Rule 7: socket absent or refusing → silent success.
    let Ok(mut stream) = UnixStream::connect(sock) else { return };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(WRITE_TIMEOUT_MS)));

    let header = header(session, event, reason, pane.as_deref(), val(args, "--road"));
    let Ok(mut buf) = serde_json::to_vec(&header) else { return };
    buf.push(b'\n');
    buf.extend_from_slice(&body);
    // Rule 6: one connect, one write, exit.
    let _ = stream.write_all(&buf);
}

/// The frame's header line. `road` says which road the frame came by
/// (T-574): absent for the hook set, `mod` when the mod mesimon laid relays
/// the same event, so the daemon can pair the two and ingest only one.
fn header(
    session: &str,
    event: &str,
    reason: Option<&str>,
    pane: Option<&str>,
    road: Option<&str>,
) -> serde_json::Value {
    let mut header = serde_json::json!({
        "v": 1u32,
        "session": session,
        "event": event,
        "reason": reason,
        "pane": pane,
    });
    if let Some(road) = road {
        header["road"] = road.into();
    }
    header
}

/// `<server pid>:<pane id>` from the environment tmux gives a pane's process,
/// or `None` outside one (and the daemon then trusts the frame as before).
pub(crate) fn pane_key_from_env() -> Option<String> {
    let tmux = std::env::var("TMUX").ok()?;
    let server_pid = tmux.split(',').nth(1).filter(|p| !p.is_empty())?;
    let pane = std::env::var("TMUX_PANE").ok().filter(|p| !p.is_empty())?;
    Some(format!("{server_pid}:{pane}"))
}

fn val<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_set_sends_no_road_and_the_mod_names_its_own() {
        let h = header("s", "Stop", None, Some("1:%0"), None);
        assert!(h.get("road").is_none(), "{h}");
        assert_eq!(h["pane"], "1:%0");
        let m = header("s", "SessionStart", Some("startup"), None, Some("mod"));
        assert_eq!(m["road"], "mod");
        assert_eq!(m["reason"], "startup");
        assert_eq!(m["v"], 1);
    }
}
