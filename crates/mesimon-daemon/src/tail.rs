//! Observe-tier transcript tailing (09 §4.3) — the cursor algorithm, exactly:
//! never seek backwards; keep the trailing fragment; a parse error skips one
//! record and continues (never resync); shrink (`len < offset`) means rotation
//! → reset to 0. State derived here is Tier-0: the caller applies it at
//! `Confidence::Low` only.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use mesimon_core::adopt::{classify_tail_record, turn_edge, TailEvent, TurnEdge};

/// The last uuid-bearing record's classification — how a transcript nobody
/// is streaming RESTED (daemon-restart recovery). Reads at most the final
/// 64 KiB. Trailing uuid-less latch records are skipped, but the walk STOPS
/// at the first uuid record whatever it classifies as — digging past a
/// trailing user/attachment record to an older `turn_duration` would call a
/// freshly-started turn "done".
pub fn last_event(path: &Path) -> Option<TailEvent> {
    for v in tail_records(path)? {
        match classify_tail_record(&v) {
            TailEvent::Latch => continue,
            ev => return Some(ev),
        }
    }
    None
}

/// Does the transcript say the turn that began at `since` (epoch ms) has
/// FINISHED? The status-file probe asks this before it calls a `status: idle`
/// an interrupt: Claude Code stamps `idle` at the end of every turn,
/// milliseconds before its Stop hook fires, so a late or lost Stop otherwise
/// reads as an Esc (simbly T-11, 2026-09-05: a relinked hook binary stalled
/// 41 s in exec and a finished turn wore "interrupted"). Walks back from the
/// end through `adopt::turn_edge`: the first record that speaks decides — a
/// finished turn is `Done` at a stamp not older than the spell (the recordless
/// Esc leaves the PREVIOUS turn's close as the last word, and that stamp is
/// older), an open turn or a missing transcript is `false`.
pub fn turn_done_since(path: &Path, since: u64) -> bool {
    let Some(records) = tail_records(path) else { return false };
    for v in records {
        match turn_edge(&v) {
            TurnEdge::Unsaid => continue,
            TurnEdge::Open => return false,
            TurnEdge::Done(at) => return at >= since,
        }
    }
    false
}

/// The parsed records of the last 64 KiB, NEWEST first. The window may open
/// mid-record, so the first line of a truncated read is dropped; a line that
/// is not JSON is skipped (09 §4.3: never resync).
fn tail_records(path: &Path) -> Option<Vec<serde_json::Value>> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(64 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    Some(
        lines
            .iter()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .collect(),
    )
}

/// Per-session cursor, daemon-held, never persisted.
#[derive(Debug)]
pub struct TailCursor {
    pub path: PathBuf,
    pub byte_offset: u64,
    pub partial: Vec<u8>,
    /// Last time the file grew (epoch ms) — feeds the quiet detector.
    pub grew_at: u64,
}

impl TailCursor {
    /// Start at EOF: attaching a session must not replay months of history
    /// as fresh activity. Preview comes from the census, not the cursor.
    pub fn at_end(path: PathBuf, now: u64) -> Self {
        let byte_offset = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self { path, byte_offset, partial: Vec::new(), grew_at: now }
    }

    /// Read whatever appeared since the last poll and return the complete
    /// records. Best-effort: any I/O failure yields no lines and no cursor
    /// movement (the next poll retries).
    pub fn poll(&mut self, now: u64) -> Vec<String> {
        let Ok(meta) = std::fs::metadata(&self.path) else { return Vec::new() };
        let len = meta.len();
        if len < self.byte_offset {
            // Rotation/truncation guard (09 §4.3).
            self.byte_offset = 0;
            self.partial.clear();
        }
        if len == self.byte_offset {
            return Vec::new();
        }
        let Ok(mut f) = std::fs::File::open(&self.path) else { return Vec::new() };
        if f.seek(SeekFrom::Start(self.byte_offset)).is_err() {
            return Vec::new();
        }
        let mut new = Vec::new();
        if f.take(len - self.byte_offset).read_to_end(&mut new).is_err() {
            return Vec::new();
        }
        self.byte_offset += new.len() as u64;
        self.grew_at = now;

        let mut buf = std::mem::take(&mut self.partial);
        buf.extend_from_slice(&new);
        let (complete, rest) = match buf.iter().rposition(|b| *b == b'\n') {
            Some(i) => (&buf[..=i], &buf[i + 1..]),
            None => (&buf[..0], &buf[..]),
        };
        let lines = String::from_utf8_lossy(complete)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect();
        self.partial = rest.to_vec();
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("msmn-tail-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("t.jsonl")
    }

    #[test]
    fn starts_at_end_and_keeps_fragments() {
        let p = tmp("frag");
        std::fs::write(&p, "{\"old\":1}\n").unwrap();
        let mut c = TailCursor::at_end(p.clone(), 0);
        assert!(c.poll(1).is_empty(), "history is not activity");

        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        write!(f, "{{\"a\":1}}\n{{\"b\":").unwrap();
        f.flush().unwrap();
        assert_eq!(c.poll(2), vec!["{\"a\":1}".to_string()]);

        writeln!(f, "2}}").unwrap();
        f.flush().unwrap();
        assert_eq!(c.poll(3), vec!["{\"b\":2}".to_string()]);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn shrink_resets_to_zero() {
        let p = tmp("rot");
        std::fs::write(&p, "{\"a\":1}\n{\"b\":2}\n").unwrap();
        let mut c = TailCursor::at_end(p.clone(), 0);
        std::fs::write(&p, "{\"c\":3}\n").unwrap();
        assert_eq!(c.poll(1), vec!["{\"c\":3}".to_string()]);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn missing_file_is_quietly_nothing() {
        let mut c = TailCursor::at_end(PathBuf::from("/nonexistent/x.jsonl"), 0);
        assert!(c.poll(1).is_empty());
    }

    #[test]
    fn last_event_reads_through_trailing_latches() {
        let p = tmp("rest");
        std::fs::write(
            &p,
            "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"done work\"}]}}\n\
             {\"uuid\":\"u2\",\"type\":\"system\",\"subtype\":\"turn_duration\"}\n\
             {\"type\":\"last-prompt\"}\n{\"type\":\"mode\"}\nnot json at all\n",
        )
        .unwrap();
        assert_eq!(last_event(&p), Some(TailEvent::TurnComplete));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn turn_done_since_reads_a_finished_turn_and_refuses_an_older_one() {
        let p = tmp("turndone");
        // The shape current Claude Code writes: the closing assistant record,
        // the stop-hook summary, then the uuid-less latches.
        std::fs::write(
            &p,
            "{\"uuid\":\"u0\",\"type\":\"user\",\"timestamp\":\"2026-09-05T16:50:00.000Z\",\"message\":{\"content\":\"go\"}}\n\
             {\"uuid\":\"u1\",\"type\":\"assistant\",\"timestamp\":\"2026-09-05T16:56:32.998Z\",\"message\":{\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n\
             {\"uuid\":\"u2\",\"type\":\"system\",\"subtype\":\"stop_hook_summary\",\"timestamp\":\"2026-09-05T16:56:33.447Z\"}\n\
             {\"type\":\"agent-name\"}\n{\"type\":\"mode\"}\n",
        )
        .unwrap();
        let close = 1_788_627_393_447;
        assert!(turn_done_since(&p, close - 60_000), "a turn that began before the close is done");
        assert!(turn_done_since(&p, close), "at the stamp itself");
        assert!(
            !turn_done_since(&p, close + 1),
            "a turn begun after the close: the recordless Esc"
        );
        // A tool result after the close: the next turn is open.
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{{\"uuid\":\"u3\",\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\"}}]}}}}").unwrap();
        writeln!(f, "{{\"uuid\":\"u4\",\"type\":\"attachment\"}}").unwrap();
        assert!(!turn_done_since(&p, close - 60_000));
        assert!(!turn_done_since(Path::new("/nonexistent/x.jsonl"), 0));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn last_event_stops_at_a_trailing_user_record() {
        // A user prompt after the previous turn's close: the turn is (maybe)
        // in flight — the older turn_duration must NOT win.
        let p = tmp("userlast");
        std::fs::write(
            &p,
            "{\"uuid\":\"u1\",\"type\":\"system\",\"subtype\":\"turn_duration\"}\n\
             {\"uuid\":\"u2\",\"type\":\"user\",\"message\":{\"content\":\"go again\"}}\n\
             {\"type\":\"atis-latch\"}\n",
        )
        .unwrap();
        assert_eq!(last_event(&p), Some(TailEvent::Other));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn last_event_reads_a_trailing_tool_call_as_in_flight() {
        // The reload shape (T-265): the last record is the call, its result
        // is minutes away, and the latches after it say nothing.
        let p = tmp("toolcall");
        std::fs::write(
            &p,
            "{\"uuid\":\"u1\",\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\"}]}}\n\
             {\"uuid\":\"u2\",\"type\":\"assistant\",\"message\":{\"stop_reason\":\"tool_use\",\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"cargo ut\"}}]}}\n\
             {\"type\":\"mode\"}\n",
        )
        .unwrap();
        assert_eq!(last_event(&p), Some(TailEvent::ToolInFlight));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn last_event_empty_or_missing_is_none() {
        assert_eq!(last_event(Path::new("/nonexistent/x.jsonl")), None);
        let p = tmp("empty");
        std::fs::write(&p, "").unwrap();
        assert_eq!(last_event(&p), None);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }
}
