//! Transcript peek (board `p`): the cursor card's latest assistant text,
//! read straight from the session's transcript file. No daemon round-trip —
//! the snapshot already carries `transcript_path`, and the read is observe-
//! tier and read-only, so the single-writer rule (D22) is untouched.
//!
//! Cost model: the board redraws ~10/s and only ONE card can peek, so the
//! steady cost is a `metadata()` call; the 64 KiB tail re-reads only when
//! the file's (len, mtime) moves.

use std::cell::RefCell;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use mesimon_core::adopt::{classify_tail_record, TailEvent};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The last assistant text block in the final 64 KiB — the census's
/// buried-preview scan (daemon/census.rs), re-walked here at draw time.
pub(crate) fn latest_assistant_text(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(64 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // the window may open mid-record
    }
    for line in lines.iter().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if let TailEvent::AssistantText { text } = classify_tail_record(&v) {
            return Some(text);
        }
    }
    None
}

/// One-entry read cache: only the cursor card peeks, so one slot suffices.
#[derive(Default)]
pub(crate) struct PeekCache(RefCell<Option<Entry>>);

struct Entry {
    path: PathBuf,
    len: u64,
    mtime_ms: u64,
    text: Option<String>,
}

impl PeekCache {
    /// Sanitized latest assistant text of `path`, re-read only when the
    /// file's (len, mtime) changed since the cached read.
    pub(crate) fn text(&self, path: &str) -> Option<String> {
        let meta = std::fs::metadata(path).ok()?;
        let len = meta.len();
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut slot = self.0.borrow_mut();
        if let Some(e) = slot.as_ref() {
            if e.path.as_os_str() == std::ffi::OsStr::new(path)
                && e.len == len
                && e.mtime_ms == mtime_ms
            {
                return e.text.clone();
            }
        }
        let text = latest_assistant_text(Path::new(path)).map(|t| sanitize(&t));
        let out = text.clone();
        *slot = Some(Entry { path: PathBuf::from(path), len, mtime_ms, text });
        out
    }
}

/// Strip what a card row must never carry: control chars (newlines become
/// spaces — the wrap re-breaks) and the drawn-structure range 0x2500–0x259F,
/// which the L1 law bans anywhere on the board and which transcript text is
/// full of the moment the agent prints a table. Also dropped: the invisible
/// width hazards — VS15/VS16 (U+FE0F turns a narrow symbol into a two-cell
/// emoji the width crate still counts as one), ZWJ and the other zero-width
/// format chars, and the combining keycap. A terminal-vs-unicode-width
/// disagreement on a peek row shifts every later cell of the selected
/// surface one column right, stranding a `selected_bg` cell past the card
/// edge that the diff never repaints (dogfood 2026-08-30, same trap as the
/// ☰ plan mark).
fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let cp = c as u32;
        if (0x2500..=0x259F).contains(&cp)
            || (0xFE00..=0xFE0F).contains(&cp) // variation selectors
            || (0x200B..=0x200F).contains(&cp) // zero-width space/joiners/marks
            || cp == 0x2060 // word joiner
            || cp == 0xFEFF // BOM / zero-width no-break space
            || cp == 0x20E3 // combining enclosing keycap
        {
            continue;
        }
        if c == '\n' || c == '\t' {
            out.push(' ');
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out
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

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("msmn-peek-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("t.jsonl")
    }

    #[test]
    fn finds_assistant_text_buried_under_latches() {
        let p = tmp("buried");
        std::fs::write(
            &p,
            "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"old reply\"}]}}\n\
             {\"uuid\":\"u2\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"latest reply\"}]}}\n\
             {\"uuid\":\"u3\",\"type\":\"system\",\"subtype\":\"turn_duration\"}\n\
             {\"type\":\"latch\"}\nnot json\n",
        )
        .unwrap();
        assert_eq!(latest_assistant_text(&p).as_deref(), Some("latest reply"));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn missing_or_textless_transcript_is_none() {
        assert_eq!(latest_assistant_text(Path::new("/nonexistent/x.jsonl")), None);
        let p = tmp("textless");
        std::fs::write(&p, "{\"type\":\"latch\"}\n").unwrap();
        assert_eq!(latest_assistant_text(&p), None);
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
        assert_eq!(cache.text(&path).as_deref(), Some("one"));
        assert_eq!(cache.text(&path).as_deref(), Some("one"));
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(
            f,
            "{{\"uuid\":\"u2\",\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"two\"}}]}}}}"
        )
        .unwrap();
        f.flush().unwrap();
        assert_eq!(cache.text(&path).as_deref(), Some("two"));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn sanitize_strips_structure_and_control() {
        assert_eq!(sanitize("a\u{2502}b\nc\td\u{7}e"), "ab c de");
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
