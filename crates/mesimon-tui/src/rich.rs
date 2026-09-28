//! Rich text for the ticket page's PREVIEW zone (author 2026-08-31).
//!
//! An agent's reply IS markdown — headings, bullets, `code`, **emphasis**,
//! fenced blocks, the odd table — and the zone used to render the source:
//! asterisks and backticks on show, every newline flattened into one grey
//! paragraph by `peek::sanitize`. This module reads that markdown and draws
//! it, inside the board's own laws rather than a web renderer's:
//!
//! * **Value, weight, paint and space — nothing else.** 06 §5.1 bans SGR 2,
//!   3, 5 and 9 outright and reserves SGR 4 for the scope chip, so italic is
//!   NOT slant and strike is NOT a line through the glyphs: emphasis is a
//!   step up the grey ramp (`dim1 → base`), strong adds bold, struck text
//!   falls to `dim3`, and a code span is the elevated surface painted behind
//!   it. `test_no_banned_sgr` renders this zone, so the rule is enforced,
//!   not merely intended.
//! * **No new colour.** Everything rides `Theme::rest`, so the one saturated
//!   colour stays reserved for needs-you (L2 / D19).
//! * **No drawn structure.** `0x2500-0x259F` is banned board-wide, so a
//!   thematic break (`---`) is a BLANK ROW, not a rule, and a code block is a
//!   painted slab with no border.
//! * **Below the paint, the marker survives.** Where the profile has no
//!   elevated surface (chalk-256, mono), an inline code span keeps its
//!   backticks — the same call `pip()` makes for tag tints: abandon the
//!   treatment, never approximate it.
//!
//! Deliberately markdown-LITE, and parsed in one pass with no dependency:
//! this is a preview of one message in a ~15-row zone, not a document
//! viewer. What an agent reply or a ticket's notes actually use is here —
//! tables, task lists, quotes holding blocks, hard breaks; reference links,
//! footnotes, setext headings, indented code and HTML (bar `<br>`) are not.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::glyphs::Tier;
use crate::theme::{Ramp, Theme};

/// Which surface the text is drawn on. Markdown's whole vocabulary here is
/// value and paint, and both are relative to the ground under them: on the
/// page the ramp is `rest` and code sits on the elevated surface; on the
/// elevated surface (the ticket page's header band, T-158) the ramp is `sel`
/// and code sinks back to the PAGE ground — the other surface is the only
/// other paint there is, and a slab in the band's own colour would vanish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    Ground,
    Elevated,
}

/// What one newline inside a paragraph means. CommonMark's answer is a
/// space, and it is the right one for text wrapped by hand at a fixed width
/// — an agent's reply, the changelog. A person typing a note into a field
/// that wraps for them presses Enter to end a line, and reads it back the
/// way a GitHub comment does: as a break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Newline {
    Space,
    Break,
}

impl Newline {
    /// A note's: `Break` when a person wrote it last — here (`local`), on a
    /// paired phone (`device:`) or as a teammate (`member:`) — and `Space`
    /// for an agent's or anything unknown, which is what it always was.
    pub(crate) fn of_note(meta: &mesimon_core::board::NoteMeta) -> Self {
        let by = meta.edited_by.as_str();
        if by == "local" || by.starts_with("device:") || by.starts_with("member:") {
            Newline::Break
        } else {
            Newline::Space
        }
    }
}

/// The ramp and the code paint for a surface.
struct Paint {
    ink: Ramp,
    code_bg: Option<ratatui::style::Color>,
}

impl Paint {
    fn of(theme: &Theme, surface: Surface) -> Self {
        match (surface, theme.selected_bg) {
            (Surface::Elevated, Some(_)) => Paint { ink: theme.sel, code_bg: theme.bg },
            _ => Paint { ink: theme.rest, code_bg: theme.code_bg() },
        }
    }
    /// Inside a quote: the whole ramp one step down, so quoted text is a
    /// value step under its surroundings and a quote in a quote another —
    /// while its own emphasis still steps up from there.
    fn quoted(&self) -> Self {
        let r = self.ink;
        Paint { ink: Ramp { base: r.dim1, dim1: r.dim2, dim2: r.dim3, dim3: r.dim3 }, ..*self }
    }
    fn dim2(&self) -> Style {
        Style::default().fg(self.ink.dim2)
    }
    fn dim3(&self) -> Style {
        Style::default().fg(self.ink.dim3)
    }
}

/// Render `src` into at most `max_lines` lines of at most `width` cells, on
/// the page ground. A cut ends in the `~` marker — 07 §4.1's vocabulary,
/// same as `truncate`. The tests' form: every caller names its surface and
/// its newlines (`render_on`, `render_all`).
#[cfg(test)]
pub(crate) fn render(
    src: &str,
    width: usize,
    max_lines: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    render_on(src, width, max_lines, theme, Surface::Ground, Newline::Space)
}

/// `render`, on a chosen surface, reading newlines as `newline` says.
pub(crate) fn render_on(
    src: &str,
    width: usize,
    max_lines: usize,
    theme: &Theme,
    surface: Surface,
    newline: Newline,
) -> Vec<Line<'static>> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let paint = Paint::of(theme, surface);
    let mut out = Out { lines: Vec::new(), max: max_lines, width, theme, paint, cut: false };
    out.blocks(blocks(src, newline));
    out.finish()
}

/// Every line of `src` at `width`, unmarked: the caller owns the window and
/// says where the cut is (`mark_cut`). The ticket page's preview scrolls
/// through this; `render` stays the one-shot form for a zone that only ever
/// shows the top.
pub(crate) fn render_all(
    src: &str,
    width: usize,
    theme: &Theme,
    newline: Newline,
) -> Vec<Line<'static>> {
    render_on(src, width, usize::MAX, theme, Surface::Ground, newline)
}

/// End `lines` in the `~` cut marker — the same mark `render` leaves when it
/// runs out of rows, for a window that stops short of the last one. Clips
/// the last row to make the cell if it has to.
pub(crate) fn mark_cut(lines: &mut [Line<'static>], width: usize, theme: &Theme) {
    let Some(last) = lines.last_mut() else { return };
    let w: usize = crate::ui::spans_width(&last.spans);
    if w >= width {
        clip(&mut last.spans, width.saturating_sub(1));
    }
    last.spans.push(Span::styled("~", theme.dim2()));
}

// ---------------------------------------------------------------------------
// parse
// ---------------------------------------------------------------------------

/// Which typographic role a run of text plays. Deliberately NOT a `Style`:
/// how a role reaches cells depends on the profile (a code span that cannot
/// be painted keeps its backticks instead), so the parse stays pure and the
/// theme decides last.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Emph {
    strong: bool,
    em: bool,
    code: bool,
    /// `~~struck~~` — dead text, said with value (dim3), never SGR 9.
    dead: bool,
    /// The target half of a link: kept, because the page cannot follow one
    /// (`^k` lists the ticket's links and opens them, T-256 — this zone
    /// only reads).
    url: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Run {
    text: String,
    emph: Emph,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Body,
    Head(u8),
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum Block {
    Blank,
    Head {
        level: u8,
        runs: Vec<Run>,
    },
    Para {
        runs: Vec<Run>,
    },
    /// Everything a `>` holds — paragraphs, a list, a fence, another quote
    /// — parsed as a document of its own and drawn behind the mark.
    Quote {
        blocks: Vec<Block>,
    },
    /// `marker: None` is a bullet — which glyph that is belongs to the
    /// tier, not to the parse. `task` is a GFM task item's box: ticked or
    /// not.
    Item {
        marker: Option<String>,
        indent: usize,
        task: Option<bool>,
        runs: Vec<Run>,
    },
    /// A fenced block. The info string is dropped: naming the language costs
    /// a whole row of a 15-row zone and the slab already says "this is code".
    Code {
        rows: Vec<String>,
    },
    /// A pipe table: a header row, the delimiter row under it (dropped — with
    /// rules banned, the header is marked by weight instead) and its body.
    /// Every cell is parsed inline and every row holds exactly the header's
    /// column count, padded or cut the way GFM does.
    Table {
        head: Vec<Vec<Run>>,
        align: Vec<Align>,
        rows: Vec<Vec<Vec<Run>>>,
    },
    /// A pipe line with no delimiter row under it: not a table, but its
    /// columns may be spaced by hand, so it is kept verbatim, not reflowed.
    Raw {
        text: String,
    },
}

/// A table column's alignment, as its delimiter cell spells it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Align {
    Left,
    Center,
    Right,
}

/// What is still being accumulated when the next line arrives. A quote
/// keeps its lines as written (one `>` off, trailing spaces kept — they may
/// be a hard break) for the parse it gets of its own.
enum Pending {
    None,
    Para(Vec<String>),
    Quote(Vec<String>),
    Item { marker: Option<String>, indent: usize, task: Option<bool>, lines: Vec<String> },
}

fn blocks(src: &str, newline: Newline) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut pending = Pending::None;
    let mut fence: Option<(char, usize, Vec<String>)> = None;

    // Indexed, because a table is only a table if the line under its header
    // is a delimiter row.
    let lines: Vec<&str> = src.lines().collect();
    let mut at = 0;
    while at < lines.len() {
        let raw = lines[at];
        at += 1;
        // Inside a fence every line is verbatim until the closing marker.
        if let Some((ch, len, rows)) = fence.as_mut() {
            let t = raw.trim();
            if t.starts_with(&ch.to_string().repeat(*len)) && t.chars().all(|c| c == *ch) {
                let (_, _, rows) = fence.take().expect("fence open");
                out.push(Block::Code { rows });
            } else {
                rows.push(raw.to_string());
            }
            continue;
        }
        if let Some((ch, len)) = fence_open(raw) {
            flush(&mut pending, &mut out, newline);
            fence = Some((ch, len, Vec::new()));
            continue;
        }
        let trimmed = raw.trim_end();
        let body = trimmed.trim_start();
        let indent = trimmed.len() - body.len();

        if body.is_empty() {
            flush(&mut pending, &mut out, newline);
            out.push(Block::Blank);
            continue;
        }
        // A thematic break is a blank row: rules are drawn structure (L1).
        if is_break(body) {
            flush(&mut pending, &mut out, newline);
            out.push(Block::Blank);
            continue;
        }
        if let Some((level, text)) = heading(body) {
            flush(&mut pending, &mut out, newline);
            out.push(Block::Head { level, runs: inline(text) });
            continue;
        }
        if let Some(align) = table_head(body, lines.get(at).copied()) {
            flush(&mut pending, &mut out, newline);
            at += 1; // the delimiter row
            let n = align.len();
            let row = |line: &str| -> Vec<Vec<Run>> {
                let mut cs: Vec<Vec<Run>> = cells(line).iter().map(|c| inline(c)).collect();
                cs.resize(n, Vec::new());
                cs
            };
            let head = row(body);
            let mut rows = Vec::new();
            // The table runs to the first line that cannot be a row of it.
            while let Some(next) = lines.get(at).map(|l| l.trim()) {
                if next.is_empty() || !next.contains('|') || fence_open(next).is_some() {
                    break;
                }
                rows.push(row(next));
                at += 1;
            }
            out.push(Block::Table { head, align, rows });
            continue;
        }
        if body.starts_with('|') {
            flush(&mut pending, &mut out, newline);
            out.push(Block::Raw { text: body.to_string() });
            continue;
        }
        if let Some((marker, text)) = list_marker(body) {
            flush(&mut pending, &mut out, newline);
            // Two source spaces per level, capped: a deep tree would spend
            // the whole column on indent.
            let depth = (indent / 2).min(3);
            let (task, text) = task_box(text);
            pending = Pending::Item {
                marker,
                indent: depth * 2,
                task,
                lines: vec![line_text(raw, text)],
            };
            continue;
        }
        if let Some(text) = quote_line(raw.trim_start()) {
            match &mut pending {
                Pending::Quote(lines) => lines.push(text.to_string()),
                _ => {
                    flush(&mut pending, &mut out, newline);
                    pending = Pending::Quote(vec![text.to_string()]);
                }
            }
            continue;
        }
        // Plain text: markdown's lazy continuation — it belongs to whatever
        // paragraph is open (a list item's second line, a quote's), else it
        // starts one. A quote whose last line was blank has no paragraph
        // open, so the text starts a paragraph after it.
        match &mut pending {
            Pending::Quote(lines) if lines.last().is_some_and(|l| !l.trim().is_empty()) => {
                lines.push(raw.trim_start().to_string())
            }
            Pending::Para(lines) | Pending::Item { lines, .. } => lines.push(line_text(raw, body)),
            _ => {
                flush(&mut pending, &mut out, newline);
                pending = Pending::Para(vec![line_text(raw, body)]);
            }
        }
    }
    if let Some((_, _, rows)) = fence.take() {
        out.push(Block::Code { rows }); // unterminated fence: show it anyway
    }
    flush(&mut pending, &mut out, newline);
    out
}

fn flush(pending: &mut Pending, out: &mut Vec<Block>, newline: Newline) {
    match std::mem::replace(pending, Pending::None) {
        Pending::None => {}
        Pending::Para(lines) => out.push(Block::Para { runs: inline(&joined(&lines, newline)) }),
        Pending::Quote(mut lines) => {
            // A GitHub alert (`> [!NOTE]`) names itself on its own first
            // line: the name in weight, on a line of its own. No colour —
            // the accent is needs-you's, and a warning in a note is not.
            if let Some(kind) = lines.first().and_then(|l| alert(l)) {
                lines[0] = format!("**{kind}**\\");
            }
            out.push(Block::Quote { blocks: blocks(&lines.join("\n"), newline) })
        }
        Pending::Item { marker, indent, task, lines } => {
            let mut runs = inline(&joined(&lines, newline));
            if task == Some(true) {
                // A ticked item is read last: the de-emphasis token, like
                // struck text, so what is left to do is what stands out.
                for r in &mut runs {
                    r.emph.dead = true;
                }
            }
            out.push(Block::Item { marker, indent, task, runs })
        }
    }
}

/// A paragraph's source lines as one string for `inline`: joined by a space
/// or, where `newline` says so, by a break — and a line that ends in its own
/// hard break (`line_text`) is never joined by a second one.
fn joined(lines: &[String], newline: Newline) -> String {
    let mut s = String::new();
    for l in lines {
        if !s.is_empty() && !s.ends_with('\n') {
            s.push(if newline == Newline::Break { '\n' } else { ' ' });
        }
        s.push_str(l);
    }
    s
}

/// One line of a paragraph's text, with CommonMark's hard break — two
/// trailing spaces, or a trailing backslash (dropped) — spelt `\n`, which
/// is what `words` breaks a line on. `body` is the line trimmed; `raw` still
/// has the spaces.
fn line_text(raw: &str, body: &str) -> String {
    if let Some(rest) = body.strip_suffix('\\') {
        if !rest.ends_with('\\') {
            return format!("{rest}\n");
        }
    }
    if raw.ends_with("  ") {
        return format!("{body}\n");
    }
    body.to_string()
}

/// `[ ] rest` / `[x] rest` at the head of a list item → (ticked?, rest).
fn task_box(text: &str) -> (Option<bool>, &str) {
    for (mark, done) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
        if let Some(rest) = text.strip_prefix(mark) {
            return (Some(done), rest);
        }
    }
    (None, text)
}

/// A quote's first line naming a GitHub alert → the alert's name.
fn alert(line: &str) -> Option<&'static str> {
    let kind = line.trim().strip_prefix("[!")?.strip_suffix(']')?;
    ["Note", "Tip", "Important", "Warning", "Caution"]
        .into_iter()
        .find(|k| k.eq_ignore_ascii_case(kind))
}

fn fence_open(line: &str) -> Option<(char, usize)> {
    let t = line.trim_start();
    for ch in ['`', '~'] {
        let n = t.chars().take_while(|c| *c == ch).count();
        if n >= 3 {
            return Some((ch, n));
        }
    }
    None
}

fn is_break(body: &str) -> bool {
    for ch in ['-', '*', '_'] {
        let n = body.chars().filter(|c| *c == ch).count();
        if n >= 3 && body.chars().all(|c| c == ch || c == ' ') {
            return true;
        }
    }
    false
}

/// Is `body` a table's header row — is `next` a delimiter row with as many
/// cells as it has? Both must carry a pipe, or `text` over `---` (a setext
/// heading, drawn here as text and a break) would read as a one-column table.
fn table_head(body: &str, next: Option<&str>) -> Option<Vec<Align>> {
    let next = next?.trim();
    if !body.contains('|') || !next.contains('|') {
        return None;
    }
    let align = cells(next)
        .iter()
        .map(|c| {
            let dashes = c.trim_start_matches(':').trim_end_matches(':');
            if dashes.is_empty() || !dashes.chars().all(|ch| ch == '-') {
                return None;
            }
            Some(match (c.starts_with(':'), c.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect::<Option<Vec<Align>>>()?;
    (cells(body).len() == align.len()).then_some(align)
}

/// A table row's cells, trimmed: split on every pipe but an escaped one
/// (`\|` stays in the cell for `inline` to unescape), the outer pipes
/// optional.
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = match t.strip_suffix('|') {
        Some(rest) if !rest.ends_with('\\') => rest,
        _ => t,
    };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for c in t.chars() {
        if c == '|' && !escaped {
            out.push(std::mem::take(&mut cur).trim().to_string());
        } else {
            cur.push(c);
        }
        escaped = c == '\\' && !escaped;
    }
    out.push(cur.trim().to_string());
    out
}

fn heading(body: &str) -> Option<(u8, &str)> {
    let n = body.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&n) {
        let rest = body[n..].strip_prefix(' ')?;
        return Some((n as u8, rest.trim_end_matches(['#', ' '])));
    }
    None
}

fn quote_line(body: &str) -> Option<&str> {
    let rest = body.strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// `- x`, `* x`, `+ x`, `1. x`, `2) x` → (marker as rendered, the text).
fn list_marker(body: &str) -> Option<(Option<String>, &str)> {
    for m in ['-', '*', '+'] {
        if let Some(rest) = body.strip_prefix(m) {
            if let Some(text) = rest.strip_prefix(' ') {
                return Some((None, text));
            }
        }
    }
    let digits = body.chars().take_while(char::is_ascii_digit).count();
    if (1..=3).contains(&digits) {
        let rest = &body[digits..];
        for sep in ['.', ')'] {
            if let Some(text) = rest.strip_prefix(sep).and_then(|r| r.strip_prefix(' ')) {
                return Some((Some(format!("{}{sep}", &body[..digits])), text));
            }
        }
    }
    None
}

// -- inline -----------------------------------------------------------------

fn push_run(out: &mut Vec<Run>, cur: &mut String, emph: Emph) {
    if !cur.is_empty() {
        out.push(Run { text: std::mem::take(cur), emph });
    }
}

fn delim_len(chars: &[char], i: usize) -> usize {
    let c = chars[i];
    let n = chars[i..].iter().take_while(|x| **x == c).count();
    match c {
        '~' if n >= 2 => 2,
        '~' => 0,
        _ if n >= 2 => 2,
        _ => 1,
    }
}

/// A delimiter opens only when it hugs the text to its right — and `_` also
/// refuses to open inside a word, or `snake_case_names` would come out
/// emphasised, which is exactly what agent replies are full of.
fn opens(chars: &[char], i: usize, len: usize) -> bool {
    let after = chars.get(i + len);
    let ok = after.is_some_and(|c| !c.is_whitespace());
    if chars[i] == '_' {
        return ok
            && !i.checked_sub(1).and_then(|p| chars.get(p)).is_some_and(|c| c.is_alphanumeric());
    }
    ok
}

fn closes(chars: &[char], i: usize, len: usize) -> bool {
    let before = i.checked_sub(1).and_then(|p| chars.get(p));
    let ok = before.is_some_and(|c| !c.is_whitespace());
    if chars[i] == '_' {
        return ok && !chars.get(i + len).is_some_and(|c| c.is_alphanumeric());
    }
    ok
}

/// How many `ch` run from `i`.
fn run_len(chars: &[char], i: usize) -> usize {
    chars[i..].iter().take_while(|x| **x == chars[i]).count()
}

/// Does a valid closer for this delimiter exist ahead? An opener with none is
/// literal text (`5 * 3` must not eat the rest of the line). A longer run
/// closes a shorter opener: `***` ends `*` and `**` both.
fn has_closer(chars: &[char], from: usize, ch: char, len: usize) -> bool {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == ch && run_len(chars, i) >= len && closes(chars, i, len) {
            return true;
        }
        i += 1;
    }
    false
}

fn flag(emph: &mut Emph, ch: char, len: usize) -> &mut bool {
    match (ch, len) {
        ('~', _) => &mut emph.dead,
        (_, 2) => &mut emph.strong,
        _ => &mut emph.em,
    }
}

fn flag_set(emph: Emph, ch: char, len: usize) -> bool {
    match (ch, len) {
        ('~', _) => emph.dead,
        (_, 2) => emph.strong,
        _ => emph.em,
    }
}

fn inline(s: &str) -> Vec<Run> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<Run> = Vec::new();
    let mut cur = String::new();
    let mut emph = Emph::default();
    let mut open: Vec<(char, usize)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Escapes first, so `\*` is a star and not an opener.
        if c == '\\' && chars.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) {
            cur.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '`' {
            let ticks = chars[i..].iter().take_while(|x| **x == '`').count();
            if let Some(close) = find_ticks(&chars, i + ticks, ticks) {
                let text: String = chars[i + ticks..close].iter().collect();
                let text = text.trim().to_string();
                if !text.is_empty() {
                    push_run(&mut out, &mut cur, emph);
                    let mut e = emph;
                    e.code = true;
                    out.push(Run { text, emph: e });
                    i = close + ticks;
                    continue;
                }
            }
        }
        // `<br>` is the one tag worth reading: GFM's way to break a line
        // inside a table cell, spelt the way `words` breaks one.
        if c == '<' {
            if let Some(n) = br_tag(&chars, i) {
                cur.push('\n');
                i += n;
                continue;
            }
        }
        if c == '[' || (c == '!' && chars.get(i + 1) == Some(&'[')) {
            if let Some((runs, next)) = link(&chars, i, emph) {
                push_run(&mut out, &mut cur, emph);
                out.extend(runs);
                i = next;
                continue;
            }
        }
        if matches!(c, '*' | '_' | '~') {
            let len = delim_len(&chars, i);
            if len > 0 {
                // The innermost open delimiter closes first, on a run at
                // least its length: `***both***` opens `**` then `*`, and
                // its closing `***` ends `*` here and `**` on the next pass.
                if let Some(&(oc, olen)) = open.last() {
                    if oc == c && run_len(&chars, i) >= olen && closes(&chars, i, olen) {
                        push_run(&mut out, &mut cur, emph);
                        open.pop();
                        *flag(&mut emph, c, olen) = false;
                        i += olen;
                        continue;
                    }
                }
                if !flag_set(emph, c, len)
                    && opens(&chars, i, len)
                    && has_closer(&chars, i + len, c, len)
                {
                    push_run(&mut out, &mut cur, emph);
                    open.push((c, len));
                    *flag(&mut emph, c, len) = true;
                    i += len;
                    continue;
                }
            }
        }
        cur.push(c);
        i += 1;
    }
    push_run(&mut out, &mut cur, emph);
    out
}

/// `<br>`, `<br/>` or `<br />`, any case, at `i` → its length.
fn br_tag(chars: &[char], i: usize) -> Option<usize> {
    let ahead: String = chars[i..].iter().take(6).collect::<String>().to_ascii_lowercase();
    ["<br>", "<br/>", "<br />"].into_iter().find(|t| ahead.starts_with(t)).map(str::len)
}

fn find_ticks(chars: &[char], from: usize, len: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '`' && chars[i..].iter().take_while(|x| **x == '`').count() == len {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// `[label](url)` / `![alt](url)` → the label, then the target in `dim3`.
/// The url is kept because this zone cannot follow a link: dropping it would
/// delete the half the reader can act on — by hand, or through `^k`, whose
/// recogniser is `core/src/links.rs` (the same `[label](target)` rule,
/// parsed again there because this walk returns painted spans).
fn link(chars: &[char], i: usize, emph: Emph) -> Option<(Vec<Run>, usize)> {
    let open = if chars[i] == '!' { i + 1 } else { i };
    let close = (open + 1..chars.len()).find(|k| chars[*k] == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len()).find(|k| chars[*k] == ')')?;
    let label: String = chars[open + 1..close].iter().collect();
    let url: String = chars[close + 2..end].iter().collect();
    let url = url.trim().to_string();
    if mesimon_core::attachment::parse_target(&url).is_some() {
        return Some((
            vec![Run { text: format!("[{label}]"), emph: Emph { em: true, ..emph } }],
            end + 1,
        ));
    }
    if label.trim().is_empty() && url.is_empty() {
        return None;
    }
    let mut runs: Vec<Run> = inline(&label)
        .into_iter()
        .map(|r| Run { text: r.text, emph: Emph { em: true, ..r.emph } })
        .collect();
    if !url.is_empty() && url != label.trim() {
        let mut e = emph;
        e.url = true;
        runs.push(Run { text: format!(" {url}"), emph: e });
    }
    Some((runs, end + 1))
}

// ---------------------------------------------------------------------------
// paint
// ---------------------------------------------------------------------------

fn style_of(e: Emph, role: Role, paint: &Paint) -> Style {
    let t = paint.ink;
    let (fg, bold) = match role {
        // Two heading levels, both by value; the top two also take weight.
        Role::Head(1..=2) => (t.base, true),
        Role::Head(_) => (t.base, false),
        Role::Body if e.strong => (t.base, true),
        // No slant available (SGR 3 is banned), so emphasis is one step up
        // the ramp — the same move de-emphasis makes, in the other direction.
        Role::Body if e.em => (t.base, false),
        Role::Body => (t.dim1, false),
    };
    // Struck and url text are both "read this last": one role, one token.
    let fg = if e.dead || e.url { t.dim3 } else { fg };
    let mut s = Style::default().fg(fg);
    if bold && !e.dead {
        s = s.add_modifier(Modifier::BOLD);
    }
    if e.code {
        if let Some(bg) = paint.code_bg {
            s = s.bg(bg);
        }
    }
    s
}

/// One display word: the parts are the styled runs it is made of, kept
/// together so `**bold**text` breaks as one word and not as two. `brk` is
/// no word at all but a hard break: the line ends there.
struct Word {
    parts: Vec<(String, Emph)>,
    width: usize,
    brk: bool,
}

impl Word {
    fn brk() -> Self {
        Word { parts: Vec::new(), width: 0, brk: true }
    }
}

/// The width `ws` wants on one line — its widest, between hard breaks.
fn natural(ws: &[Word]) -> usize {
    ws.split(|w| w.brk)
        .map(|seg| seg.iter().map(|w| w.width).sum::<usize>() + seg.len().saturating_sub(1))
        .max()
        .unwrap_or(0)
}

fn words(runs: &[Run], paint: &Paint) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut open = false; // is the last word still being extended?
    for r in runs {
        // Where the profile cannot paint, the marker survives instead.
        let text = if r.emph.code && paint.code_bg.is_none() {
            format!("`{}`", r.text)
        } else {
            r.text.clone()
        };
        let mut rest = text.as_str();
        while !rest.is_empty() {
            let ws = rest.starts_with(char::is_whitespace);
            let cut = rest
                .char_indices()
                .find(|(_, c)| c.is_whitespace() != ws)
                .map(|(k, _)| k)
                .unwrap_or(rest.len());
            let (chunk, tail) = rest.split_at(cut);
            rest = tail;
            if ws {
                // A `\n` that survived the parse is a hard break.
                out.extend(chunk.matches('\n').map(|_| Word::brk()));
                open = false;
                continue;
            }
            if open {
                if let Some(w) = out.last_mut() {
                    w.width += chunk.width();
                    w.parts.push((chunk.to_string(), r.emph));
                    continue;
                }
            }
            out.push(Word {
                width: chunk.width(),
                parts: vec![(chunk.to_string(), r.emph)],
                brk: false,
            });
            open = true;
        }
        // A run that ended mid-word keeps the next run attached.
        open = open && !text.ends_with(char::is_whitespace);
    }
    out
}

/// Greedy word wrap of one block into `width` cells. `lead` opens the first
/// line (a bullet, a quote mark, an indent) and `hang` is the inset every
/// later line of the same block keeps.
fn wrap(
    runs: &[Run],
    lead: Vec<Span<'static>>,
    hang: usize,
    role: Role,
    width: usize,
    paint: &Paint,
) -> Vec<Vec<Span<'static>>> {
    let mut out = Vec::new();
    let ws = words(runs, paint);
    if ws.is_empty() {
        return out;
    }
    let mut cur: Vec<Span<'static>> = lead;
    cur.retain(|s| !s.content.is_empty());
    let mut w: usize = crate::ui::spans_width(&cur);
    let start = w;
    let mut first = true;
    // The space BETWEEN two words of one code span must be painted too, or
    // the slab comes out with a hole in it — so a separator inherits the run
    // it sits inside, and only falls back to the block's own style at a
    // boundary between two different runs.
    let mut prev: Option<Emph> = None;
    for word in ws {
        if word.brk {
            out.push(std::mem::replace(&mut cur, indent_spans(hang)));
            first = false;
            w = hang;
            prev = None;
            continue;
        }
        let sep = usize::from(w > if first { start } else { hang });
        if w + sep + word.width > width {
            // Flush unless the line is still empty — an over-wide word on a
            // fresh line hard-splits below instead of looping.
            if w > if first { start } else { hang } {
                out.push(std::mem::take(&mut cur));
                first = false;
                cur = indent_spans(hang);
                w = hang;
            }
            if word.width > width.saturating_sub(hang) {
                for (text, emph) in &word.parts {
                    for g in text.graphemes(true) {
                        if w + g.width() > width {
                            out.push(std::mem::take(&mut cur));
                            first = false;
                            cur = indent_spans(hang);
                            w = hang;
                        }
                        w += g.width();
                        push_text(&mut cur, g, style_of(*emph, role, paint));
                    }
                }
                prev = word.parts.last().map(|(_, e)| *e);
                continue;
            }
        }
        if w > if first { start } else { hang } {
            let next = word.parts.first().map(|(_, e)| *e);
            let joined = match (prev, next) {
                (Some(a), Some(b)) if a == b => a,
                _ => Emph::default(),
            };
            push_text(&mut cur, " ", style_of(joined, role, paint));
            w += 1;
        }
        for (text, emph) in &word.parts {
            w += text.width();
            push_text(&mut cur, text, style_of(*emph, role, paint));
        }
        prev = word.parts.last().map(|(_, e)| *e);
    }
    if cur.iter().any(|s| !s.content.trim().is_empty()) {
        out.push(cur);
    }
    out
}

/// The narrowest a squeezed table column may be. Below it a column of prose
/// is a word per row, and the table reads better as records.
const MIN_COL: usize = 10;

/// Widths for columns whose cells want `natural` cells, in `width`, and the
/// gutter between them — `None` when a column would get fewer than
/// `MIN_COL`. A table that fits keeps every column natural, with three
/// cells of air if there is room for them and two if not. One that does not
/// fit is water-filled: a column narrower than an even share keeps its
/// width, and the wide ones split what is left evenly and wrap in it.
fn columns(natural: &[usize], width: usize) -> Option<(Vec<usize>, usize)> {
    let n = natural.len();
    let total: usize = natural.iter().sum();
    for gutter in [3, 2] {
        if total + gutter * n.saturating_sub(1) <= width {
            return Some((natural.to_vec(), gutter));
        }
    }
    let gutter = 2;
    let mut left = width.checked_sub(gutter * n.saturating_sub(1))?;
    let mut widths = natural.to_vec();
    let mut open: Vec<usize> = (0..n).collect();
    while !open.is_empty() {
        let share = left / open.len();
        let (fits, wide): (Vec<usize>, Vec<usize>) =
            open.iter().partition(|&&c| natural[c] <= share);
        if fits.is_empty() {
            if share < MIN_COL {
                return None;
            }
            let extra = left % wide.len();
            for (k, c) in wide.iter().enumerate() {
                widths[*c] = share + usize::from(k < extra);
            }
            break;
        }
        left -= fits.iter().map(|c| natural[*c]).sum::<usize>();
        open = wide;
    }
    Some((widths, gutter))
}

struct Out<'a> {
    lines: Vec<Line<'static>>,
    max: usize,
    width: usize,
    theme: &'a Theme,
    paint: Paint,
    cut: bool,
}

impl Out<'_> {
    fn full(&self) -> bool {
        self.lines.len() >= self.max
    }

    fn line(&mut self, spans: Vec<Span<'static>>) -> bool {
        if self.full() {
            self.cut = true;
            return false;
        }
        self.lines.push(Line::from(spans));
        true
    }

    /// A breathing row: never at the top, never doubled, never the reason a
    /// preview claims to be cut.
    fn blank(&mut self) {
        if self.full() || self.lines.is_empty() {
            return;
        }
        if self.lines.last().is_some_and(is_blank) {
            return;
        }
        self.lines.push(Line::default());
    }

    /// A document's blocks, in order, until the rows run out.
    fn blocks(&mut self, mut bs: Vec<Block>) {
        while matches!(bs.last(), Some(Block::Blank)) {
            bs.pop();
        }
        let tier = self.theme.glyph_tier();
        for b in bs {
            if self.full() {
                self.cut = true;
                return;
            }
            match b {
                Block::Blank => self.blank(),
                Block::Head { level, runs } => {
                    // A heading earns its air: the row above it is what makes
                    // it read as a heading at all, since it cannot have a rule.
                    self.blank();
                    self.flow(&runs, Vec::new(), 0, Role::Head(level));
                }
                Block::Para { runs } => self.flow(&runs, Vec::new(), 0, Role::Body),
                Block::Quote { blocks } => self.quote(blocks),
                Block::Item { marker, indent, task, runs } => {
                    let bullet = if tier == Tier::Ascii { "-" } else { "\u{2022}" };
                    // A task's box stands in for the bullet, or follows the
                    // number; ticked, it wears the board's own check.
                    let tick = if tier == Tier::Ascii { "[x]" } else { "[\u{2713}]" };
                    let mark = match (marker, task) {
                        (None, None) => bullet.to_string(),
                        (None, Some(done)) => (if done { tick } else { "[ ]" }).to_string(),
                        (Some(m), None) => m,
                        (Some(m), Some(done)) => format!("{m} {}", if done { tick } else { "[ ]" }),
                    };
                    let hang = indent + mark.width() + 1;
                    let lead = vec![
                        Span::raw(" ".repeat(indent)),
                        Span::styled(format!("{mark} "), self.paint.dim2()),
                    ];
                    self.flow(&runs, lead, hang, Role::Body);
                }
                Block::Code { rows } => self.slab(&rows),
                Block::Table { head, align, rows } => self.table(&head, &align, &rows),
                Block::Raw { text } => self.raw(&text),
            }
        }
    }

    /// A quote: whatever it holds, drawn a value step down (`Paint::quoted`)
    /// behind 06 §5.1's own prescription for quoted text, the `›` prefix —
    /// on every row, a blank one included, so the quote reads as one piece.
    /// (The peek's `> user's words` fallback arrives here, which is exactly
    /// what it should look like.)
    fn quote(&mut self, blocks: Vec<Block>) {
        let mark = if self.theme.glyph_tier() == Tier::Ascii { ">" } else { "\u{203A}" };
        if self.width < 3 {
            return self.blocks(blocks);
        }
        let mut inner = Out {
            lines: Vec::new(),
            max: self.max.saturating_sub(self.lines.len()),
            width: self.width - 2,
            theme: self.theme,
            paint: self.paint.quoted(),
            cut: false,
        };
        inner.blocks(blocks);
        while inner.lines.last().is_some_and(is_blank) {
            inner.lines.pop();
        }
        for line in inner.lines {
            let lead = if is_blank(&line) { mark.to_string() } else { format!("{mark} ") };
            let mut spans = vec![Span::styled(lead, self.paint.dim3())];
            spans.extend(line.spans);
            if !self.line(spans) {
                return;
            }
        }
        self.cut |= inner.cut;
    }

    /// One block, word-wrapped to the column (`wrap`).
    fn flow(&mut self, runs: &[Run], lead: Vec<Span<'static>>, hang: usize, role: Role) {
        for line in wrap(runs, lead, hang, role, self.width, &self.paint) {
            if !self.line(line) {
                return;
            }
        }
    }

    /// A fenced block: the elevated surface painted behind it, no border
    /// (L1), no reflow — code that rewraps is code that lies. The slab is
    /// shrink-wrapped to its widest row, so it reads as a block of code and
    /// not as a band across the page.
    fn slab(&mut self, rows: &[String]) {
        let ink = Style::default().fg(self.paint.ink.dim1);
        let dim2 = self.paint.dim2();
        let inner =
            rows.iter().map(|r| r.width()).max().unwrap_or(0).min(self.width.saturating_sub(3));
        for r in rows {
            let body = crate::text::truncate(r, inner);
            let spans = match self.paint.code_bg {
                Some(bg) => {
                    let pad = inner.saturating_sub(body.width());
                    vec![
                        Span::raw(" "),
                        Span::styled(format!(" {body}{} ", " ".repeat(pad)), ink.bg(bg)),
                    ]
                }
                // Nothing to paint with: the inset carries the block.
                None => vec![Span::styled(format!("   {body}"), dim2)],
            };
            if !self.line(spans) {
                return;
            }
        }
    }

    /// A table, set in columns: each as wide as its widest cell, set apart by
    /// space (a rule is drawn structure, L1), aligned as its delimiter cell
    /// asked, the header in weight. Too wide for the page, the widest columns
    /// share what is left and their cells wrap — with a blank row between
    /// rows then, or a wrapped row runs into the next. Too narrow for even
    /// that, the table is read out a row at a time (`records`).
    fn table(&mut self, head: &[Vec<Run>], align: &[Align], rows: &[Vec<Vec<Run>>]) {
        let all = || std::iter::once(head).chain(rows.iter().map(Vec::as_slice));
        let mut want = vec![0usize; align.len()];
        for row in all() {
            for (c, cell) in row.iter().enumerate() {
                want[c] = want[c].max(natural(&words(cell, &self.paint)));
            }
        }
        let Some((widths, gutter)) = columns(&want, self.width) else {
            return self.records(head, rows);
        };
        let laid: Vec<Vec<Vec<Vec<Span<'static>>>>> = all()
            .enumerate()
            .map(|(k, row)| {
                let role = if k == 0 { Role::Head(1) } else { Role::Body };
                row.iter()
                    .zip(&widths)
                    .map(|(cell, &w)| wrap(cell, Vec::new(), 0, role, w, &self.paint))
                    .collect()
            })
            .collect();
        let airy = laid.iter().flatten().any(|cell| cell.len() > 1);
        for (k, row) in laid.iter().enumerate() {
            if airy && k > 0 {
                self.blank();
            }
            let height = row.iter().map(Vec::len).max().unwrap_or(0).max(1);
            for y in 0..height {
                let mut spans: Vec<Span<'static>> = Vec::new();
                for (c, cell) in row.iter().enumerate() {
                    let text = cell.get(y).cloned().unwrap_or_default();
                    let room = widths[c].saturating_sub(crate::ui::spans_width(&text));
                    let (before, after) = match align[c] {
                        Align::Left => (0, room),
                        Align::Right => (room, 0),
                        Align::Center => (room / 2, room - room / 2),
                    };
                    let last = c + 1 == row.len();
                    let pad = before + if c > 0 { gutter } else { 0 };
                    spans.extend(indent_spans(pad));
                    spans.extend(text);
                    if !last {
                        spans.extend(indent_spans(after));
                    }
                }
                // An empty last cell leaves only padding behind it.
                while spans
                    .last()
                    .is_some_and(|s| s.style == Style::default() && s.content.trim().is_empty())
                {
                    spans.pop();
                }
                if !self.line(spans) {
                    return;
                }
            }
        }
    }

    /// A table too wide to set in columns, read out one body row at a time:
    /// a `header: value` line per cell, a blank row between rows — what the
    /// table says, in the order a reader would say it.
    fn records(&mut self, head: &[Vec<Run>], rows: &[Vec<Vec<Run>>]) {
        let label = |cell: &[Run]| -> Vec<Run> {
            cell.iter()
                .map(|r| Run { text: r.text.clone(), emph: Emph { strong: true, ..r.emph } })
                .collect()
        };
        if rows.is_empty() {
            let runs: Vec<Run> = head
                .iter()
                .flat_map(|c| {
                    let mut l = label(c);
                    l.push(Run { text: " ".into(), emph: Emph::default() });
                    l
                })
                .collect();
            return self.flow(&runs, Vec::new(), 0, Role::Body);
        }
        for row in rows {
            self.blank();
            for (name, cell) in head.iter().zip(row) {
                if cell.is_empty() {
                    continue;
                }
                let mut runs = label(name);
                if !runs.is_empty() {
                    runs.push(Run { text: ": ".into(), emph: Emph::default() });
                }
                runs.extend(cell.iter().cloned());
                self.flow(&runs, Vec::new(), 2, Role::Body);
            }
        }
    }

    /// A pipe line that is not a table, verbatim: its columns may be spaced
    /// by hand, and a reflow would destroy that.
    fn raw(&mut self, text: &str) {
        let body = crate::text::truncate(text, self.width);
        self.line(vec![Span::styled(body, Style::default().fg(self.paint.ink.dim1))]);
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        while self.lines.last().is_some_and(is_blank) {
            self.lines.pop();
        }
        if self.cut {
            mark_cut(&mut self.lines, self.width, self.theme);
        }
        self.lines
    }
}

fn indent_spans(hang: usize) -> Vec<Span<'static>> {
    if hang == 0 {
        Vec::new()
    } else {
        vec![Span::raw(" ".repeat(hang))]
    }
}

fn is_blank(l: &Line<'static>) -> bool {
    l.spans.iter().all(|s| s.content.trim().is_empty())
}

fn push_text(spans: &mut Vec<Span<'static>>, text: &str, style: Style) {
    if let Some(last) = spans.last_mut() {
        if last.style == style {
            last.content.to_mut().push_str(text);
            return;
        }
    }
    spans.push(Span::styled(text.to_string(), style));
}

/// Trim a rendered line to `cells`, dropping whole graphemes off the tail.
fn clip(spans: &mut Vec<Span<'static>>, cells: usize) {
    let mut used = 0usize;
    let mut keep = 0usize;
    for (i, s) in spans.iter().enumerate() {
        let w = s.content.width();
        if used + w <= cells {
            used += w;
            keep = i + 1;
            continue;
        }
        let style = s.style;
        let mut head = String::new();
        for g in s.content.graphemes(true) {
            if used + g.width() > cells {
                break;
            }
            used += g.width();
            head.push_str(g);
        }
        spans.truncate(i);
        if !head.is_empty() {
            spans.push(Span::styled(head, style));
        }
        return;
    }
    spans.truncate(keep);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Profile, Theme};

    fn dark() -> Theme {
        Theme::new(crate::theme::Flavor::Graphite, Profile::TrueColor)
    }

    /// The rendered text, one String per row.
    fn plain(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
    }

    /// Every (text, style) pair, flattened — what actually reaches the cells.
    fn styled(lines: &[Line<'static>]) -> Vec<(String, Style)> {
        lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| (s.content.to_string(), s.style)))
            .collect()
    }

    fn style_for(lines: &[Line<'static>], needle: &str) -> Style {
        styled(lines)
            .into_iter()
            .find(|(t, _)| t.trim() == needle)
            .unwrap_or_else(|| panic!("{needle:?} not rendered in {:?}", plain(lines)))
            .1
    }

    /// The whole point: `**x**` must not reach the screen as four asterisks,
    /// and the emphasis it carries is a step up the ramp plus weight — never
    /// SGR 3, which 06 §5.1 bans and `test_no_banned_sgr` enforces.
    #[test]
    fn picture_references_render_as_compact_labels() {
        let body = format!(
            "See [Image #2]({}) here.",
            mesimon_core::attachment::target(ulid::Ulid::nil())
        );
        assert_eq!(plain(&render(&body, 80, 3, &dark())), vec!["See [Image #2] here."]);
    }

    #[test]
    fn emphasis_is_a_value_step_never_a_slant() {
        let t = dark();
        let out = render("plain **strong** and *soft* words", 60, 6, &t);
        assert_eq!(plain(&out), vec!["plain strong and soft words"]);
        let strong = style_for(&out, "strong");
        assert_eq!(strong.fg, Some(t.rest.base));
        assert!(strong.add_modifier.contains(Modifier::BOLD));
        let soft = style_for(&out, "soft");
        assert_eq!(soft.fg, Some(t.rest.base), "emphasis is the ramp step");
        assert!(!soft.add_modifier.contains(Modifier::BOLD));
        assert_eq!(style_for(&out, "plain").fg, Some(t.rest.dim1), "body stays where it was");
    }

    /// Agent replies are full of `snake_case` and `5 * 3`; a delimiter that
    /// does not flank real text, or has no closer at all, is just a character.
    #[test]
    fn stray_delimiters_stay_literal() {
        let t = dark();
        for src in ["call foo_bar_baz now", "5 * 3 = 15", "glob *.rs and *.toml", "a_b_c_d"] {
            let out = render(src, 60, 4, &t);
            assert_eq!(plain(&out), vec![src.to_string()], "{src}");
            let styles: Vec<Style> = styled(&out).into_iter().map(|(_, s)| s).collect();
            assert!(styles.iter().all(|s| s.fg == Some(t.rest.dim1)), "{src} picked up emphasis");
        }
    }

    /// A code span rides the one elevated surface; where the profile has none
    /// to paint, the backticks survive instead of the treatment being faked.
    #[test]
    fn code_is_painted_or_keeps_its_backticks() {
        let t = dark();
        let out = render("run `cargo test` twice", 60, 4, &t);
        assert_eq!(plain(&out), vec!["run cargo test twice"]);
        assert_eq!(style_for(&out, "cargo test").bg, t.code_bg());
        assert!(t.code_bg().is_some());

        let mono = Theme::new(crate::theme::Flavor::Graphite, Profile::Mono);
        assert_eq!(mono.code_bg(), None);
        let out = render("run `cargo test` twice", 60, 4, &mono);
        assert_eq!(plain(&out), vec!["run `cargo test` twice"]);
    }

    /// A list is a marker plus a hanging indent — the continuation lines up
    /// under the text, not under the bullet.
    #[test]
    fn a_list_hangs_under_its_marker() {
        let t = dark();
        let out = render("- alpha beta gamma delta\n- second\n\n1. numbered", 14, 8, &t);
        assert_eq!(
            plain(&out),
            vec!["\u{2022} alpha beta", "  gamma delta", "\u{2022} second", "", "1. numbered"]
        );
        // ASCII tier gets a hyphen, never a bullet it cannot draw.
        let ascii = Theme::new(crate::theme::Flavor::Graphite, Profile::Mono);
        assert_eq!(plain(&render("- x", 10, 2, &ascii)), vec!["- x"]);
    }

    /// L1: a fence is a painted slab, a thematic break is a BLANK ROW, and
    /// no drawn-structure codepoint appears anywhere.
    #[test]
    fn blocks_are_painted_never_drawn() {
        let t = dark();
        let out = render("before\n\n```rust\nlet x = 1;\n```\n\n---\n\nafter", 20, 10, &t);
        assert_eq!(
            plain(&out),
            vec!["before", "", "  let x = 1; ", "", "after"],
            "the fence markers and the info string are structure, not text"
        );
        let slab = &out[2];
        assert_eq!(slab.spans.last().expect("painted").style.bg, t.code_bg());
        for (text, _) in styled(&out) {
            for c in text.chars() {
                assert!(!(0x2500..=0x259F).contains(&(c as u32)), "drawn glyph {c:?}");
            }
        }
    }

    /// 06 §5.1's own prescription for quoted text — the `›` prefix and a
    /// value step. The peek's `> the user's own words` fallback lands here.
    #[test]
    fn a_quote_is_a_prefix_and_a_value_step() {
        let t = dark();
        let out = render("> fix the peek", 40, 4, &t);
        assert_eq!(plain(&out), vec!["\u{203A} fix the peek"]);
        assert_eq!(style_for(&out, "fix the peek").fg, Some(t.rest.dim2));
    }

    /// A heading takes weight and value, and buys a breathing row above it —
    /// with rules banned, that row is what makes it read as a heading.
    #[test]
    fn a_heading_takes_weight_and_air() {
        let t = dark();
        let out = render("body text\n## What changed\nmore", 40, 8, &t);
        assert_eq!(plain(&out), vec!["body text", "", "What changed", "more"]);
        let h = style_for(&out, "What changed");
        assert_eq!(h.fg, Some(t.rest.base));
        assert!(h.add_modifier.contains(Modifier::BOLD));
        // Never a leading blank: the zone already sits under one.
        assert_eq!(plain(&render("# Title\nbody", 40, 8, &t)), vec!["Title", "body"]);
    }

    /// A table is set in columns: its pipes and delimiter row are markup, so
    /// they go; the columns line up on space; the header is said with
    /// weight; and a cell's own markdown renders like any other text.
    #[test]
    fn a_table_is_set_in_columns() {
        let t = dark();
        let src = "| key | state |\n|---|---|\n| T-1 | **done** |\n| T-22 | `open` |";
        let out = render(src, 40, 6, &t);
        assert_eq!(plain(&out), vec!["key    state", "T-1    done", "T-22   open"]);
        let head = style_for(&out, "key");
        assert_eq!(head.fg, Some(t.rest.base));
        assert!(head.add_modifier.contains(Modifier::BOLD));
        assert_eq!(style_for(&out, "T-1").fg, Some(t.rest.dim1), "the body is body");
        assert!(style_for(&out, "done").add_modifier.contains(Modifier::BOLD));
        assert_eq!(style_for(&out, "open").bg, t.code_bg());
    }

    /// The delimiter row's colons are honoured; the outer pipes are optional
    /// (GFM); `\|` is a pipe inside a cell; a short row is padded and a long
    /// one cut to the header's column count.
    #[test]
    fn a_table_aligns_and_forgives_its_source() {
        let t = dark();
        let src = "name | n | mid\n:--- | --: | :-:\na | 1 | x\nbb | 200 | yyy | extra\nc\\|d | 3";
        assert_eq!(
            plain(&render(src, 40, 8, &t)),
            vec!["name     n   mid", "a        1    x", "bb     200   yyy", "c|d      3"]
        );
    }

    /// Too wide for the page, the narrow column keeps its width and the wide
    /// one wraps in what is left — with a blank row between rows, or the
    /// wrapped row would run into the next one.
    #[test]
    fn a_wide_table_wraps_its_widest_column() {
        let t = dark();
        let src = "| file | why |\n|---|---|\n| a.rs | the parse reads a delimiter row \
                   now |\n| b.rs | short |";
        let out = render(src, 30, 12, &t);
        assert_eq!(
            plain(&out),
            vec![
                "file  why",
                "",
                "a.rs  the parse reads a",
                "      delimiter row now",
                "",
                "b.rs  short",
            ]
        );
        assert!(out.iter().all(|l| crate::ui::spans_width(&l.spans) <= 30));
    }

    /// Too narrow for columns worth reading, the table is read out a row at
    /// a time: `header: value`, the header in weight.
    #[test]
    fn a_table_too_wide_for_columns_reads_as_records() {
        let t = dark();
        let src = "| key | what changed | why it changed |\n|---|---|---|\n\
                   | T-1 | the parser | a table |\n| T-2 | the layout | columns |";
        let out = render(src, 24, 12, &t);
        assert_eq!(
            plain(&out),
            vec![
                "key: T-1",
                "what changed: the parser",
                "why it changed: a table",
                "",
                "key: T-2",
                "what changed: the layout",
                "why it changed: columns",
            ]
        );
        assert!(style_for(&out, "key").add_modifier.contains(Modifier::BOLD));
    }

    /// A pipe line with no delimiter row under it is not a table — it may
    /// be spaced by hand, so it stays verbatim — and a pipe in prose over a
    /// thematic break is a paragraph, not a one-column table.
    #[test]
    fn a_pipe_line_without_a_delimiter_is_not_a_table() {
        let t = dark();
        assert_eq!(
            plain(&render("| a  | b |\n| cc | d |", 40, 4, &t)),
            vec!["| a  | b |", "| cc | d |"]
        );
        assert_eq!(plain(&render("a | b\n---\nc", 40, 4, &t)), vec!["a | b", "", "c"]);
    }

    /// The zone cannot follow a link, so the target is the half worth
    /// keeping — quietly, one step below the body.
    #[test]
    fn a_link_keeps_its_target() {
        let t = dark();
        let out = render("see [the spec](docs/06.md) first", 60, 4, &t);
        assert_eq!(plain(&out), vec!["see the spec docs/06.md first"]);
        assert_eq!(style_for(&out, "docs/06.md").fg, Some(t.rest.dim3));
    }

    /// `***x***` opens `**` then `*`, and its closing run ends both — the
    /// closer used to match only a run of exactly the open length, leaked
    /// `**` and left every later emphasis on the line inverted.
    #[test]
    fn triple_emphasis_closes_both() {
        let t = dark();
        let out = render("a ***b*** c **d** e", 40, 2, &t);
        assert_eq!(plain(&out), vec!["a b c d e"]);
        assert!(style_for(&out, "b").add_modifier.contains(Modifier::BOLD));
        assert!(style_for(&out, "d").add_modifier.contains(Modifier::BOLD));
        assert_eq!(style_for(&out, "c").fg, Some(t.rest.dim1), "nothing leaks past it");
        // A longer run still closes a shorter opener, and a lone star stays.
        assert_eq!(plain(&render("*x** and **y*", 40, 2, &t)), vec!["x* and *y"]);
    }

    /// CommonMark's hard breaks — two trailing spaces, a trailing backslash
    /// (which is markup, so it goes) — and `<br>`, all end the line there.
    #[test]
    fn a_hard_break_ends_the_line() {
        let t = dark();
        let out = render("one  \ntwo\\\nthree four\n\na<br>b<BR />c", 40, 8, &t);
        assert_eq!(plain(&out), vec!["one", "two", "three four", "", "a", "b", "c"]);
        // Inside a list item the broken line hangs under the text.
        assert_eq!(plain(&render("- one  \n  two", 40, 4, &t)), vec!["\u{2022} one", "  two"]);
        // Emphasis spans a break.
        let out = render("**bold  \nstill**", 40, 4, &t);
        assert!(style_for(&out, "still").add_modifier.contains(Modifier::BOLD));
    }

    /// A person's newline is a line break; an agent's is a space, as
    /// CommonMark has it, because an agent wraps by hand.
    #[test]
    fn a_persons_newlines_are_breaks() {
        let t = dark();
        let src = "repro:\nopen the board\npress X\n\n- item\n  more";
        let keep = render_on(src, 40, 8, &t, Surface::Ground, Newline::Break);
        assert_eq!(
            plain(&keep),
            vec!["repro:", "open the board", "press X", "", "\u{2022} item", "  more"]
        );
        let join = render_on(src, 40, 8, &t, Surface::Ground, Newline::Space);
        assert_eq!(plain(&join), vec!["repro: open the board press X", "", "\u{2022} item more"]);

        let meta = |by: &str| mesimon_core::board::NoteMeta {
            id: ulid::Ulid::nil(),
            name: String::new(),
            rev: 1,
            created_at: String::new(),
            created_by: String::new(),
            edited_at: String::new(),
            edited_by: by.into(),
        };
        for by in ["local", "device:phone", "member:dana"] {
            assert_eq!(Newline::of_note(&meta(by)), Newline::Break, "{by}");
        }
        for by in ["agent:0000", "automation:rule", ""] {
            assert_eq!(Newline::of_note(&meta(by)), Newline::Space, "{by}");
        }
    }

    /// A task item's box stands in for the bullet (or follows the number),
    /// ticked with the board's own check, and a ticked item is read last.
    #[test]
    fn a_task_list_wears_boxes() {
        let t = dark();
        let src = "- [ ] write it\n- [x] test it\n* [X] ship it\n1. [ ] numbered";
        let out = render(src, 40, 6, &t);
        assert_eq!(
            plain(&out),
            vec!["[ ] write it", "[\u{2713}] test it", "[\u{2713}] ship it", "1. [ ] numbered"]
        );
        assert_eq!(style_for(&out, "write it").fg, Some(t.rest.dim1), "what is left stands out");
        assert_eq!(style_for(&out, "test it").fg, Some(t.rest.dim3), "what is done is read last");
        let ascii = Theme::new(crate::theme::Flavor::Graphite, Profile::Mono);
        assert_eq!(plain(&render("- [x] done", 20, 2, &ascii)), vec!["[x] done"]);
        // Not a task: no space after the box, or a box mid-line.
        assert_eq!(plain(&render("- [x]", 20, 2, &t)), vec!["\u{2022} [x]"]);
    }

    /// A quote holds blocks — paragraphs, a list, another quote — each
    /// drawn a value step down behind the mark, which runs down blank rows
    /// too; lazy continuation joins its paragraph, but not across a blank.
    #[test]
    fn a_quote_holds_blocks() {
        let t = dark();
        let out = render("> para one\n>\n> - a\n> - b\n> > inner", 40, 8, &t);
        assert_eq!(
            plain(&out),
            vec![
                "\u{203A} para one",
                "\u{203A}",
                "\u{203A} \u{2022} a",
                "\u{203A} \u{2022} b",
                "\u{203A} \u{203A} inner"
            ]
        );
        assert_eq!(style_for(&out, "para one").fg, Some(t.rest.dim2), "one step down");
        assert_eq!(style_for(&out, "inner").fg, Some(t.rest.dim3), "and another");
        assert_eq!(plain(&render("> quoted\nlazy", 40, 4, &t)), vec!["\u{203A} quoted lazy"]);
        assert_eq!(
            plain(&render("> quoted\n>\nafter", 40, 4, &t)),
            vec!["\u{203A} quoted", "after"]
        );
        // Emphasis inside a quote still steps up from the quote's value.
        let out = render("> a **b** c", 40, 2, &t);
        assert_eq!(style_for(&out, "b").fg, Some(t.rest.dim1));
    }

    /// A GitHub alert names itself in weight, on its own line — no colour.
    #[test]
    fn a_github_alert_names_itself() {
        let t = dark();
        let out = render("> [!WARNING]\n> Careful here.", 40, 4, &t);
        assert_eq!(plain(&out), vec!["\u{203A} Warning", "\u{203A} Careful here."]);
        assert!(style_for(&out, "Warning").add_modifier.contains(Modifier::BOLD));
        // An unknown kind is just text.
        assert_eq!(plain(&render("> [!FOO] x", 40, 2, &t)), vec!["\u{203A} [!FOO] x"]);
    }

    /// `<br>` in a cell is GFM's multi-line cell: the column is as wide as
    /// its widest line, and the table takes air between its rows.
    #[test]
    fn a_br_breaks_a_table_cell() {
        let t = dark();
        let src = "| a | b |\n|---|---|\n| one<br>two | x |\n| y | z |";
        assert_eq!(
            plain(&render(src, 40, 8, &t)),
            vec!["a     b", "", "one   x", "two", "", "y     z"]
        );
    }

    /// Overflow ends in the `~` marker (07 §4.1's vocabulary), and nothing
    /// ever escapes the column it was given.
    #[test]
    fn the_cut_is_marked_and_the_column_holds() {
        let t = dark();
        let out = render("one two three four five six seven eight", 12, 2, &t);
        assert_eq!(plain(&out), vec!["one two", "three four~"]);
        // A word wider than the column hard-splits instead of vanishing.
        assert_eq!(plain(&render("abcdefghij", 4, 3, &t)), vec!["abcd", "efgh", "ij"]);
        assert!(render("x", 0, 4, &t).is_empty());
        assert!(render("x", 10, 0, &t).is_empty());
        // A trailing blank line is not a cut.
        assert_eq!(plain(&render("done\n\n", 20, 1, &t)), vec!["done"]);
    }

    /// The law this module lives under: the transcript may spend value,
    /// weight, the elevated surface and space — and nothing else. No banned
    /// SGR, and never one of the three chromatic tokens, which belong to
    /// needs-you (and to err/calm), never to prose.
    #[test]
    fn markdown_never_spends_a_banned_attribute_or_the_accent() {
        let kitchen = "# Title\n\nbody with **strong**, *soft*, `code`, ~~dead~~ and a \
                       [link](http://x.test/y).\n\n- one\n- two with a much longer tail\n\n\
                       ```sh\ncargo test --workspace\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n\
                       | file | what | why it changed, at length |\n|:--|:-:|--:|\n| `a.rs` | \
                       **parse** | a delimiter row makes a table now |\n\n\
                       > quoted\n\n---\n\ntail_with_snake_case and 5 * 3\n\n\
                       - [ ] open task\n- [x] **done** task\n\n> [!WARNING]\n> a *quoted* list:\n\
                       > - one\n> > nested `code`\n\nhard  \nbreak and ***both*** and a<br>b\n\n\
                       | cell | other |\n|---|---|\n| one<br>two | x |";
        for profile in
            [Profile::TrueColor, Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono]
        {
            for t in crate::theme::Flavor::ALL.map(|f| Theme::new(f, profile)) {
                for width in [12usize, 40, 83] {
                    let out = render(kitchen, width, 40, &t);
                    let ramp = [t.rest.base, t.rest.dim1, t.rest.dim2, t.rest.dim3];
                    for line in &out {
                        let w: usize = crate::ui::spans_width(&line.spans);
                        assert!(w <= width, "{profile:?} w={width}: line is {w} cells");
                        for s in &line.spans {
                            let m = s.style.add_modifier;
                            assert!(!m.contains(Modifier::DIM));
                            assert!(!m.contains(Modifier::ITALIC));
                            assert!(!m.contains(Modifier::CROSSED_OUT));
                            assert!(!m.contains(Modifier::REVERSED));
                            assert!(!m.contains(Modifier::SLOW_BLINK));
                            assert!(!m.contains(Modifier::UNDERLINED));
                            if let Some(fg) = s.style.fg {
                                assert!(ramp.contains(&fg), "{profile:?}: off-ramp fg {fg:?}");
                            }
                            if let Some(bg) = s.style.bg {
                                assert_eq!(Some(bg), t.code_bg(), "{profile:?}: stray paint");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Struck text is dead text: said with the de-emphasis token, since SGR 9
    /// is banned (and absent from every terminfo entry the corpus checked).
    #[test]
    fn struck_text_falls_to_the_de_emphasis_token() {
        let t = dark();
        let out = render("keep ~~drop~~ keep", 40, 4, &t);
        assert_eq!(plain(&out), vec!["keep drop keep"]);
        assert_eq!(style_for(&out, "drop").fg, Some(t.rest.dim3));
    }

    /// Markdown's lazy continuation: a wrapped source line belongs to the
    /// block above it, so a reply that hard-wraps at 80 does not come out as
    /// a stack of one-line paragraphs.
    #[test]
    fn wrapped_source_lines_rejoin() {
        let t = dark();
        let out = render("one two\nthree four\n\nnext", 40, 6, &t);
        assert_eq!(plain(&out), vec!["one two three four", "", "next"]);
        let out = render("- item that was\n  hard wrapped", 40, 4, &t);
        assert_eq!(plain(&out), vec!["\u{2022} item that was hard wrapped"]);
    }
}
