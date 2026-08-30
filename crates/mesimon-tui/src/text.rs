//! Cell-budget text handling (07 §4.1): truncation is by grapheme cluster to a
//! measured display-cell budget, marked with ASCII `~` — one cell, every font,
//! every locale. A string that fits exactly is returned untouched.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Truncate `s` to at most `max` display cells. When truncation happens the
/// last cell is the `~` marker. Never splits a grapheme cluster; a wide
/// cluster that would straddle the budget is dropped entirely.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let budget = max - 1; // reserve the marker cell
    let mut width = 0usize;
    let mut out = String::new();
    for g in s.graphemes(true) {
        let w = g.width();
        if width + w > budget {
            break;
        }
        width += w;
        out.push_str(g);
    }
    out.push('~');
    out
}

/// The 3-cell age slot (06 §5.4): fixed vocabulary, right-aligned by the
/// caller. `now` under 10 s, then s/m/h/d/w buckets, `>1y` past a year.
///
/// `ticking` gates the seconds band: only a `Running` session earns a
/// per-second count-up (watching an agent work). Everywhere else the
/// sub-minute range holds a stable `now` — an idle board must not tick.
pub(crate) fn age_slot(now_ms: u64, then_ms: u64, ticking: bool) -> String {
    let secs = now_ms.saturating_sub(then_ms) / 1000;
    match secs {
        0..=9 => "now".into(),
        10..=99 if !ticking => "now".into(),
        10..=99 => format!("{secs}s"),
        100..=5_999 => format!("{}m", (secs / 60).max(1)),
        6_000..=86_399 => format!("{}h", secs / 3600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=31_535_999 => format!("{}w", secs / 604_800),
        _ => ">1y".into(),
    }
}

/// A marquee window into `s`: skip `offset` display cells (whole grapheme
/// clusters), then hard-clip to `max` cells with no truncation marker — the
/// motion itself says "there is more". `offset == 0` with an overflowing
/// string is the caller's cue to use `truncate` instead.
pub(crate) fn marquee_window(s: &str, max: usize, offset: usize) -> String {
    let mut skipped = 0usize;
    let mut width = 0usize;
    let mut out = String::new();
    for g in s.graphemes(true) {
        let w = g.width();
        if skipped < offset {
            skipped += w;
            continue;
        }
        if width + w > max {
            break;
        }
        width += w;
        out.push_str(g);
    }
    out
}

/// How far a marquee has scrolled at `elapsed_ms`, for a title `overflow`
/// cells too wide: hold at the start, walk one cell per step to the end,
/// hold, then rest at 0 for good — one pass per cursor landing, the clock
/// resets when the cursor returns to the card.
pub(crate) fn marquee_offset(elapsed_ms: u64, overflow: usize) -> usize {
    const STEP_MS: u64 = 200;
    const HOLD_STEPS: u64 = 6;
    if overflow == 0 {
        return 0;
    }
    let cycle = HOLD_STEPS + overflow as u64 + HOLD_STEPS;
    let step = elapsed_ms / STEP_MS;
    if step >= cycle {
        return 0;
    }
    // Steps 0..HOLD hold at 0; each step after walks one cell.
    (step.saturating_sub(HOLD_STEPS - 1) as usize).min(overflow)
}

/// In-place title editing: the text plus a byte cursor kept on grapheme
/// boundaries. Word ops are whitespace-delimited (readline's unix-word):
/// a title is prose, not code, so `-`/`_` stay inside a word.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EditBuffer {
    text: String,
    cursor: usize,
}

impl EditBuffer {
    pub(crate) fn new() -> Self {
        Self { text: String::new(), cursor: 0 }
    }

    /// Start editing existing text, cursor at the end.
    pub(crate) fn from_text(text: String) -> Self {
        let cursor = text.len();
        Self { text, cursor }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    pub(crate) fn into_text(self) -> String {
        self.text
    }

    /// Display cells left of the cursor — the renderer's scroll anchor.
    pub(crate) fn width_before_cursor(&self) -> usize {
        self.text[..self.cursor].width()
    }

    fn prev_boundary(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map(|g| self.cursor + g.len())
            .unwrap_or(self.text.len())
    }

    /// Backward over any whitespace, then over the word — its start.
    fn word_start_before(&self) -> usize {
        let mut idx = self.cursor;
        let mut in_word = false;
        for (i, ch) in self.text[..self.cursor].char_indices().rev() {
            if ch.is_whitespace() {
                if in_word {
                    break;
                }
            } else {
                in_word = true;
            }
            idx = i;
        }
        idx
    }

    /// Forward over any whitespace, then over the word — just past its end.
    fn word_end_after(&self) -> usize {
        let mut in_word = false;
        for (i, ch) in self.text[self.cursor..].char_indices() {
            if ch.is_whitespace() {
                if in_word {
                    return self.cursor + i;
                }
            } else {
                in_word = true;
            }
        }
        self.text.len()
    }

    pub(crate) fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub(crate) fn backspace(&mut self) {
        let start = self.prev_boundary();
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    pub(crate) fn delete(&mut self) {
        let end = self.next_boundary();
        self.text.drain(self.cursor..end);
    }

    pub(crate) fn delete_word_back(&mut self) {
        let start = self.word_start_before();
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    pub(crate) fn kill_to_start(&mut self) {
        self.text.drain(..self.cursor);
        self.cursor = 0;
    }

    pub(crate) fn left(&mut self) {
        self.cursor = self.prev_boundary();
    }

    pub(crate) fn right(&mut self) {
        self.cursor = self.next_boundary();
    }

    pub(crate) fn word_left(&mut self) {
        self.cursor = self.word_start_before();
    }

    pub(crate) fn word_right(&mut self) {
        self.cursor = self.word_end_after();
    }

    pub(crate) fn home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn end(&mut self) {
        self.cursor = self.text.len();
    }
}

/// The visible window of an edit buffer under a `budget` of cells: scrolls
/// left only as far as needed to keep the cursor inside, so mid-string edits
/// show their left context. Returns (shown text, cursor x within the window).
pub(crate) fn edit_window(text: &str, w_before: usize, budget: usize) -> (String, u16) {
    let skip = w_before.saturating_sub(budget);
    let shown = if skip == 0 && text.width() <= budget {
        text.to_string()
    } else {
        marquee_window(text, budget, skip)
    };
    (shown, w_before.saturating_sub(skip) as u16)
}

/// `created_at` → epoch ms. The daemon writes `@<epoch-secs>` (server.rs
/// `now_iso`); RFC3339 (`2026-08-29T17:04:41Z`, offset forms) is accepted
/// for compatibility. None on anything unparsable — no age renders then.
pub(crate) fn created_at_epoch_ms(s: &str) -> Option<u64> {
    if let Some(rest) = s.strip_prefix('@') {
        return rest.parse::<u64>().ok().map(|secs| secs * 1000);
    }
    let bytes = s.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Timezone tail: 'Z', or ±HH:MM after an optional fractional part.
    let tail = &s[19..];
    let tz_pos = tail.find(['Z', '+', '-']);
    let tz_secs = match tz_pos.map(|p| &tail[p..]) {
        Some("Z") | None => 0i64,
        Some(t) if t.len() >= 6 => {
            let sign = if t.starts_with('-') { -1 } else { 1 };
            let th = t.get(1..3)?.parse::<i64>().ok()?;
            let tm = t.get(4..6)?.parse::<i64>().ok()?;
            sign * (th * 3600 + tm * 60)
        }
        _ => return None,
    };
    // Howard Hinnant's days_from_civil.
    let (y, mo) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (mo - 3) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - tz_secs;
    u64::try_from(secs).ok().map(|s| s * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_fit_is_untouched() {
        // The old char-based truncate could never return a string of width
        // == max; this is the off-by-one regression test.
        assert_eq!(truncate("abcde", 5), "abcde");
        assert_eq!(truncate("abcde", 6), "abcde");
    }

    #[test]
    fn overflow_gets_ascii_marker() {
        assert_eq!(truncate("abcdef", 5), "abcd~");
        assert_eq!(truncate("abcdef", 1), "~");
        assert_eq!(truncate("abcdef", 0), "");
    }

    #[test]
    fn wide_cluster_never_straddles() {
        // "你" is 2 cells; budget 4 → marker at cell 4, cluster dropped whole.
        assert_eq!(truncate("你好吗", 4), "你~");
        assert_eq!(truncate("你好吗", 6), "你好吗");
    }

    #[test]
    fn combining_cluster_stays_whole() {
        // e + combining acute is one cluster, one cell.
        let s = "cafe\u{301} bar";
        assert_eq!(truncate(s, 5), "cafe\u{301}~");
    }

    #[test]
    fn hebrew_stays_within_budget() {
        use unicode_width::UnicodeWidthStr;
        let s = "תיקון הפניית אימות";
        let t = truncate(s, 10);
        assert!(t.width() <= 10);
        assert!(t.ends_with('~'));
    }

    #[test]
    fn marquee_walks_and_holds() {
        // 6-step hold, then one cell per 200 ms, capped at overflow.
        assert_eq!(marquee_offset(0, 5), 0);
        assert_eq!(marquee_offset(1199, 5), 0); // still holding
        assert_eq!(marquee_offset(1200, 5), 1);
        assert_eq!(marquee_offset(2000, 5), 5);
        assert_eq!(marquee_offset(2199, 5), 5); // end hold
        assert_eq!(marquee_offset(3399, 5), 5); // last step of end hold
        assert_eq!(marquee_offset(3400, 5), 0); // one pass done, rest at 0
        assert_eq!(marquee_offset(60_000, 5), 0); // no loop
        assert_eq!(marquee_offset(0, 0), 0);
        assert_eq!(marquee_window("abcdefgh", 4, 2), "cdef");
        assert_eq!(marquee_window("你好吗x", 4, 2), "好吗");
    }

    #[test]
    fn edit_word_delete() {
        // ctrl+backspace at the end: word goes, trailing space too.
        let mut b = EditBuffer::from_text("fix auth bug".into());
        b.delete_word_back();
        assert_eq!(b.as_str(), "fix auth ");
        b.delete_word_back();
        assert_eq!(b.as_str(), "fix ");
        // Mid-string: only the word left of the cursor dies.
        let mut b = EditBuffer::from_text("fix auth bug".into());
        b.word_left(); // cursor before "bug"
        b.delete_word_back();
        assert_eq!(b.as_str(), "fix bug");
        // Empty and all-whitespace never panic.
        let mut b = EditBuffer::new();
        b.delete_word_back();
        assert_eq!(b.as_str(), "");
        let mut b = EditBuffer::from_text("   ".into());
        b.delete_word_back();
        assert_eq!(b.as_str(), "");
    }

    #[test]
    fn edit_word_jump() {
        let mut b = EditBuffer::from_text("fix auth bug".into());
        b.word_left();
        assert_eq!(b.width_before_cursor(), 9); // before "bug"
        b.word_left();
        assert_eq!(b.width_before_cursor(), 4); // before "auth"
        b.word_left();
        b.word_left(); // clamped at start
        assert_eq!(b.width_before_cursor(), 0);
        b.word_right();
        assert_eq!(b.width_before_cursor(), 3); // after "fix"
        b.word_right();
        assert_eq!(b.width_before_cursor(), 8); // after "auth"
        b.word_right();
        b.word_right(); // clamped at end
        assert_eq!(b.width_before_cursor(), 12);
    }

    #[test]
    fn edit_cursor_insert_delete() {
        let mut b = EditBuffer::from_text("abd".into());
        b.left();
        b.insert('c');
        assert_eq!(b.as_str(), "abcd");
        b.home();
        b.delete();
        assert_eq!(b.as_str(), "bcd");
        b.end();
        b.backspace();
        assert_eq!(b.as_str(), "bc");
        b.kill_to_start();
        assert_eq!(b.as_str(), "");
        // Grapheme moves: combining accent travels with its base.
        let mut b = EditBuffer::from_text("cafe\u{301}!".into());
        b.left();
        b.backspace();
        assert_eq!(b.as_str(), "caf!");
    }

    #[test]
    fn edit_window_keeps_cursor_visible() {
        // Fits: no scroll, cursor at its true column.
        assert_eq!(edit_window("abc", 3, 10), ("abc".into(), 3));
        assert_eq!(edit_window("abc", 1, 10), ("abc".into(), 1));
        // Overflow with cursor at the end: tail shown, cursor pinned right.
        assert_eq!(edit_window("abcdefgh", 8, 4), ("efgh".into(), 4));
        // Overflow with cursor at the start: head shown, no scroll.
        assert_eq!(edit_window("abcdefgh", 0, 4), ("abcd".into(), 0));
        assert_eq!(edit_window("abcdefgh", 2, 4), ("abcd".into(), 2));
    }

    #[test]
    fn created_at_parses() {
        // The daemon's own form.
        assert_eq!(created_at_epoch_ms("@0"), Some(0));
        assert_eq!(created_at_epoch_ms("@86400"), Some(86_400_000));
        // RFC3339 compatibility.
        assert_eq!(created_at_epoch_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(created_at_epoch_ms("1970-01-02T00:00:00Z"), Some(86_400_000));
        assert_eq!(
            created_at_epoch_ms("2026-08-29T00:00:00Z"),
            Some(1_787_961_600_000)
        );
        assert_eq!(created_at_epoch_ms("1970-01-01T02:00:00+02:00"), Some(0));
        assert_eq!(created_at_epoch_ms("1970-01-01T00:00:00.123Z"), Some(0));
        assert_eq!(created_at_epoch_ms("garbage"), None);
        assert_eq!(created_at_epoch_ms("@garbage"), None);
    }

    #[test]
    fn age_vocabulary() {
        let s = 1000u64;
        assert_eq!(age_slot(9 * s, 0, true), "now");
        assert_eq!(age_slot(45 * s, 0, true), "45s");
        assert_eq!(age_slot(180 * s, 0, true), "3m");
        assert_eq!(age_slot(4 * 3600 * s, 0, true), "4h");
        assert_eq!(age_slot(2 * 86_400 * s, 0, true), "2d");
        assert_eq!(age_slot(21 * 86_400 * s, 0, true), "3w");
        assert_eq!(age_slot(400 * 86_400 * s, 0, true), ">1y");
        // Every word fits the 3-cell slot.
        for t in [0, 9, 45, 180, 14_400, 172_800, 1_814_400, 40_000_000] {
            assert!(age_slot(t * s, 0, true).len() <= 3, "{}", age_slot(t * s, 0, true));
        }
    }

    #[test]
    fn age_seconds_only_tick_when_running() {
        // Non-running sessions hold a stable `now` through the whole seconds
        // band — an idle board must not repaint every second.
        let s = 1000u64;
        assert_eq!(age_slot(45 * s, 0, false), "now");
        assert_eq!(age_slot(99 * s, 0, false), "now");
        // The minute band and up is identical either way.
        assert_eq!(age_slot(180 * s, 0, false), "3m");
        assert_eq!(age_slot(400 * 86_400 * s, 0, false), ">1y");
    }
}
