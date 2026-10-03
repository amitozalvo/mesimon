//! One-shot PermissionRequest bridge. Observers and the deny-only gate are separate.
//!
//! `--hold <secs>` is how long it waits on the daemon (45 s for a hook file
//! written before T-632, which gave the entry 50). `--renew` is the mod's
//! round: the daemon passes its wait to the next run, and a round that ran
//! out exits `PERMISSION_RENEW_EXIT` so the mod runs that next one.
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use mesimon_core::mesophon::{PermissionDecision, PERMISSION_RENEW_EXIT};

enum Outcome {
    Decided(PermissionDecision),
    /// The round ran out with the daemon still holding the dialog.
    RoundOver,
    None,
}

pub fn run(args: &[String]) -> ! {
    let hold = hold_of(args);
    let renew = args.iter().any(|a| a == "--renew");
    std::thread::spawn(move || {
        std::thread::sleep(hold + Duration::from_secs(2));
        std::process::exit(if renew { PERMISSION_RENEW_EXIT } else { 0 });
    });
    match decide(args, hold, renew) {
        Outcome::Decided(decision) => {
            let _ = std::io::stdout().write_all(decision.hook_output().to_string().as_bytes());
            let _ = std::io::stdout().flush();
            std::process::exit(0)
        }
        Outcome::RoundOver if renew => std::process::exit(PERMISSION_RENEW_EXIT),
        _ => std::process::exit(0),
    }
}

fn hold_of(args: &[String]) -> Duration {
    let secs = args
        .windows(2)
        .find(|a| a[0] == "--hold")
        .and_then(|a| a[1].parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(45)
        .min(mesimon_core::mesophon::PERMISSION_HOLD_SECS);
    Duration::from_secs(secs)
}

fn decide(args: &[String], hold: Duration, renew: bool) -> Outcome {
    let val = |key| args.windows(2).find(|a| a[0] == key).map(|a| a[1].as_str());
    let mut body = Vec::new();
    if std::io::stdin().take(16 * 1024 + 1).read_to_end(&mut body).is_err()
        || body.len() > 16 * 1024
    {
        return Outcome::None;
    }
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return Outcome::None;
    };
    if payload["hook_event_name"] != "PermissionRequest"
        || payload.get("agent_id").is_some()
        || payload["tool_name"]
            .as_str()
            .is_none_or(|t| matches!(t, "AskUserQuestion" | "ExitPlanMode"))
        || !payload["tool_input"].is_object()
    {
        return Outcome::None;
    }
    let (Some(sock), Some(session)) = (val("--sock"), val("--session")) else {
        return Outcome::None;
    };
    let Ok(mut stream) = UnixStream::connect(sock) else { return Outcome::None };
    if stream.set_write_timeout(Some(Duration::from_millis(250))).is_err()
        || stream.set_read_timeout(Some(hold)).is_err()
    {
        return Outcome::None;
    }
    let mut header = serde_json::json!({"v": 1, "session": session, "event": "RemotePermission"});
    if renew {
        header["reason"] = "renew".into();
    }
    if writeln!(stream, "{header}").is_err() || writeln!(stream, "{payload}").is_err() {
        return Outcome::None;
    }
    let mut reply = Vec::new();
    match stream.take(128).read_to_end(&mut reply) {
        Ok(_) => serde_json::from_slice(&reply).map_or(Outcome::None, Outcome::Decided),
        // The daemon still holds the wait: this round is over, not the hold.
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            Outcome::RoundOver
        }
        Err(_) => Outcome::None,
    }
}
