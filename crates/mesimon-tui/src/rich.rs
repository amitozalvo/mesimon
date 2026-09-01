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
//! viewer. What an agent reply actually uses is here; reference links,
//! footnotes, HTML and nested block quoting are not.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::glyphs::Tier;
use crate::theme::Theme;

/// Render `src` into at most `max_lines` lines of at most `width` cells.
/// A cut ends in the `~` marker — 07 §4.1's vocabulary, same as `truncate`.
pub(crate) fn render(
    src: &str,
    width: usize,
    max_lines: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let mut out = Out { lines: Vec::new(), max: max_lines, width, theme, cut: false };
    let tier = theme.glyph_tier();
    let mut bs = blocks(src);
    while matches!(bs.last(), Some(Block::Blank)) {
        bs.pop();
    }
    for b in bs {
        if out.full() {
            out.cut = true;
            break;
        }
        match b {
            Block::Blank => out.blank(),
            Block::Head { level, runs } => {
                // A heading earns its air: the row above it is what makes it
                // read as a heading at all, since it cannot have a rule.
                out.blank();
                out.flow(&runs, Vec::new(), 0, Role::Head(level));
            }
            Block::Para { runs, indent } => {
                out.flow(&runs, vec![Span::raw(" ".repeat(indent))], indent, Role::Body);
            }
            Block::Quote { runs } => {
                // 06 §5.1's own prescription for quoted text: a `›` prefix
                // plus a value step. (The peek's `> user's words` fallback
                // arrives here, which is exactly what it should look like.)
                let mark = if tier == Tier::Ascii { "> " } else { "› " };
                let lead = vec![Span::styled(mark, theme.dim3())];
                out.flow(&runs, lead, 2, Role::Quote);
            }
            Block::Item { marker, indent, runs } => {
                let marker = marker.unwrap_or_else(|| {
                    if tier == Tier::Ascii { "-" } else { "\u{2022}" }.to_string()
                });
                let hang = indent + marker.width() + 1;
                let lead = vec![
                    Span::raw(" ".repeat(indent)),
                    Span::styled(format!("{marker} "), theme.dim2()),
                ];
                out.flow(&runs, lead, hang, Role::Body);
            }
            Block::Code { rows } => out.slab(&rows),
            Block::Row { text, head } => out.row(&text, head),
        }
    }
    out.finish()
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
    /// The target half of a link: kept, because a terminal cannot follow one.
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
    Quote,
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
        indent: usize,
    },
    Quote {
        runs: Vec<Run>,
    },
    /// `marker: None` is a bullet — which glyph that is belongs to the
    /// tier, not to the parse.
    Item {
        marker: Option<String>,
        indent: usize,
        runs: Vec<Run>,
    },
    /// A fenced block. The info string is dropped: naming the language costs
    /// a whole row of a 15-row zone and the slab already says "this is code".
    Code {
        rows: Vec<String>,
    },
    /// One row of a pipe table, kept verbatim so its columns stay lined up
    /// (reflowing a table is what destroys it). `head` is the row above the
    /// delimiter, which is dropped — with rules banned, the header is marked
    /// by value instead.
    Row {
        text: String,
        head: bool,
    },
}

/// What is still being accumulated when the next line arrives.
enum Pending {
    None,
    Para(Vec<String>),
    Quote(Vec<String>),
    Item { marker: Option<String>, indent: usize, lines: Vec<String> },
}

fn blocks(src: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut pending = Pending::None;
    let mut fence: Option<(char, usize, Vec<String>)> = None;

    for raw in src.lines() {
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
            flush(&mut pending, &mut out);
            fence = Some((ch, len, Vec::new()));
            continue;
        }
        let trimmed = raw.trim_end();
        let body = trimmed.trim_start();
        let indent = trimmed.len() - body.len();

        if body.is_empty() {
            flush(&mut pending, &mut out);
            out.push(Block::Blank);
            continue;
        }
        // A thematic break is a blank row: rules are drawn structure (L1).
        if is_break(body) {
            flush(&mut pending, &mut out);
            out.push(Block::Blank);
            continue;
        }
        if let Some((level, text)) = heading(body) {
            flush(&mut pending, &mut out);
            out.push(Block::Head { level, runs: inline(text) });
            continue;
        }
        if body.starts_with('|') {
            flush(&mut pending, &mut out);
            if is_table_rule(body) {
                // Mark the row above as the header and drop this one.
                if let Some(Block::Row { head, .. }) = out.last_mut() {
                    *head = true;
                }
            } else {
                out.push(Block::Row { text: body.to_string(), head: false });
            }
            continue;
        }
        if let Some((marker, text)) = list_marker(body) {
            flush(&mut pending, &mut out);
            // Two source spaces per level, capped: a deep tree would spend
            // the whole column on indent.
            let depth = (indent / 2).min(3);
            pending = Pending::Item { marker, indent: depth * 2, lines: vec![text.to_string()] };
            continue;
        }
        if let Some(text) = quote_line(body) {
            match &mut pending {
                Pending::Quote(lines) => lines.push(text.to_string()),
                _ => {
                    flush(&mut pending, &mut out);
                    pending = Pending::Quote(vec![text.to_string()]);
                }
            }
            continue;
        }
        // Plain text: markdown's lazy continuation — it belongs to whatever
        // block is open (a list item's second line, a quote's), else it
        // starts a paragraph.
        match &mut pending {
            Pending::Para(lines) | Pending::Quote(lines) => lines.push(body.to_string()),
            Pending::Item { lines, .. } => lines.push(body.to_string()),
            Pending::None => pending = Pending::Para(vec![body.to_string()]),
        }
    }
    if let Some((_, _, rows)) = fence.take() {
        out.push(Block::Code { rows }); // unterminated fence: show it anyway
    }
    flush(&mut pending, &mut out);
    out
}

fn flush(pending: &mut Pending, out: &mut Vec<Block>) {
    match std::mem::replace(pending, Pending::None) {
        Pending::None => {}
        Pending::Para(lines) => out.push(Block::Para { runs: inline(&lines.join(" ")), indent: 0 }),
        Pending::Quote(lines) => out.push(Block::Quote { runs: inline(&lines.join(" ")) }),
        Pending::Item { marker, indent, lines } => {
            out.push(Block::Item { marker, indent, runs: inline(&lines.join(" ")) })
        }
    }
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

fn is_table_rule(body: &str) -> bool {
    body.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) && body.contains('-')
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

/// Does a valid closer for this delimiter exist ahead? An opener with none is
/// literal text (`5 * 3` must not eat the rest of the line).
fn has_closer(chars: &[char], from: usize, ch: char, len: usize) -> bool {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == ch && delim_len(chars, i) == len && closes(chars, i, len) {
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
                if open.last() == Some(&(c, len)) && closes(&chars, i, len) {
                    push_run(&mut out, &mut cur, emph);
                    open.pop();
                    *flag(&mut emph, c, len) = false;
                    i += len;
                    continue;
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
/// The url is kept because nothing in a terminal can follow a link: dropping
/// it would delete the only half the reader can act on.
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

fn style_of(e: Emph, role: Role, theme: &Theme) -> Style {
    let t = &theme.rest;
    let (fg, bold) = match role {
        // Two heading levels, both by value; the top two also take weight.
        Role::Head(1..=2) => (t.base, true),
        Role::Head(_) => (t.base, false),
        Role::Quote => (t.dim2, e.strong),
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
        if let Some(bg) = theme.code_bg() {
            s = s.bg(bg);
        }
    }
    s
}

/// One display word: the parts are the styled runs it is made of, kept
/// together so `**bold**text` breaks as one word and not as two.
struct Word {
    parts: Vec<(String, Emph)>,
    width: usize,
}

fn words(runs: &[Run], theme: &Theme) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut open = false; // is the last word still being extended?
    for r in runs {
        // Where the profile cannot paint, the marker survives instead.
        let text = if r.emph.code && theme.code_bg().is_none() {
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
            out.push(Word { width: chunk.width(), parts: vec![(chunk.to_string(), r.emph)] });
            open = true;
        }
        // A run that ended mid-word keeps the next run attached.
        open = open && !text.ends_with(char::is_whitespace);
    }
    out
}

struct Out<'a> {
    lines: Vec<Line<'static>>,
    max: usize,
    width: usize,
    theme: &'a Theme,
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

    /// Greedy word wrap of one block. `lead` opens the first line (a bullet,
    /// a quote mark, an indent) and `hang` is the inset every later line of
    /// the same block keeps.
    fn flow(&mut self, runs: &[Run], lead: Vec<Span<'static>>, hang: usize, role: Role) {
        let ws = words(runs, self.theme);
        if ws.is_empty() {
            return;
        }
        let mut cur: Vec<Span<'static>> = lead;
        cur.retain(|s| !s.content.is_empty());
        let mut w: usize = cur.iter().map(|s| s.content.width()).sum();
        let start = w;
        let mut first = true;
        // The space BETWEEN two words of one code span must be painted too,
        // or the slab comes out with a hole in it — so a separator inherits
        // the run it sits inside, and only falls back to the block's own
        // style at a boundary between two different runs.
        let mut prev: Option<Emph> = None;
        for word in ws {
            let sep = usize::from(w > if first { start } else { hang });
            if w + sep + word.width > self.width {
                // Flush unless the line is still empty — an over-wide word on
                // a fresh line hard-splits below instead of looping.
                if w > if first { start } else { hang } {
                    if !self.line(std::mem::take(&mut cur)) {
                        return;
                    }
                    first = false;
                    cur = indent_spans(hang);
                    w = hang;
                }
                if word.width > self.width.saturating_sub(hang) {
                    for (text, emph) in &word.parts {
                        for g in text.graphemes(true) {
                            if w + g.width() > self.width {
                                if !self.line(std::mem::take(&mut cur)) {
                                    return;
                                }
                                first = false;
                                cur = indent_spans(hang);
                                w = hang;
                            }
                            w += g.width();
                            push_text(&mut cur, g, style_of(*emph, role, self.theme));
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
                push_text(&mut cur, " ", style_of(joined, role, self.theme));
                w += 1;
            }
            for (text, emph) in &word.parts {
                w += text.width();
                push_text(&mut cur, text, style_of(*emph, role, self.theme));
            }
            prev = word.parts.last().map(|(_, e)| *e);
        }
        if cur.iter().any(|s| !s.content.trim().is_empty()) {
            self.line(cur);
        }
    }

    /// A fenced block: the elevated surface painted behind it, no border
    /// (L1), no reflow — code that rewraps is code that lies. The slab is
    /// shrink-wrapped to its widest row, so it reads as a block of code and
    /// not as a band across the page.
    fn slab(&mut self, rows: &[String]) {
        let theme = self.theme;
        let ink = Style::default().fg(theme.rest.dim1);
        let inner =
            rows.iter().map(|r| r.width()).max().unwrap_or(0).min(self.width.saturating_sub(3));
        for r in rows {
            let body = crate::text::truncate(r, inner);
            let spans = match theme.code_bg() {
                Some(bg) => {
                    let pad = inner.saturating_sub(body.width());
                    vec![
                        Span::raw(" "),
                        Span::styled(format!(" {body}{} ", " ".repeat(pad)), ink.bg(bg)),
                    ]
                }
                // Nothing to paint with: the inset carries the block.
                None => vec![Span::styled(format!("   {body}"), theme.dim2())],
            };
            if !self.line(spans) {
                return;
            }
        }
    }

    /// One table row, verbatim: the columns are already aligned and any
    /// reflow would destroy that. The header row is marked by value.
    fn row(&mut self, text: &str, head: bool) {
        let style = if head {
            Style::default().fg(self.theme.rest.base).add_modifier(Modifier::BOLD)
        } else {
            self.theme.dim1()
        };
        let body = crate::text::truncate(text, self.width.saturating_sub(1));
        self.line(vec![Span::raw(" "), Span::styled(body, style)]);
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        while self.lines.last().is_some_and(is_blank) {
            self.lines.pop();
        }
        if self.cut {
            let width = self.width;
            let style = self.theme.dim2();
            if let Some(last) = self.lines.last_mut() {
                let w: usize = last.spans.iter().map(|s| s.content.width()).sum();
                if w >= width {
                    clip(&mut last.spans, width.saturating_sub(1));
                }
                last.spans.push(Span::styled("~", style));
            }
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
        Theme::graphite(Profile::TrueColor)
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

        let mono = Theme::graphite(Profile::Mono);
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
        let ascii = Theme::graphite(Profile::Mono);
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

    /// A table is the one thing that must NOT reflow: its columns are its
    /// meaning. The delimiter row is markup, so it goes — and the header it
    /// marked is said with value instead.
    #[test]
    fn a_table_keeps_its_columns() {
        let t = dark();
        let src = "| key | state |\n|-----|-------|\n| T-1 | done  |";
        let out = render(src, 40, 6, &t);
        assert_eq!(plain(&out), vec![" | key | state |", " | T-1 | done  |"]);
        assert!(style_for(&out, "| key | state |").add_modifier.contains(Modifier::BOLD));
    }

    /// A terminal cannot follow a link, so the target is the half worth
    /// keeping — quietly, one step below the body.
    #[test]
    fn a_link_keeps_its_target() {
        let t = dark();
        let out = render("see [the spec](docs/06.md) first", 60, 4, &t);
        assert_eq!(plain(&out), vec!["see the spec docs/06.md first"]);
        assert_eq!(style_for(&out, "docs/06.md").fg, Some(t.rest.dim3));
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
                       > quoted\n\n---\n\ntail_with_snake_case and 5 * 3";
        for profile in
            [Profile::TrueColor, Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono]
        {
            for t in [Theme::graphite(profile), Theme::chalk(profile)] {
                for width in [12usize, 40, 83] {
                    let out = render(kitchen, width, 40, &t);
                    let ramp = [t.rest.base, t.rest.dim1, t.rest.dim2, t.rest.dim3];
                    for line in &out {
                        let w: usize = line.spans.iter().map(|s| s.content.width()).sum();
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
