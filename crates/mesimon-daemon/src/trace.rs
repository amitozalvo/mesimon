//! The trace recorder, a test seam (T-1014): with `MESIMON_RECORD_TRACE`
//! naming a file, the daemon appends to it every frame its hook socket
//! takes (the hook set's and the mod's, in the order the writer is handed
//! them) and every line a session's transcript gains, each as one JSON line
//! stamped with the epoch millisecond the recorder saw it. It is how the
//! e2es' fixtures under `crates/mesimon/tests/traces/` are taken from a real
//! Claude Code (`ci/rig.py --record`), so a stub replays what the engine
//! sends rather than what a test author chose.
//!
//! The recorder is its own thread fed by a channel: the hook reader hands it
//! a copy of each frame and moves on, and nothing on the writer thread waits
//! on a disk. A transcript is followed from the moment a frame names it
//! (`transcript_path`), from its length then, polled every 50 ms; a line is
//! recorded once whole. Unset, nothing is spawned and nothing is written.
//!
//! Lines:
//! - `{"at_ms", "kind": "hook" | "mod", "session", "event", "reason", "payload"}`
//! - `{"at_ms", "kind": "transcript", "session", "line"}`, `line` the
//!   record as parsed (a line that is not JSON is kept as a string).

use crate::ingest::HookFrame;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::Duration;

const POLL: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct Recorder {
    tx: Sender<HookFrame>,
}

impl Recorder {
    /// The recorder `MESIMON_RECORD_TRACE` asks for, its thread started;
    /// `None` when the seam is unset or its file will not open.
    pub fn from_env() -> Option<Self> {
        let path = PathBuf::from(std::env::var_os("MESIMON_RECORD_TRACE")?);
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
        let (tx, rx) = channel::<HookFrame>();
        std::thread::spawn(move || {
            let mut out = Out { file, tails: HashMap::new() };
            loop {
                match rx.recv_timeout(POLL) {
                    Ok(frame) => {
                        out.tail_all();
                        out.frame(&frame);
                    }
                    Err(RecvTimeoutError::Timeout) => out.tail_all(),
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Some(Self { tx })
    }

    pub fn frame(&self, frame: &HookFrame) {
        let _ = self.tx.send(frame.clone());
    }
}

struct Tail {
    session: String,
    offset: u64,
    partial: Vec<u8>,
}

struct Out {
    file: std::fs::File,
    tails: HashMap<PathBuf, Tail>,
}

impl Out {
    fn write(&mut self, line: &Value) {
        let _ = writeln!(self.file, "{line}");
    }

    fn frame(&mut self, frame: &HookFrame) {
        let kind = if frame.road == mesimon_core::road::Road::Mod { "mod" } else { "hook" };
        self.write(&json!({
            "at_ms": mesimon_core::clock::now_ms(),
            "kind": kind,
            "session": frame.session,
            "event": frame.event,
            "reason": frame.reason,
            "payload": frame.payload,
        }));
        let Some(path) = frame.payload.get("transcript_path").and_then(Value::as_str) else {
            return;
        };
        let path = PathBuf::from(path);
        // Lines from before the frame named the file are not this
        // recording's: a resume's transcript already holds its past.
        let offset = std::fs::metadata(&path).map_or(0, |m| m.len());
        self.tails.entry(path).or_insert_with(|| Tail {
            session: frame.session.clone(),
            offset,
            partial: Vec::new(),
        });
    }

    fn tail_all(&mut self) {
        let mut lines = Vec::new();
        for (path, tail) in &mut self.tails {
            let Ok(mut f) = std::fs::File::open(path) else { continue };
            let len = f.metadata().map_or(0, |m| m.len());
            if len < tail.offset {
                tail.offset = 0;
                tail.partial.clear();
            }
            if len == tail.offset || f.seek(SeekFrom::Start(tail.offset)).is_err() {
                continue;
            }
            let mut bytes = Vec::new();
            let Ok(n) = f.read_to_end(&mut bytes) else { continue };
            tail.offset += n as u64;
            tail.partial.extend_from_slice(&bytes);
            while let Some(nl) = tail.partial.iter().position(|b| *b == b'\n') {
                let raw: Vec<u8> = tail.partial.drain(..=nl).collect();
                let text = String::from_utf8_lossy(&raw[..raw.len() - 1]).into_owned();
                if text.trim().is_empty() {
                    continue;
                }
                let line = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
                lines.push((tail.session.clone(), line));
            }
        }
        let at_ms = mesimon_core::clock::now_ms();
        for (session, line) in lines {
            self.write(&json!({"at_ms": at_ms, "kind": "transcript", "session": session,
                "line": line}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_and_the_transcript_lines_after_it_are_recorded_in_order() {
        let dir = std::env::temp_dir().join(format!("msmn-trace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("t.jsonl");
        std::fs::write(&transcript, "{\"before\":true}\n").unwrap();
        let out = dir.join("trace.jsonl");
        let file = std::fs::File::create(&out).unwrap();
        let mut o = Out { file, tails: HashMap::new() };
        let frame = HookFrame {
            session: "s".into(),
            event: "SessionStart".into(),
            reason: Some("resume".into()),
            pane: None,
            road: mesimon_core::road::Road::Hooks,
            payload: json!({"transcript_path": transcript}),
        };
        o.frame(&frame);
        // A line in two writes is recorded once, whole.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap()
            .write_all(b"{\"type\":\"us")
            .unwrap();
        o.tail_all();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap()
            .write_all(b"er\"}\nnot json\n")
            .unwrap();
        o.tail_all();
        let lines: Vec<Value> = std::fs::read_to_string(&out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(
            (lines[0]["kind"].as_str(), lines[0]["event"].as_str()),
            (Some("hook"), Some("SessionStart"))
        );
        assert_eq!(lines[0]["reason"], "resume");
        assert_eq!(lines[1]["line"], json!({"type": "user"}), "the earlier line is not taken");
        assert_eq!(lines[2]["line"], "not json");
        assert_eq!(lines[2]["session"], "s");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
