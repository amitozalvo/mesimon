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
    /// What launched the process (`cli`, `sdk-cli`, `sdk-py`, …), read off
    /// the record that carried `cwd`. Absent on older transcripts.
    pub entrypoint: Option<String>,
}

impl TranscriptHead {
    /// A conversation an Agent SDK program ran, not one a person started
    /// (T-441): the `security-guidance` plugin's review hook alone left 225
    /// `sdk-py` transcripts in one repo's worktrees, measured. `sdk-cli` is
    /// the CLI driven by an app, and measured with a person's prompts — it
    /// stays. `sdk-ts` is the TypeScript SDK's word, by symmetry.
    pub fn agent_sdk_run(&self) -> bool {
        matches!(self.entrypoint.as_deref(), Some("sdk-py" | "sdk-ts"))
    }
}

/// Scan the head of a transcript (a chunk of newline-delimited JSON) for the
/// first `sessionId` + `cwd` pair. Unparseable lines are skipped, never fatal
/// (09 §4.3: skip-and-continue). The last line may be truncated by the caller's
/// bounded read — a parse failure there is expected and harmless.
pub fn parse_transcript_head(head: &str) -> Option<TranscriptHead> {
    let mut scan = HeadScan::default();
    head.lines().find_map(|line| scan.feed(line))
}

/// The record-by-record form of [`parse_transcript_head`]: a caller that reads
/// a transcript line by line feeds each one and stops at the first `Some`.
/// The two keys need not share a record — current Claude Code transcripts open
/// with latch records (`last-prompt`, `mode`, `permission-mode`, `atis-latch`)
/// that carry `sessionId` and no `cwd`, and the first record that carries both
/// can be tens of KB (T-425: a 22 KB `attachment`, measured).
#[derive(Debug, Default)]
pub struct HeadScan {
    session_id: Option<uuid::Uuid>,
    cwd: Option<String>,
    entrypoint: Option<String>,
}

impl HeadScan {
    pub fn feed(&mut self, line: &str) -> Option<TranscriptHead> {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return None };
        if self.session_id.is_none() {
            self.session_id =
                v.get("sessionId").and_then(Value::as_str).and_then(|s| s.parse().ok());
        }
        if self.cwd.is_none() {
            self.cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
            self.entrypoint = v.get("entrypoint").and_then(Value::as_str).map(str::to_string);
        }
        match (self.session_id, self.cwd.as_ref()) {
            (Some(session_id), Some(cwd)) => Some(TranscriptHead {
                session_id,
                cwd: cwd.clone(),
                entrypoint: self.entrypoint.clone(),
            }),
            _ => None,
        }
    }
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
    /// `idle` | `busy` | `waiting` observed on 2.1.266; kept as raw text.
    /// `waiting` was captured at a visible permission dialog in the state lab.
    pub status: Option<String>,
    /// Epoch ms of the last `status` write (`statusUpdatedAt`). Present since
    /// at least Claude Code 2.1.25x; the interrupt probe needs it to know the
    /// `idle` is this turn's and not the last one's.
    pub status_updated_at: Option<u64>,
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
    /// The assistant called a tool and said nothing else: the tool is IN
    /// FLIGHT until its `tool_result` lands, and the transcript writes nothing
    /// for as long as it runs — a build or a test suite keeps the file still
    /// for minutes while the pane is busy (T-265, 2026-09-06: a reload during
    /// a 3.5-minute `cargo` call read the session as idle until the tool came
    /// back). Distinct from `Other` precisely so a quiet-file rule cannot call
    /// it a dead turn.
    ToolInFlight,
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

/// Display text is independent of lifecycle: final replies and tool calls can
/// both carry text. Callers must not use preview selection as a state detector.
///
/// The words are a `text` block's, or a progress update's (a `thinking` block
/// the server wrote for a person, [`narration`]): the pane prints both as
/// prose, so both are what the agent last said. The newest block wins.
pub fn assistant_text(v: &Value) -> Option<&str> {
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    v.get("message")?.get("content")?.as_array()?.iter().rev().find_map(|b| {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => b.get("text").and_then(Value::as_str),
            Some("thinking") => narration(b),
            _ => None,
        }
    })
}

/// The words of a `thinking` block written for a person to read, or `None`
/// for the model's private reasoning. A Claude 5 model under
/// `thinking.display: "updates"` — what Claude Code 2.1.28x asks for — puts
/// its progress notes between tool calls ("the suite is green; now the
/// clippy pass") in `thinking` blocks of their own, one at most before each
/// tool call, and the pane prints them as ordinary prose where it folds
/// private thinking away (T-604: the card showed the reply before the one
/// on screen). The record carries nothing else that tells the two apart: the
/// block kind is in the signature, base64 of a protobuf whose header (field
/// 2 → field 1 → field 8) spells `thinking` or `narration`, which is also
/// how Claude Code's own stream-json schema says a renderer should tell
/// them apart (`narration_block_indexes`). Measured 2026-10-03 over 10,586
/// signed blocks in 762 local transcripts: every Claude 5 block carries the
/// header; Claude 4 blocks, Fable 5's older format and an interrupted block's
/// empty signature carry no kind and read as private, which is what they
/// were before — the walk never shows a thought by accident.
pub fn narration(block: &Value) -> Option<&str> {
    if block.get("type").and_then(Value::as_str) != Some("thinking") {
        return None;
    }
    let signature = block.get("signature").and_then(Value::as_str)?;
    if signature_block_kind(signature)? != b"narration" {
        return None;
    }
    // The server's notes end in two blanks; a card has no room for them.
    Some(block.get("thinking").and_then(Value::as_str)?.trim()).filter(|t| !t.is_empty())
}

/// The block kind a thinking block's signature carries (see [`narration`]),
/// read from its first 48 bytes: the header ends inside 32 on every measured
/// signature, and the rest is the ciphertext. `None` for any other shape.
fn signature_block_kind(signature: &str) -> Option<Vec<u8>> {
    let bytes = base64_prefix(signature, 64);
    let outer = proto_field(&bytes, 2)?;
    let header = proto_field(outer, 1)?;
    proto_field(header, 8).map(<[u8]>::to_vec)
}

/// Decode up to `chars` of standard (or URL-safe) base64, stopping at the
/// first character outside the alphabet. A short last group yields its whole
/// bytes; nothing here needs padding.
fn base64_prefix(text: &str, chars: usize) -> Vec<u8> {
    let sextet = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let digits: Vec<u8> = text.bytes().take(chars).map_while(sextet).collect();
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for group in digits.chunks(4) {
        let mut acc: u32 = 0;
        for (i, d) in group.iter().enumerate() {
            acc |= u32::from(*d) << (18 - 6 * i);
        }
        let whole = [(acc >> 16) as u8, (acc >> 8) as u8, acc as u8];
        out.extend_from_slice(&whole[..group.len().saturating_sub(1)]);
    }
    out
}

/// The bytes of length-delimited protobuf field `want` at the top level of
/// `buf`, walking the fields before it; a wanted field cut off by the end of
/// the buffer is returned as far as it goes (the caller reads a prefix). Any
/// other field cut off, a wire type past the four, or a malformed varint is
/// `None`.
fn proto_field(buf: &[u8], want: u64) -> Option<&[u8]> {
    let varint = |i: &mut usize| -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *buf.get(*i)?;
            *i += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    };
    let mut i = 0;
    while i < buf.len() {
        let tag = varint(&mut i)?;
        let (field, wire) = (tag >> 3, tag & 7);
        match wire {
            0 => {
                varint(&mut i)?;
            }
            1 => i += 8,
            5 => i += 4,
            2 => {
                let len = usize::try_from(varint(&mut i)?).ok()?;
                let end = i.checked_add(len)?;
                if field == want {
                    return Some(&buf[i..end.min(buf.len())]);
                }
                i = end;
            }
            _ => return None,
        }
    }
    None
}

/// Outstanding tool identity survives quiet polls, including parallel calls.
#[derive(Debug, Default)]
pub struct ToolLedger(std::collections::BTreeSet<String>);

impl ToolLedger {
    pub fn is_busy(&self) -> bool {
        !self.0.is_empty()
    }
    pub fn observe(&mut self, v: &Value) {
        if matches!(classify_tail_record(v), TailEvent::Aborted | TailEvent::TurnComplete) {
            self.0.clear();
            return;
        }
        let Some(blocks) =
            v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array)
        else {
            return;
        };
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") if v.get("type").and_then(Value::as_str) == Some("assistant") => {
                    self.0.insert(
                        b.get("id").and_then(Value::as_str).unwrap_or("unidentified").to_string(),
                    );
                }
                Some("tool_result") if v.get("type").and_then(Value::as_str) == Some("user") => {
                    self.0.remove(
                        b.get("tool_use_id").and_then(Value::as_str).unwrap_or("unidentified"),
                    );
                }
                _ => {}
            }
        }
    }
}

pub fn classify_tail_record(v: &Value) -> TailEvent {
    // The latch rule is absence-of-uuid, never a type allowlist — that is
    // what survived `cost-state`/`pr-link` appearing mid-corpus (09 §4.2).
    if v.get("uuid").is_none() {
        return TailEvent::Latch;
    }
    // Three spellings of an interrupt: the two mid-stream flags, and the Esc
    // press itself, which current Claude Code records as a `user` record
    // (verified live 2026-08-30 — spike S-E's "the transcript may get no
    // record" does not hold on current builds; `is_interrupt` for its shape).
    if v.get("isAbortedMidStream").and_then(Value::as_bool) == Some(true)
        || v.get("interruptedByShutdown").and_then(Value::as_bool) == Some(true)
        || is_interrupt(v)
    {
        return TailEvent::Aborted;
    }
    // Completion is structural and shared with the status-file corroborator.
    // A final text block is not continuing activity. Capture: 2.1.266 / complete.
    if matches!(turn_edge(v), TurnEdge::Done(_)) {
        return TailEvent::TurnComplete;
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
            let calls_a_tool =
                blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"));
            match text {
                _ if calls_a_tool => TailEvent::ToolInFlight,
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
        || is_interrupt(v)
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

/// The Esc press's own sentence: `[Request interrupted by user]`, or `… for
/// tool use]` when the press landed on a running tool. The prefix covers both.
const INTERRUPT_TAG: &str = "[Request interrupted by user";

/// Is this `user` record the Esc press? Known by its WORDS, the way the
/// task-notification wake is known by its tag, and only secondarily by the
/// `interruptedMessageId` beside them: that flag is optional in practice.
/// Census of the local corpus 2026-09-04 (Claude Code 2.1.220–2.1.259): the
/// tool-use form carries it about half the time (9 of 22 records lacked it on
/// 2.1.251–2.1.258) and an SDK-driven interrupt never does — and a record
/// without it was classed `Other`, so the card read "working" until the next
/// prompt (dogfood 2026-09-04: "escape to interrupt claude causes ticket
/// status always running"; the pane-quiet fallback never fired because the
/// idle prompt keeps repainting). The sentence is the harness's, written as a
/// single text block; a person typing those exact words as a prompt would be
/// read the same way, and the cost is a cosmetic "interrupted" that the
/// turn's next hook corrects.
/// What one transcript record says about whether the turn it belongs to is
/// OVER — the daemon's status-file probe reads the tail through this before
/// calling a `status: idle` an interrupt. Current Claude Code (2.1.26x) writes
/// no `turn_duration` record at all; a finished turn ends in an `assistant`
/// record with `stop_reason: end_turn` and, once the Stop hooks ran, a
/// `system`/`stop_hook_summary` — both carry `uuid` and `timestamp`. A `user`
/// record (a tool result, or a prompt) or a mid-turn assistant record means
/// the turn is open; the uuid-less latches and the attachment records say
/// nothing either way (`Unsaid`), so a walk skips them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEdge {
    /// The turn finished, at this epoch ms (0 when the record has no
    /// timestamp — a caller comparing against a spell start will refuse it).
    Done(u64),
    /// The turn is in flight (or was cut mid-stream).
    Open,
    /// This record says nothing about the turn.
    Unsaid,
}

pub fn turn_edge(v: &Value) -> TurnEdge {
    if v.get("uuid").is_none() {
        return TurnEdge::Unsaid;
    }
    match v.get("type").and_then(Value::as_str) {
        Some("system") => match v.get("subtype").and_then(Value::as_str) {
            Some("stop_hook_summary" | "turn_duration") => {
                TurnEdge::Done(record_ms(v).unwrap_or(0))
            }
            _ => TurnEdge::Unsaid,
        },
        Some("assistant") => {
            let stop = v.get("message").and_then(|m| m.get("stop_reason")).and_then(Value::as_str);
            let cut = v.get("isAbortedMidStream").and_then(Value::as_bool) == Some(true)
                || v.get("interruptedByShutdown").and_then(Value::as_bool) == Some(true)
                || is_interrupt(v);
            if stop == Some("end_turn") && !cut {
                TurnEdge::Done(record_ms(v).unwrap_or(0))
            } else {
                TurnEdge::Open
            }
        }
        Some("user") => TurnEdge::Open,
        _ => TurnEdge::Unsaid,
    }
}

/// Epoch ms of a record's `timestamp` (`2026-09-05T16:56:32.998Z`).
pub fn record_ms(v: &Value) -> Option<u64> {
    v.get("timestamp").and_then(Value::as_str).and_then(iso_ms)
}

/// `YYYY-MM-DDTHH:MM:SS[.fff]Z` → epoch ms. UTC only, which is the only form
/// Claude Code writes; anything else is `None` rather than a guess.
pub fn iso_ms(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, u32, u32) =
        (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    if d.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    let mut t = hms.split(':');
    let (h, mi, sec): (u64, u64, u64) =
        (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    if t.next().is_some() || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let millis: u64 = match frac.len() {
        0 => 0,
        _ => {
            let digits: String = frac.chars().take(3).collect();
            if !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            digits.parse::<u64>().ok()? * 10u64.pow(3 - digits.len() as u32)
        }
    };
    // Howard Hinnant's days_from_civil, m as 1-12.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = (m as u64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    let secs = days.checked_mul(86_400)?.checked_add((h * 3600 + mi * 60 + sec) as i64)?;
    u64::try_from(secs).ok()?.checked_mul(1000)?.checked_add(millis)
}

fn is_interrupt(v: &Value) -> bool {
    if v.get("interruptedMessageId").is_some_and(|x| !x.is_null()) {
        return true;
    }
    if v.get("type").and_then(Value::as_str) != Some("user") {
        return false;
    }
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else { return false };
    let text = match content.as_str() {
        Some(s) => Some(s),
        None => content.as_array().and_then(|blocks| {
            if blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")) {
                return None;
            }
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .find_map(|b| b.get("text").and_then(Value::as_str))
        }),
    };
    text.is_some_and(|t| t.trim_start().starts_with(INTERRUPT_TAG))
}

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
        assert_eq!(h.entrypoint, None);
        assert!(!h.agent_sdk_run());
        assert!(parse_transcript_head("{}\n{\"foo\":1}").is_none());
    }

    /// The measured shape of an Agent SDK run (T-441): queue latches that
    /// carry `sessionId` and no `cwd`, then the prompt, which carries both
    /// and the entrypoint.
    #[test]
    fn head_reads_the_entrypoint_off_the_record_that_carries_cwd() {
        let head = |entrypoint: &str| {
            format!(
                "{{\"type\":\"queue-operation\",\"operation\":\"enqueue\",\"sessionId\":\"{SID}\"}}\n\
                 {{\"type\":\"queue-operation\",\"operation\":\"dequeue\",\"sessionId\":\"{SID}\"}}\n\
                 {{\"type\":\"user\",\"uuid\":\"u0\",\"promptSource\":\"sdk\",\"entrypoint\":\"{entrypoint}\",\
                 \"cwd\":\"/repo\",\"sessionId\":\"{SID}\"}}\n"
            )
        };
        for (word, program) in
            [("sdk-py", true), ("sdk-ts", true), ("sdk-cli", false), ("cli", false)]
        {
            let h = parse_transcript_head(&head(word)).expect("head");
            assert_eq!(h.entrypoint.as_deref(), Some(word));
            assert_eq!(h.agent_sdk_run(), program, "{word}");
        }
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
    fn a_textless_tool_call_is_a_tool_in_flight() {
        // Current Claude Code writes one record per content block, and stamps
        // the whole message's stop_reason on each — so the BLOCK is the
        // evidence, never `stop_reason: tool_use` (a mid-turn thinking record
        // carries it too).
        let v =
            val(r#"{"uuid":"u1","type":"assistant","message":{"stop_reason":"tool_use","content":[
                {"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}]}}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::ToolInFlight);
        // Text beside the call still previews; the tool remains explicit state evidence.
        let v =
            val(r#"{"uuid":"u2","type":"assistant","message":{"stop_reason":"tool_use","content":[
                {"type":"text","text":"running the suite"},
                {"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}]}}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::ToolInFlight);
        assert_eq!(assistant_text(&v), Some("running the suite"));
        // The two human-facing tools keep their own event.
        let v =
            val(r#"{"uuid":"u3","type":"assistant","message":{"stop_reason":"tool_use","content":[
                {"type":"tool_use","name":"AskUserQuestion","input":{}}]}}"#);
        assert_eq!(
            classify_tail_record(&v),
            TailEvent::NeedsHuman { tool: TailTool::AskUserQuestion }
        );
        // A thinking-only record is still nothing the observe tier can use.
        let v =
            val(r#"{"uuid":"u4","type":"assistant","message":{"stop_reason":"tool_use","content":[
                {"type":"thinking","thinking":"hm"}]}}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::Other);
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
        // …and the same press with no flag beside it, in the tool-use spelling
        // (the shape that reached a card as `> [Request interrupted by user for
        // tool use]`, 2026-09-04).
        let v = val(r#"{"uuid":"u5b","type":"user","message":{"content":[
                {"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#);
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

        // The flag is optional (census 2026-09-04): both spellings of the
        // sentence are the press on their own, with the flag null or absent.
        let v = val(
            r#"{"uuid":"u9","type":"user","interruptedMessageId":null,"message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#,
        );
        assert_eq!(classify_tail_record(&v), TailEvent::Aborted);
        let v = val(
            r#"{"uuid":"u10","type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
        );
        assert_eq!(classify_tail_record(&v), TailEvent::Aborted);
        let v = val(
            r#"{"uuid":"u11","type":"user","message":{"content":"[Request interrupted by user]"}}"#,
        );
        assert_eq!(classify_tail_record(&v), TailEvent::Aborted);
        // A tool result that merely QUOTES the sentence is a tool result.
        let v = val(
            r#"{"uuid":"u12","type":"user","toolUseResult":{},"message":{"content":[{"type":"tool_result","content":"[Request interrupted by user]"}]}}"#,
        );
        assert_eq!(classify_tail_record(&v), TailEvent::Other);
        // And an assistant record saying the words is not an interrupt.
        let v = val(
            r#"{"uuid":"u13","type":"assistant","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
        );
        assert!(matches!(classify_tail_record(&v), TailEvent::AssistantText { .. }));

        let v = val(r#"{"uuid":"u6","type":"user","message":{}}"#);
        assert_eq!(classify_tail_record(&v), TailEvent::Other);
    }

    #[test]
    fn iso_ms_reads_claude_codes_timestamps() {
        assert_eq!(iso_ms("2026-09-05T16:56:32.998Z"), Some(1_788_627_392_998));
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_ms("2000-03-01T00:00:00.5Z"), Some(951_868_800_500));
        assert_eq!(iso_ms("2026-09-05T16:56:32.998+03:00"), None, "only Z");
        assert_eq!(iso_ms("2026-13-05T16:56:32Z"), None);
        assert_eq!(iso_ms("not a time"), None);
    }

    #[test]
    fn turn_edge_reads_the_end_of_a_turn_and_nothing_else() {
        let j = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        // The two records a finished turn ends in on current Claude Code.
        assert_eq!(
            turn_edge(&j(
                r#"{"uuid":"u","type":"assistant","timestamp":"2026-09-05T16:56:32.998Z","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}}"#
            )),
            TurnEdge::Done(1_788_627_392_998)
        );
        assert_eq!(
            turn_edge(&j(
                r#"{"uuid":"u","type":"system","subtype":"stop_hook_summary","timestamp":"2026-09-05T16:56:33.447Z"}"#
            )),
            TurnEdge::Done(1_788_627_393_447)
        );
        assert_eq!(
            turn_edge(&j(r#"{"uuid":"u","type":"system","subtype":"turn_duration"}"#)),
            TurnEdge::Done(0),
            "no timestamp is a zero, never a guess"
        );
        // Mid-turn: a tool call, a tool result, a prompt, an abort.
        assert_eq!(
            turn_edge(&j(
                r#"{"uuid":"u","type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Bash"}]}}"#
            )),
            TurnEdge::Open
        );
        assert_eq!(
            turn_edge(&j(
                r#"{"uuid":"u","type":"assistant","message":{"stop_reason":null,"content":[]}}"#
            )),
            TurnEdge::Open
        );
        assert_eq!(
            turn_edge(&j(
                r#"{"uuid":"u","type":"user","message":{"content":[{"type":"tool_result"}]}}"#
            )),
            TurnEdge::Open
        );
        assert_eq!(
            turn_edge(&j(r#"{"uuid":"u","type":"user","message":{"content":"go"}}"#)),
            TurnEdge::Open
        );
        assert_eq!(
            turn_edge(&j(
                r#"{"uuid":"u","type":"assistant","isAbortedMidStream":true,"message":{"stop_reason":"end_turn","content":[]}}"#
            )),
            TurnEdge::Open
        );
        // Silent: latches, attachments, other system records.
        assert_eq!(turn_edge(&j(r#"{"type":"last-prompt"}"#)), TurnEdge::Unsaid);
        assert_eq!(turn_edge(&j(r#"{"uuid":"u","type":"attachment"}"#)), TurnEdge::Unsaid);
        assert_eq!(
            turn_edge(&j(r#"{"uuid":"u","type":"system","subtype":"compact_boundary"}"#)),
            TurnEdge::Unsaid
        );
    }

    /// T-604: a Claude 5 model's progress notes between tool calls are
    /// `thinking` blocks the server tagged `narration` in the signature, and
    /// the pane prints them as prose. Shapes from T-601's transcript (Claude
    /// Code 2.1.288, Opus 5.5): the words scrubbed, the signature prefixes
    /// real — the header is the first 32 bytes, the rest is ciphertext.
    #[test]
    fn a_progress_update_is_the_agents_words_and_private_thinking_is_not() {
        const NARRATION: &str = "CAQSqAYKEQgSGAI4AUIJbmFycmF0aW9uEgz6RfzbZ3WGlSWDhZgaDMPYsCHMosTFaCYSbyIw5iwbGg2IIGHyFn57q3rfGqRCddO0A0KzShOu2yD3Wgv4piiOp8iViofNNe3nQo3IKsQFZ5FDP5BFYQVq";
        const THINKING: &str = "CAQS3wcKEAgSGAI4AUIIdGhpbmtpbmcSDB8YT6rdRh3KJTjRCBoM8rfhnSQrS4xo2VnjIjBsRWkpUTlTCt34Y5LTY3S6oISt0dQYaNqT0JhKtJG1HNG+jGilr6Y19GRv6PC8VN4q/AZSXXadvsdJSprS54NXkquSYR2XI52WtTvAlJ1zRdHRspR2B4";
        let record = |sig: &str| {
            val(&format!(
                r#"{{"uuid":"u1","type":"assistant","message":{{"stop_reason":"tool_use","content":[
                {{"type":"thinking","thinking":"The suite is green; now the clippy pass.  ","signature":"{sig}"}}]}}}}"#
            ))
        };
        let v = record(NARRATION);
        assert_eq!(assistant_text(&v), Some("The suite is green; now the clippy pass."));
        // Words, not state: the note introduces the tool call that follows
        // it, so the observe tier still reads nothing from the record.
        assert_eq!(classify_tail_record(&v), TailEvent::Other);
        assert_eq!(assistant_text(&record(THINKING)), None, "private reasoning");
        // The shapes that carry no kind are all private: an interrupted
        // block's empty signature, Fable 5's and Opus 4.7's formats, and a
        // signature that is not base64.
        for sig in [
            "",
            "CAIS8xkKiAIIEhgCKkDvRYCjgsWA+Dm/TW5bKVqDsnRU10oXIvPpCBIUIDsvdBqR",
            "EukHCqgBCBIYAipA1D3r1rObW4SSPU+vNFd1/zldtT3WS/nuZ27bKEEgqQeuxLw6",
            "not a signature",
        ] {
            assert_eq!(assistant_text(&record(sig)), None, "{sig:?}");
        }
        let v = val(
            r#"{"uuid":"u2","type":"assistant","message":{"content":[{"type":"thinking","thinking":"hm"}]}}"#,
        );
        assert_eq!(assistant_text(&v), None, "no signature at all");
        // A 2.1.283 pair, its header one byte shorter: the same answer.
        assert_eq!(
            signature_block_kind(
                "CAQS8wYKEQgSGAI4AUIJbmFycmF0aW9uEgxt85/pr/BS8b1X7FcaDMcRkfB/WwnI"
            )
            .as_deref(),
            Some(&b"narration"[..])
        );
        assert_eq!(
            signature_block_kind(
                "CAQS+wQKEAgSGAI4AUIIdGhpbmtpbmcSDGSAfxQye8rOVXTKSRoMvkpU5WLq/C/Y"
            )
            .as_deref(),
            Some(&b"thinking"[..])
        );
        // A blank note is no words; a text block beside a note is the later
        // of the two, whichever order the blocks come in.
        let v = val(&format!(
            r#"{{"uuid":"u3","type":"assistant","message":{{"content":[
                {{"type":"thinking","thinking":"  ","signature":"{NARRATION}"}}]}}}}"#
        ));
        assert_eq!(assistant_text(&v), None);
        let v = val(&format!(
            r#"{{"uuid":"u4","type":"assistant","message":{{"content":[
                {{"type":"thinking","thinking":"first the note","signature":"{NARRATION}"}},
                {{"type":"text","text":"then the reply"}}]}}}}"#
        ));
        assert_eq!(assistant_text(&v), Some("then the reply"));
        let v = val(&format!(
            r#"{{"uuid":"u5","type":"assistant","message":{{"content":[
                {{"type":"text","text":"first the reply"}},
                {{"type":"thinking","thinking":"then the note","signature":"{NARRATION}"}}]}}}}"#
        ));
        assert_eq!(assistant_text(&v), Some("then the note"));
    }

    #[test]
    fn the_signature_readers_take_a_prefix_and_refuse_the_rest() {
        assert_eq!(base64_prefix("QUJDRA==", 64), b"ABCD");
        assert_eq!(base64_prefix("QUI", 64), b"AB", "a short last group yields its whole bytes");
        assert_eq!(base64_prefix("QUJD", 2), b"A", "the cap is in characters");
        assert_eq!(base64_prefix("QU/D-_", 64).len(), 4, "both alphabets");
        assert!(base64_prefix("", 64).is_empty());
        // Field 2 after a varint field; a wanted field cut off by the buffer
        // is a prefix, any other cut-off field ends the walk.
        assert_eq!(proto_field(&[0x08, 0x04, 0x12, 0x03, b'a', b'b', b'c'], 2), Some(&b"abc"[..]));
        assert_eq!(proto_field(&[0x12, 0x09, b'a', b'b'], 2), Some(&b"ab"[..]));
        assert_eq!(proto_field(&[0x0a, 0x09, b'a', b'b'], 2), None);
        assert_eq!(proto_field(&[0x3f, 0x01], 2), None, "wire type 7");
        assert_eq!(proto_field(&[0x08, 0x80], 2), None, "a varint cut off");
        assert_eq!(proto_field(&[], 2), None);
    }
}
