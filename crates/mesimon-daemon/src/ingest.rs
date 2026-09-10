//! Bounded hook transport, shared by adapters and tmux lifecycle.

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct HookFrame {
    /// A session UUID (Claude hooks) or a sid16 (the tmux pane-died hook).
    pub session: String,
    pub event: String,
    /// Which registration fired — the matcher travels in argv, not payload
    /// (11 §11.2.3: `SessionStart` has `source`, not `session_start_reason`).
    pub reason: Option<String>,
    pub payload: Value,
}

/// One header line + `\n` + raw payload bytes (may be absent or malformed).
pub fn parse_frame(bytes: &[u8]) -> Option<HookFrame> {
    let nl = bytes.iter().position(|b| *b == b'\n').unwrap_or(bytes.len());
    let header: Value = serde_json::from_slice(&bytes[..nl]).ok()?;
    let session = header.get("session")?.as_str()?.to_string();
    let event = header.get("event")?.as_str()?.to_string();
    let reason = header.get("reason").and_then(Value::as_str).map(str::to_string);
    let payload = bytes
        .get(nl + 1..)
        .filter(|rest| !rest.is_empty())
        .and_then(|rest| serde_json::from_slice(rest).ok())
        .unwrap_or(Value::Null);
    Some(HookFrame { session, event, reason, payload })
}

/// A whole-frame deadline (not a fresh timeout per byte) bounds how long a
/// partial sender can hold up connection ordering. Malformed frames are skipped.
pub fn read_hook_frame(
    mut stream: std::os::unix::net::UnixStream,
    timeout: std::time::Duration,
) -> Option<HookFrame> {
    use std::io::{ErrorKind, Read};
    use std::os::fd::AsRawFd;
    // macOS can reject SO_RCVTIMEO updates after the peer has closed, even
    // with a complete frame buffered. Poll a nonblocking descriptor instead.
    stream.set_nonblocking(true).ok()?;
    let deadline = std::time::Instant::now() + timeout;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
        let mut fd = libc::pollfd { fd: stream.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let millis = remaining.as_millis().saturating_add(1).min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(&mut fd, 1, millis) };
        if ready < 0 && std::io::Error::last_os_error().kind() == ErrorKind::Interrupted {
            continue;
        }
        if ready <= 0 {
            return None;
        }
        let capacity = (HOOK_FRAME_MAX_BYTES as usize).saturating_sub(bytes.len());
        if capacity == 0 {
            return None;
        }
        let length = capacity.min(chunk.len());
        match stream.read(&mut chunk[..length]) {
            Ok(0) => return parse_frame(&bytes),
            Ok(got) => bytes.extend_from_slice(&chunk[..got]),
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                continue
            }
            Err(_) => return None,
        }
    }
}

/// The most of a hook frame either side keeps. A frame is a one-line header
/// and a few JSON keys the daemon reads; a `Write` payload carries the whole
/// file after them and is not needed. Any same-uid process can open
/// `hook.sock`, so what arrives is bounded on both sides of it.
pub const HOOK_FRAME_MAX_BYTES: u64 = 1 << 20;

// Compatibility exports for the existing Claude replay API. Provider-specific
// interpretation lives with the Claude adapter.
pub use crate::agents::claude::hooks::{
    detail_of, plan_of, signal_of, signal_with_monitors, transcript_of,
};
