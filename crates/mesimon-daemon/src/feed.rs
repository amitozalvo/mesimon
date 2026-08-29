//! Append-only activity feed — JSONL in the state dir (D34.10; 14 §1.7 wins
//! over 13's SQLite projection for v0.1). One buffered writer owned by the
//! daemon's main thread: `push` only buffers, `flush` runs from the 250 ms
//! wheel and issues at most ONE `write()` per flush, never an fsync. Rotation
//! by size at 8 MB, a rename off the critical path.
//!
//! D11: prompt text and keystrokes are never logged. The `rule` field is
//! reserved (always null) for the D33m auto-approve decider's audit line.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use mesimon_core::attention;
use mesimon_core::board::{SessionRecord, SessionState};
use serde_json::{json, Value};

const ROTATE_BYTES: u64 = 8 * 1024 * 1024;

pub struct FeedWriter {
    path: PathBuf,
    file: std::fs::File,
    pending: Vec<String>,
    bytes: u64,
    seq: u64,
    rotate_at: u64,
}

impl FeedWriter {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_limit(path, ROTATE_BYTES)
    }

    fn open_with_limit(path: &Path, rotate_at: u64) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self { path: path.to_path_buf(), file, pending: Vec::new(), bytes, seq: 0, rotate_at })
    }

    fn push(&mut self, mut line: Value) {
        self.seq += 1;
        line["v"] = json!(1);
        line["seq"] = json!(self.seq);
        line["at_ms"] = json!(now_ms());
        if let Ok(s) = serde_json::to_string(&line) {
            self.pending.push(s);
        }
    }

    /// One debounced session transition, read off the just-updated record
    /// (the schema the D33m decider will later extend with a non-null `rule`).
    pub fn session_state(&mut self, rec: &SessionRecord, from: &SessionState, hook: Option<&str>) {
        let (to_tag, reason) = split_state(&rec.state);
        let (from_tag, _) = split_state(from);
        self.push(json!({
            "kind": "session_state",
            "session": rec.id,
            "ticket": rec.ticket,
            "from": from_tag,
            "to": to_tag,
            "reason": reason,
            "rank": attention::rank(&rec.state),
            "confidence": rec.confidence,
            "hook": hook,
            "detail": rec.detail,
            "rule": null,
        }));
    }

    /// One received hook frame, by name only — never its payload (D11).
    pub fn hook_event(&mut self, session: &str, event: &str, reason: Option<&str>) {
        self.push(json!({
            "kind": "hook",
            "session": session,
            "event": event,
            "reason": reason,
        }));
    }

    /// One board mutation.
    pub fn board(&mut self, actor: &str, cmd: &str, ticket: Option<ulid::Ulid>) {
        self.push(json!({
            "kind": "board",
            "actor": actor,
            "cmd": cmd,
            "ticket": ticket,
        }));
    }

    /// ≤1 `write()` per call; rotates by size afterwards, off the hot path.
    pub fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut buf = String::with_capacity(self.pending.iter().map(|l| l.len() + 1).sum());
        for line in self.pending.drain(..) {
            buf.push_str(&line);
            buf.push('\n');
        }
        self.file.write_all(buf.as_bytes())?;
        self.bytes += buf.len() as u64;
        if self.bytes >= self.rotate_at {
            let old = self.path.with_extension("jsonl.1");
            let _ = std::fs::rename(&self.path, old);
            self.file =
                std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
            self.bytes = 0;
        }
        Ok(())
    }
}

/// The internally-tagged state → (tag, reason-ish payload) for the feed line.
fn split_state(s: &SessionState) -> (String, Option<String>) {
    let v = serde_json::to_value(s).unwrap_or(Value::Null);
    let tag = v.get("state").and_then(Value::as_str).unwrap_or("unknown").to_string();
    let reason = v
        .get("reason")
        .or_else(|| v.get("stop_reason"))
        .and_then(Value::as_str)
        .map(str::to_string);
    (tag, reason)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::Reason;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("msmn-feed-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("activity.jsonl")
    }

    #[test]
    fn push_buffers_flush_writes() {
        let path = tmp("batch");
        let mut w = FeedWriter::open(&path).unwrap();
        w.board("local", "create_ticket", Some(ulid::Ulid::new()));
        w.hook_event("abc", "Stop", None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "", "push must not write");
        w.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn session_state_line_schema() {
        let path = tmp("schema");
        let mut w = FeedWriter::open(&path).unwrap();
        let mut rec = SessionRecord::new(
            uuid::Uuid::nil(),
            mesimon_core::board::SessionKind::Claude,
            ulid::Ulid::nil(),
            vec![],
            "/tmp".into(),
            SessionState::RequiresAction { reason: Reason::Permission },
        );
        rec.detail = Some("Bash(npm test)".into());
        w.session_state(&rec, &SessionState::Running, Some("PermissionRequest"));
        w.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(v["kind"], "session_state");
        assert_eq!(v["from"], "running");
        assert_eq!(v["to"], "requires_action");
        assert_eq!(v["reason"], "permission");
        assert_eq!(v["rank"], 0);
        assert_eq!(v["confidence"], "high");
        assert_eq!(v["hook"], "PermissionRequest");
        assert_eq!(v["detail"], "Bash(npm test)");
        assert!(v["rule"].is_null(), "rule reserved for the D33m decider");
        assert_eq!(v["v"], 1);
        assert_eq!(v["seq"], 1);
        assert!(v["at_ms"].as_u64().unwrap() > 0);
    }

    #[test]
    fn rotates_by_size() {
        let path = tmp("rotate");
        let mut w = FeedWriter::open_with_limit(&path, 256).unwrap();
        for _ in 0..10 {
            w.hook_event("abcdefabcdefabcd", "UserPromptSubmit", Some("padding-padding"));
        }
        w.flush().unwrap();
        assert!(path.with_extension("jsonl.1").is_file(), "rotated file exists");
        // Post-rotation writes land in the fresh file.
        w.hook_event("abc", "Stop", None);
        w.flush().unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"Stop\""));
    }
}
