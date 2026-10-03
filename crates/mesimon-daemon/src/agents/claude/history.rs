//! Read-only Claude transcript previews. The bounded reverse scan preserves
//! assistant identity, current-prompt fallback, and post-reply tool activity.
//! "What the agent said" is `adopt::assistant_text`'s word: a text block, or
//! the progress note a Claude 5 model writes before a tool call (T-604).

use crate::agents::{AgentActivity, AgentPreview};
use mesimon_core::adopt::{
    assistant_text, is_interrupt, narration, record_ms, tool_activity, tool_label, user_prompt,
};
use mesimon_core::mesophon::{RowKind, TranscriptRow};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Exact existing Claude recovery rule: a learned path only vouches for the
/// requested conversation if its filename matches; otherwise check each
/// project directory because a conversation can have moved with its cwd.
pub(super) fn missing(record: &mesimon_core::board::SessionRecord, projects: &Path) -> bool {
    let identity = record.claude_session_id.unwrap_or(record.id);
    let name = format!("{identity}.jsonl");
    if let Some(path) = &record.transcript_path {
        let path = Path::new(path);
        if path.file_name().and_then(|f| f.to_str()) == Some(name.as_str()) && path.is_file() {
            return false;
        }
    }
    if let Ok(dirs) = std::fs::read_dir(projects) {
        for directory in dirs.flatten() {
            if directory.path().join(&name).is_file() {
                return false;
            }
        }
    }
    true
}

/// The first window a peek reads. The agent's last words sit ~16 KiB back at
/// p50 — but one `Read` of a big file, or a grep with a wide net, buries them
/// under a few hundred KiB of tool records, and the peek then went blank with
/// the message it wanted still in the file (measured over 120 local
/// transcripts: blank at 22% of record boundaries, 11% of them purely because
/// the reply had scrolled out of this window).
const TAIL_BYTES: u64 = 64 * 1024;
/// Read only when the first window holds no reply — same escalation, same
/// size, and for the same measured reason as the census's drawer preview.
const TAIL_BYTES_MAX: u64 = 256 * 1024;

pub(super) fn latest_preview(path: &Path) -> Option<AgentPreview> {
    let len = std::fs::metadata(path).ok()?.len();
    let mut tail = scan_window(path, len, TAIL_BYTES)?;
    // The deep read is for a walk that ran out of window, not for one that
    // stopped where it meant to: the tool call and the `last-prompt` latch
    // both ride near EOF by construction.
    if tail.assistant.is_none() && tail.prompt.is_none() && len > TAIL_BYTES {
        // The deep walk repeats the shallow one from EOF, so it re-finds the
        // same newest tool call and simply reaches further back.
        tail = scan_window(path, len, TAIL_BYTES_MAX)?;
    }
    let words = tail.prompt.or(tail.latch_prompt);
    Some(AgentPreview {
        text: match tail.assistant {
            Some(reply) => Some(reply),
            // The user's own words, marked as theirs (census parity).
            None => words.map(|p| format!("> {p}")),
        },
        activity: tail.activity,
        reply_key: tail.assistant_key,
    })
}

#[derive(Default)]
struct Tail {
    assistant: Option<String>,
    /// The record `assistant` came from, hashed (see `AgentPreview::reply_key`).
    assistant_key: Option<u64>,
    /// The user's newest message, when it is newer than any reply.
    prompt: Option<String>,
    /// The `last-prompt` latch — a turn stale, so only ever a last resort.
    latch_prompt: Option<String>,
    activity: Option<AgentActivity>,
}

/// Reverse-scan the final `window` bytes; the first hit of each kind is the
/// latest write, so an assistant reply ends the walk immediately. `None` only
/// when the file cannot be read.
fn scan_window(path: &Path, len: u64, window: u64) -> Option<Tail> {
    let mut f = std::fs::File::open(path).ok()?;
    let start = len.saturating_sub(window);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // the window may open mid-record
    }
    let mut tail = Tail::default();
    for line in lines.iter().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v.get("uuid").is_none() {
            // Uuid-less records are latches (09 §4.2); `last-prompt` is the
            // only one a peek has any use for.
            if tail.latch_prompt.is_none()
                && v.get("type").and_then(serde_json::Value::as_str) == Some("last-prompt")
            {
                tail.latch_prompt =
                    v.get("lastPrompt").and_then(serde_json::Value::as_str).map(str::to_string);
            }
            continue;
        }
        // Order matters: one record can hold both the words and the call
        // that followed them ("now let me check X" + the tool_use), and the
        // call is the later of the two.
        if tail.activity.is_none() {
            tail.activity = tool_activity(&v).map(AgentActivity::Tool);
        }
        // A message from the user ends the walk. Whatever the agent said
        // below it answers an older question — showing that reply next to a
        // live spinner is the lie this peek is trying not to tell — and the
        // prompt itself is what the agent is on. Nothing newer than the
        // message means nothing has happened yet: it is thinking.
        if let Some(p) = user_prompt(&v) {
            tail.prompt = Some(p);
            tail.activity.get_or_insert(AgentActivity::Thinking);
            return Some(tail);
        }
        // `adopt::assistant_text`, never `classify_tail_record`: the classifier
        // answers what STATE a record puts the session in, and the record that
        // closes a turn — `stop_reason: end_turn`, which is exactly where the
        // agent's last words live — answers `TurnComplete` there. Reading the
        // words out of the classifier meant every idle session's peek walked
        // straight past the reply to the prompt above it and showed the user
        // their own words back (T-350). Mid-turn, the newest words are as
        // often a progress note as a text block (T-604): `assistant_text`
        // reads both, and the walk still stops at the first it meets.
        if let Some(text) = assistant_text(&v) {
            tail.assistant = Some(text.to_string());
            tail.assistant_key = v.get("uuid").and_then(serde_json::Value::as_str).map(record_key);
            return Some(tail);
        }
    }
    Some(tail)
}

/// What one record shows on a phone's transcript (T-626): the person's
/// words, the agent's (a text block or a progress note, as the peek reads
/// them), one line per tool call, and what happened to the conversation.
/// Tool results, latches, attachments, injected context and a subagent's
/// records are not rows.
pub(in crate::agents) fn rows(at: u64, v: &Value) -> Vec<TranscriptRow> {
    let ms = record_ms(v);
    let row = |kind, text: &str| TranscriptRow::new(at, kind, text, ms);
    if v.get("uuid").is_none() || v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return Vec::new();
    }
    match v.get("type").and_then(Value::as_str) {
        Some("user") if is_interrupt(v) => {
            row(RowKind::Notice, "interrupted").into_iter().collect()
        }
        // The summary a compaction opens the new context with is the
        // harness's words; the boundary record above it is the row.
        Some("user") if v.get("isCompactSummary").and_then(Value::as_bool) == Some(true) => {
            Vec::new()
        }
        Some("user") => {
            let Some(words) = user_prompt(v) else { return Vec::new() };
            if let Some(command) = super::super::transcript::command_words(&words) {
                let said = if command == "/clear" { "conversation cleared" } else { &command };
                return row(RowKind::Notice, said).into_iter().collect();
            }
            if let Some(shell) = tagged(&words, "bash-input") {
                return row(RowKind::Prompt, &format!("! {shell}")).into_iter().collect();
            }
            // A local command's or a shell's output: the terminal's, not words.
            if ["<local-command-", "<bash-stdout>", "<bash-stderr>"]
                .iter()
                .any(|t| words.starts_with(t))
            {
                return Vec::new();
            }
            row(RowKind::Prompt, &words).into_iter().collect()
        }
        Some("assistant") => {
            let Some(blocks) =
                v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array)
            else {
                return Vec::new();
            };
            blocks
                .iter()
                .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                    Some("text") => row(RowKind::Reply, b.get("text")?.as_str()?),
                    Some("thinking") => row(RowKind::Reply, narration(b)?),
                    Some("tool_use") => TranscriptRow::tool(at, &tool_label(b), ms),
                    _ => None,
                })
                .collect()
        }
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
            row(RowKind::Notice, "conversation compacted").into_iter().collect()
        }
        _ => Vec::new(),
    }
}

/// The words between `<tag>` and `</tag>` when `text` opens with the tag.
fn tagged<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(&format!("<{tag}>"))?;
    Some(rest.split(&format!("</{tag}>")).next().unwrap_or(rest).trim())
}

/// A record's identity as a number the board can compare and keep.
fn record_key(uuid: &str) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hash = DefaultHasher::new();
    uuid.hash(&mut hash);
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("msmn-claude-preview-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("t.jsonl")
    }

    fn reply(uuid: &str, text: &str) -> String {
        format!(
            "{{\"uuid\":\"{uuid}\",\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
        )
    }

    /// Tool traffic as it really arrives: a call, then a result big enough to
    /// push what came before it out of a window.
    fn tool_noise(kb: usize) -> String {
        let mut s = String::new();
        s.push_str(
            "{\"uuid\":\"t0\",\"type\":\"assistant\",\"message\":{\"content\":[\
             {\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"grep -r x\",\"description\":\"Count the lines\"}}]}}\n",
        );
        for i in 0..kb {
            s.push_str(&format!(
                "{{\"uuid\":\"r{i}\",\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"content\":\"{}\"}}]}}}}\n",
                "x".repeat(1000)
            ));
        }
        s
    }

    #[test]
    fn finds_assistant_text_buried_under_latches() {
        let p = tmp("buried");
        std::fs::write(
            &p,
            format!(
                "{}{}{}{}",
                reply("u1", "old reply"),
                reply("u2", "latest reply"),
                "{\"uuid\":\"u3\",\"type\":\"system\",\"subtype\":\"turn_duration\"}\n",
                "{\"type\":\"latch\"}\nnot json\n"
            ),
        )
        .unwrap();
        let pk = latest_preview(&p).expect("peek");
        assert_eq!(pk.text.as_deref(), Some("latest reply"));
        assert_eq!(pk.activity, None, "the agent spoke last: no step to show");
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// The shape every finished turn ends in (Claude Code 2.1.26x): the closing
    /// assistant record carries `stop_reason: end_turn`, then the stop-hook
    /// summary, then the latches. T-350: `classify_tail_record` reads that
    /// record as `TurnComplete` — correct for the state machine, fatal here —
    /// so the peek walked past the reply and showed the user their own prompt.
    /// No fixture above carries a `stop_reason`, which is how it shipped.
    #[test]
    fn the_closing_reply_of_a_finished_turn_is_the_preview() {
        let p = tmp("endturn");
        std::fs::write(
            &p,
            format!(
                "{}{}{}{}",
                "{\"uuid\":\"p1\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"commit\"}}\n",
                "{\"uuid\":\"u1\",\"type\":\"assistant\",\"timestamp\":\"2026-09-11T11:39:00.203Z\",\"message\":{\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"Committed as d5d114e.\"}]}}\n",
                "{\"uuid\":\"u2\",\"type\":\"system\",\"subtype\":\"stop_hook_summary\",\"timestamp\":\"2026-09-11T11:39:00.500Z\"}\n",
                "{\"type\":\"last-prompt\",\"lastPrompt\":\"commit\"}\n{\"type\":\"mode\"}\n",
            ),
        )
        .unwrap();
        let pk = latest_preview(&p).expect("peek");
        assert_eq!(pk.text.as_deref(), Some("Committed as d5d114e."));
        assert!(pk.reply_key.is_some(), "a closing reply is a reply, and has a key");
        assert_eq!(pk.activity, None);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn missing_or_textless_transcript_is_none() {
        assert!(latest_preview(Path::new("/nonexistent/x.jsonl")).is_none());
        let p = tmp("textless");
        std::fs::write(&p, "{\"type\":\"latch\"}\n").unwrap();
        assert_eq!(latest_preview(&p).expect("peek").text, None);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn a_reply_buried_under_tool_traffic_still_shows() {
        // The dogfood report (2026-08-31): a live session's card peeked blank
        // while the reply it wanted sat 88 KiB back, behind a tool run.
        let p = tmp("deep");
        std::fs::write(&p, format!("{}{}", reply("u1", "the reply"), tool_noise(80))).unwrap();
        let len = std::fs::metadata(&p).unwrap().len();
        assert!(
            scan_window(&p, len, TAIL_BYTES).unwrap().assistant.is_none(),
            "fixture must actually bury the reply past the first window"
        );
        let pk = latest_preview(&p).expect("peek");
        assert_eq!(pk.text.as_deref(), Some("the reply"));
        assert_eq!(pk.activity, Some(AgentActivity::Tool("Count the lines".into())));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn falls_back_to_the_users_own_words() {
        // A session that has only ever run tools has no reply to show; the
        // prompt it is working on is the honest stand-in (census parity).
        let p = tmp("prompt");
        std::fs::write(
            &p,
            format!(
                "{}{}",
                "{\"type\":\"last-prompt\",\"lastPrompt\":\"fix the peek\"}\n",
                tool_noise(2)
            ),
        )
        .unwrap();
        let pk = latest_preview(&p).expect("peek");
        assert_eq!(pk.text.as_deref(), Some("> fix the peek"));
        assert_eq!(pk.activity, Some(AgentActivity::Tool("Count the lines".into())));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn a_message_from_the_user_makes_the_last_reply_stale() {
        // The dogfood case (2026-08-31): the agent is Running, but everything
        // it has said answers the PREVIOUS question. The card must not put a
        // live spinner next to a stale reply.
        let p = tmp("stale");
        std::fs::write(
            &p,
            format!(
                "{}{}",
                reply("u1", "old answer"),
                "{\"uuid\":\"u2\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"now do the other thing\"}}\n"
            ),
        )
        .unwrap();
        let pk = latest_preview(&p).expect("peek");
        assert_eq!(pk.text.as_deref(), Some("> now do the other thing"));
        assert_eq!(pk.activity, Some(AgentActivity::Thinking));

        // Once it reaches for a tool, the step replaces `thinking` — the
        // words stay the user's, because it still has not answered.
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(
            "{\"uuid\":\"u3\",\"type\":\"assistant\",\"message\":{\"content\":[\
             {\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"/a/card.rs\"}}]}}\n"
                .as_bytes(),
        )
        .unwrap();
        let pk = latest_preview(&p).expect("peek");
        assert_eq!(pk.text.as_deref(), Some("> now do the other thing"));
        assert_eq!(pk.activity, Some(AgentActivity::Tool("Read card.rs".into())));

        // And once it answers, the reply is current again.
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(reply("u4", "new answer").as_bytes()).unwrap();
        let pk = latest_preview(&p).expect("peek");
        assert_eq!(pk.text.as_deref(), Some("new answer"));
        assert_eq!(pk.activity, None);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn tool_results_are_not_messages_from_the_user() {
        // 862 of every 940 `user` records are tool results; mistaking one for
        // a prompt would call every tool run "thinking".
        let p = tmp("toolresult");
        std::fs::write(&p, format!("{}{}", reply("u1", "the reply"), tool_noise(1))).unwrap();
        let pk = latest_preview(&p).expect("peek");
        assert_eq!(pk.text.as_deref(), Some("the reply"));
        assert_eq!(pk.activity, Some(AgentActivity::Tool("Count the lines".into())));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn activity_is_the_step_that_came_after_the_words() {
        let p = tmp("activity");
        std::fs::write(
            &p,
            format!(
                "{}{}{}",
                reply("u1", "now the counts"),
                "{\"uuid\":\"u2\",\"type\":\"assistant\",\"message\":{\"content\":[\
                 {\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"/a/b/peek.rs\"}}]}}\n",
                "{\"uuid\":\"u3\",\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"ok\"}]}}\n"
            ),
        )
        .unwrap();
        let pk = latest_preview(&p).expect("peek");
        assert_eq!(pk.text.as_deref(), Some("now the counts"));
        assert_eq!(pk.activity, Some(AgentActivity::Tool("Read peek.rs".into())));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// The reply's identity is the RECORD, not its words: the same sentence
    /// after a second prompt is a second reply, and the user's own prompt is
    /// no reply at all.
    #[test]
    fn reply_key_names_the_record_and_never_the_prompt() {
        let p = tmp("key");
        std::fs::write(&p, reply("u1", "Done.")).unwrap();
        let first = latest_preview(&p).unwrap();
        assert_eq!(first.text.as_deref(), Some("Done."));
        let k1 = first.reply_key.expect("an assistant record has a key");
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(
            f,
            "{{\"uuid\":\"p1\",\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"and now?\"}}}}"
        )
        .unwrap();
        f.flush().unwrap();
        let asked = latest_preview(&p).unwrap();
        assert_eq!(asked.text.as_deref(), Some("> and now?"));
        assert_eq!(asked.reply_key, None, "a prompt is the user's, not a reply");
        write!(f, "{}", reply("u2", "Done.")).unwrap();
        f.flush().unwrap();
        let second = latest_preview(&p).unwrap();
        assert_eq!(second.text.as_deref(), Some("Done."));
        assert_ne!(second.reply_key, Some(k1), "same words, second record, second key");
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }
    #[test]
    fn recovery_requires_the_exact_conversation_and_finds_relocated_history() {
        use mesimon_core::board::{SessionKind, SessionRecord, SessionState};
        let stale = tmp("recovery-exact");
        let directory = stale.parent().unwrap();
        let mut record = SessionRecord::new(
            uuid::Uuid::from_u128(101),
            SessionKind::Claude,
            ulid::Ulid(1),
            vec![],
            "/repo".into(),
            SessionState::Sleeping,
        );
        std::fs::write(&stale, "old history").unwrap();
        record.transcript_path = Some(stale.display().to_string());
        let projects = directory.join("projects");
        assert!(
            missing(&record, &projects),
            "an existing path with another filename proves nothing"
        );
        let relocated = projects.join("different-cwd");
        std::fs::create_dir_all(&relocated).unwrap();
        std::fs::write(relocated.join(format!("{}.jsonl", record.id)), "owned fixture").unwrap();
        assert!(!missing(&record, &projects));
        record.claude_session_id = Some(uuid::Uuid::from_u128(102));
        assert!(
            missing(&record, &projects),
            "in-app resume selects a different exact conversation"
        );
        let exact = directory.join(format!("{}.jsonl", record.claude_session_id.unwrap()));
        std::fs::write(&exact, "owned fixture").unwrap();
        record.transcript_path = Some(exact.display().to_string());
        assert!(!missing(&record, &projects));
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// T-604 (the author: "transcript not showing latest … showing 1 before"):
    /// the records of T-601's session between two replies, Claude Code
    /// 2.1.288 on the mod road, one block per record. The pane printed the
    /// progress note at `n2` as prose; the walk read `thinking` as private
    /// and showed the text block before the tool run — one reply behind. The
    /// note is a `thinking` block whose signature's block kind is `narration`
    /// (`adopt::narration`); the prefix here is T-601's own.
    #[test]
    fn a_progress_update_before_a_tool_call_is_the_newest_words() {
        const NARRATION: &str = "CAQSqAYKEQgSGAI4AUIJbmFycmF0aW9uEgz6RfzbZ3WGlSWDhZgaDMPYsCHMosTFaCYSbyIw5iwbGg2IIGHyFn57q3rfGqRCddO0A0KzShOu2yD3Wgv4piiOp8iViofNNe3nQo3IKsQFZ5FDP5BFYQVq";
        const THINKING: &str = "CAQS3wcKEAgSGAI4AUIIdGhpbmtpbmcSDB8YT6rdRh3KJTjRCBoM8rfhnSQrS4xo2VnjIjBsRWkpUTlTCt34Y5LTY3S6oISt0dQYaNqT0JhKtJG1HNG+jGilr6Y19GRv6PC8VN4q/AZSXXadvsdJSprS54NXkquSYR2XI52WtTvAlJ1zRdHRspR2B4";
        let thought = |uuid: &str, words: &str, sig: &str| {
            format!(
                "{{\"uuid\":\"{uuid}\",\"type\":\"assistant\",\"message\":{{\"stop_reason\":\"tool_use\",\"content\":[\
                 {{\"type\":\"thinking\",\"thinking\":\"{words}\",\"signature\":\"{sig}\"}}]}}}}\n"
            )
        };
        let call = |uuid: &str, what: &str| {
            format!(
                "{{\"uuid\":\"{uuid}\",\"type\":\"assistant\",\"message\":{{\"stop_reason\":\"tool_use\",\"content\":[\
                 {{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{{\"command\":\"cargo test\",\"description\":\"{what}\"}}}}]}}}}\n"
            )
        };
        let result = |uuid: &str| {
            format!(
                "{{\"uuid\":\"{uuid}\",\"type\":\"user\",\"toolUseResult\":{{}},\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"content\":\"ok\"}}]}}}}\n"
            )
        };
        let latches =
            "{\"type\":\"last-prompt\",\"lastPrompt\":\"the prompt\"}\n{\"type\":\"mode\"}\n";
        let p = tmp("narration");
        std::fs::write(
            &p,
            format!(
                "{}{}{}{}{}{}{}{}{}",
                reply("a1", "Alone it passes, so it was a flake. Re-running the suite."),
                call("a2", "Rerun the bounded full suite"),
                result("r1"),
                latches,
                thought("n1", "All passed, but one e2e failed on the second pass.  ", THINKING),
                thought(
                    "n2",
                    "The first pass finished clean, but the second stopped on one test; rerunning it alone.  ",
                    NARRATION
                ),
                call("a3", "Rerun the e2e three times"),
                result("r2"),
                call("a4", "Run the whole pass without fail-fast"),
            ),
        )
        .unwrap();
        let before = latest_preview(&p).expect("peek");
        assert_eq!(
            before.text.as_deref(),
            Some("The first pass finished clean, but the second stopped on one test; rerunning it alone."),
            "the note, not the text block before the tool run, and not the private thought"
        );
        assert_eq!(
            before.activity,
            Some(AgentActivity::Tool("Run the whole pass without fail-fast".into()))
        );
        let note_key = before.reply_key.expect("a note is the agent's words, and has a key");

        // The turn's closing reply is newer than every note, and its own record.
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(format!("{}{}", result("r3"), reply("a5", "Mod pass: all green.")).as_bytes())
            .unwrap();
        let after = latest_preview(&p).expect("peek");
        assert_eq!(after.text.as_deref(), Some("Mod pass: all green."));
        assert_eq!(after.activity, None);
        assert_ne!(after.reply_key, Some(note_key));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// T-626: a phone's transcript reads a session as its conversation —
    /// the person's words, the agent's (a progress note among them, never a
    /// private thought), one line per tool call, and what happened to the
    /// conversation — and nothing of the harness's.
    #[test]
    fn a_transcript_page_is_the_conversation() {
        const NARRATION: &str = "CAQSqAYKEQgSGAI4AUIJbmFycmF0aW9uEgz6RfzbZ3WGlSWDhZgaDMPYsCHMosTFaCYSbyIw5iwbGg2IIGHyFn57q3rfGqRCddO0A0KzShOu2yD3Wgv4piiOp8iViofNNe3nQo3IKsQFZ5FDP5BFYQVq";
        const THINKING: &str = "CAQS3wcKEAgSGAI4AUIIdGhpbmtpbmcSDB8YT6rdRh3KJTjRCBoM8rfhnSQrS4xo2VnjIjBsRWkpUTlTCt34Y5LTY3S6oISt0dQYaNqT0JhKtJG1HNG+jGilr6Y19GRv6PC8VN4q/AZSXXadvsdJSprS54NXkquSYR2XI52WtTvAlJ1zRdHRspR2B4";
        let user = |uuid: &str, extra: &str, content: &str| {
            format!(
                "{{\"uuid\":\"{uuid}\",\"type\":\"user\",\"timestamp\":\"2026-10-03T10:00:00.000Z\"{extra},\"message\":{{\"role\":\"user\",\"content\":{content}}}}}\n"
            )
        };
        let said = |uuid: &str, blocks: &str| {
            format!(
                "{{\"uuid\":\"{uuid}\",\"type\":\"assistant\",\"message\":{{\"content\":[{blocks}]}}}}\n"
            )
        };
        let text = |t: &str| format!("{{\"type\":\"text\",\"text\":\"{t}\"}}");
        let thought = |t: &str, sig: &str| {
            format!("{{\"type\":\"thinking\",\"thinking\":\"{t}  \",\"signature\":\"{sig}\"}}")
        };
        let tool = "{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"/r/src/main.rs\"}}";
        let body = [
            user("u1", "", "\"fix the bug\\nin main\""),
            said("a1", &format!("{},{}", thought("private reasoning", THINKING), text("Looking."))),
            said("a2", &format!("{},{tool}", thought("Reading main first.", NARRATION))),
            user("r1", ",\"toolUseResult\":{}", "[{\"type\":\"tool_result\",\"content\":\"fn main\"}]"),
            "{\"type\":\"last-prompt\",\"lastPrompt\":\"fix\"}\n".into(),
            user("i1", "", "[{\"type\":\"text\",\"text\":\"[Request interrupted by user for tool use]\"}]"),
            user("m1", ",\"isMeta\":true", "\"<local-command-caveat>x</local-command-caveat>\""),
            user("c1", "", "\"<command-name>/effort</command-name><command-args>xhigh</command-args>\""),
            user("o1", "", "\"<local-command-stdout>Set effort</local-command-stdout>\""),
            user("t1", "", "\"<task-notification><task-id>x</task-id></task-notification>\""),
            user("s1", ",\"isSidechain\":true", "\"a subagent's brief\""),
            "{\"uuid\":\"b1\",\"type\":\"system\",\"subtype\":\"compact_boundary\",\"content\":\"Conversation compacted\"}\n".into(),
            user("k1", ",\"isCompactSummary\":true", "\"This session is being continued\""),
            user("x1", "", "\"<bash-input>ls</bash-input>\""),
            said("a3", &text("Fixed in **main.rs**.")),
        ]
        .concat();
        let p = tmp("page");
        std::fs::write(&p, &body).unwrap();
        let page = crate::agents::read_transcript(
            mesimon_core::board::SessionKind::Claude,
            &p,
            crate::agents::transcript::Ask::default(),
        )
        .expect("page");
        let rows: Vec<(RowKind, &str)> =
            page.rows.iter().map(|r| (r.kind, r.text.as_str())).collect();
        assert_eq!(
            rows,
            [
                (RowKind::Prompt, "fix the bug\nin main"),
                (RowKind::Reply, "Looking."),
                (RowKind::Reply, "Reading main first."),
                (RowKind::Tool, "Read main.rs"),
                (RowKind::Notice, "interrupted"),
                (RowKind::Notice, "/effort xhigh"),
                (RowKind::Notice, "conversation compacted"),
                (RowKind::Prompt, "! ls"),
                (RowKind::Reply, "Fixed in **main.rs**."),
            ]
        );
        assert_eq!(page.rows[0].ms, Some(1_791_021_600_000), "the record's own time");
        assert_eq!(page.rows[0].at, 0);
        assert_eq!(page.rows[2].at, page.rows[3].at, "one record, two rows");
        assert_eq!((page.from, page.end, page.next_before), (0, body.len() as u64, None));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }
}
