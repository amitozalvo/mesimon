//! Transcript peek (board `p`): the cursor card's latest assistant text,
//! read straight from the session's transcript file. No daemon round-trip —
//! the snapshot already carries `transcript_path`, and the read is observe-
//! tier and read-only, so the single-writer rule (D22) is untouched.
//!
//! Cost model: the board redraws ~10/s and only ONE card can peek, so the
//! draw's steady cost is a `metadata()` call; the 64 KiB tail re-reads only
//! when the file's (len, mtime) moves. Since T-173 the cache is per PATH,
//! because `App::scan_spoke` reads every paned claude's transcript once a
//! second to learn whether it spoke (`Peek::reply_key`): one `metadata()`
//! per card per second at rest, and for a Running session — whose file
//! moves on every tool result — one reverse scan per second, 64 KiB (256 KiB
//! when the window holds neither reply nor prompt) plus a `serde_json`
//! parse per line walked. Ten busy agents cost the draw thread ~10–20 ms a
//! second. Entries for paths no session names any more are pruned there.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::rc::Rc;

use mesimon_core::adopt::{classify_tail_record, tool_activity, user_prompt, TailEvent};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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

/// What one transcript can say for itself right now.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct Peek {
    /// The agent's last words — or, when the window holds none, the user's
    /// own, prefixed `>`. The census settled on that fallback for the drawer
    /// (daemon/census.rs) because a session's final stretch is routinely
    /// nothing but tool traffic; "nothing" is the one thing a peek must not
    /// say while the transcript plainly has something to show.
    pub(crate) text: Option<String>,
    /// What the agent has done SINCE those words. `None` once the agent has
    /// spoken with nothing after it — the reply is then the whole story, and
    /// a stale step under it would lie.
    pub(crate) activity: Option<Doing>,
    /// WHICH reply `text` is, when it is the agent's: a hash of the assistant
    /// record's `uuid`, so two "Done." replies to two prompts are two replies
    /// and a `/resume` that re-reads the same record is still one. `None`
    /// while `text` is the user's own words — a prompt is not the agent
    /// speaking, and the board's spoke mark (`App::scan_spoke`) keys on this
    /// and nothing else.
    pub(crate) reply_key: Option<u64>,
}

/// What the transcript says the agent is up to, for a session the board
/// already knows is `Running`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Doing {
    /// The newest tool call's own title.
    Tool(String),
    /// Nothing at all since the user's message: it has been handed the
    /// prompt and has neither spoken nor reached for a tool yet.
    Thinking,
}

pub(crate) fn latest_preview(path: &Path) -> Option<Peek> {
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
    Some(Peek {
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
    /// The record `assistant` came from, hashed (see `Peek::reply_key`).
    assistant_key: Option<u64>,
    /// The user's newest message, when it is newer than any reply.
    prompt: Option<String>,
    /// The `last-prompt` latch — a turn stale, so only ever a last resort.
    latch_prompt: Option<String>,
    activity: Option<Doing>,
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
            tail.activity = tool_activity(&v).map(Doing::Tool);
        }
        // A message from the user ends the walk. Whatever the agent said
        // below it answers an older question — showing that reply next to a
        // live spinner is the lie this peek is trying not to tell — and the
        // prompt itself is what the agent is on. Nothing newer than the
        // message means nothing has happened yet: it is thinking.
        if let Some(p) = user_prompt(&v) {
            tail.prompt = Some(p);
            tail.activity.get_or_insert(Doing::Thinking);
            return Some(tail);
        }
        if let TailEvent::AssistantText { text } = classify_tail_record(&v) {
            tail.assistant = Some(text);
            tail.assistant_key = v.get("uuid").and_then(serde_json::Value::as_str).map(record_key);
            return Some(tail);
        }
    }
    Some(tail)
}

/// A record's identity as a number the board can compare and keep.
fn record_key(uuid: &str) -> u64 {
    crate::text::hash64(uuid)
}

/// Read cache, one entry per transcript path. It was one slot — only the
/// cursor card peeked — until the spoke mark (T-173) made every paned
/// claude's transcript a once-a-second read; see the module doc for what
/// that costs.
#[derive(Default)]
pub(crate) struct PeekCache(RefCell<HashMap<String, Entry>>);

struct Entry {
    len: u64,
    mtime_ms: u64,
    peek: Rc<Peek>,
}

impl PeekCache {
    /// Sanitized preview of `path`, re-read only when the file's (len, mtime)
    /// changed since the cached read. Shared, not cloned: the board asks per
    /// open card per frame, and the reply text is the bulk of it.
    pub(crate) fn peek(&self, path: &str) -> Option<Rc<Peek>> {
        let meta = std::fs::metadata(path).ok()?;
        let len = meta.len();
        let mtime_ms = meta.modified().ok().and_then(mesimon_core::clock::epoch_ms).unwrap_or(0);
        let mut map = self.0.borrow_mut();
        if let Some(e) = map.get(path) {
            if e.len == len && e.mtime_ms == mtime_ms {
                return Some(Rc::clone(&e.peek));
            }
        }
        let raw = latest_preview(Path::new(path))?;
        let peek = Rc::new(Peek {
            text: raw.text.as_deref().map(sanitize),
            activity: raw.activity.map(|d| match d {
                // A step title is a row, never a block: flatten it here.
                Doing::Tool(t) => Doing::Tool(crate::text::one_line(&sanitize(&t))),
                Doing::Thinking => Doing::Thinking,
            }),
            reply_key: raw.reply_key,
        });
        map.insert(path.to_string(), Entry { len, mtime_ms, peek: Rc::clone(&peek) });
        Some(peek)
    }

    /// Drop every entry whose path `keep` refuses — the scan calls it with
    /// the set of transcripts the snapshot still names, so a session that
    /// left the board takes its 64 KiB of cached tail with it.
    pub(crate) fn retain(&self, keep: impl Fn(&str) -> bool) {
        self.0.borrow_mut().retain(|p, _| keep(p));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.borrow().len()
    }
}

/// Strip what a card row must never carry: control chars and the
/// drawn-structure range 0x2500–0x259F,
/// which the L1 law bans anywhere on the board and which transcript text is
/// full of the moment the agent prints a table. Also dropped: the invisible
/// width hazards — VS15/VS16 (U+FE0F turns a narrow symbol into a two-cell
/// emoji the width crate still counts as one), ZWJ and the other zero-width
/// format chars, and the combining keycap. A terminal-vs-unicode-width
/// disagreement on a peek row shifts every later cell of the selected
/// surface one column right, stranding a `selected_bg` cell past the card
/// edge that the diff never repaints (dogfood 2026-08-30, same trap as the
/// ☰ plan mark).
///
/// Newlines SURVIVE (author 2026-08-31): the ticket page renders the reply as
/// rich text (rich.rs), and a reply's block structure — its bullets, its
/// fences, its paragraphs — is carried entirely by them. The board card is
/// unaffected: `wrap` splits on whitespace, so a newline was only ever a word
/// break there. Anything that must stay on ONE row flattens at its own
/// boundary (`text::one_line`), which is where that call belongs.
pub(crate) fn sanitize(s: &str) -> String {
    mesimon_core::text::scrub_cells(s, true)
}

/// Greedy display-cell word wrap to at most `max_lines`; a word wider than
/// `width` hard-splits by grapheme. When content is cut, the last line ends
/// in the `~` marker (07 §4.1's vocabulary, via `text::truncate`).
pub(crate) fn wrap(s: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    let mut truncated = false;
    'words: for word in s.split_whitespace() {
        let mut word = word.to_string();
        loop {
            let ww = word.width();
            let sep = if cur_w == 0 { 0 } else { 1 };
            if cur_w + sep + ww <= width {
                if sep == 1 {
                    cur.push(' ');
                }
                cur.push_str(&word);
                cur_w += sep + ww;
                continue 'words;
            }
            // Line is full: flush, or hard-split an over-wide word.
            if cur_w > 0 {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
                if lines.len() == max_lines {
                    truncated = true;
                    break 'words;
                }
                continue;
            }
            let mut head = String::new();
            let mut head_w = 0usize;
            for g in word.graphemes(true) {
                let gw = g.width();
                if head_w + gw > width {
                    break;
                }
                head_w += gw;
                head.push_str(g);
            }
            if head.is_empty() {
                continue 'words; // a single cluster wider than the column
            }
            lines.push(head.clone());
            if lines.len() == max_lines {
                truncated = true;
                break 'words;
            }
            word = word[head.len()..].to_string();
        }
    }
    if cur_w > 0 {
        if lines.len() < max_lines {
            lines.push(cur);
        } else {
            truncated = true;
        }
    }
    if truncated {
        if let Some(last) = lines.last_mut() {
            if last.width() < width {
                last.push('~');
            } else {
                *last = crate::text::truncate(last, width);
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("msmn-peek-{name}-{}", std::process::id()));
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
        assert_eq!(pk.activity, Some(Doing::Tool("Count the lines".into())));
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
        assert_eq!(pk.activity, Some(Doing::Tool("Count the lines".into())));
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
        assert_eq!(pk.activity, Some(Doing::Thinking));

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
        assert_eq!(pk.activity, Some(Doing::Tool("Read card.rs".into())));

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
        assert_eq!(pk.activity, Some(Doing::Tool("Count the lines".into())));
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
        assert_eq!(pk.activity, Some(Doing::Tool("Read peek.rs".into())));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn cache_rereads_only_on_growth() {
        let p = tmp("cache");
        std::fs::write(
            &p,
            "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"one\"}]}}\n",
        )
        .unwrap();
        let cache = PeekCache::default();
        let path = p.to_string_lossy().to_string();
        let txt = |c: &PeekCache| c.peek(&path).and_then(|p| p.text.clone());
        assert_eq!(txt(&cache).as_deref(), Some("one"));
        assert_eq!(txt(&cache).as_deref(), Some("one"));
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(
            f,
            "{{\"uuid\":\"u2\",\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"two\"}}]}}}}"
        )
        .unwrap();
        f.flush().unwrap();
        assert_eq!(txt(&cache).as_deref(), Some("two"));
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

    /// One entry per path: two transcripts are cached side by side, each
    /// invalidated by its own file, and `retain` drops one without the other.
    #[test]
    fn cache_holds_every_path_and_prunes_on_request() {
        let a = tmp("cache-a");
        let b = tmp("cache-b");
        std::fs::write(&a, reply("a1", "alpha")).unwrap();
        std::fs::write(&b, reply("b1", "beta")).unwrap();
        let cache = PeekCache::default();
        let (pa, pb) = (a.to_string_lossy().to_string(), b.to_string_lossy().to_string());
        assert_eq!(cache.peek(&pa).unwrap().text.as_deref(), Some("alpha"));
        assert_eq!(cache.peek(&pb).unwrap().text.as_deref(), Some("beta"));
        assert_eq!(cache.len(), 2, "reading b must not evict a");
        // A's growth is visible through len alone, whatever the filesystem's
        // mtime granularity.
        let mut f = std::fs::OpenOptions::new().append(true).open(&a).unwrap();
        write!(f, "{}", reply("a2", "alpha two")).unwrap();
        f.flush().unwrap();
        assert_eq!(cache.peek(&pa).unwrap().text.as_deref(), Some("alpha two"));
        assert_eq!(cache.peek(&pb).unwrap().text.as_deref(), Some("beta"));
        cache.retain(|p| p == pb);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.peek(&pa).unwrap().text.as_deref(), Some("alpha two"), "a re-reads");
        std::fs::remove_dir_all(a.parent().unwrap()).ok();
        std::fs::remove_dir_all(b.parent().unwrap()).ok();
    }

    #[test]
    fn sanitize_strips_structure_and_control() {
        // The newline lives (rich.rs needs the block structure); the tab,
        // the bell and the drawn glyph do not.
        assert_eq!(sanitize("a\u{2502}b\nc\td\u{7}e"), "ab\nc de");
    }

    #[test]
    fn sanitize_strips_width_hazards() {
        // VS16 emoji presentation, ZWJ sequences, keycaps, BOM: the base
        // chars survive, the invisible width-flippers don't.
        assert_eq!(sanitize("done \u{2705} ok \u{26A0}\u{FE0F}!"), "done \u{2705} ok \u{26A0}!");
        assert_eq!(sanitize("a\u{200D}b\u{200B}c 1\u{FE0F}\u{20E3}"), "abc 1");
        assert_eq!(sanitize("\u{FEFF}x\u{2060}y"), "xy");
    }

    #[test]
    fn wrap_breaks_on_words_and_marks_the_cut() {
        assert_eq!(wrap("one two three four", 9, 2), vec!["one two", "three~"]);
        assert_eq!(wrap("short", 10, 3), vec!["short"]);
        assert!(wrap("", 10, 3).is_empty());
        // An over-wide word hard-splits instead of vanishing.
        assert_eq!(wrap("abcdefgh", 4, 3), vec!["abcd", "efgh"]);
    }
}
