//! Transcript peek (board `p`): the cursor card's latest assistant text,
//! read straight from the session's transcript file. No daemon round-trip —
//! the snapshot already carries `transcript_path`, and the read is observe-
//! tier and read-only, so the single-writer rule (D22) is untouched.
//!
//! Cost model: the board redraws ~10/s (60/s through a glide) and asks per
//! OPEN card — every card with a transcript under `P` — so the draw's
//! steady cost is a `metadata()` call per open card per frame, unless the
//! entry was checked within `FRESH` (T-255), in which case it is a map
//! lookup. The 64 KiB tail re-reads only when the file's (len, mtime) moves. Since T-173 the cache is per PATH,
//! because `App::scan_spoke` reads every paned claude's transcript once a
//! second to learn whether it spoke (`Peek::reply_key`): one `metadata()`
//! per card per second at rest, and for a Running session — whose file
//! moves on every tool result — one reverse scan per second, 64 KiB (256 KiB
//! when the window holds neither reply nor prompt) plus a `serde_json`
//! parse per line walked. Ten busy agents cost the draw thread ~10–20 ms a
//! second. Entries for paths no session names any more are pruned there.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionRecord};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub(crate) use mesimon_daemon::agents::{AgentActivity as Doing, AgentPreview as Peek};

/// Read cache, one entry per transcript path. It was one slot — only the
/// cursor card peeked — until the spoke mark (T-173) made every paned
/// claude's transcript a once-a-second read; see the module doc for what
/// that costs.
#[derive(Default)]
pub(crate) struct PeekCache(RefCell<HashMap<String, Entry>>);

struct Entry {
    kind: SessionKind,
    len: u64,
    mtime_ms: u64,
    peek: Rc<Peek>,
    /// When the file was last stat'ed. A read under `FRESH` after it is
    /// answered from the entry without touching the disk.
    checked_at: Instant,
}

/// How long a stat stays good for. A reply lands on a card at most this
/// late — well inside the 1 s spoke scan, which already bounds how fresh
/// the card's verdict can be.
const FRESH: Duration = Duration::from_millis(250);

impl PeekCache {
    /// Sanitized preview of `path`, re-read only when the file's (len, mtime)
    /// changed since the cached read. Shared, not cloned: the board asks per
    /// open card per frame, and the reply text is the bulk of it.
    #[cfg(test)]
    pub(crate) fn peek(&self, path: &str) -> Option<Rc<Peek>> {
        self.peek_for(SessionKind::Claude, path)
    }

    pub(crate) fn peek_for(&self, kind: SessionKind, path: &str) -> Option<Rc<Peek>> {
        self.read(kind, path, true)
    }

    /// `peek_for` without the stat window: the once-a-second spoke scan
    /// (`App::scan_spoke`) always asks the disk, which is what makes it the
    /// bound on how late a reply can show. Its stat renews the entry, so the
    /// frames that follow it are the ones the window spares.
    pub(crate) fn peek_fresh(&self, kind: SessionKind, path: &str) -> Option<Rc<Peek>> {
        self.read(kind, path, false)
    }

    fn read(&self, kind: SessionKind, path: &str, windowed: bool) -> Option<Rc<Peek>> {
        if !kind.is_agent() {
            return None;
        }
        let now = Instant::now();
        let mut map = self.0.borrow_mut();
        if windowed {
            if let Some(e) = map.get(path) {
                if e.kind == kind && now.duration_since(e.checked_at) < FRESH {
                    return Some(Rc::clone(&e.peek));
                }
            }
        }
        let meta = std::fs::metadata(path).ok()?;
        let len = meta.len();
        let mtime_ms = meta.modified().ok().and_then(mesimon_core::clock::epoch_ms).unwrap_or(0);
        if let Some(e) = map.get_mut(path) {
            if e.kind == kind && e.len == len && e.mtime_ms == mtime_ms {
                e.checked_at = now;
                return Some(Rc::clone(&e.peek));
            }
        }
        let raw = mesimon_daemon::agents::read_preview(kind, Path::new(path))?;
        let peek = Rc::new(Peek {
            text: raw.text.as_deref().map(sanitize),
            activity: raw.activity.map(|d| match d {
                // A step title is a row, never a block: flatten it here.
                Doing::Tool(t) => Doing::Tool(crate::text::one_line(&sanitize(&t))),
                Doing::Thinking => Doing::Thinking,
            }),
            reply_key: raw.reply_key,
        });
        map.insert(
            path.to_string(),
            Entry { kind, len, mtime_ms, peek: Rc::clone(&peek), checked_at: now },
        );
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

    /// Age every stat past `FRESH`, so the next read goes to the disk — a
    /// test's stand-in for the 250 ms a real frame sequence would wait.
    #[cfg(test)]
    pub(crate) fn expire(&self) {
        let old = Instant::now() - FRESH * 2;
        for e in self.0.borrow_mut().values_mut() {
            e.checked_at = old;
        }
    }
}

/// Provider-neutral preview location. Codex's normalized artifact is separate
/// from its native history; a Claude transcript only reaches the Claude reader.
pub(crate) fn preview_path(session: &SessionRecord) -> Option<&str> {
    session.agent_preview_path.as_deref().or(session.transcript_path.as_deref())
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

    #[test]
    fn codex_preview_uses_its_adapter_and_cache_cannot_cross_provider() {
        use mesimon_daemon::agents::{AgentActivity, AgentPreview};
        let path = tmp("codex-provider-cache");
        let preview = AgentPreview {
            text: Some("Reply with https://example.com\u{1b}[31m".into()),
            activity: Some(AgentActivity::Tool("checking\nfiles".into())),
            reply_key: Some(42),
        };
        std::fs::write(&path, serde_json::to_vec(&preview).unwrap()).unwrap();
        let cache = PeekCache::default();
        let name = path.to_str().unwrap();
        let codex = cache.peek_for(SessionKind::Codex, name).unwrap();
        assert_eq!(codex.reply_key, Some(42));
        assert!(!codex.text.as_ref().unwrap().contains('\u{1b}'));
        assert_eq!(codex.activity, Some(Doing::Tool("checking files".into())));
        // Same path and metadata must not return another provider's cached
        // record. Claude's own reader cannot interpret this artifact.
        assert_eq!(cache.peek_for(SessionKind::Claude, name).unwrap().reply_key, None);
        assert_eq!(cache.peek_for(SessionKind::Codex, name).unwrap().reply_key, Some(42));
        assert!(cache.peek_for(SessionKind::Bash, name).is_none());
        std::fs::remove_file(path).unwrap();
    }

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
        // Within the stat window the growth is not yet seen; past it, it is.
        assert_eq!(txt(&cache).as_deref(), Some("one"), "under FRESH, no stat");
        cache.expire();
        assert_eq!(txt(&cache).as_deref(), Some("two"));
        // The scan's read never waits for the window.
        writeln!(f, "{}", reply("u3", "three").trim_end()).unwrap();
        f.flush().unwrap();
        let fresh = cache.peek_fresh(SessionKind::Claude, &path).and_then(|p| p.text.clone());
        assert_eq!(fresh.as_deref(), Some("three"), "peek_fresh stats at once");
        assert_eq!(txt(&cache).as_deref(), Some("three"), "and renews the entry for the draw");
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
        cache.expire();
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
