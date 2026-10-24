//! Observe-tier transcript tailing (09 §4.3) — the cursor algorithm, exactly:
//! never seek backwards; keep the trailing fragment; a parse error skips one
//! record and continues (never resync); shrink (`len < offset`) means rotation
//! → reset to 0. State derived here is Tier-0: the caller applies it at
//! `Confidence::Low` only.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use mesimon_core::adopt::{
    classify_tail_record, record_ms, turn_edge, TailEvent, TailTool, ToolLedger, TurnEdge,
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

/// Did a person's prompt (`adopt::user_prompt`: a typed or pasted prompt, a
/// `!` command's input, never a tool result, an interrupt or the harness's
/// own words) reach this transcript at or after `since` (epoch ms)? A
/// restart asks it of a told hand (T-922): the prompt that answered it may
/// have come while no daemon listened, and its `UserPromptSubmit` is lost,
/// never queued. Reads the last `PROMPT_WINDOW` bytes, newest first, and
/// stops at the first dated record older than `since`; a prompt further
/// back than the window is not seen, and the hand is kept.
pub fn prompted_since(path: &Path, since: u64) -> bool {
    let Some(len) = std::fs::metadata(path).ok().map(|m| m.len()) else { return false };
    let Some(text) = window_text(path, len, PROMPT_WINDOW) else { return false };
    let end = text.len();
    for v in (TailRecords { text, end }) {
        let Some(at) = record_ms(&v) else { continue };
        if at < since {
            return false;
        }
        if mesimon_core::adopt::user_prompt(&v).is_some() {
            return true;
        }
    }
    false
}

/// A plan or question dialog the transcript says is still open: the
/// interaction tool's call, its `tool_use` id (the hook frames'
/// `tool_use_id`) and its input.
pub struct OpenDialog {
    pub tool: TailTool,
    pub id: String,
    pub input: serde_json::Value,
}

/// The plan or question dialog the transcript leaves open, if any (T-933):
/// a restart empties the dialogs the hook stream filled, and this is what
/// rebuilds one. Walks back from the end over latches, attachments, the
/// sibling calls of a parallel batch and their results; the dialog's call
/// with no `tool_result` of its id after it is open. Anything else that
/// speaks first (a prompt, an interrupt, text, the turn's close, its own
/// result) means the dialog is gone.
pub fn open_dialog(path: &Path) -> Option<OpenDialog> {
    use serde_json::Value;
    let mut answered = Vec::new();
    // Once per spell, so as far back as `prompted_since`: a long plan's call
    // is one record past the per-second readers' 64 KiB (T-936).
    let len = std::fs::metadata(path).ok()?.len();
    let text = window_text(path, len, PROMPT_WINDOW)?;
    let end = text.len();
    for v in (TailRecords { text, end }) {
        match classify_tail_record(&v) {
            TailEvent::Latch | TailEvent::ToolInFlight => continue,
            TailEvent::NeedsHuman { .. } => {
                let (tool, call) = dialog_block(&v)?;
                let id = call["id"].as_str()?;
                if answered.iter().any(|a| a == id) {
                    return None;
                }
                return Some(OpenDialog { tool, id: id.into(), input: call["input"].clone() });
            }
            TailEvent::Other => match v.get("type").and_then(Value::as_str) {
                Some("attachment") => continue,
                Some("system") if !matches!(turn_edge(&v), TurnEdge::Done(_)) => continue,
                Some("user") if carries_tool_result(&v) => {
                    let blocks = v["message"]["content"].as_array()?;
                    answered.extend(
                        blocks.iter().filter_map(|b| b["tool_use_id"].as_str()).map(String::from),
                    );
                }
                _ => return None,
            },
            _ => return None,
        }
    }
    None
}

/// The plan or question call `v` makes, if it makes one: the tool and the
/// call's `tool_use` id, the hook frames' `tool_use_id` (T-957).
pub fn dialog_call(v: &serde_json::Value) -> Option<(TailTool, &str)> {
    let (tool, call) = dialog_block(v)?;
    Some((tool, call["id"].as_str()?))
}

/// Whether `v` carries the result of the tool call `id`: the dialog's own
/// end, whatever closed it (T-957). Claude Code writes it at the keypress,
/// on 2.1.310 as on 2.1.287: `is_error` with "the user said: …" for a plan
/// sent back with words, `is_error` with "STOP what you are doing" and an
/// interrupt record after it for an Esc, the approval or the answers for an
/// accept. A sibling's result is another id and is not it (T-595).
pub fn carries_result_of(v: &serde_json::Value, id: &str) -> bool {
    v.get("type").and_then(serde_json::Value::as_str) == Some("user")
        && v["message"]["content"].as_array().is_some_and(|blocks| {
            blocks.iter().any(|b| {
                b["type"].as_str() == Some("tool_result") && b["tool_use_id"].as_str() == Some(id)
            })
        })
}

/// The interaction tool's `tool_use` block of an assistant record that
/// calls one (`classify_tail_record`'s `NeedsHuman`).
fn dialog_block(v: &serde_json::Value) -> Option<(TailTool, &serde_json::Value)> {
    let TailEvent::NeedsHuman { tool } = classify_tail_record(v) else { return None };
    let name = match tool {
        TailTool::AskUserQuestion => "AskUserQuestion",
        TailTool::ExitPlanMode => "ExitPlanMode",
    };
    let call = v["message"]["content"]
        .as_array()?
        .iter()
        .find(|b| b["type"].as_str() == Some("tool_use") && b["name"].as_str() == Some(name))?;
    Some((tool, call))
}

/// How far back `prompted_since` reads: once per told hand at a start, so
/// wider than the per-second readers' 64 KiB, which one turn's tool output
/// fills.
const PROMPT_WINDOW: u64 = 1024 * 1024;

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

/// The records of the last 64 KiB, NEWEST first, each parsed as it is
/// asked for (T-688): every reader above walks back from the end and stops
/// at the first record that decides, and this runs once a second per
/// Running pane past the quiet threshold, on the writer thread. The window
/// may open mid-record, so the first line of a truncated read is dropped; a
/// line that is not JSON is skipped (09 §4.3: never resync).
fn tail_records(path: &Path) -> Option<TailRecords> {
    let len = std::fs::metadata(path).ok()?.len();
    let text = window_text(path, len, 64 * 1024)?;
    let end = text.len();
    Some(TailRecords { text, end })
}

/// `tail_records`' walk: `end` is where the next line to try ends.
struct TailRecords {
    text: String,
    end: usize,
}

impl Iterator for TailRecords {
    type Item = serde_json::Value;
    fn next(&mut self) -> Option<Self::Item> {
        while self.end > 0 {
            let head = &self.text[..self.end];
            let start = head.rfind('\n').map_or(0, |i| i + 1);
            let line = &head[start..];
            self.end = start.saturating_sub(1);
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str(line) {
                return Some(v);
            }
        }
        None
    }
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
            let records: Vec<_> = records.collect();
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

    /// T-933: the dialog a restart rebuilds is the interaction tool's call
    /// with no result of its id after it; its sibling calls in a batch and
    /// their results do not close it, its own result or a later word does.
    #[test]
    fn an_open_dialog_is_a_call_with_no_result_after_it() {
        let path = tmp("open-dialog");
        let call = |uuid: &str, id: &str, name: &str, input: serde_json::Value| {
            serde_json::json!({"uuid": uuid, "type": "assistant", "message": {"content": [
                {"type": "tool_use", "id": id, "name": name, "input": input}]}})
        };
        let result = |uuid: &str, id: &str| {
            serde_json::json!({"uuid": uuid, "type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": id, "content": "ok"}]}})
        };
        let write = |records: &[serde_json::Value]| {
            let body: String = records.iter().map(|r| format!("{r}\n")).collect();
            std::fs::write(&path, body).unwrap();
        };
        let prompt = serde_json::json!({"uuid": "p", "type": "user", "message": {"content": "go"}});
        let plan = call("c", "toolu_plan", "ExitPlanMode", serde_json::json!({"plan": "1. a"}));
        let latch = serde_json::json!({"type": "last-prompt"});

        write(&[prompt.clone(), plan.clone(), latch.clone()]);
        let open = open_dialog(&path).expect("the plan is open");
        assert_eq!((open.tool, open.id.as_str()), (TailTool::ExitPlanMode, "toolu_plan"));
        assert_eq!(open.input["plan"], "1. a");

        // A batch: a sibling call and its result after the question.
        let ask = call("q", "toolu_q", "AskUserQuestion", serde_json::json!({"questions": []}));
        let bash = call("b", "toolu_b", "Bash", serde_json::json!({}));
        write(&[prompt.clone(), ask.clone(), bash.clone(), result("rb", "toolu_b")]);
        let open = open_dialog(&path).expect("the question stands beside its batch");
        assert_eq!((open.tool, open.id.as_str()), (TailTool::AskUserQuestion, "toolu_q"));

        // Answered in the restart's window: its own result, then the turn on.
        write(&[prompt.clone(), plan.clone(), result("rp", "toolu_plan")]);
        assert!(open_dialog(&path).is_none(), "its own result closes it");
        write(&[prompt.clone(), plan.clone(), result("rp", "toolu_plan"), bash.clone()]);
        assert!(open_dialog(&path).is_none(), "a call after its result");
        let esc = serde_json::json!({"uuid": "x", "type": "user",
            "message": {"content": "[Request interrupted by user]"}});
        write(&[prompt.clone(), plan.clone(), esc]);
        assert!(open_dialog(&path).is_none(), "an Esc out of it");
        write(&[plan, prompt]);
        assert!(open_dialog(&path).is_none(), "a newer prompt");
        write(&[serde_json::json!({"uuid": "t", "type": "assistant",
            "message": {"content": [{"type": "text", "text": "done"}]}})]);
        assert!(open_dialog(&path).is_none(), "no dialog at all");
        assert!(open_dialog(std::path::Path::new("/nonexistent/x.jsonl")).is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
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

    /// T-688: the walk is newest first, skips blank and non-JSON lines, and
    /// drops the torn first line of a window that opened mid-record.
    #[test]
    fn tail_records_walk_newest_first_and_drop_the_torn_head() {
        let p = tmp("walk");
        std::fs::write(&p, "{\"n\":1}\n\n{\"n\":2}\r\nnot json\n{\"n\":3}\n").unwrap();
        let seen: Vec<u64> = tail_records(&p).unwrap().map(|v| v["n"].as_u64().unwrap()).collect();
        assert_eq!(seen, vec![3, 2, 1]);
        // A window shorter than the file opens mid-record: that line goes.
        let text = window_text(&p, std::fs::metadata(&p).unwrap().len(), 14).unwrap();
        assert_eq!(text, "{\"n\":3}\n", "the torn `not json` head is gone");
        let mut torn = TailRecords { end: text.len(), text };
        assert_eq!(torn.next().unwrap()["n"], 3);
        assert!(torn.next().is_none());
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
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
    fn prompted_since_reads_a_persons_prompt_and_nothing_else() {
        let p = tmp("prompted");
        // The hand's turn: a prompt, a tool's result, the close.
        std::fs::write(
            &p,
            "{\"uuid\":\"u0\",\"type\":\"user\",\"timestamp\":\"2026-09-05T16:50:00.000Z\",\"message\":{\"content\":\"go\"}}\n\
             {\"uuid\":\"u1\",\"type\":\"user\",\"timestamp\":\"2026-09-05T16:51:00.000Z\",\"message\":{\"content\":[{\"type\":\"tool_result\"}]}}\n\
             {\"uuid\":\"u2\",\"type\":\"assistant\",\"timestamp\":\"2026-09-05T16:52:00.000Z\",\"message\":{\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n\
             {\"type\":\"mode\"}\n",
        )
        .unwrap();
        let go = 1_788_627_000_000;
        assert!(prompted_since(&p, go), "the prompt at the stamp itself");
        assert!(!prompted_since(&p, go + 1), "a tool's result and a close are no prompt");
        // An interrupt and the harness's wake-up are no person's words either.
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{{\"uuid\":\"u3\",\"type\":\"user\",\"timestamp\":\"2026-09-05T16:53:00.000Z\",\"message\":{{\"content\":\"[Request interrupted by user]\"}}}}").unwrap();
        writeln!(f, "{{\"uuid\":\"u4\",\"type\":\"user\",\"timestamp\":\"2026-09-05T16:54:00.000Z\",\"message\":{{\"content\":\"<task-notification><task-id>x</task-id>\"}}}}").unwrap();
        assert!(!prompted_since(&p, go + 1));
        // The next turn's prompt.
        writeln!(f, "{{\"uuid\":\"u5\",\"type\":\"user\",\"timestamp\":\"2026-09-05T16:55:00.000Z\",\"message\":{{\"content\":\"staging\"}}}}").unwrap();
        assert!(prompted_since(&p, go + 1));
        assert!(!prompted_since(&p, go + 300_001), "a prompt older than the stamp");
        assert!(!prompted_since(Path::new("/nonexistent/x.jsonl"), 0));
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

    /// T-957: the plan's own result, as Claude Code 2.1.310 wrote it when a
    /// plan was sent back with words (the board's transcript, 19:10:31,
    /// trimmed), is that call's end and no other's; the call's record names
    /// its tool and id, and a record calling only an ordinary tool is none.
    #[test]
    fn a_dialog_s_call_and_its_own_result_are_read_by_id() {
        let rejected = serde_json::json!({"parentUuid":"e313", "isSidechain":false,
            "type":"user", "message":{"role":"user", "content":[{"type":"tool_result",
                "content":"The user doesn't want to proceed with this tool use. The tool use was \
                    rejected (eg. if it was a file edit, the new_string was NOT written to the \
                    file). To tell you how to proceed, the user said: Name the file b.txt.",
                "is_error":true, "tool_use_id":"toolu_01R3iFiXyHgSp58LBwYrzMwP"}]},
            "uuid":"7196", "timestamp":"2026-10-24T16:10:31.528Z",
            "toolDenialKind":"user-rejected", "userFeedback":"Name the file b.txt.",
            "permissionDecision":{"decision":"reject", "source":"user_reject"}});
        assert!(carries_result_of(&rejected, "toolu_01R3iFiXyHgSp58LBwYrzMwP"));
        assert!(!carries_result_of(&rejected, "toolu_sibling"));
        let call = |name: &str| {
            serde_json::json!({"uuid":"c", "type":"assistant", "message":{"content":[
                {"type":"tool_use","id":"toolu_1","name":name,"input":{"plan":"# Plan"}}]}})
        };
        assert!(matches!(
            dialog_call(&call("ExitPlanMode")),
            Some((TailTool::ExitPlanMode, "toolu_1"))
        ));
        assert!(matches!(
            dialog_call(&call("AskUserQuestion")),
            Some((TailTool::AskUserQuestion, "toolu_1"))
        ));
        assert!(dialog_call(&call("Write")).is_none());
        // A tool's call is not its result.
        assert!(!carries_result_of(&call("ExitPlanMode"), "toolu_1"));
    }
}
