//! Observe-tier transcript tailing (09 §4.3) — the cursor algorithm, exactly:
//! never seek backwards; keep the trailing fragment; a parse error skips one
//! record and continues (never resync); shrink (`len < offset`) means rotation
//! → reset to 0. State derived here is Tier-0: the caller applies it at
//! `Confidence::Low` only.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use mesimon_core::adopt::{
    classify_tail_record, record_ms, turn_edge, TailEvent, ToolLedger, TurnEdge,
};

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
            TailEvent::Other
                if v.get("type").and_then(serde_json::Value::as_str) == Some("attachment") =>
            {
                continue
            }
            ev => return Some(ev),
        }
    }
    None
}

/// Is a turn IN FLIGHT on this transcript as of `now` — a tool running, or a
/// reply under way? The pane probe asks this before it calls a silent pane an
/// interrupt (T-439, 2026-09-23): a live Claude Code pane wrote no byte
/// through a 63 s test suite, so it read as an Esc and the card wore
/// "interrupted" until the tool's `PostToolUse` put it back. The last uuid
/// record decides, trailing latches and attachments skipped. A trailing tool
/// call with no result is a tool in flight for as long as it runs — the
/// transcript is still throughout (T-265), so no clock bounds it. Any other
/// trailing assistant record, or a tool's result the model has yet to answer,
/// is a reply under way while its stamp is inside `quiet_ms`; past that the
/// fallback is the fallback again. The recordless Esc leaves the user's
/// prompt as the last word, a mid-turn Esc lands an abort record, a finished
/// turn its close: none of those reads as in flight.
pub fn turn_in_flight(path: &Path, now: u64, quiet_ms: u64) -> bool {
    let Some(records) = tail_records(path) else { return false };
    for v in records {
        let kind = v.get("type").and_then(serde_json::Value::as_str);
        let fresh = || record_ms(&v).is_some_and(|at| now.saturating_sub(at) < quiet_ms);
        match classify_tail_record(&v) {
            TailEvent::Latch => continue,
            TailEvent::Other if kind == Some("attachment") => continue,
            TailEvent::ToolInFlight => return true,
            TailEvent::AssistantText { .. } => return fresh(),
            TailEvent::Other if kind == Some("assistant") => return fresh(),
            TailEvent::Other if kind == Some("user") && carries_tool_result(&v) => return fresh(),
            _ => return false,
        }
    }
    false
}

/// A `user` record whose content holds a `tool_result` block: a tool came
/// back and the model owes the next word.
fn carries_tool_result(v: &serde_json::Value) -> bool {
    v.get("message")
        .and_then(|m| m.get("content"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|blocks| {
            blocks
                .iter()
                .any(|b| b.get("type").and_then(serde_json::Value::as_str) == Some("tool_result"))
        })
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

/// Recover only a recent explicit cancellation when a cursor is first minted.
/// A newer user/assistant turn record stops the search; old aborts cannot cancel
/// a new turn just because they remain in the bounded history window.
pub fn aborted_since(path: &Path, since: u64) -> bool {
    let Some(records) = tail_records(path) else { return false };
    for record in records {
        if classify_tail_record(&record) == TailEvent::Aborted {
            return mesimon_core::adopt::record_ms(&record).is_some_and(|at| at >= since);
        }
        if !matches!(turn_edge(&record), TurnEdge::Unsaid) {
            return false;
        }
    }
    false
}

/// The API error a failed turn ends on, as the transcript renders it.
pub struct ApiError {
    /// The hook set's class word (`rate_limit`, `model_not_found`, …).
    pub class: String,
    /// The rendered text, the hook set's `last_assistant_message`.
    pub text: Option<String>,
}

/// The error the LAST turn failed on, if the transcript says it yet (T-659).
/// Claude Code closes a failed turn with one `assistant` record of model
/// `<synthetic>` carrying `isApiErrorMessage: true`, the class in `error` in
/// the hook set's own words, and the rendered message as its one text block
/// (2.1.289, measured for `model_not_found`, `authentication_failed` and
/// `max_output_tokens`). The walk skips latches, attachments and `system`
/// rows — a retry's `api_error` row is one, written mid-turn and not the
/// class — and stops at the first other `user` or `assistant` record: an
/// error row behind a newer prompt or reply belongs to an older turn, and a
/// turn whose error is not flushed yet has its prompt or its last reply as
/// the last word.
pub fn api_error(path: &Path) -> Option<ApiError> {
    use serde_json::Value;
    for v in tail_records(path)? {
        if v.get("uuid").is_none() {
            continue;
        }
        match v.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                if v.get("isApiErrorMessage").and_then(Value::as_bool) != Some(true) {
                    return None;
                }
                let class = v
                    .get("error")
                    .or_else(|| v.get("apiError"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let text = mesimon_core::adopt::assistant_text(&v).map(str::to_string);
                return Some(ApiError { class, text });
            }
            Some("user") => return None,
            _ => continue,
        }
    }
    None
}

/// The parsed records of the last 64 KiB, NEWEST first. The window may open
/// mid-record, so the first line of a truncated read is dropped; a line that
/// is not JSON is skipped (09 §4.3: never resync).
fn tail_records(path: &Path) -> Option<Vec<serde_json::Value>> {
    let len = std::fs::metadata(path).ok()?.len();
    let text = window_text(path, len, 64 * 1024)?;
    Some(
        text.lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .collect(),
    )
}

/// The whole lines in the final `window` bytes of a `len`-byte JSONL file:
/// the read can open mid-record and mid-UTF-8, so it is lossy and its first
/// line is dropped when it did not start at the top. `None` only when the
/// file cannot be read.
pub(crate) fn window_text(path: &Path, len: u64, window: u64) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let start = len.saturating_sub(window);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if start == 0 {
        return Some(text);
    }
    Some(text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default())
}

/// Per-session cursor, daemon-held, never persisted.
#[derive(Debug)]
pub struct TailCursor {
    pub path: PathBuf,
    pub byte_offset: u64,
    pub partial: Vec<u8>,
    /// Last time the file grew (epoch ms) — feeds the quiet detector.
    pub grew_at: u64,
    pub tools: ToolLedger,
}

impl TailCursor {
    /// Start at EOF: attaching a session must not replay months of history
    /// as fresh activity. Preview comes from the census, not the cursor.
    pub fn at_end(path: PathBuf, now: u64) -> Self {
        let byte_offset = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut tools = ToolLedger::default();
        if let Some(records) = tail_records(&path) {
            for record in records.iter().rev() {
                tools.observe(record);
            }
        }
        Self { path, byte_offset, partial: Vec::new(), grew_at: now, tools }
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
            self.tools = ToolLedger::default();
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
        let lines: Vec<String> = String::from_utf8_lossy(complete)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect();
        self.partial = rest.to_vec();
        for line in &lines {
            if let Ok(record) = serde_json::from_str(line) {
                self.tools.observe(&record);
            }
        }
        lines
    }
}

/// A failed turn's closing record as Claude Code 2.1.289 writes it (T-659,
/// measured: `--model claude-no-such-model-0`, an invalid
/// `ANTHROPIC_API_KEY`, `CLAUDE_CODE_MAX_OUTPUT_TOKENS=1`), trimmed of the
/// envelope fields nothing here reads.
#[cfg(test)]
pub(crate) fn error_row(class: &str, text: &str) -> serde_json::Value {
    serde_json::json!({"uuid": format!("err-{class}"), "type": "assistant",
        "timestamp": "2026-10-05T11:35:48.686Z",
        "message": {"model": "<synthetic>", "role": "assistant", "stop_reason": "stop_sequence",
            "stop_sequence": "", "type": "message", "usage": {"input_tokens": 0, "output_tokens": 0},
            "content": [{"type": "text", "text": text}]},
        "error": class, "isApiErrorMessage": true, "apiErrorStatus": 404})
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
    fn abort_backfill_requires_current_timestamp_and_no_newer_turn() {
        let path = tmp("abort-backfill");
        let record = serde_json::json!({"uuid":"abort", "type":"user",
            "timestamp":"2026-09-05T16:56:32.998Z",
            "message":{"content":"[Request interrupted by user]"}});
        let stamp = mesimon_core::adopt::record_ms(&record).unwrap();
        std::fs::write(&path, format!("{record}\n")).unwrap();
        assert!(aborted_since(&path, stamp - 1));
        assert!(!aborted_since(&path, stamp + 1));
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            file,
            "{{\"uuid\":\"new\",\"type\":\"user\",\"message\":{{\"content\":\"next prompt\"}}}}"
        )
        .unwrap();
        assert!(!aborted_since(&path, stamp - 1));
        std::fs::write(&path, "{\"uuid\":\"abort\",\"type\":\"user\",\"message\":{\"content\":\"[Request interrupted by user]\"}}\n").unwrap();
        assert!(!aborted_since(&path, 0), "undated history is not fresh evidence");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
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

    /// T-439: the pane probe's question. A tool with no result is in flight
    /// however long it runs; a reply or a returned tool is in flight while
    /// fresh; a prompt, a close and an abort never are.
    #[test]
    fn turn_in_flight_reads_the_tail() {
        let path = tmp("inflight");
        let quiet = 60_000;
        let at = |s: &str| mesimon_core::adopt::iso_ms(s).unwrap();
        let t0 = "2026-09-23T08:33:11.000Z";
        let write = |lines: &[serde_json::Value]| {
            let text: Vec<String> = lines.iter().map(|v| v.to_string()).collect();
            std::fs::write(&path, text.join("\n") + "\n").unwrap();
        };
        let prompt = serde_json::json!({"uuid":"p","type":"user","timestamp":t0,
            "message":{"role":"user","content":"do the thing"}});
        let call = serde_json::json!({"uuid":"c","type":"assistant","timestamp":t0,
            "message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}});
        let result = serde_json::json!({"uuid":"r","type":"user","timestamp":t0,
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}});
        let attachment =
            serde_json::json!({"uuid":"a","type":"attachment","timestamp":t0,"attachment":{}});
        let text = serde_json::json!({"uuid":"x","type":"assistant","timestamp":t0,
            "message":{"stop_reason":null,"content":[{"type":"text","text":"Build is clean."}]}});
        let thinking = serde_json::json!({"uuid":"k","type":"assistant","timestamp":t0,
            "message":{"stop_reason":"tool_use","content":[{"type":"thinking","thinking":"hm"}]}});
        let close = serde_json::json!({"uuid":"d","type":"system","subtype":"turn_duration","timestamp":t0,"durationMs":5});
        let abort = serde_json::json!({"uuid":"e","type":"user","timestamp":t0,
            "message":{"role":"user","content":"[Request interrupted by user for tool use]"}});
        let latch = serde_json::json!({"type":"cost-state","timestamp":t0});

        // The recordless Esc: the prompt is the last word, however fresh.
        write(std::slice::from_ref(&prompt));
        assert!(!turn_in_flight(&path, at(t0) + 1_000, quiet));
        // A tool in flight, for as long as it runs — a trailing latch or
        // attachment does not hide it.
        write(&[prompt.clone(), call.clone()]);
        assert!(turn_in_flight(&path, at(t0) + 10 * 60_000, quiet));
        write(&[prompt.clone(), call.clone(), latch.clone(), attachment.clone()]);
        assert!(turn_in_flight(&path, at(t0) + 10 * 60_000, quiet));
        // The tool came back: the model owes a word, while that is fresh.
        write(&[prompt.clone(), call.clone(), result.clone(), attachment.clone()]);
        assert!(turn_in_flight(&path, at(t0) + quiet - 1, quiet));
        assert!(!turn_in_flight(&path, at(t0) + quiet, quiet));
        // A reply under way — text or a thinking block — same window.
        for reply in [text.clone(), thinking.clone()] {
            write(&[prompt.clone(), call.clone(), result.clone(), reply]);
            assert!(turn_in_flight(&path, at(t0) + quiet - 1, quiet));
            assert!(!turn_in_flight(&path, at(t0) + quiet, quiet));
        }
        // A finished turn and an Esc are not in flight, however fresh.
        write(&[prompt.clone(), call.clone(), result.clone(), text.clone(), close]);
        assert!(!turn_in_flight(&path, at(t0), quiet));
        write(&[prompt, call, abort]);
        assert!(!turn_in_flight(&path, at(t0), quiet));
        let _ = std::fs::remove_file(&path);
    }

    /// T-659: the error row the failed turn closed on, in the hook set's
    /// words; a retry's `system` row is skipped, and an error behind a newer
    /// prompt, or a turn whose error is not written yet, says nothing.
    #[test]
    fn a_failed_turns_error_row_is_its_class_and_message() {
        let path = tmp("api-error");
        let line = |v: serde_json::Value| format!("{v}\n");
        let prompt = serde_json::json!({"uuid":"p", "type":"user", "message":{"content":"say hi"}});
        let retry = serde_json::json!({"uuid":"r", "type":"system", "subtype":"api_error",
            "level":"error", "error":{"status":401, "formatted":"401 API key is invalid."},
            "retryAttempt":1, "maxRetries":10});
        std::fs::write(&path, line(prompt.clone()) + &line(retry.clone())).unwrap();
        assert!(api_error(&path).is_none(), "not flushed: the prompt is the last word");
        let failed = error_row(
            "authentication_failed",
            "Failed to authenticate. API Error: 401 API key is invalid.",
        );
        let latch = serde_json::json!({"type":"last-prompt", "lastPrompt":"say hi"});
        std::fs::write(&path, line(prompt.clone()) + &line(retry) + &line(failed) + &line(latch))
            .unwrap();
        let error = api_error(&path).unwrap();
        assert_eq!(error.class, "authentication_failed");
        assert_eq!(
            error.text.as_deref(),
            Some("Failed to authenticate. API Error: 401 API key is invalid.")
        );
        // The next prompt opens a turn the old error does not speak for.
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(file, "{}", line(prompt)).unwrap();
        assert!(api_error(&path).is_none());
        // A reply that is not an error is the turn's last word.
        std::fs::write(
            &path,
            line(serde_json::json!({"uuid":"a", "type":"assistant",
            "message":{"stop_reason":"end_turn", "content":[{"type":"text","text":"hi"}]}})),
        )
        .unwrap();
        assert!(api_error(&path).is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
