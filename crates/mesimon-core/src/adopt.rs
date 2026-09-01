//! Pure parsers for session adoption (19 §4) — no I/O; the daemon's census
//! reads files and feeds strings/values here.
//!
//! D24 rules encoded below: identity comes from the `sessionId` FIELD inside
//! transcript records (camelCase — `session_id` appears in only 82% of
//! corpora), and repo membership from the `cwd` field content. The projects/
//! directory slug is never derived or parsed — a hand-rolled slugifier is
//! right ~95% of the time, the worst possible hit rate (09 §4.1).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

/// Identity read from a transcript's first records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptHead {
    pub session_id: uuid::Uuid,
    pub cwd: String,
}

/// Scan the head of a transcript (a chunk of newline-delimited JSON) for the
/// first `sessionId` + `cwd` pair. Unparseable lines are skipped, never fatal
/// (09 §4.3: skip-and-continue). The last line may be truncated by the caller's
/// fixed-size read — a parse failure there is expected and harmless.
pub fn parse_transcript_head(head: &str) -> Option<TranscriptHead> {
    let mut session_id: Option<uuid::Uuid> = None;
    let mut cwd: Option<String> = None;
    for line in head.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if session_id.is_none() {
            session_id = v.get("sessionId").and_then(Value::as_str).and_then(|s| s.parse().ok());
        }
        if cwd.is_none() {
            cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
        }
        if let (Some(session_id), Some(cwd)) = (session_id, cwd.clone()) {
            return Some(TranscriptHead { session_id, cwd });
        }
    }
    None
}

/// Does a session's `cwd` place it in this repo? `roots` = the main checkout
/// plus worktree roots, pre-canonicalized by the caller. Component-wise prefix
/// match: a session running in a subdirectory of the repo belongs to it;
/// `/repo-other` does not match `/repo`.
pub fn cwd_matches(cwd: &str, roots: &[PathBuf]) -> bool {
    let cwd = Path::new(cwd);
    roots.iter().any(|root| cwd.starts_with(root))
}

/// `~/.claude/sessions/<pid>.json` — best-effort enrichment only (11 §11.3).
/// MAY contribute a display name and a running-elsewhere hint; MUST NOT set
/// any §11.7 state, attention entry, or liveness verdict. Join key is
/// `session_id` (the file's `sessionId`), never the filename pid. Every key
/// is optional (measured presence varies file to file); unknown keys pass.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsPidFile {
    pub session_id: Option<uuid::Uuid>,
    pub cwd: Option<String>,
    /// `idle` | `busy` — the only observed values; kept as raw text.
    pub status: Option<String>,
    pub pid: Option<i32>,
    pub name: Option<String>,
    pub tmux: Option<String>,
}

/// What one complete transcript record means for the observe tier (09 §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailEvent {
    /// No `uuid` field: a last-write-wins state latch (25.4% of records).
    /// Emit nothing — surfacing these as activity is the classic tail bug.
    Latch,
    /// An assistant turn produced text; carries the last text block (preview).
    AssistantText { text: String },
    /// The assistant called a tool that needs a human.
    NeedsHuman { tool: TailTool },
    /// `system`/`turn_duration`: the turn finished.
    TurnComplete,
    /// The stream was cut mid-turn.
    Aborted,
    /// A record with identity but nothing the observe tier can use.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailTool {
    AskUserQuestion,
    ExitPlanMode,
}

pub fn classify_tail_record(v: &Value) -> TailEvent {
    // The latch rule is absence-of-uuid, never a type allowlist — that is
    // what survived `cost-state`/`pr-link` appearing mid-corpus (09 §4.2).
    if v.get("uuid").is_none() {
        return TailEvent::Latch;
    }
    // Three spellings of an interrupt: the two mid-stream flags, and the Esc
    // press itself, which current Claude Code records as a `user` record
    // carrying `interruptedMessageId` ("[Request interrupted by user]";
    // verified live 2026-08-30 — spike S-E's "the transcript may get no
    // record" does not hold on current builds).
    if v.get("isAbortedMidStream").and_then(Value::as_bool) == Some(true)
        || v.get("interruptedByShutdown").and_then(Value::as_bool) == Some(true)
        || v.get("interruptedMessageId").is_some_and(|x| !x.is_null())
    {
        return TailEvent::Aborted;
    }
    match v.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            let blocks = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for b in blocks {
                if b.get("type").and_then(Value::as_str) == Some("tool_use") {
                    match b.get("name").and_then(Value::as_str) {
                        Some("AskUserQuestion") => {
                            return TailEvent::NeedsHuman { tool: TailTool::AskUserQuestion }
                        }
                        Some("ExitPlanMode") => {
                            return TailEvent::NeedsHuman { tool: TailTool::ExitPlanMode }
                        }
                        _ => {}
                    }
                }
            }
            let text = blocks
                .iter()
                .rev()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .find_map(|b| b.get("text").and_then(Value::as_str));
            match text {
                Some(t) => TailEvent::AssistantText { text: t.to_string() },
                None => TailEvent::Other,
            }
        }
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("turn_duration") => {
            TailEvent::TurnComplete
        }
        _ => TailEvent::Other,
    }
}

/// The label a peek row shows for a tool call: the caller's own title where
/// the tool carries one (`description` — Bash, Agent, Artifact), else the
/// tool's name plus its target (a path's last component, a pattern, a query,
/// a url, the command's first line). `None` for a record with no `tool_use`
/// block. Parallel calls in one record share one row — the first is as good
/// a summary as any, and the row is a hint, not a ledger.
pub fn tool_activity(v: &Value) -> Option<String> {
    let blocks = v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array)?;
    let b = blocks.iter().find(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))?;
    let input = b.get("input");
    let field = |key: &str| {
        input
            .and_then(|i| i.get(key))
            .and_then(Value::as_str)
            .and_then(|t| t.lines().find(|l| !l.trim().is_empty()))
            .map(str::trim)
            .filter(|t| !t.is_empty())
    };
    if let Some(d) = field("description") {
        return Some(d.to_string());
    }
    let name = b.get("name").and_then(Value::as_str).unwrap_or("tool");
    // A path shows its last component only: the card has ~40 cells and the
    // directory is the least distinguishing part of a repo-relative path.
    if let Some(p) = field("file_path").or_else(|| field("notebook_path")) {
        return Some(format!("{name} {}", p.rsplit('/').next().unwrap_or(p)));
    }
    match ["pattern", "query", "url", "skill", "command"].iter().find_map(|k| field(k)) {
        Some(t) => Some(format!("{name} {t}")),
        None => Some(name.to_string()),
    }
}

/// The user's own words, when this record is one of THEIR messages. Measured
/// over 25 local corpora (2026-08-31): 862 of every 940 `user` records are
/// tool results, and the rest divide into plain-string prompts (the real
/// thing), `isMeta` injections (`<local-command-caveat>`, a skill's preamble)
/// and the Esc interrupt's `[Request interrupted by user]`. Only the first is
/// a prompt. The array form is accepted too — an attachment rides alongside
/// the text — as long as no `tool_result` block is in it.
///
/// Why the record and not the `last-prompt` latch: the latch is written a
/// turn late (verified in a live transcript — the latch for the message being
/// worked on lands mid-tool-run, after the agent has already answered), so
/// only the record's position says when the user actually spoke.
///
/// One plain-string record is the harness's, not the user's, and carries no
/// flag saying so: the `<task-notification>` that wakes a turn parked on a
/// background task (`isMeta` false; 22 of 22 in the local corpus). Reading it
/// as a prompt put `> <task-notification><task-id>…` on a card and called the
/// agent "thinking" under it (dogfood 2026-09-02). It is skipped by its tag,
/// so the walk continues to the agent's real last words.
pub fn user_prompt(v: &Value) -> Option<String> {
    if v.get("type").and_then(Value::as_str) != Some("user")
        || v.get("uuid").is_none()
        || v.get("toolUseResult").is_some()
        || v.get("isMeta").and_then(Value::as_bool) == Some(true)
        || v.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || v.get("interruptedMessageId").is_some_and(|x| !x.is_null())
    {
        return None;
    }
    let content = v.get("message")?.get("content")?;
    let text = match content.as_str() {
        Some(s) => s,
        None => {
            let blocks = content.as_array()?;
            if blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")) {
                return None;
            }
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .find_map(|b| b.get("text").and_then(Value::as_str))?
        }
    };
    let text = text.trim();
    if text.is_empty() || text.starts_with(TASK_NOTIFICATION_TAG) {
        return None;
    }
    Some(text.to_string())
}

/// The opening tag of the harness's background-task wake-up, written as a
/// plain `user` record with no `isMeta`.
const TASK_NOTIFICATION_TAG: &str = "<task-notification>";

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b";

    #[test]
    fn head_parses_and_skips_garbage() {
        let head = format!(
            "not json at all\n{{\"cwd\":\"/repo\"}}\n{{\"sessionId\":\"{SID}\",\"type\":\"user\"}}\ntrunc"
        );
        let h = parse_transcript_head(&head).expect("head");
        assert_eq!(h.session_id.to_string(), SID);
        assert_eq!(h.cwd, "/repo");
        assert!(parse_transcript_head("{}\n{\"foo\":1}").is_none());
    }

    #[test]
    fn cwd_matches_is_component_wise() {
        let roots = vec![PathBuf::from("/Users/a/repo"), PathBuf::from("/tmp/wt1")];
        assert!(cwd_matches("/Users/a/repo", &roots));
        assert!(cwd_matches("/Users/a/repo/crates/x", &roots));
        assert!(cwd_matches("/tmp/wt1", &roots));
        assert!(!cwd_matches("/Users/a/repo-other", &roots));
        assert!(!cwd_matches("/Users/a", &roots));
    }

    #[test]
    fn pid_file_tolerates_partial_and_unknown_keys() {
        let f: SessionsPidFile = serde_json::from_str(&format!(
            "{{\"sessionId\":\"{SID}\",\"pid\":123,\"status\":\"busy\",\"pidDomain\":\"x\",\"weird\":[1]}}"
        ))
        .unwrap();
        assert_eq!(f.session_id.unwrap().to_string(), SID);
        assert_eq!(f.pid, Some(123));
        assert!(f.name.is_none());
        let empty: SessionsPidFile = serde_json::from_str("{}").unwrap();
        assert!(empty.session_id.is_none());
    }

    fn val(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn tool_activity_prefers_the_caller_title_then_the_target() {
        // The description IS the title the agent wrote for the step.
        let v = val(r#"{"uuid":"u1","type":"assistant","message":{"content":[
                {"type":"text","text":"now the counts"},
                {"type":"tool_use","name":"Bash","input":{"command":"wc -l x","description":"Count lines"}}]}}"#);
        assert_eq!(tool_activity(&v).as_deref(), Some("Count lines"));

        // No description: the tool and what it is pointed at, path by leaf.
        let v = val(r#"{"uuid":"u2","type":"assistant","message":{"content":[
                {"type":"tool_use","name":"Read","input":{"file_path":"/a/b/peek.rs","offset":9}}]}}"#);
        assert_eq!(tool_activity(&v).as_deref(), Some("Read peek.rs"));

        let v = val(r#"{"uuid":"u3","type":"assistant","message":{"content":[
                {"type":"tool_use","name":"Grep","input":{"pattern":"transcript","path":"crates"}}]}}"#);
        assert_eq!(tool_activity(&v).as_deref(), Some("Grep transcript"));

        // A heredoc command contributes its first line, never the body.
        let v = val(r#"{"uuid":"u4","type":"assistant","message":{"content":[
                {"type":"tool_use","name":"Bash","input":{"command":"python3 - <<PY\nimport os\nPY"}}]}}"#);
        assert_eq!(tool_activity(&v).as_deref(), Some("Bash python3 - <<PY"));

        // Nothing recognizable still names the tool; a textless turn is None.
        let v = val(r#"{"uuid":"u5","type":"assistant","message":{"content":[
                {"type":"tool_use","name":"StructuredOutput","input":{"findings":[]}}]}}"#);
        assert_eq!(tool_activity(&v).as_deref(), Some("StructuredOutput"));
        let v = val(r#"{"uuid":"u6","type":"assistant","message":{"content":[
                {"type":"text","text":"done"}]}}"#);
        assert_eq!(tool_activity(&v), None);
        assert_eq!(tool_activity(&val(r#"{"uuid":"u7","type":"user","message":{}}"#)), None);
    }

    #[test]
    fn user_prompt_is_only_the_users_own_message() {
        let v = val(
            r#"{"uuid":"u1","type":"user","message":{"role":"user","content":"enter didn't register"}}"#,
        );
        assert_eq!(user_prompt(&v).as_deref(), Some("enter didn't register"));

        // A prompt that carried an attachment keeps its text.
        let v = val(r#"{"uuid":"u2","type":"user","message":{"content":[
                {"type":"text","text":"look at this"},{"type":"image"}]}}"#);
        assert_eq!(user_prompt(&v).as_deref(), Some("look at this"));

        // The three impostors: a tool result, an injection, the Esc record.
        let v = val(r#"{"uuid":"u3","type":"user","toolUseResult":{"ok":1},"message":{"content":[
                {"type":"tool_result","content":"out"}]}}"#);
        assert_eq!(user_prompt(&v), None);
        let v = val(
            r#"{"uuid":"u4","type":"user","isMeta":true,"message":{"content":"<local-command-caveat>x"}}"#,
        );
        assert_eq!(user_prompt(&v), None);
        let v = val(
            r#"{"uuid":"u5","type":"user","interruptedMessageId":"msg_1","message":{"content":[
                {"type":"text","text":"[Request interrupted by user]"}]}}"#,
        );
        assert_eq!(user_prompt(&v), None);

        // The background-task wake-up is the harness speaking, in a plain
        // string record with no flag — only its tag says so.
        let v = val(
            r#"{"uuid":"u8","type":"user","message":{"content":"<task-notification>\n<task-id>aac805ad</task-id>\n<status>completed</status>\n</task-notification>"}}"#,
        );
        assert_eq!(user_prompt(&v), None);
        let v = val(r#"{"uuid":"u9","type":"user","message":{"content":[
                {"type":"text","text":"  <task-notification><task-id>x</task-id></task-notification>"}]}}"#);
        assert_eq!(user_prompt(&v), None);

        // A subagent's prompt is not the user speaking, and neither is a
        // latch or an empty message.
        let v = val(
            r#"{"uuid":"u6","type":"user","isSidechain":true,"message":{"content":"do the thing"}}"#,
        );
        assert_eq!(user_prompt(&v), None);
        assert_eq!(user_prompt(&val(r#"{"type":"user","message":{"content":"x"}}"#)), None);
        assert_eq!(
            user_prompt(&val(r#"{"uuid":"u7","type":"user","message":{"content":"  "}}"#)),
            None
        );
    }

    #[test]
    fn uuidless_records_are_latches() {
        // A naive last-N-lines tail surfaces one of these 1 time in 4 (09 §4.2).
        let v = val(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::Latch);
        let v = val(r#"{"mode":"plan"}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::Latch);
    }

    #[test]
    fn classifier_maps_the_detection_table() {
        let v = val(r#"{"uuid":"u1","type":"assistant","message":{"content":[
                {"type":"text","text":"first"},{"type":"text","text":"last"}]}}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::AssistantText { text: "last".into() });

        let v = val(r#"{"uuid":"u2","type":"assistant","message":{"content":[
                {"type":"tool_use","name":"AskUserQuestion","input":{}}]}}"#);
        assert_eq!(
            classify_tail_record(&v),
            TailEvent::NeedsHuman { tool: TailTool::AskUserQuestion }
        );

        let v = val(r#"{"uuid":"u3","type":"assistant","message":{"content":[
                {"type":"tool_use","name":"ExitPlanMode","input":{"plan":"p"}},
                {"type":"text","text":"t"}]}}"#);
        assert_eq!(
            classify_tail_record(&v),
            TailEvent::NeedsHuman { tool: TailTool::ExitPlanMode }
        );

        let v = val(r#"{"uuid":"u4","type":"system","subtype":"turn_duration","durationMs":1}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::TurnComplete);

        let v = val(r#"{"uuid":"u5","type":"assistant","isAbortedMidStream":true,"message":{}}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::Aborted);

        // The Esc press itself (verified against a live transcript
        // 2026-08-30): a `user` record carrying `interruptedMessageId`.
        let v = val(
            r#"{"uuid":"u7","type":"user","interruptedMessageId":"msg_011","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
        );
        assert_eq!(classify_tail_record(&v), TailEvent::Aborted);
        let v = val(r#"{"uuid":"u8","type":"user","interruptedMessageId":null,"message":{}}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::Other);

        let v = val(r#"{"uuid":"u6","type":"user","message":{}}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::Other);
    }
}
