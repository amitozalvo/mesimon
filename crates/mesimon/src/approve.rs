//! One-shot PermissionRequest bridge. Observers and the deny-only gate are separate.
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use mesimon_core::mesophon::PermissionDecision;

pub fn run(args: &[String]) -> ! {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(47));
        std::process::exit(0);
    });
    if let Some(decision) = decide(args) {
        let _ = std::io::stdout().write_all(decision.hook_output().to_string().as_bytes());
        let _ = std::io::stdout().flush();
    }
    std::process::exit(0);
}

fn decide(args: &[String]) -> Option<PermissionDecision> {
    let val = |key| args.windows(2).find(|a| a[0] == key).map(|a| a[1].as_str());
    let mut body = Vec::new();
    std::io::stdin().take(16 * 1024 + 1).read_to_end(&mut body).ok()?;
    if body.len() > 16 * 1024 {
        return None;
    }
    let payload: serde_json::Value = serde_json::from_slice(&body).ok()?;
    if payload["hook_event_name"] != "PermissionRequest"
        || payload.get("agent_id").is_some()
        || matches!(payload["tool_name"].as_str()?, "AskUserQuestion" | "ExitPlanMode")
        || !payload["tool_input"].is_object()
    {
        return None;
    }
    let mut stream = UnixStream::connect(val("--sock")?).ok()?;
    stream.set_write_timeout(Some(Duration::from_millis(250))).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(45))).ok()?;
    let header =
        serde_json::json!({"v": 1, "session": val("--session")?, "event": "RemotePermission"});
    writeln!(stream, "{header}").ok()?;
    writeln!(stream, "{payload}").ok()?;
    let mut reply = Vec::new();
    stream.take(128).read_to_end(&mut reply).ok()?;
    serde_json::from_slice(&reply).ok()
}
