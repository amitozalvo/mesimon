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
    /// The tmux pane the frame came from, as `<server pid>:<pane id>`: the
    /// pane-died hook's `--pane`, or the `TMUX` + `TMUX_PANE` a Claude hook
    /// inherits. Absent from a hook binary older than T-245 or a frame sent
    /// from outside a pane.
    pub pane: Option<String>,
    /// Which road the frame came by (T-574): the hook set's, or the mod's
    /// relay of the same event. A mod frame is shadow-paired, never ingested.
    pub road: mesimon_core::road::Road,
    /// When the hook socket accepted it, epoch ms (0 until the reader thread
    /// stamps it): the shadow's 2 s window runs from here, so a writer stall
    /// cannot make a twin look late.
    pub accepted_ms: u64,
    pub payload: Value,
}

/// One header line + `\n` + raw payload bytes (may be absent or malformed).
pub fn parse_frame(bytes: &[u8]) -> Option<HookFrame> {
    let nl = bytes.iter().position(|b| *b == b'\n').unwrap_or(bytes.len());
    let header: Value = serde_json::from_slice(&bytes[..nl]).ok()?;
    let session = header.get("session")?.as_str()?.to_string();
    let event = header.get("event")?.as_str()?.to_string();
    let reason = header.get("reason").and_then(Value::as_str).map(str::to_string);
    let pane =
        header.get("pane").and_then(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string);
    let road = mesimon_core::road::Road::from_header(header.get("road").and_then(Value::as_str))?;
    let payload = bytes
        .get(nl + 1..)
        .filter(|rest| !rest.is_empty())
        .and_then(|rest| serde_json::from_slice(rest).ok())
        .unwrap_or(Value::Null);
    Some(HookFrame { session, event, reason, pane, road, accepted_ms: 0, payload })
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
            Ok(got) => {
                bytes.extend_from_slice(&chunk[..got]);
                // A deciding hook stays duplex while waiting. Its frame is
                // two NDJSON records, so process EOF remains a cancellation
                // signal instead of also delimiting the request body.
                if let Some(nl) = bytes.iter().position(|b| *b == b'\n') {
                    let header: Value = serde_json::from_slice(&bytes[..nl]).unwrap_or(Value::Null);
                    if header["event"] == "RemotePermission" && bytes[nl + 1..].contains(&b'\n') {
                        return parse_frame(&bytes);
                    }
                }
            }
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
    detail_of, plan_of, signal_of, signal_with_background, transcript_of,
};

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    #[test]
    fn the_header_names_its_pane_when_it_has_one() {
        let with = parse_frame(b"{\"session\":\"s\",\"event\":\"PaneDied\",\"pane\":\"41:%3\"}\n")
            .unwrap();
        assert_eq!(with.pane.as_deref(), Some("41:%3"));
        // An older hook binary, or a frame sent from outside any pane.
        let without = parse_frame(b"{\"session\":\"s\",\"event\":\"PaneDied\"}\n").unwrap();
        assert_eq!(without.pane, None);
        let blank =
            parse_frame(b"{\"session\":\"s\",\"event\":\"PaneDied\",\"pane\":\"\"}\n").unwrap();
        assert_eq!(blank.pane, None);
    }

    #[test]
    fn deciding_frame_finishes_without_eof_but_observer_waits_for_eof() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer
            .write_all(
                b"{\"session\":\"s\",\"event\":\"RemotePermission\"}\n{\"tool_name\":\"Bash\"}\n",
            )
            .unwrap();
        let frame = read_hook_frame(reader, Duration::from_millis(100)).unwrap();
        assert_eq!(frame.payload["tool_name"], "Bash");
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"{\"session\":\"s\",\"event\":\"Stop\"}\n{}\n").unwrap();
        assert!(read_hook_frame(reader, Duration::from_millis(20)).is_none());
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"{\"session\":\"s\",\"event\":\"Stop\"}\n{}").unwrap();
        drop(writer);
        assert_eq!(read_hook_frame(reader, Duration::from_millis(100)).unwrap().event, "Stop");
    }

    #[test]
    fn a_frame_names_its_road_and_a_foreign_road_is_dropped() {
        use mesimon_core::road::Road;
        let hooks = parse_frame(b"{\"session\":\"s\",\"event\":\"Stop\"}\n{}").unwrap();
        assert_eq!(hooks.road, Road::Hooks);
        let modded =
            parse_frame(b"{\"session\":\"s\",\"event\":\"Stop\",\"road\":\"mod\"}\n{}").unwrap();
        assert_eq!(modded.road, Road::Mod);
        assert!(parse_frame(b"{\"session\":\"s\",\"event\":\"Stop\",\"road\":\"x\"}\n{}").is_none());
    }
}
