//! agy's transcript (T-1237): the card's words, the phone's rows. Measured
//! on agy 1.5.9.
//!
//! The file is the hooks' `transcriptPath`, `brain/<conversation>/
//! .system_generated/logs/transcript_full.jsonl` under agy's home, one line
//! per step: `step_index`, `source` (`USER_EXPLICIT`, `MODEL`, `SYSTEM`,
//! `SYSTEM_SDK`), `type` (`USER_INPUT`, `PLANNER_RESPONSE`, `GENERIC`,
//! `SYSTEM_MESSAGE`), `status`, `created_at`, `content`, and on a model's
//! reply `model`, `tool_calls [{name, args}]`, `thinking` and its tokens. A
//! person's step wraps their words in `<USER_REQUEST>`, with agy's own
//! metadata after them. `GENERIC` is a tool's output; `thinking` is the
//! model's own reasoning, and neither is shown.

use std::path::Path;

use mesimon_core::mesophon::{RowKind, TranscriptRow};
use serde_json::Value;

use crate::agents::{AgentActivity, AgentPreview};

/// Back from the end of the file; a reply further back than the second
/// window is past a run of tool output no card shows.
const TAIL_BYTES: u64 = 64 * 1024;
const TAIL_BYTES_MAX: u64 = 1024 * 1024;

/// The person's own words in a `USER_INPUT` step: what `<USER_REQUEST>`
/// holds, or the whole content where agy wrapped nothing.
pub(super) fn request(content: &str) -> Option<String> {
    let words = match content.split_once("<USER_REQUEST>") {
        Some((_, rest)) => rest.split_once("</USER_REQUEST>").map_or(rest, |(inner, _)| inner),
        None if content.trim_start().starts_with('<') => return None,
        None => content,
    };
    let words = words.trim();
    (!words.is_empty()).then(|| words.to_string())
}

fn is_step(v: &Value, source: &str, kind: &str) -> bool {
    v["source"].as_str() == Some(source) && v["type"].as_str() == Some(kind)
}

fn prompt_of(v: &Value) -> Option<String> {
    is_step(v, "USER_EXPLICIT", "USER_INPUT").then(|| request(v["content"].as_str()?))?
}

/// A model reply's words: none for a step that only calls tools.
fn reply_of(v: &Value) -> Option<&str> {
    if !is_step(v, "MODEL", "PLANNER_RESPONSE") {
        return None;
    }
    v["content"].as_str().map(str::trim).filter(|t| !t.is_empty())
}

fn calls_of(v: &Value) -> &[Value] {
    if !is_step(v, "MODEL", "PLANNER_RESPONSE") {
        return &[];
    }
    v["tool_calls"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|l| !l.is_empty())
}

/// A board tool reached through agy's `call_mcp_tool`: its own name and
/// arguments, so `raise_hand` reads as Claude's and Codex's do.
fn mcp_call(call: &Value) -> Option<(&str, &Value)> {
    (call["name"].as_str()? == "call_mcp_tool").then_some(())?;
    Some((call["args"]["ToolName"].as_str()?, &call["args"]["Arguments"]))
}

/// One tool call on one line: the summary agy's model wrote for it, else
/// the tool and what it is pointed at, a path by its leaf.
pub(super) fn tool_label(call: &Value) -> String {
    let args = &call["args"];
    let field = |k: &str| args[k].as_str().and_then(first_line);
    if let Some(summary) = field("toolSummary") {
        return summary.to_string();
    }
    if let Some((tool, _)) = mcp_call(call) {
        return tool.to_string();
    }
    let name = call["name"].as_str().unwrap_or("tool");
    for key in ["Path", "AbsolutePath", "TargetFile", "File"] {
        if let Some(p) = field(key) {
            return format!("{name} {}", p.rsplit('/').next().unwrap_or(p));
        }
    }
    let what = field("CommandLine")
        .or_else(|| field("Query"))
        .or_else(|| args.as_object().and_then(|o| o.values().find_map(Value::as_str)));
    what.and_then(first_line).map_or(name.to_string(), |w| format!("{name} {w}"))
}

fn step_ms(v: &Value) -> Option<u64> {
    v["created_at"].as_str().and_then(mesimon_core::adopt::iso_ms)
}

/// A step's key for the spoke mark: its place and its time, which a later
/// step never repeats in one conversation.
fn step_key(v: &Value) -> u64 {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(v["step_index"].as_u64().unwrap_or(0).to_le_bytes());
    hash.update(v["created_at"].as_str().unwrap_or_default().as_bytes());
    u64::from_le_bytes(hash.finalize()[..8].try_into().expect("hash length"))
}

#[derive(Default)]
struct Tail {
    reply: Option<String>,
    prompt: Option<String>,
    key: Option<u64>,
    said_ms: Option<u64>,
    activity: Option<AgentActivity>,
}

/// Walk `text`'s steps from the newest: the newest tool call is what the
/// agent is doing, and the walk ends at its newest words or the person's
/// prompt, whichever is later, as Claude's peek does.
fn walk(text: &str) -> Tail {
    let mut tail = Tail::default();
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if tail.activity.is_none() {
            if let Some(call) = calls_of(&v).last() {
                tail.activity = Some(AgentActivity::Tool(tool_label(call)));
            }
        }
        if let Some(words) = reply_of(&v) {
            tail.reply = Some(words.to_string());
            tail.key = Some(step_key(&v));
            tail.said_ms = step_ms(&v);
            return tail;
        }
        if let Some(words) = prompt_of(&v) {
            tail.prompt = Some(words);
            tail.said_ms = step_ms(&v);
            tail.activity.get_or_insert(AgentActivity::Thinking);
            return tail;
        }
    }
    tail
}

/// The card's words: the newest reply, else the person's prompt marked as
/// theirs. `None` when the file cannot be read.
pub(super) fn preview(path: &Path) -> Option<AgentPreview> {
    let len = std::fs::metadata(path).ok()?.len();
    let mut tail = Tail::default();
    for window in [TAIL_BYTES, TAIL_BYTES_MAX] {
        let text = crate::agents::claude::tail::window_text(path, len, window)?;
        tail = walk(&text);
        if tail.reply.is_some() || tail.prompt.is_some() || window >= len {
            break;
        }
    }
    Some(AgentPreview {
        text: tail.reply.or_else(|| tail.prompt.map(|p| format!("> {p}"))),
        activity: tail.activity,
        reply_key: tail.key,
        said_ms: tail.said_ms,
    })
}

/// Chat folds a "Write <file>" label into its count of written files. A
/// model can give that summary to a command as well as a file tool.
fn write_label(call: &Value, label: &str) -> bool {
    let name = label.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
        .next();
    matches!(call["name"].as_str(), Some("write_to_file" | "write_file")) || name == Some("Write")
}

/// agy gives each call in a planner step its own following GENERIC step.
/// Emit a write at its successful result's offset, so an incremental page
/// beginning after the call still adds it exactly once. Use step indices
/// because interrupted calls can leave holes in the transcript.
fn completed_write(path: &Path, at: u64, result: &Value) -> Option<TranscriptRow> {
    let result_index = result["step_index"].as_u64()?;
    for window in [4096, TAIL_BYTES, TAIL_BYTES_MAX] {
        let text = crate::agents::claude::tail::window_text(path, at, window)?;
        for line in text.lines().rev() {
            let step: Value = serde_json::from_str(line).ok()?;
            if is_step(&step, "MODEL", "GENERIC") {
                continue;
            }
            let index = result_index.checked_sub(step["step_index"].as_u64()?)?.checked_sub(1)?;
            let call = calls_of(&step).get(usize::try_from(index).ok()?)?;
            let label = tool_label(call);
            return if write_label(call, &label) {
                TranscriptRow::tool(at, &label, step_ms(result))
            } else {
                None
            };
        }
        if window >= at {
            break;
        }
    }
    None
}

/// What one step shows on a phone (T-626): the person's words, the
/// agent's, and a line per tool call. Tool output, injected system
/// messages and the model's reasoning are not rows.
pub(super) fn rows(path: &Path, at: u64, v: &Value) -> Vec<TranscriptRow> {
    if is_step(v, "MODEL", "GENERIC") && v["status"] == "DONE" {
        return completed_write(path, at, v).into_iter().collect();
    }
    let ms = step_ms(v);
    if let Some(words) = prompt_of(v) {
        return TranscriptRow::new(at, RowKind::Prompt, &words, ms).into_iter().collect();
    }
    let mut out: Vec<TranscriptRow> = Vec::new();
    if let Some(words) = reply_of(v) {
        out.extend(TranscriptRow::new(at, RowKind::Reply, words, ms));
    }
    for call in calls_of(v) {
        if let Some((tool, args)) = mcp_call(call) {
            if let Some(reason) =
                args["reason"].as_str().filter(|_| mesimon_core::mesophon::is_raise_hand(tool))
            {
                out.extend(TranscriptRow::hand(at, tool, reason, ms));
                continue;
            }
        }
        let label = tool_label(call);
        if !write_label(call, &label) {
            out.extend(TranscriptRow::tool(at, &label, ms));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorded on agy 1.5.9, scrubbed: a ticket's brief arriving, a schema
    /// read and a board tool call, and the reply.
    const TOOLS: &str = include_str!("fixtures/transcript-tools.jsonl");
    /// A shell command and the reply after it.
    const SHELL: &str = include_str!("fixtures/transcript-shell.jsonl");
    /// Six prompts and their replies, an injected notice among them.
    const TURNS: &str = include_str!("fixtures/transcript-turns.jsonl");

    fn file(dir: &Path, body: &str) -> std::path::PathBuf {
        let p = dir.join("transcript_full.jsonl");
        std::fs::write(&p, body).unwrap();
        p
    }

    fn phone_rows(body: &str) -> Vec<TranscriptRow> {
        let dir = tempfile::tempdir().unwrap();
        crate::agents::read_transcript(
            mesimon_core::board::SessionKind::Agy,
            &file(dir.path(), body),
            Default::default(),
        )
        .unwrap()
        .rows
    }

    #[test]
    fn the_card_reads_the_newest_reply_and_what_the_agent_does() {
        let dir = tempfile::tempdir().unwrap();
        let p = preview(&file(dir.path(), TOOLS)).unwrap();
        assert_eq!(p.text.as_deref(), Some("PERIWINKLE-98E18D1C"));
        assert_eq!(p.activity, None, "the reply is the newest step: nothing runs after it");
        assert_eq!(p.said_ms, mesimon_core::adopt::iso_ms("2026-11-14T13:56:21Z"));
        assert!(p.reply_key.is_some());

        // Cut after the board tool's call: the prompt is the newest words,
        // and the call is what the agent is on.
        let cut: Vec<&str> = TOOLS.lines().take(5).collect();
        let p = preview(&file(dir.path(), &(cut.join("\n") + "\n"))).unwrap();
        assert_eq!(p.text.as_deref(), Some("> Answer the question on this ticket"));
        assert_eq!(p.activity, Some(AgentActivity::Tool("get_ticket".into())));
        assert_eq!(p.reply_key, None);

        // A shell command reads by the summary agy's model gave it.
        let cut: Vec<&str> = SHELL.lines().take(2).collect();
        let p = preview(&file(dir.path(), &(cut.join("\n") + "\n"))).unwrap();
        assert_eq!(p.activity, Some(AgentActivity::Tool("Echo hi".into())));
        assert_eq!(preview(&file(dir.path(), SHELL)).unwrap().text.as_deref(), Some("DONE"));

        // The prompt alone: thinking.
        let first = TURNS.lines().next().unwrap();
        let p = preview(&file(dir.path(), &format!("{first}\n"))).unwrap();
        assert_eq!(p.text.as_deref(), Some("> Say only the word PONG."));
        assert_eq!(p.activity, Some(AgentActivity::Thinking));
    }

    #[test]
    fn a_newer_reply_moves_the_spoke_mark_and_an_injected_message_is_no_words() {
        let dir = tempfile::tempdir().unwrap();
        let all: Vec<&str> = TURNS.lines().collect();
        let at = |n: usize| preview(&file(dir.path(), &(all[..n].join("\n") + "\n"))).unwrap();
        let (one, two) = (at(2), at(4));
        assert_eq!((one.text.as_deref(), two.text.as_deref()), (Some("PONG"), Some("PING")));
        assert_ne!(one.reply_key, two.reply_key);
        // Step 11 is agy's own notice, after the last prompt: the prompt
        // still reads as the newest words.
        assert_eq!(
            at(12).text.as_deref(),
            Some("> What word did you say first? Answer with that word alone.")
        );
        assert_eq!(at(13).text.as_deref(), Some("PONG"));
    }

    #[test]
    fn the_phone_reads_prompts_replies_and_one_line_per_tool_call() {
        let got: Vec<(RowKind, String)> = phone_rows(TOOLS)
            .into_iter()
            .map(|r| (r.kind, r.text))
            .collect();
        assert_eq!(
            got,
            vec![
                (RowKind::Prompt, "Answer the question on this ticket".into()),
                (RowKind::Tool, "read_file get_ticket.json".into()),
                (RowKind::Tool, "get_ticket".into()),
                (RowKind::Reply, "PERIWINKLE-98E18D1C".into()),
            ],
            "no brief, no tool output, no reasoning"
        );
        let shell: Vec<RowKind> =
            phone_rows(SHELL).into_iter().map(|r| r.kind).collect();
        assert_eq!(shell, vec![RowKind::Prompt, RowKind::Tool, RowKind::Reply]);
        for r in phone_rows(TURNS) {
            assert!(!r.text.contains("ADDITIONAL_METADATA"), "{}", r.text);
            assert!(!r.text.contains("SYSTEM_MESSAGE"), "{}", r.text);
        }
    }

    #[test]
    fn chat_counts_a_write_only_on_its_successful_result_even_on_a_later_page() {
        use crate::agents::{read_transcript, transcript::Ask};
        use mesimon_core::board::SessionKind;
        let dir = tempfile::tempdir().unwrap();
        let calls = serde_json::json!({
            "step_index": 1, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE",
            "tool_calls": [
                {"name": "run_command", "args": {"toolSummary": "Write approved.txt"}},
                {"name": "run_command", "args": {"toolSummary": "Write denied.txt"}}
            ]
        });
        let result = |index, status: &str| serde_json::json!({
            "step_index": index, "source": "MODEL", "type": "GENERIC", "status": status
        });
        let p = file(dir.path(), &format!("{calls}\n"));
        let page = |after| read_transcript(SessionKind::Agy, &p, Ask { after, ..Ask::default() }).unwrap();
        let pending = page(None);
        assert!(pending.rows.is_empty());
        file(dir.path(), &format!("{calls}\n{}\n{}\n", result(2, "DONE"), result(3, "ERROR")));
        for after in [None, Some(pending.end)] {
            let rows = page(after).rows;
            assert_eq!(rows.len(), 1);
            assert_eq!((rows[0].at, rows[0].text.as_str()), (pending.end, "Write approved.txt"));
        }
    }

    #[test]
    fn a_raised_hand_is_its_step_and_its_reason() {
        let v = serde_json::json!({
            "step_index": 4, "source": "MODEL", "type": "PLANNER_RESPONSE",
            "created_at": "2026-11-14T13:56:18Z",
            "tool_calls": [{"name": "call_mcp_tool", "args": {
                "ServerName": "mesimon", "ToolName": "raise_hand",
                "Arguments": {"reason": "Which colour?"}}}],
        });
        let got: Vec<(RowKind, String)> =
            phone_rows(&format!("{v}\n")).into_iter().map(|r| (r.kind, r.text)).collect();
        assert_eq!(
            got,
            vec![(RowKind::Tool, "raise_hand".into()), (RowKind::Reply, "Which colour?".into())]
        );
    }

    #[test]
    fn the_request_is_the_persons_words_without_agys_metadata() {
        assert_eq!(
            request("<USER_REQUEST>\nfix it\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nt\n</ADDITIONAL_METADATA>")
                .as_deref(),
            Some("fix it")
        );
        assert_eq!(request("plain").as_deref(), Some("plain"));
        assert_eq!(request("<ADDITIONAL_METADATA>x</ADDITIONAL_METADATA>"), None);
        assert_eq!(request("<USER_REQUEST>\n  \n</USER_REQUEST>"), None);
    }
}
