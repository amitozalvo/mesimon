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

/// Flatten arbitrary text into one renderable status line. Ratatui spans must
/// never carry control characters — a raw `\n` flushed to the terminal moves
/// the real cursor while the diff buffer thinks nothing happened, and the
/// desync leaves stale cells on every later frame (the multi-line git stderr
/// a merge refusal ships did exactly that). Whitespace runs collapse to one
/// space so `error:\n\tfile` reads as a sentence, not a gap.
pub(crate) fn one_line(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
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

/// The ticket page's "time in column" clause, from the same slot the card
/// wears, spoken right after the column word: `IN PROGRESS for 3d`. It was
/// `3d here`, and "here" never read as "in this column"; a slot of `now`
/// takes no preposition at all, so a fresh arrival is `just now` (author
/// 2026-09-03).
pub(crate) fn age_in_column(now_ms: u64, then_ms: u64) -> String {
    match age_slot(now_ms, then_ms, false).as_str() {
        "now" => "just now".into(),
        age => format!("for {age}"),
    }
}

/// The age slot's vocabulary pointed forward: how long until `then`. A
/// snoozed ticket's row in the archived dialog reads `wakes in 3h`; once the
/// deadline has passed (the tick wheel is about to act) it reads `wakes
/// now`.
pub(crate) fn until_word(now_ms: u64, then_ms: u64) -> String {
    if then_ms <= now_ms {
        return "now".into();
    }
    // The slot never ticks seconds: "in 40s" is not a promise worth making.
    match age_slot(then_ms, now_ms, false).as_str() {
        "now" => "in <2m".into(),
        age => format!("in {age}"),
    }
}

/// The ticket page's "created …" clause. `created now ago` was the slot
/// read literally; `just now` is what a person says.
pub(crate) fn age_created(now_ms: u64, then_ms: u64) -> String {
    match age_slot(now_ms, then_ms, false).as_str() {
        "now" => "created just now".into(),
        age => format!("created {age} ago"),
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

/// The grapheme boundary before `cursor` in `text`; 0 at the start. Shared
/// by both fields: a cursor that lands inside a cluster splits an accent
/// from its base on the next keystroke, on one line or many.
fn prev_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor].grapheme_indices(true).next_back().map(|(i, _)| i).unwrap_or(0)
}

/// The grapheme boundary after `cursor`; the text's end at the end.
fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..].graphemes(true).next().map(|g| cursor + g.len()).unwrap_or(text.len())
}

/// Backward over any whitespace, then over the word — its start.
fn word_start_before(text: &str, cursor: usize) -> usize {
    let mut idx = cursor;
    let mut in_word = false;
    for (i, ch) in text[..cursor].char_indices().rev() {
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
fn word_end_after(text: &str, cursor: usize) -> usize {
    let mut in_word = false;
    for (i, ch) in text[cursor..].char_indices() {
        if ch.is_whitespace() {
            if in_word {
                return cursor + i;
            }
        } else {
            in_word = true;
        }
    }
    text.len()
}

/// In-place title editing: the text plus a byte cursor kept on grapheme
/// boundaries. Word ops are whitespace-delimited (readline's unix-word):
/// a title is prose, not code, so `-`/`_` stay inside a word.
///
/// Every field has a byte `limit`, the same number the daemon caps the text
/// at on arrival (`TITLE_MAX_BYTES`, `TAG_MAX_BYTES`, `PROMPT_MAX_BYTES`), so
/// what the field shows is what the daemon keeps: a cap the field did not
/// mirror would let a pasted document look accepted and land truncated.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EditBuffer {
    text: String,
    cursor: usize,
    limit: usize,
}

/// What a paste did to the field: how much of it went in, and whether the
/// limit cut it — the one case the status line has to say something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pasted {
    pub(crate) trimmed: bool,
}

impl EditBuffer {
    pub(crate) fn new(limit: usize) -> Self {
        Self { text: String::new(), cursor: 0, limit }
    }

    /// Start editing existing text, cursor at the end. Text already past the
    /// limit (an older board, a title minted by another client) is kept
    /// whole — the field refuses to grow it, never eats it.
    pub(crate) fn from_text(text: String, limit: usize) -> Self {
        let cursor = text.len();
        Self { text, cursor, limit }
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    /// Room left under the limit, in bytes.
    fn room(&self) -> usize {
        self.limit.saturating_sub(self.text.len())
    }

    /// A bracketed paste into a one-line field. Every field here is one
    /// line — a title, a tag, a prompt — so the newlines a multi-line paste
    /// carries become spaces rather than Enters (which is what an unbracketed
    /// paste turned them into: the first line saved and the rest typed onto
    /// the board). Whitespace runs collapse and the ends are trimmed, since
    /// the commonest paste is a copied line with its newline still on it.
    /// Inserted at the cursor, grapheme by grapheme, up to the limit; the
    /// cut never splits a cluster.
    pub(crate) fn paste(&mut self, raw: &str) -> Pasted {
        let flat = one_line(raw);
        let mut trimmed = false;
        for g in flat.graphemes(true) {
            if g.len() > self.room() {
                trimmed = true;
                break;
            }
            self.text.insert_str(self.cursor, g);
            self.cursor += g.len();
        }
        Pasted { trimmed }
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

    /// One typed character. Past the limit the key is inert: the same bound
    /// the daemon applies, felt here rather than discovered on save.
    pub(crate) fn insert(&mut self, c: char) {
        if c.len_utf8() > self.room() {
            return;
        }
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub(crate) fn backspace(&mut self) {
        let start = prev_boundary(&self.text, self.cursor);
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    pub(crate) fn delete(&mut self) {
        let end = next_boundary(&self.text, self.cursor);
        self.text.drain(self.cursor..end);
    }

    pub(crate) fn delete_word_back(&mut self) {
        let start = word_start_before(&self.text, self.cursor);
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    pub(crate) fn kill_to_start(&mut self) {
        self.text.drain(..self.cursor);
        self.cursor = 0;
    }

    pub(crate) fn left(&mut self) {
        self.cursor = prev_boundary(&self.text, self.cursor);
    }

    pub(crate) fn right(&mut self) {
        self.cursor = next_boundary(&self.text, self.cursor);
    }

    pub(crate) fn word_left(&mut self) {
        self.cursor = word_start_before(&self.text, self.cursor);
    }

    pub(crate) fn word_right(&mut self) {
        self.cursor = word_end_after(&self.text, self.cursor);
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

/// A multi-line field: a note, not a title. One `String` with `'\n'` inside
/// it rather than a `Vec<String>` of lines, because the limit is a byte cap
/// the daemon applies to the whole text (newlines count), `into_text` is
/// free, and `'\n'` is its own grapheme cluster under UAX 29 — so the
/// boundary routines the one-line field uses work unchanged, and a
/// backspace at column 0 joins lines for nothing. (`"\r\n"` is ONE cluster,
/// which is why `'\r'` is normalised away on every entry road.)
///
/// Clean by construction: nothing a renderer would scrub can get in.
/// `insert` refuses controls, format hazards and cell hazards, `newline` is
/// the only road for a `'\n'`, and `from_text`/`paste` run the same
/// `scrub_cells` a zone runs before drawing. The renderer therefore never
/// scrubs, and the cursor arithmetic can never desync from what is drawn —
/// a scrubbed-out byte the cursor still counted is a cursor one cell off.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextArea {
    /// `'\n'`-separated; never holds `'\r'`, `'\t'` or any hazard char.
    text: String,
    /// Byte offset, always on a grapheme boundary.
    cursor: usize,
    /// The viewport width and sticky display column vertical moves aim for: set on the
    /// first vertical move, kept while the walk continues, cleared by any
    /// horizontal edit or motion — so a walk through a short line comes back
    /// out at the column it went in on. A width change resets the target.
    want_col: Option<(usize, usize)>,
    /// Byte cap over the WHOLE text, newlines included.
    limit: usize,
}

impl TextArea {
    pub(crate) fn new(limit: usize) -> Self {
        Self { text: String::new(), cursor: 0, want_col: None, limit }
    }

    /// Start editing existing text, cursor at 0 — a note opens at its top.
    /// Line endings are normalised and hazards scrubbed on the way in; text
    /// already past the limit is kept whole, the field just refuses to grow
    /// it (the same rule as `EditBuffer::from_text`).
    pub(crate) fn from_text(text: &str, limit: usize) -> Self {
        Self { text: Self::clean(text), cursor: 0, want_col: None, limit }
    }

    /// `"\r\n"` and a lone `'\r'` become `'\n'`, then `scrub_cells` keeps the
    /// newlines and turns tabs into spaces. The order matters: scrubbing
    /// first would drop the `'\r'` of a `"\r\n"` and the pair's newline would
    /// survive, but a lone `'\r'` (an old Mac file) would vanish with its
    /// line break.
    fn clean(raw: &str) -> String {
        let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
        mesimon_core::text::scrub_cells(&unified, true)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    #[cfg(test)]
    pub(crate) fn into_text(self) -> String {
        self.text
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Bytes, the unit the limit is in.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.text.len()
    }

    /// Byte offset of the cursor.
    #[cfg(test)]
    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    /// Room left under the limit, in bytes.
    fn room(&self) -> usize {
        self.limit.saturating_sub(self.text.len())
    }

    /// Byte offset where the cursor's line starts.
    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    /// Byte offset where the cursor's line ends (at its `'\n'`, or the
    /// text's end).
    fn line_end(&self) -> usize {
        self.text[self.cursor..].find('\n').map_or(self.text.len(), |i| self.cursor + i)
    }

    /// Zero-based line the cursor is on.
    pub(crate) fn cursor_line(&self) -> usize {
        self.text[..self.cursor].matches('\n').count()
    }

    /// Display cells before the cursor on its line — the column, in the
    /// renderer's unit.
    #[cfg(test)]
    pub(crate) fn cursor_col_cells(&self) -> usize {
        self.text[self.line_start()..self.cursor].width()
    }

    /// An empty text has one empty line; a trailing `'\n'` yields a trailing
    /// empty line — what `lines()` yields, and what the window draws.
    #[cfg(test)]
    pub(crate) fn line_count(&self) -> usize {
        self.text.split('\n').count()
    }

    pub(crate) fn lines(&self) -> impl Iterator<Item = &str> {
        self.text.split('\n')
    }

    /// One typed character. Controls (a `'\n'` only ever arrives through
    /// `newline`), format hazards and cell hazards are inert, and so is
    /// anything past the limit.
    pub(crate) fn insert(&mut self, c: char) {
        use mesimon_core::text::{is_cell_hazard, is_format_hazard};
        if c.is_control() || is_format_hazard(c) || is_cell_hazard(c) {
            return;
        }
        if c.len_utf8() > self.room() {
            return;
        }
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.want_col = None;
    }

    /// Enter. A newline is a byte under the limit like any other.
    pub(crate) fn newline(&mut self) {
        if self.room() == 0 {
            return;
        }
        self.text.insert(self.cursor, '\n');
        self.cursor += 1;
        self.want_col = None;
    }

    /// A bracketed paste. Unlike `EditBuffer::paste` the newlines are KEPT —
    /// this is a multi-line field, and a pasted note keeps its shape — but
    /// the line endings are unified and the hazards scrubbed first, then the
    /// text goes in grapheme by grapheme under the limit; the cut never
    /// splits a cluster.
    pub(crate) fn paste(&mut self, raw: &str) -> Pasted {
        let clean = Self::clean(raw);
        let mut trimmed = false;
        for g in clean.graphemes(true) {
            if g.len() > self.room() {
                trimmed = true;
                break;
            }
            self.text.insert_str(self.cursor, g);
            self.cursor += g.len();
        }
        self.want_col = None;
        Pasted { trimmed }
    }

    /// Grapheme-wise; at column 0 the `'\n'` before the cursor is the
    /// cluster removed, which joins the line to the one above.
    pub(crate) fn backspace(&mut self) {
        let start = prev_boundary(&self.text, self.cursor);
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.want_col = None;
    }

    /// Grapheme-wise; at a line's end it eats the `'\n'` and joins the next
    /// line up.
    pub(crate) fn delete(&mut self) {
        let end = next_boundary(&self.text, self.cursor);
        self.text.drain(self.cursor..end);
        self.want_col = None;
    }

    /// Like the one-line field's, but never past the line start: a `^w` at
    /// column 0 does nothing rather than eating the line above.
    pub(crate) fn delete_word_back(&mut self) {
        let start = self.line_start();
        let start =
            start + word_start_before(&self.text[start..self.line_end()], self.cursor - start);
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.want_col = None;
    }

    /// `^u`: to the LINE start.
    pub(crate) fn kill_to_start(&mut self) {
        let start = self.line_start();
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.want_col = None;
    }

    /// One cluster back, crossing a line break from column 0.
    pub(crate) fn left(&mut self) {
        self.cursor = prev_boundary(&self.text, self.cursor);
        self.want_col = None;
    }

    /// One cluster forward, crossing a line break from a line's end.
    pub(crate) fn right(&mut self) {
        self.cursor = next_boundary(&self.text, self.cursor);
        self.want_col = None;
    }

    /// Bounded by the current line: a word never crosses a break.
    pub(crate) fn word_left(&mut self) {
        let start = self.line_start();
        self.cursor =
            start + word_start_before(&self.text[start..self.line_end()], self.cursor - start);
        self.want_col = None;
    }

    pub(crate) fn word_right(&mut self) {
        let start = self.line_start();
        self.cursor =
            start + word_end_after(&self.text[start..self.line_end()], self.cursor - start);
        self.want_col = None;
    }

    pub(crate) fn home(&mut self) {
        self.cursor = self.line_start();
        self.want_col = None;
    }

    pub(crate) fn end(&mut self) {
        self.cursor = self.line_end();
        self.want_col = None;
    }

    #[cfg(test)]
    pub(crate) fn up(&mut self) {
        self.page(-1);
    }

    #[cfg(test)]
    pub(crate) fn down(&mut self) {
        self.page(1);
    }

    /// Move by logical lines when restoring a position after external editing.
    pub(crate) fn page(&mut self, delta: isize) {
        self.move_rows(delta, usize::MAX);
    }

    pub(crate) fn cursor_row(&self, width: usize) -> usize {
        self.wrapped_rows(width).cursor_row(self.cursor)
    }

    /// Move through displayed rows, retaining the desired cell column across
    /// short rows. A soft boundary belongs to the following row, so landing
    /// at the end of a continuation must stop before that boundary.
    pub(crate) fn move_rows(&mut self, delta: isize, width: usize) {
        let layout = self.wrapped_rows(width);
        let cur = layout.cursor_row(self.cursor);
        let col = self.text[layout.rows[cur].start..self.cursor].width();
        let want = match self.want_col {
            Some((old_width, want)) if old_width == width => want,
            _ => col,
        };
        self.want_col = Some((width, want));
        let target = cur.saturating_add_signed(delta).min(layout.rows.len() - 1);
        if target == cur {
            return;
        }
        let row = &layout.rows[target];
        let end = if layout.rows.get(target + 1).is_some_and(|next| next.start == row.end) {
            prev_boundary(&self.text, row.end)
        } else {
            row.end
        };
        let mut cursor = row.start;
        let mut cells = 0;
        for g in self.text[row.start..end].graphemes(true) {
            if cells + g.width() > want {
                break;
            }
            cells += g.width();
            cursor += g.len();
        }
        self.cursor = cursor;
    }

    /// Byte ranges keep all whitespace and explicit newlines in the buffer.
    /// Prefer breaks after whitespace; split an oversized word only at a
    /// grapheme boundary. Even a viewport narrower than a cluster advances.
    fn wrapped_rows(&self, width: usize) -> AreaLayout {
        let width = width.max(1);
        let mut rows = Vec::new();
        let mut offset = 0;
        for line in self.lines() {
            let mut start = 0;
            while start < line.len() {
                let mut end = start;
                let mut cells = 0;
                let mut word_break = None;
                let mut has_word = false;
                for (i, g) in line[start..].grapheme_indices(true) {
                    if cells + g.width() > width && end > start {
                        break;
                    }
                    cells += g.width();
                    end = start + i + g.len();
                    if g.chars().all(char::is_whitespace) {
                        if has_word {
                            word_break = Some(end);
                        }
                    } else {
                        has_word = true;
                    }
                }
                if end < line.len() {
                    end = word_break.unwrap_or(end);
                }
                rows.push(offset + start..offset + end);
                start = end;
            }
            if line.is_empty() {
                rows.push(offset..offset);
            }
            offset += line.len() + 1;
        }
        AreaLayout { rows }
    }
}

struct AreaLayout {
    rows: Vec<std::ops::Range<usize>>,
}

impl AreaLayout {
    fn cursor_row(&self, cursor: usize) -> usize {
        self.rows.partition_point(|row| row.start <= cursor).saturating_sub(1)
    }
}

/// Soft-wrapped body rows and the hardware cursor within a vertical window.
/// `top` counts visual rows; follow the cursor with the smallest scroll.
/// The caller reserves one extra cell after `width` for an end-of-line cursor.
pub(crate) fn area_window(
    ta: &TextArea,
    top: usize,
    rows: usize,
    width: usize,
) -> (usize, Vec<String>, (u16, u16)) {
    if rows == 0 {
        return (0, Vec::new(), (0, 0));
    }
    let layout = ta.wrapped_rows(width);
    let cur = layout.cursor_row(ta.cursor);
    let count = layout.rows.len();
    let top = top.min(cur).max((cur + 1).saturating_sub(rows)).min(count.saturating_sub(rows));
    let out = layout
        .rows
        .iter()
        .skip(top)
        .take(rows)
        .map(|row| truncate(&ta.text[row.clone()], width))
        .collect();
    let col = ta.text[layout.rows[cur].start..ta.cursor].width().min(width);
    (top, out, ((cur - top) as u16, col as u16))
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

/// The cursor and deletion moves a text field answers to, whether it is the
/// one-line `EditBuffer` or the multi-line `TextArea`: the note editor's
/// keys dispatch on the focused field through this and nothing else.
pub(crate) trait EditOps {
    fn backspace(&mut self);
    fn delete(&mut self);
    fn delete_word_back(&mut self);
    fn kill_to_start(&mut self);
    fn left(&mut self);
    fn right(&mut self);
    fn word_left(&mut self);
    fn word_right(&mut self);
    fn home(&mut self);
    fn end(&mut self);
}

macro_rules! edit_ops {
    ($t:ty) => {
        impl EditOps for $t {
            fn backspace(&mut self) {
                <$t>::backspace(self)
            }
            fn delete(&mut self) {
                <$t>::delete(self)
            }
            fn delete_word_back(&mut self) {
                <$t>::delete_word_back(self)
            }
            fn kill_to_start(&mut self) {
                <$t>::kill_to_start(self)
            }
            fn left(&mut self) {
                <$t>::left(self)
            }
            fn right(&mut self) {
                <$t>::right(self)
            }
            fn word_left(&mut self) {
                <$t>::word_left(self)
            }
            fn word_right(&mut self) {
                <$t>::word_right(self)
            }
            fn home(&mut self) {
                <$t>::home(self)
            }
            fn end(&mut self) {
                <$t>::end(self)
            }
        }
    };
}
edit_ops!(EditBuffer);
edit_ops!(TextArea);

/// A value's identity as one number — a document key the draw can compare
/// and keep (`DefaultHasher`, stable within a run, which is all a key needs).
pub(crate) fn hash64(v: impl std::hash::Hash) -> u64 {
    use std::hash::{DefaultHasher, Hasher};
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
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
    fn one_line_flattens_control_and_whitespace() {
        // The shape a merge refusal ships: git stderr with newlines + tabs.
        assert_eq!(
            one_line("error: Your local changes:\n\tCLAUDE.md\n\tsrc/app.rs\nAborting"),
            "error: Your local changes: CLAUDE.md src/app.rs Aborting"
        );
        assert_eq!(one_line("already ∙ one line"), "already ∙ one line");
        assert_eq!(one_line(""), "");
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
        let mut b = EditBuffer::from_text("fix auth bug".into(), 64);
        b.delete_word_back();
        assert_eq!(b.as_str(), "fix auth ");
        b.delete_word_back();
        assert_eq!(b.as_str(), "fix ");
        // Mid-string: only the word left of the cursor dies.
        let mut b = EditBuffer::from_text("fix auth bug".into(), 64);
        b.word_left(); // cursor before "bug"
        b.delete_word_back();
        assert_eq!(b.as_str(), "fix bug");
        // Empty and all-whitespace never panic.
        let mut b = EditBuffer::new(64);
        b.delete_word_back();
        assert_eq!(b.as_str(), "");
        let mut b = EditBuffer::from_text("   ".into(), 64);
        b.delete_word_back();
        assert_eq!(b.as_str(), "");
    }

    #[test]
    fn edit_word_jump() {
        let mut b = EditBuffer::from_text("fix auth bug".into(), 64);
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
        let mut b = EditBuffer::from_text("abd".into(), 64);
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
        let mut b = EditBuffer::from_text("cafe\u{301}!".into(), 64);
        b.left();
        b.backspace();
        assert_eq!(b.as_str(), "caf!");
    }

    #[test]
    fn paste_is_one_line_at_the_cursor() {
        // The bug: a multi-line paste's newlines arrived as Enters and saved
        // the first line. Now they are spaces, and the copied line's own
        // trailing newline is nothing.
        let mut b = EditBuffer::from_text("fix ".into(), 64);
        assert_eq!(b.paste("the auth\nbug\r\n"), Pasted { trimmed: false });
        assert_eq!(b.as_str(), "fix the auth bug");
        assert_eq!(b.width_before_cursor(), 16);
        // Mid-string, at the cursor, cursor rides to the end of the paste.
        let mut b = EditBuffer::from_text("ab".into(), 64);
        b.left();
        b.paste("x\ty");
        assert_eq!(b.as_str(), "ax yb");
        assert_eq!(b.width_before_cursor(), 4);
        // Blank pastes are nothing.
        let mut b = EditBuffer::new(64);
        assert_eq!(b.paste("\n\n  \n"), Pasted { trimmed: false });
        assert_eq!(b.as_str(), "");
    }

    #[test]
    fn the_limit_holds_for_pastes_and_keys() {
        // A paste past the limit is cut at a cluster boundary and says so.
        let mut b = EditBuffer::new(10);
        assert_eq!(b.paste("abcdefghijklmnop"), Pasted { trimmed: true });
        assert_eq!(b.as_str(), "abcdefghij");
        // Hebrew is two bytes a letter: the cut never splits one.
        let mut b = EditBuffer::new(5);
        assert_eq!(b.paste("שלום"), Pasted { trimmed: true });
        assert_eq!(b.as_str(), "של");
        // A combining cluster goes in whole or not at all.
        let mut b = EditBuffer::new(5);
        b.paste("cafe\u{301}");
        assert_eq!(b.as_str(), "caf");
        // Typing at the limit is inert, and deleting makes room again.
        let mut b = EditBuffer::new(3);
        for c in "abcd".chars() {
            b.insert(c);
        }
        assert_eq!(b.as_str(), "abc");
        b.backspace();
        b.insert('z');
        assert_eq!(b.as_str(), "abz");
        // Text loaded over the limit is kept, just not grown.
        let mut b = EditBuffer::from_text("abcdef".into(), 3);
        b.insert('x');
        assert_eq!(b.as_str(), "abcdef");
        assert_eq!(b.paste("y"), Pasted { trimmed: true });
        assert_eq!(b.as_str(), "abcdef");
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
        assert_eq!(created_at_epoch_ms("2026-08-29T00:00:00Z"), Some(1_787_961_600_000));
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
    }

    #[test]
    fn ticket_page_age_phrases_read_as_sentences() {
        let s = 1000;
        // A slot of "now" never lands next to "for" or "ago".
        assert_eq!(age_in_column(5 * s, 0), "just now");
        assert_eq!(age_created(5 * s, 0), "created just now");
        assert_eq!(age_in_column(45 * s, 0), "just now");
        assert_eq!(age_created(45 * s, 0), "created just now");
        // Past the "now" band the slot is the number the card wears.
        assert_eq!(age_in_column(3 * 86_400 * s, 0), "for 3d");
        assert_eq!(age_created(2 * 604_800 * s, 0), "created 2w ago");
        // The minute band and up is identical either way.
        assert_eq!(age_slot(180 * s, 0, false), "3m");
        assert_eq!(age_slot(400 * 86_400 * s, 0, false), ">1y");
    }

    #[test]
    fn area_newline_and_join() {
        let mut t = TextArea::new(64);
        for c in "ab".chars() {
            t.insert(c);
        }
        t.newline();
        t.insert('c');
        t.newline();
        t.insert('d');
        assert_eq!(t.as_str(), "ab\nc\nd");
        assert_eq!(t.line_count(), 3);
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (2, 1));
        // Backspace at column 0 joins with the line above: the newline is
        // the cluster removed.
        t.home();
        t.backspace();
        assert_eq!(t.as_str(), "ab\ncd");
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 1));
        // Delete at a line's end joins the line below.
        t.up();
        t.end();
        t.delete();
        assert_eq!(t.as_str(), "abcd");
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (0, 2));
        assert_eq!(t.lines().collect::<Vec<_>>(), vec!["abcd"]);
        // An empty text is one empty line; a trailing newline is a trailing
        // empty line.
        assert_eq!(TextArea::new(8).lines().collect::<Vec<_>>(), vec![""]);
        assert_eq!(TextArea::from_text("a\n", 8).lines().collect::<Vec<_>>(), vec!["a", ""]);
    }

    #[test]
    fn area_up_down_keeps_a_sticky_column() {
        let mut t = TextArea::from_text("abcdef\nab\nabcdef", 64);
        for _ in 0..5 {
            t.right();
        }
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (0, 5));
        t.down(); // the short line clamps...
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 2));
        t.down(); // ...and the walk comes back out at the column it went in on.
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (2, 5));
        // Past the last line: no-op, column kept.
        t.down();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (2, 5));
        t.up();
        t.up();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (0, 5));
        // A horizontal move clears the stickiness: the next walk starts
        // from where the cursor now is.
        t.left();
        t.down();
        t.down();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (2, 4));
        // page() is up/down repeated, clamped.
        t.page(-10);
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (0, 4));
        t.page(1);
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 2));
    }

    #[test]
    fn area_up_down_never_lands_inside_a_wide_cluster() {
        let mut t = TextArea::from_text("abcd\n你好", 64);
        for _ in 0..3 {
            t.right();
        }
        t.down();
        // Column 3 falls inside 好 (cells 2-3); the landing is after 你.
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 2));
        assert_eq!(t.cursor(), "abcd\n你".len());
        t.up();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (0, 3));
    }

    #[test]
    fn area_combining_cluster_travels_whole() {
        let mut t = TextArea::from_text("cafe\u{301}\nx", 64);
        t.down();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 0));
        t.left(); // crosses the break, lands after the accent
        assert_eq!(t.cursor(), "cafe\u{301}".len());
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (0, 4));
        t.backspace();
        assert_eq!(t.as_str(), "caf\nx");
        // And right crosses the break the other way.
        t.right();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 0));
    }

    #[test]
    fn area_word_ops_stop_at_the_line() {
        let mut t = TextArea::from_text("foo bar\nbaz qux", 64);
        t.down();
        // ^w at column 0 does nothing — the line above is not a word.
        t.delete_word_back();
        assert_eq!(t.as_str(), "foo bar\nbaz qux");
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 0));
        // word_left at column 0 crosses nothing.
        t.word_left();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 0));
        // word_right at a line's end crosses nothing either.
        t.up();
        t.end();
        t.word_right();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (0, 7));
        // Inside a line the words behave as on one line.
        t.word_left();
        assert_eq!(t.cursor_col_cells(), 4);
        t.delete_word_back();
        assert_eq!(t.as_str(), "bar\nbaz qux");
        // ^u kills only the current line's prefix.
        t.down();
        t.end();
        t.kill_to_start();
        assert_eq!(t.as_str(), "bar\n");
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 0));
    }

    #[test]
    fn area_home_end_are_line_wise() {
        let mut t = TextArea::from_text("ab\ncde\nf", 64);
        t.down();
        t.end();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 3));
        assert_eq!(t.cursor(), "ab\ncde".len());
        t.home();
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 0));
        assert_eq!(t.cursor(), "ab\n".len());
        // The last line's end is the text's end.
        t.down();
        t.end();
        assert_eq!(t.cursor(), t.len());
    }

    #[test]
    fn area_paste_keeps_newlines_and_scrubs() {
        let mut t = TextArea::new(64);
        assert_eq!(t.paste("a\r\nb\u{202e}\tc\n"), Pasted { trimmed: false });
        assert_eq!(t.as_str(), "a\nb c\n");
        assert_eq!(t.line_count(), 3);
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (2, 0));
        // Past the limit the cut is at a cluster boundary and says so.
        let mut t = TextArea::new(4);
        assert_eq!(t.paste("ab\ncd"), Pasted { trimmed: true });
        assert_eq!(t.as_str(), "ab\nc");
        let mut t = TextArea::new(5);
        assert_eq!(t.paste("שלום"), Pasted { trimmed: true });
        assert_eq!(t.as_str(), "של");
        let mut t = TextArea::new(5);
        t.paste("cafe\u{301}");
        assert_eq!(t.as_str(), "caf");
        // Mid-text, at the cursor, cursor rides to the end of the paste.
        let mut t = TextArea::from_text("ab", 64);
        t.right();
        t.paste("x\ny");
        assert_eq!(t.as_str(), "ax\nyb");
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (1, 1));
    }

    #[test]
    fn area_limit_counts_the_newline() {
        let mut t = TextArea::new(3);
        t.insert('a');
        t.insert('b');
        t.newline();
        t.insert('c');
        assert_eq!(t.as_str(), "ab\n");
        assert_eq!(t.len(), 3);
        // Deleting makes room again; text loaded over the limit is kept.
        t.backspace();
        t.insert('z');
        assert_eq!(t.as_str(), "abz");
        let mut t = TextArea::from_text("abcdef", 3);
        t.insert('x');
        t.newline();
        assert_eq!(t.as_str(), "abcdef");
        assert_eq!(t.paste("y"), Pasted { trimmed: true });
        assert_eq!(t.as_str(), "abcdef");
        assert_eq!(t.limit(), 3);
    }

    #[test]
    fn area_window_follows_the_cursor() {
        let mut t = TextArea::from_text("0\n1\n2\n3\n4", 64);
        t.page(4);
        assert_eq!(t.cursor_line(), 4);
        let (top, rows, cursor) = area_window(&t, 0, 3, 10);
        assert_eq!(top, 2);
        assert_eq!(rows, vec!["2", "3", "4"]);
        assert_eq!(cursor, (2, 0));
        t.page(-4);
        let (top, rows, cursor) = area_window(&t, 2, 3, 10);
        assert_eq!(top, 0);
        assert_eq!(rows, vec!["0", "1", "2"]);
        assert_eq!(cursor, (0, 0));
        // Inside the window the top does not move: no jitter.
        t.down();
        t.down();
        let (top, _, cursor) = area_window(&t, 2, 3, 10);
        assert_eq!(top, 2);
        assert_eq!(cursor, (0, 0));
        let (top, _, cursor) = area_window(&t, 0, 3, 10);
        assert_eq!(top, 0);
        assert_eq!(cursor, (2, 0));
        // A top past what the text fills is pulled back; a short text fits.
        let (top, rows, _) = area_window(&t, 4, 3, 10);
        assert_eq!((top, rows.len()), (2, 3));
        let (top, rows, _) = area_window(&t, 3, 10, 10);
        assert_eq!((top, rows.len()), (0, 5));
        // No rows, no window.
        assert_eq!(area_window(&t, 2, 0, 10), (0, Vec::new(), (0, 0)));
        // An empty text is one empty row with the cursor on it.
        assert_eq!(area_window(&TextArea::new(8), 0, 3, 10), (0, vec![String::new()], (0, 0)));
    }

    #[test]
    fn area_window_wraps_words_and_long_tokens_without_changing_text() {
        let text = "one two three\n\nabcdefgh\n";
        let mut t = TextArea::from_text(text, 128);
        let (_, rows, cursor) = area_window(&t, 0, 10, 8);
        assert_eq!(rows, ["one two ", "three", "", "abcdefgh", ""]);
        assert_eq!(cursor, (0, 0));
        t.end();
        assert_eq!(area_window(&t, 0, 10, 8).2, (1, 5));
        let (_, rows, _) = area_window(&t, 0, 10, 4);
        assert_eq!(rows, ["one ", "two ", "thre", "e", "", "abcd", "efgh", ""]);
        assert_eq!(t.as_str(), text);
    }

    #[test]
    fn area_wrapped_navigation_scrolls_and_edits_at_soft_boundaries() {
        let mut t = TextArea::from_text("one two three four five", 128);
        t.right();
        t.move_rows(1, 8);
        assert_eq!(t.cursor(), 9); // t|hree
        t.move_rows(1, 8);
        assert_eq!(t.cursor(), 15); // f|our
        let (top, rows, cursor) = area_window(&t, 0, 2, 8);
        assert_eq!((top, rows, cursor), (1, vec!["three ".into(), "four ".into()], (1, 1)));
        t.move_rows(-2, 8);
        assert_eq!(t.cursor(), 1);
        t.cursor = 8; // the soft boundary belongs to the next row
        assert_eq!(area_window(&t, 0, 4, 8).2, (1, 0));
        t.backspace();
        assert_eq!(t.as_str(), "one twothree four five");
        t.insert(' ');
        t.delete();
        assert_eq!(t.as_str(), "one two hree four five");
        t.newline();
        assert_eq!(t.as_str(), "one two \nhree four five");
    }

    #[test]
    fn area_wrapped_navigation_keeps_column_across_short_rows() {
        let mut t = TextArea::from_text("abcdefghi\nx\nabcdefghi", 128);
        t.cursor = 3;
        t.move_rows(1, 6);
        assert_eq!(t.cursor(), 9);
        t.move_rows(1, 6);
        assert_eq!(t.cursor(), 11);
        t.move_rows(1, 6);
        assert_eq!(t.cursor(), 15);
        t.move_rows(-3, 6);
        assert_eq!(t.cursor(), 3);
        // Do not land on the next row when aiming past a soft row's end.
        t.cursor = 19;
        t.want_col = None;
        t.move_rows(-1, 6);
        assert_eq!(t.cursor_row(6), 3);
    }

    #[test]
    fn area_wraps_unicode_on_grapheme_boundaries_and_handles_narrow_views() {
        let mut t = TextArea::from_text("你好cafe\u{301}xyz", 128);
        let (_, rows, _) = area_window(&t, 0, 10, 4);
        assert_eq!(rows, ["你好", "cafe\u{301}", "xyz"]);
        t.right();
        t.move_rows(1, 4);
        assert_eq!(&t.as_str()[..t.cursor()], "你好ca");
        t.move_rows(-1, 4);
        assert_eq!(t.cursor(), "你".len());
        let (_, rows, cursor) = area_window(&t, 0, 20, 1);
        assert_eq!(rows, ["~", "~", "c", "a", "f", "e\u{301}", "x", "y", "z"]);
        assert_eq!(cursor, (1, 0));
        let (_, rows, cursor) = area_window(&t, 0, 20, 0);
        assert!(rows.iter().all(String::is_empty));
        assert_eq!(cursor, (1, 0));
    }

    #[test]
    fn area_resize_reflows_and_keeps_cursor_visible() {
        let mut t = TextArea::from_text("abcdefghijklmnop", 64);
        t.end();
        assert_eq!(
            area_window(&t, 0, 2, 8),
            (0, vec!["abcdefgh".into(), "ijklmnop".into()], (1, 8))
        );
        assert_eq!(area_window(&t, 0, 2, 4), (2, vec!["ijkl".into(), "mnop".into()], (1, 4)));
        assert_eq!(area_window(&t, 2, 2, 16), (0, vec![t.as_str().into()], (0, 16)));
        assert_eq!(t.cursor(), 16);
    }

    #[test]
    fn area_insert_refuses_hazards() {
        let mut t = TextArea::new(64);
        t.insert('a');
        t.insert('\u{202e}'); // bidi override
        t.insert('\x07'); // bell
        t.insert('█'); // block element
        t.insert('\t');
        t.insert('\n'); // only newline() adds one
        t.insert('b');
        assert_eq!(t.as_str(), "ab");
        assert_eq!(t.cursor(), 2);
        // from_text and paste scrub the same set.
        assert_eq!(TextArea::from_text("a\u{202e}\x07█\tb", 64).as_str(), "a b");
    }

    #[test]
    fn area_from_text_opens_at_the_top_and_normalises_crlf() {
        let t = TextArea::from_text("a\r\nb\rc\n", 64);
        assert_eq!(t.as_str(), "a\nb\nc\n");
        assert_eq!(t.cursor(), 0);
        assert_eq!((t.cursor_line(), t.cursor_col_cells()), (0, 0));
        assert_eq!(t.line_count(), 4);
        assert!(!t.is_empty());
        assert!(TextArea::new(8).is_empty());
        assert_eq!(t.clone().into_text(), "a\nb\nc\n");
    }
}
