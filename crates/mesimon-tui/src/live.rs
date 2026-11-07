//! The live window (T-1143): the selected agent session's pane, as tmux
//! holds it, at the foot of the ticket page's zone — the composer, a
//! question, a permission prompt, a plan, in the agent's own colours — with
//! the pane's rows above it fading into the page, and mesimon's own reading
//! of the conversation above that.
//!
//! The pane is read by the client itself, on a thread of its own, through
//! the private server's socket the attach already uses: `capture-pane -e`
//! gives the screen with its SGR attributes and `display-message` in the
//! same command queue gives the cursor, the road the daemon's composer check
//! reads the screen by. No daemon round-trip and no fork on its writer
//! (T-857); a capture that reads the same as the last sends nothing.
//!
//! While the window shows a pane, the pane is asked to be the zone's size
//! (`resize-window`, which marks the window `manual`), so the agent lays
//! its screen out for the room it has — a pane nobody is attached to is
//! 80 by 24 otherwise. Never while a client is attached to it, and undone
//! (`window-size` unset, back to the server's `latest`) when the window
//! leaves the pane, which the attach handover does before it execs.
//!
//! Measured 2026-11-06 on Claude Code 2.1.319 under tmux 3.6a: every agent
//! pane runs on the alternate screen, so the visible screen is the whole
//! interface, and its foot reads, bottom up, two footer rows, a rule of
//! `─`, the `❯` row, a rule, and a status or spinner row; a dialog replaces
//! the box and is taller.

use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use mesimon_daemon::agents::claude::composer::is_rule;
use ratatui::style::{Color, Modifier, Style};

/// How often the pane is captured while the window shows it.
pub(crate) const EVERY: Duration = Duration::from_millis(200);
/// After a fit the agent redraws for its new size; a capture inside this
/// is its old screen cut to the new one. Claude Code 2.1.319, idle, redrew
/// within 12–20 ms of a `resize-window` (measured 2026-11-06); a busy turn
/// gets the margin.
const SETTLE: Duration = Duration::from_millis(60);
/// How long a capture of another size than the zone's is held back before
/// it is sent all the same: a fit refused while a client is attached, an
/// agent that never redraws.
const SIZE_WAIT: Duration = Duration::from_millis(1000);
/// The most rows of the pane that fade above the live rows.
pub(crate) const FADE_ROWS: usize = 4;
/// Rows of the pane's own context above the composer's rule kept live: the
/// status or spinner row Claude Code hangs there.
const CONTEXT_ROWS: usize = 1;
/// The window when the screen shows neither a composer nor a dialog.
const FALLBACK_ROWS: usize = 8;
/// How far above a dialog's marker row its top is looked for: a plan
/// approval is the whole screen (Claude Code 2.1.319, measured 2026-11-06).
const DIALOG_REACH: usize = 40;
/// How far below a composer's prompt row its bottom rule is looked for:
/// the typed words wrap inside the box.
const BOX_REACH: usize = 12;

/// One row of the pane: its styled runs, and its words for the seam search.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Row {
    pub runs: Vec<(String, Style)>,
    pub text: String,
}

impl Row {
    pub(crate) fn blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// One capture of the pane's screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Shot {
    pub rows: Vec<Row>,
    pub cols: u16,
    pub cursor: Option<(u16, u16)>,
}

impl Shot {
    /// The first row of the live window: the composer's top rule and the
    /// row above it; a dialog from its top (`dialog_top`); or the screen's
    /// last `FALLBACK_ROWS` rows with words when neither shape is on it.
    pub(crate) fn anchor(&self) -> usize {
        if let Some(p) = self.boxed_prompt() {
            return (p - 1).saturating_sub(CONTEXT_ROWS);
        }
        // Codex's composer (measured 2026-11-06, 0.166.0): a `›` row with no
        // rule on either side, the cursor on it, a blank row over it.
        if let Some(p) = self.cursor_prompt() {
            return p.saturating_sub(CONTEXT_ROWS);
        }
        let n = self.rows.len();
        if let Some(m) = (0..n).rev().find(|&i| is_marker(&self.rows[i].text)) {
            return self.dialog_top(m);
        }
        let end = self.rows.iter().rposition(|r| !r.blank()).map_or(n, |e| e + 1);
        end.saturating_sub(FALLBACK_ROWS)
    }

    /// The composer's own prompt row, where the words are typed: the row
    /// under its top rule, or the unframed prompt row the cursor is on.
    /// `None` while a dialog has the box's place.
    pub(crate) fn prompt_row(&self) -> Option<usize> {
        self.boxed_prompt().or_else(|| self.cursor_prompt())
    }

    /// The composer's box (T-1156): its top rule to its bottom rule
    /// (Claude Code), or Codex's lone prompt row. `None` while a dialog has
    /// the box's place.
    pub(crate) fn box_rows(&self) -> Option<(usize, usize)> {
        if let Some(p) = self.boxed_prompt() {
            let n = self.rows.len();
            let bottom = (p + 1..(p + BOX_REACH).min(n)).find(|&j| is_rule(&self.rows[j].text))?;
            return Some((p - 1, bottom));
        }
        self.cursor_prompt().map(|p| (p, p))
    }

    /// Whether the box holds words (T-1156). A placeholder is none: the
    /// agent draws it where the input starts and parks the cursor before it
    /// (the daemon's `composer::read` reads the same). `None` with no box.
    pub(crate) fn box_holds(&self) -> Option<bool> {
        let (top, bottom) = self.box_rows()?;
        let p = self.prompt_row()?;
        let row = &self.rows[p].text;
        let col = words_col(row);
        let first = row.chars().skip(col).collect::<String>();
        let wrapped = (p + 1..bottom).any(|i| !self.rows[i].blank());
        if first.trim_matches([' ', '\u{a0}']).is_empty() && !wrapped {
            return Some(false);
        }
        // The cursor parked at the input's start, past the mark and its
        // space (a no-break space on Claude Code 2.1.3xx), with nothing
        // wrapped under it: the words are the agent's placeholder.
        let parked = self
            .cursor
            .is_some_and(|(x, y)| y as usize == p && (x as usize) <= col + 1 && !wrapped);
        let _ = top;
        Some(!parked)
    }

    /// The screen as the ticket page draws it (T-1156): nothing under the
    /// box — Claude Code's footer, the person's status line, Codex's hints
    /// row; the page draws its own facts row there — and the agent's key
    /// hints out of its status rows (`strip_hints`), whose keys the page
    /// answers differently. A screen with no box keeps its rows: a dialog's
    /// own keys are the dialog's.
    pub(crate) fn for_page(&self) -> Shot {
        let mut rows = self.rows.clone();
        let Some((top, bottom)) = self.box_rows() else {
            return Shot { rows, ..self.clone() };
        };
        rows.truncate(bottom + 1);
        for row in rows.iter_mut().take(top) {
            *row = strip_hints(row);
        }
        // Codex's box is its prompt row alone: the page frames it with rules
        // of its own (`PAGE_RULE`), the blank row over it and one under it,
        // so it closes and opens as Claude Code's does and the field's
        // delivery row has the row it covers.
        if top == bottom {
            let rule = "-".repeat(self.cols as usize);
            let rule = Row { runs: vec![(rule.clone(), Style::default())], text: rule };
            if top > 0 && rows[top - 1].blank() {
                rows[top - 1] = rule.clone();
            }
            rows.push(rule);
        }
        Shot { rows, cols: self.cols, cursor: self.cursor }
    }

    /// The box on a screen `for_page` made (T-1156): its top rule, its
    /// prompt row and its bottom rule — Codex's the page's own, the top one
    /// only where a blank row stood over the prompt.
    pub(crate) fn page_box(&self) -> Option<(usize, usize, usize)> {
        let (top, bottom) = self.box_rows()?;
        if top != bottom {
            return Some((top, top + 1, bottom));
        }
        let page_rule = |i: usize| self.rows.get(i).is_some_and(|r| is_page_rule(&r.text));
        let p = top;
        let up = if p > 0 && page_rule(p - 1) { p - 1 } else { p };
        page_rule(p + 1).then_some((up, p, p + 1))
    }

    /// Claude Code's composer: a prompt row under a rule, with a rule
    /// below it (the box's bottom, under any wrapped words). A question's
    /// `❯ 4. Chat about this` sits under a rule with none below it, and
    /// is a numbered option besides.
    fn boxed_prompt(&self) -> Option<usize> {
        let n = self.rows.len();
        (1..n).rev().find(|&i| {
            let t = &self.rows[i].text;
            is_prompt(t)
                && !is_option(t)
                && is_rule(&self.rows[i - 1].text)
                && (i + 1..(i + BOX_REACH).min(n)).any(|j| is_rule(&self.rows[j].text))
        })
    }

    /// The prompt row the pane's cursor sits on, at the screen's left edge
    /// — a composer, never a dialog's option: a question parks the cursor
    /// on its selected `❯ 1. Teal` (measured 2026-11-06).
    fn cursor_prompt(&self) -> Option<usize> {
        let (_, cy) = self.cursor?;
        let row = self.rows.get(cy as usize)?;
        let t = &row.text;
        (is_prompt(t) && !is_option(t) && !t.starts_with([' ', '│'])).then_some(cy as usize)
    }

    /// A dialog's first row, from its selected option `m` up: the topmost
    /// rule or frame corner before the transcript's last mark (`is_mark`)
    /// — a plan approval's top rule is twenty rows over its options, past
    /// the plan's box and a run of padding — else the first row with words
    /// after that mark, which is a Codex dialog's heading.
    fn dialog_top(&self, m: usize) -> usize {
        let floor = m.saturating_sub(DIALOG_REACH);
        let mut top = None;
        let mut from = floor;
        for i in (floor..m).rev() {
            let t = self.rows[i].text.trim_start();
            if is_mark(t) {
                from = i + 1;
                break;
            }
            if is_rule(t) || t.starts_with('╭') {
                top = Some(i);
            }
        }
        top.unwrap_or_else(|| (from..m).find(|&i| !self.rows[i].blank()).unwrap_or(m))
    }

    /// The rows the page draws, top to bottom, by their index on the
    /// screen: down to the last row with words (a dialog drawn at the
    /// screen's top leaves the rest blank), a run of blank rows drawn as
    /// one (a plan approval pads ten between its plan and its question).
    pub(crate) fn view(&self) -> Vec<usize> {
        let Some(end) = self.rows.iter().rposition(|r| !r.blank()) else { return Vec::new() };
        let mut out: Vec<usize> = Vec::with_capacity(end + 1);
        for i in 0..=end {
            let blank = self.rows[i].blank();
            if blank && out.last().is_some_and(|&p| self.rows[p].blank()) {
                continue;
            }
            out.push(i);
        }
        out
    }
}

/// A row's words past a dialog frame's left edge and the pad after it.
fn unframed(t: &str) -> &str {
    t.trim_start_matches([' ', '│'])
}

/// The column a prompt row's words start in: past its frame, its mark and
/// the one space after the mark — where the agent puts what is typed.
pub(crate) fn words_col(t: &str) -> usize {
    let lead = t.chars().take_while(|c| matches!(c, ' ' | '│')).count();
    let rest = unframed(t);
    let mark = rest.chars().next().map_or(0, |c| c.len_utf8());
    lead + usize::from(mark > 0) + usize::from(rest[mark..].starts_with(' '))
}

/// The marks Claude Code's status rows open with (T-1156): its spinner
/// frames and the tool result's elbow. Only these rows carry its key hints
/// on screen; prose and prompts never pass through `strip_hints`.
const CHROME: &[char] = &['✻', '✶', '✳', '✢', '✽', '·', '*', '⎿'];

/// A status row with the agent's key hints taken out (T-1156): a
/// parenthesised group whose every `·` segment is a hint goes whole, with
/// the space before it (`(ctrl+o to expand)`); a group that also says
/// something keeps it (`(12s · esc to interrupt)` reads `(12s)`); a `⎿ Tip:`
/// row goes. Any other row is returned as it came.
pub(crate) fn strip_hints(row: &Row) -> Row {
    let lead = row.text.trim_start();
    if !lead.starts_with(CHROME) {
        return row.clone();
    }
    if lead.trim_start_matches(['⎿', ' ', '\u{a0}']).starts_with("Tip:") {
        return Row::default();
    }
    let chars: Vec<char> = row.text.chars().collect();
    let mut cut: Vec<std::ops::Range<usize>> = Vec::new();
    let mut i = 0;
    while let Some(open) = (i..chars.len()).find(|&j| chars[j] == '(') {
        let Some(close) = (open + 1..chars.len()).find(|&j| chars[j] == ')') else { break };
        // The group's segments, split at its `·` separators.
        let mut segs: Vec<(usize, usize)> = Vec::new();
        let mut from = open + 1;
        for (j, c) in chars.iter().enumerate().take(close + 1).skip(open + 1) {
            if j == close || *c == '·' || *c == '∙' {
                segs.push((from, j));
                from = j + 1;
            }
        }
        let hint = |&(a, b): &(usize, usize)| is_hint(&chars[a..b].iter().collect::<String>());
        if segs.iter().all(hint) {
            let start = if open > 0 && chars[open - 1] == ' ' { open - 1 } else { open };
            cut.push(start..close + 1);
        } else {
            for (k, seg) in segs.iter().enumerate() {
                if !hint(seg) {
                    continue;
                }
                // The segment with the separator before it, or after it
                // when it is the first.
                let (a, b) = *seg;
                if k > 0 {
                    let sep = segs[k - 1].1;
                    let from = if sep > 0 && chars[sep - 1] == ' ' { sep - 1 } else { sep };
                    cut.push(from..b);
                } else {
                    let next = segs[k + 1].0;
                    let to = if chars.get(next) == Some(&' ') { next + 1 } else { next };
                    cut.push(a..to);
                }
            }
        }
        i = close + 1;
    }
    if cut.is_empty() {
        return row.clone();
    }
    let keep = |c: usize| !cut.iter().any(|r| r.contains(&c));
    let mut out = Row::default();
    let mut at = 0;
    for (text, style) in &row.runs {
        let kept: String =
            text.chars().enumerate().filter(|(k, _)| keep(at + k)).map(|(_, c)| c).collect();
        at += text.chars().count();
        if !kept.is_empty() {
            out.text.push_str(&kept);
            out.runs.push((kept, *style));
        }
    }
    out
}

/// One `·` segment of a status row's parentheses that names a key and what
/// it does: `esc to interrupt`, `ctrl+o to expand`, `shift+tab to cycle`,
/// `? for shortcuts`, `Press Esc again to clear`.
fn is_hint(seg: &str) -> bool {
    let s = seg.trim().to_lowercase();
    let s = s.strip_prefix("press ").unwrap_or(&s);
    let Some((key, rest)) = s.split_once(' ') else { return false };
    let rest = rest.strip_prefix("again ").unwrap_or(rest);
    let Some(what) = rest.strip_prefix("to ").or_else(|| rest.strip_prefix("for ")) else {
        return false;
    };
    if what.trim().is_empty() {
        return false;
    }
    let mut parts: Vec<&str> = key.split('+').collect();
    let last = parts.pop().unwrap_or_default();
    let mods = ["ctrl", "shift", "alt", "cmd", "opt", "option", "meta"];
    let named = [
        "esc",
        "tab",
        "enter",
        "return",
        "space",
        "backspace",
        "delete",
        "up",
        "down",
        "left",
        "right",
        "↑",
        "↓",
        "←",
        "→",
        "?",
    ];
    parts.iter().all(|m| mods.contains(m)) && (named.contains(&last) || last.chars().count() == 1)
}

/// A rule the page drew into a screen (`Shot::for_page`).
fn is_page_rule(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c == '-')
}

/// The composer's row: Claude Code's `❯` (`!` in bash mode, T-876), Codex's `›`.
fn is_prompt(t: &str) -> bool {
    let t = unframed(t);
    t.starts_with('❯') || t.starts_with('›') || t.starts_with('!')
}

/// A dialog's selected option: the marker, a space, the option's words.
fn is_marker(t: &str) -> bool {
    let t = unframed(t);
    (t.starts_with("❯ ") || t.starts_with("› ")) && t.chars().count() > 2
}

/// A numbered option row: the marker, then `1.`, `12.` (a question's, an
/// approval's, a plan's).
fn is_option(t: &str) -> bool {
    let t = unframed(t);
    let Some(rest) = t.strip_prefix(['❯', '›']) else { return false };
    let rest = rest.trim_start();
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with('.')
}

/// A row the transcript part of the screen begins with: Claude Code's
/// message and tool marks, its spinners and a box's foot; Codex's bullet.
/// A dialog is never above one of these.
fn is_mark(t: &str) -> bool {
    t.starts_with(['⏺', '⎿', '•', '▔', '✻', '✶', '✳', '✢', '✽'])
}

/// The shortest row the fade drops as already said: "ok" or "Done." is
/// in every conversation, and saying it twice costs less than dropping a
/// fresh one.
const REPEAT_MIN: usize = 8;

/// The marks a row opens with, on the pane (Claude Code's message, tool
/// and prompt marks, Codex's bullet and prompt, the spinners) and on the
/// page (a person's quote bar, its ASCII form, a list bullet).
const MARKS: &[char] =
    &['⏺', '⎿', '❯', '›', '•', '▎', '>', '!', '✻', '✶', '✳', '✢', '✽', '·', '*', '-'];

/// A row's words, for telling the pane's rows from the conversation's: its
/// leading marks gone and its spaces collapsed, so a sentence wrapped at
/// the pane's width and at the zone's reads the same.
pub(crate) fn words_of(t: &str) -> String {
    let t = t.trim_start().trim_start_matches(MARKS);
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the conversation's newest rows say, as one run of words: a pane row
/// whose words are in it is a row the page already shows (T-1143, the
/// author: "last sentence is duplicated").
pub(crate) struct Said(String);

impl Said {
    pub(crate) fn of<'a>(rows: impl Iterator<Item = &'a str>) -> Said {
        let words: Vec<String> = rows.map(words_of).filter(|w| !w.is_empty()).collect();
        Said(words.join(" "))
    }

    /// Whether `row`'s words are already said. A short row never is.
    pub(crate) fn holds(&self, row: &str) -> bool {
        let w = words_of(row);
        w.chars().count() >= REPEAT_MIN && self.0.contains(&w)
    }
}

/// How much of a fade row's colour is kept: row `r` of `f`, top first.
pub(crate) fn fade_keep(r: usize, f: usize) -> f32 {
    (r as f32 + 1.0) / (f as f32 + 1.0)
}

// ---------------------------------------------------------------------------
// The escapes
// ---------------------------------------------------------------------------

/// tmux's capture, one row per line, its SGR sequences folded into runs.
/// Every other escape is dropped; the words cross `scrub_text`.
pub(crate) fn parse(raw: &str) -> Vec<Row> {
    raw.lines().map(parse_row).collect()
}

fn parse_row(line: &str) -> Row {
    let mut row = Row::default();
    let mut style = Style::default();
    let mut run = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            run.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                let mut params = String::new();
                let mut fin = None;
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        fin = Some(c);
                        break;
                    }
                    params.push(c);
                }
                if fin == Some('m') {
                    flush(&mut row, &mut run, style);
                    style = sgr(style, &params);
                }
            }
            Some(']') => {
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if c == '\x07' || (prev == '\x1b' && c == '\\') {
                        break;
                    }
                    prev = c;
                }
            }
            _ => {}
        }
    }
    flush(&mut row, &mut run, style);
    row
}

fn flush(row: &mut Row, run: &mut String, style: Style) {
    if run.is_empty() {
        return;
    }
    // `scrub_text`, not `scrub_cells`: the pane's rules and boxes are the
    // picture, and the window is a position-scoped exception to the L1 law
    // the way the shin is (`App::live_rect`). Controls and format hazards
    // still go.
    let clean = mesimon_core::text::scrub_text(run);
    run.clear();
    if clean.is_empty() {
        return;
    }
    row.text.push_str(&clean);
    match row.runs.last_mut() {
        Some((t, s)) if *s == style => t.push_str(&clean),
        _ => row.runs.push((clean, style)),
    }
}

/// One SGR sequence applied to `st`. tmux joins its codes with `;` and
/// spells an underline style `4:n`; the first number of each piece is read.
fn sgr(mut st: Style, params: &str) -> Style {
    let nums: Vec<u16> = params
        .split(';')
        .map(|p| p.split(':').next().unwrap_or("0").parse().unwrap_or(0))
        .collect();
    let mut i = 0;
    while i < nums.len() {
        match nums[i] {
            0 => st = Style::default(),
            1 => st = st.add_modifier(Modifier::BOLD),
            2 => st = st.add_modifier(Modifier::DIM),
            3 => st = st.add_modifier(Modifier::ITALIC),
            4 => st = st.add_modifier(Modifier::UNDERLINED),
            5 => st = st.add_modifier(Modifier::SLOW_BLINK),
            7 => st = st.add_modifier(Modifier::REVERSED),
            8 => st = st.add_modifier(Modifier::HIDDEN),
            9 => st = st.add_modifier(Modifier::CROSSED_OUT),
            22 => st = st.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => st = st.remove_modifier(Modifier::ITALIC),
            24 => st = st.remove_modifier(Modifier::UNDERLINED),
            25 => st = st.remove_modifier(Modifier::SLOW_BLINK),
            27 => st = st.remove_modifier(Modifier::REVERSED),
            28 => st = st.remove_modifier(Modifier::HIDDEN),
            29 => st = st.remove_modifier(Modifier::CROSSED_OUT),
            n @ 30..=37 => st = st.fg(ansi(n - 30, false)),
            39 => st.fg = None,
            n @ 40..=47 => st = st.bg(ansi(n - 40, false)),
            49 => st.bg = None,
            n @ 90..=97 => st = st.fg(ansi(n - 90, true)),
            n @ 100..=107 => st = st.bg(ansi(n - 100, true)),
            n @ (38 | 48 | 58) => {
                let (colour, used) = match nums.get(i + 1) {
                    Some(5) => (nums.get(i + 2).map(|&c| Color::Indexed(c as u8)), 2),
                    Some(2) => (
                        match (nums.get(i + 2), nums.get(i + 3), nums.get(i + 4)) {
                            (Some(&r), Some(&g), Some(&b)) => {
                                Some(Color::Rgb(r as u8, g as u8, b as u8))
                            }
                            _ => None,
                        },
                        4,
                    ),
                    _ => (None, 0),
                };
                match (n, colour) {
                    (38, Some(c)) => st = st.fg(c),
                    (48, Some(c)) => st = st.bg(c),
                    _ => {}
                }
                i += used;
            }
            _ => {}
        }
        i += 1;
    }
    st
}

fn ansi(n: u16, bright: bool) -> Color {
    match (n, bright) {
        (0, false) => Color::Black,
        (1, false) => Color::Red,
        (2, false) => Color::Green,
        (3, false) => Color::Yellow,
        (4, false) => Color::Blue,
        (5, false) => Color::Magenta,
        (6, false) => Color::Cyan,
        (7, false) => Color::Gray,
        (0, true) => Color::DarkGray,
        (1, true) => Color::LightRed,
        (2, true) => Color::LightGreen,
        (3, true) => Color::LightYellow,
        (4, true) => Color::LightBlue,
        (5, true) => Color::LightMagenta,
        (6, true) => Color::LightCyan,
        _ => Color::White,
    }
}

/// An xterm-256 index as the RGB xterm draws it, so a fade can mix it
/// toward the page ground.
pub(crate) fn indexed_rgb(n: u8) -> Color {
    const LOW: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match n {
        0..=15 => {
            let (r, g, b) = LOW[n as usize];
            Color::Rgb(r, g, b)
        }
        16..=231 => {
            let i = n - 16;
            let v = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            Color::Rgb(v(i / 36), v((i % 36) / 6), v(i % 6))
        }
        _ => {
            let v = 8 + (n - 232) * 10;
            Color::Rgb(v, v, v)
        }
    }
}

// ---------------------------------------------------------------------------
// The reader
// ---------------------------------------------------------------------------

enum Ask {
    Size(u16, u16),
    Stop,
}

/// The window on one session's pane: the newest capture, and the thread
/// that keeps it fresh for as long as this lives.
pub(crate) struct Live {
    pub session: uuid::Uuid,
    pub shot: Option<Rc<Shot>>,
    rx: Receiver<Shot>,
    ask: Option<Sender<Ask>>,
    wished: Cell<Option<(u16, u16)>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Live {
    /// Start reading `sid16`'s pane through `sock`. `size` is the zone's,
    /// when the page knows it: the pane is fitted before the first capture,
    /// so the agent's screen at the attached terminal's size never shows.
    /// `seed` is this session's last picture, shown until a capture at the
    /// zone's size replaces it: back from the pane (`ctrl-]`), the page is
    /// the one that was left.
    pub(crate) fn start(
        sock: PathBuf,
        session: uuid::Uuid,
        sid16: String,
        size: Option<(u16, u16)>,
        seed: Option<Rc<Shot>>,
    ) -> Self {
        let (tx, rx) = channel();
        let (ask_tx, ask_rx) = channel::<Ask>();
        let reader = std::thread::spawn(move || read_loop(&sock, &sid16, &tx, &ask_rx, size));
        Live {
            session,
            shot: seed,
            rx,
            ask: Some(ask_tx),
            wished: Cell::new(size),
            reader: Some(reader),
        }
    }

    /// The size the reader was last asked for.
    pub(crate) fn wished(&self) -> Option<(u16, u16)> {
        self.wished.get()
    }

    /// A window over a capture already taken, for a test: no thread, no
    /// socket, nothing to undo.
    #[cfg(test)]
    pub(crate) fn fixed(session: uuid::Uuid, shot: Shot) -> Self {
        let (_tx, rx) = channel();
        Live {
            session,
            shot: Some(Rc::new(shot)),
            rx,
            ask: None,
            wished: Cell::new(None),
            reader: None,
        }
    }

    /// Take what the reader sent since; whether the screen changed.
    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(shot) = self.rx.try_recv() {
            self.shot = Some(Rc::new(shot));
            changed = true;
        }
        changed
    }

    /// The size the zone has for the pane; the reader fits the pane to it
    /// when it can.
    pub(crate) fn wish(&self, cols: u16, rows: u16) {
        if self.wished.get() == Some((cols, rows)) {
            return;
        }
        self.wished.set(Some((cols, rows)));
        if let Some(ask) = &self.ask {
            let _ = ask.send(Ask::Size(cols, rows));
        }
    }
}

impl Drop for Live {
    /// The reader stops and undoes its fit before this returns, so an
    /// attach that follows finds the pane sized by its own client again.
    fn drop(&mut self) {
        if let Some(ask) = self.ask.take() {
            let _ = ask.send(Ask::Stop);
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// The reader's fit: the size last asked, since when, whether the pane was
/// ever fitted (so the way out undoes it), and the settle a fit starts.
struct Fitting {
    want: Option<(u16, u16)>,
    since: Instant,
    settle: Option<Instant>,
    fitted: bool,
    /// The last time a fit was tried, so a pane that keeps another size is
    /// fitted again at most once a `SIZE_WAIT`.
    tried: Instant,
}

impl Fitting {
    fn to(&mut self, sock: &Path, sid16: &str, cols: u16, rows: u16) {
        self.fit_with(cols, rows, |c, r| fit(sock, sid16, c, r));
    }

    /// `to` with the fit itself handed in, so a test can refuse it.
    fn fit_with(&mut self, cols: u16, rows: u16, fit: impl FnOnce(u16, u16) -> bool) {
        if self.want != Some((cols, rows)) {
            self.want = Some((cols, rows));
            self.since = Instant::now();
        }
        self.tried = Instant::now();
        if fit(cols, rows) {
            self.fitted = true;
            self.settle = Some(Instant::now() + SETTLE);
        }
    }

    /// One capture read: fit again if it says the fit was lost, then
    /// whether it goes. A refit that lands holds the capture back by its
    /// settle; one refused (a terminal attached) leaves the size wait
    /// running, so the capture goes once that wait is out.
    fn sends(
        &mut self,
        shot: &Shot,
        hash: u64,
        last: &mut u64,
        fit: impl FnOnce(u16, u16) -> bool,
    ) -> bool {
        if let Some((cols, rows)) = self.lost(shot) {
            self.fit_with(cols, rows, fit);
            *last = 0;
        }
        hash != *last && self.settle.is_none() && self.takes(shot)
    }

    /// The size to fit again to, when `shot` is not at the size asked and
    /// the last try is a `SIZE_WAIT` old. A wake (T-1143) replaces the pane
    /// under the same session name at tmux's default size, and the zone,
    /// whose size has not moved, asks for nothing new: the capture is what
    /// says the fit was lost. A fit refused (a terminal attached) is tried
    /// again on the same clock.
    fn lost(&self, shot: &Shot) -> Option<(u16, u16)> {
        let (c, r) = self.want?;
        let off = shot.cols != c || shot.rows.len() != r as usize;
        (off && self.tried.elapsed() >= SIZE_WAIT).then_some((c, r))
    }

    /// Whether a capture of `shot`'s size may go: the size asked, or the
    /// size wait run out.
    fn takes(&self, shot: &Shot) -> bool {
        let sized = self.want.is_none_or(|(c, r)| shot.cols == c && shot.rows.len() == r as usize);
        sized || self.since.elapsed() >= SIZE_WAIT
    }

    fn done(&self, sock: &Path, sid16: &str) {
        if self.fitted {
            unfit(sock, sid16);
        }
    }
}

/// The reader: fit to the newest size asked, capture every `EVERY`, send a
/// screen only when it changed and only at the size asked (`SIZE_WAIT`
/// at most), and never inside a fit's `SETTLE`; undo the fit on the way
/// out.
fn read_loop(
    sock: &Path,
    sid16: &str,
    tx: &Sender<Shot>,
    ask: &Receiver<Ask>,
    first: Option<(u16, u16)>,
) {
    let mut last = 0u64;
    let mut f = Fitting {
        want: None,
        since: Instant::now(),
        settle: None,
        fitted: false,
        tried: Instant::now(),
    };
    if let Some((cols, rows)) = first {
        f.to(sock, sid16, cols, rows);
    }
    loop {
        // Every ask that came while the last capture ran, the newest size
        // winning; a stop ends the loop at once.
        let mut size = None;
        loop {
            match ask.try_recv() {
                Ok(Ask::Size(c, r)) => size = Some((c, r)),
                Ok(Ask::Stop) | Err(TryRecvError::Disconnected) => return f.done(sock, sid16),
                Err(TryRecvError::Empty) => break,
            }
        }
        if let Some((cols, rows)) = size {
            f.to(sock, sid16, cols, rows);
        }
        if f.settle.is_none_or(|t| Instant::now() >= t) {
            f.settle = None;
            if let Some((hash, shot)) = capture(sock, sid16) {
                if f.sends(&shot, hash, &mut last, |c, r| fit(sock, sid16, c, r)) {
                    last = hash;
                    if tx.send(shot).is_err() {
                        return f.done(sock, sid16);
                    }
                }
            }
        }
        let wait =
            f.settle.map_or(EVERY, |t| t.saturating_duration_since(Instant::now()).min(EVERY));
        match ask.recv_timeout(wait) {
            Ok(Ask::Size(c, r)) => f.to(sock, sid16, c, r),
            Ok(Ask::Stop) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return f.done(sock, sid16)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn tmux(sock: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(mesimon_backend_tmux::tmux_bin())
        .arg("-S")
        .arg(sock)
        .args(args)
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The screen and the cursor in one command queue, and a hash of the raw
/// answer so an unchanged screen is not sent again.
fn capture(sock: &Path, sid16: &str) -> Option<(u64, Shot)> {
    let out = tmux(
        sock,
        &[
            "capture-pane",
            "-e",
            "-p",
            "-t",
            sid16,
            ";",
            "display-message",
            "-p",
            "-t",
            sid16,
            "#{pane_width}|#{pane_height}|#{cursor_flag}|#{cursor_x}|#{cursor_y}",
        ],
    )?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    out.hash(&mut hasher);
    let hash = hasher.finish();
    let out = out.trim_end_matches('\n');
    let (body, meta) = out.rsplit_once('\n').unwrap_or(("", out));
    let parts: Vec<&str> = meta.split('|').collect();
    if parts.len() != 5 {
        return None;
    }
    let cols: u16 = parts[0].trim().parse().ok()?;
    let lines: usize = parts[1].trim().parse().ok()?;
    let cursor =
        (parts[2] == "1").then(|| Some((parts[3].parse().ok()?, parts[4].parse().ok()?))).flatten();
    let mut rows = parse(body);
    if rows.len() < lines {
        rows.resize(lines, Row::default());
    }
    Some((hash, Shot { rows, cols, cursor }))
}

/// Size the pane's window to the zone — never while a client is attached
/// to the session, whose terminal is the size that counts.
fn fit(sock: &Path, sid16: &str, cols: u16, rows: u16) -> bool {
    let Some(clients) = tmux(sock, &["list-clients", "-t", sid16, "-F", "#{client_tty}"]) else {
        return false;
    };
    if !clients.trim().is_empty() {
        return false;
    }
    tmux(sock, &["resize-window", "-t", sid16, "-x", &cols.to_string(), "-y", &rows.to_string()])
        .is_some()
}

/// Back to the server's `window-size` (latest): `resize-window` had marked
/// the window `manual`.
fn unfit(sock: &Path, sid16: &str) {
    let _ = tmux(sock, &["set-option", "-w", "-t", sid16, "-u", "window-size"]);
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: &str = "────────────────────────────────────────";

    fn shot(lines: &[&str]) -> Shot {
        Shot { rows: parse(&lines.join("\n")), cols: 80, cursor: None }
    }

    /// Codex 0.166.0's screen as captured on 2026-11-06: no rule on either
    /// side of the `›` row, the cursor on it, a blank row over it, the
    /// status and the hints under it (truecolour inks). The seam is the
    /// blank row and the prompt row is the cursor's.
    #[test]
    fn codex_composer_is_found_by_the_cursor() {
        let mut lines: Vec<&str> = vec![
            "\x1b[1m•\x1b[0m Ran touch /tmp/mesimon-codex-probe-1143",
            "  └ (no output)",
            "\x1b[1m•\x1b[0m done",
            "\x1b[2m  Worked for 8s • 16:42\x1b[0m",
            "",
            "\x1b[1m›\x1b[0m \x1b[2mAsk Codex to do anything\x1b[0m",
            "",
            "  \x1b[38;2;72;154;54mT-1143-ticket-page-rehaul\x1b[39m · \x1b[38;2;213;144;48mGPT-6-Astra xhigh\x1b[39m · \x1b[38;2;236;105;30mContext 3% used\x1b[39m",
            "  \x1b[1m←\x1b[0m for agents · \x1b[1m?\x1b[0m for shortcuts                    ⚠ \x1b[38;2;196;167;103m2 warnings\x1b[39m · \x1b[1mf2\x1b[0m to view",
        ];
        while lines.len() < 30 {
            lines.insert(0, "");
        }
        let mut s = shot(&lines);
        assert_eq!(s.rows[26].text, "› Ask Codex to do anything");
        s.cursor = Some((2, 26));
        assert_eq!(s.anchor(), 25, "the blank row over the composer");
        assert_eq!(s.prompt_row(), Some(26));
        // Without the cursor (hidden mid-redraw) the `›` row is read as a
        // dialog's marker, and the window opens under the last bullet.
        s.cursor = None;
        assert_eq!(s.anchor(), 24);
        assert_eq!(s.prompt_row(), None, "no rule, no cursor: no row to type on");
    }

    /// A screen of `h` rows, `lines` from the top, the rest blank: the
    /// captures below, from the throwaway board of 2026-11-06 (Claude Code
    /// 2.1.319 and Codex 0.166.0 at 107×32).
    fn screen(lines: &[&str], h: usize) -> Shot {
        let mut lines = lines.to_vec();
        lines.resize(h, "");
        shot(&lines)
    }

    const CLAUDE_RULE: &str = "──────────────────────────────────────────────────────────────";

    /// Claude Code's folder trust, drawn at the screen's top over fourteen
    /// blank rows: the window is the dialog from its rule to its last
    /// words, and the blank rows under it are not drawn.
    #[test]
    fn a_dialog_at_the_screen_top_is_whole_and_ends_at_its_words() {
        let s = screen(
            &[
                "",
                CLAUDE_RULE,
                " Accessing workspace:",
                "",
                " /Users/x/.local/state/mesimon/ac41/worktrees/T-1-calm-say-hi",
                "",
                " Quick safety check: Is this a project you created or one you trust?",
                "",
                " Security guide",
                "",
                " ❯ No, exit",
                "   Yes, I trust this folder",
                "",
                " Enter to confirm · Esc to cancel",
            ],
            32,
        );
        assert_eq!(s.anchor(), 1, "the dialog's rule");
        assert_eq!(s.prompt_row(), None);
        let view = s.view();
        assert_eq!(view.last(), Some(&13), "the view ends at the dialog's last words");
    }

    /// A question parks the cursor on its selected option: that row is not
    /// a composer, and the window is the question from its rule.
    #[test]
    fn a_question_is_whole_with_the_cursor_on_its_option() {
        let mut s = screen(
            &[
                "⏺ Ticket has no notes beyond the title. Asking now.",
                CLAUDE_RULE,
                " ☐ Accent",
                "",
                "Which colour do you prefer for the board accent?",
                "",
                "❯ 1. Teal",
                "     Teal accent",
                "  2. Amber",
                "  4. Type something.",
                CLAUDE_RULE,
                "  5. Chat about this",
                "",
                "Enter to select · ↑/↓ to navigate · Esc to cancel",
            ],
            24,
        );
        s.cursor = Some((0, 6));
        assert_eq!(s.prompt_row(), None, "a numbered option is never the composer");
        assert_eq!(s.anchor(), 1, "the question's rule, under the transcript's mark");
        // On `Chat about this` its `❯` is under a rule with none below.
        s.rows[6] = parse_row("  1. Teal");
        s.rows[11] = parse_row("❯ 5. Chat about this");
        s.cursor = Some((0, 11));
        assert_eq!(s.prompt_row(), None);
        assert_eq!(s.anchor(), 1);
    }

    /// A plan approval is the whole screen: its top rule and the plan's box,
    /// ten padding rows, the question's rule and the options. The window
    /// is all of it, the padding drawn as one row.
    #[test]
    fn a_plan_approval_shows_its_plan() {
        let mut lines = vec![
            "",
            "  ⎿  /plan to preview",
            "▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔",
            "",
            CLAUDE_RULE,
            "   Ready to code?",
            "",
            "   Here is Claude's plan:",
            "  ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌",
            "   1. Open README.md.",
            "   2. Append the line at the end of the file.",
            "  ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌",
        ];
        lines.extend([""; 10]);
        lines.extend([
            CLAUDE_RULE,
            "   Claude has written up a plan and is ready to execute. Would you like to proceed?",
            "",
            "   ❯ 1. Yes, and use auto mode",
            "     2. Yes, manually approve edits",
            "     3. Tell Claude what to change",
            "",
            "   ctrl+g to edit in Nvim · ~/.claude/plans/plan.md",
        ]);
        let mut s = screen(&lines, 32);
        s.cursor = Some((3, 25));
        assert_eq!(s.anchor(), 4, "the plan's rule, not the question's");
        let view = s.view();
        let from = view.iter().position(|&i| i == 4).expect("the rule is drawn");
        assert_eq!(view.len() - from, 17, "27 rows less the padding's nine");
    }

    /// Codex's trust and hook prompts are no box: no rule, no mark over
    /// them. The window starts at the prompt's first words.
    #[test]
    fn a_codex_prompt_starts_at_its_first_words() {
        let mut s = screen(
            &[
                "",
                "  Folder access",
                "  /Users/x/.local/state/mesimon/ac41/worktrees/T-6-codex-say-hi",
                "",
                "  Trust this folder? Codex can read, edit, and run files here.",
                "",
                "› 1. Trust and continue",
                "  2. Back to Agent Command Center",
                "",
                "  enter continue · esc back",
            ],
            32,
        );
        s.cursor = Some((1, 31));
        assert_eq!(s.anchor(), 1);
        assert_eq!(s.prompt_row(), None);
        assert_eq!(s.view().last(), Some(&9));
    }

    /// Codex's command approval (T-846's capture) under its transcript: from
    /// the bullet down is the transcript, the approval's heading is the
    /// window's first row.
    #[test]
    fn a_codex_approval_starts_under_the_transcript() {
        let s = screen(
            &[
                "• I'll request approval to run that exact command outside the sandbox.",
                "",
                "  Would you like to run the following command?",
                "",
                "  Reason: May I write this disposable fixture file?",
                "",
                "  $ printf MESIMON_REFUSED_WRITE > /tmp/approval-refused.txt",
                "",
                "",
                "› 1. Yes, proceed (y)",
                "  2. Yes, and don't ask again for commands that start with `printf` (p)",
                "  3. No, and tell Codex what to do differently (esc)",
                "",
                "  Press enter to confirm or esc to cancel",
            ],
            30,
        );
        assert_eq!(s.anchor(), 2);
    }

    /// The pane's rows of a reply are its words, wherever either side
    /// wrapped: the marks and the indents fall away, a short row is never
    /// counted, and a row the conversation does not hold is fresh.
    #[test]
    fn a_pane_row_the_conversation_holds_is_said() {
        let said = Said::of(
            [
                "claude  18:03",
                "The pane's marks stand out to the left, as they do in Claude",
                "Code itself. Moving the pane right instead would break that line.",
                "▎ press U on your board",
                "Done.",
            ]
            .into_iter(),
        );
        assert!(said
            .holds("⏺ The pane's marks stand out to the left, as they do in Claude Code itself."));
        assert!(said.holds("  itself. Moving the pane right instead would break   that line."));
        assert!(said.holds("❯ press U on your board"), "the person's words, echoed");
        assert!(!said.holds("⏺ Done."), "too short to count");
        assert!(!said.holds("✻ Churned for 28s · done 6:03 PM"), "the pane's own line");
        assert!(!said.holds("⏺ Bash(cargo test -p mesimon-tui)"), "newer than the transcript");
    }

    /// A capture at another size than the zone's is held back (the agent
    /// has not redrawn for the fit yet), until the size wait runs out.
    #[test]
    fn a_capture_of_another_size_waits_for_the_fit() {
        let screen =
            |cols: u16, rows: usize| Shot { rows: vec![Row::default(); rows], cols, cursor: None };
        let mut f = Fitting {
            want: Some((107, 32)),
            since: Instant::now(),
            settle: None,
            fitted: true,
            tried: Instant::now(),
        };
        assert!(!f.takes(&screen(140, 41)), "the attached terminal's size, not yet refitted");
        assert!(f.takes(&screen(107, 32)));
        f.since = Instant::now() - SIZE_WAIT;
        assert!(f.takes(&screen(140, 41)), "a fit that never lands is not waited on for good");
        f.want = None;
        assert!(f.takes(&screen(80, 24)), "no size asked: any capture");
    }

    /// A capture at another size than the one asked says the fit was lost
    /// (a wake replaced the pane at tmux's default size): fit again, once a
    /// `SIZE_WAIT` at most, and never with no size asked.
    #[test]
    fn a_pane_that_lost_its_fit_is_fitted_again() {
        let screen =
            |cols: u16, rows: usize| Shot { rows: vec![Row::default(); rows], cols, cursor: None };
        let mut f = Fitting {
            want: Some((107, 32)),
            since: Instant::now() - SIZE_WAIT,
            settle: None,
            fitted: true,
            tried: Instant::now(),
        };
        assert_eq!(f.lost(&screen(80, 24)), None, "a try a moment ago is not repeated");
        f.tried = Instant::now() - SIZE_WAIT;
        assert_eq!(f.lost(&screen(80, 24)), Some((107, 32)), "the woken pane, fitted again");
        assert_eq!(f.lost(&screen(107, 32)), None, "a pane at its size is left alone");
        f.want = None;
        assert_eq!(f.lost(&screen(80, 24)), None, "no size asked: nothing to fit");
    }

    /// A fit refused while a terminal is attached does not hold the pane
    /// back for good: the refit is tried once a `SIZE_WAIT`, and the
    /// capture at the attached size goes once the size wait is out.
    #[test]
    fn a_refused_refit_still_sends_the_capture() {
        let screen =
            |cols: u16, rows: usize| Shot { rows: vec![Row::default(); rows], cols, cursor: None };
        let mut f = Fitting {
            want: Some((107, 32)),
            since: Instant::now() - SIZE_WAIT,
            settle: None,
            fitted: false,
            tried: Instant::now() - SIZE_WAIT,
        };
        let mut last = 0;
        let mut asked = false;
        let went = f.sends(&screen(140, 41), 7, &mut last, |_, _| {
            asked = true;
            false
        });
        assert!(asked, "the lost fit is tried again");
        assert!(went, "a refused refit still sends the attached size's capture");
        let mut f =
            Fitting { since: Instant::now() - SIZE_WAIT, tried: Instant::now() - SIZE_WAIT, ..f };
        assert!(
            !f.sends(&screen(80, 24), 8, &mut last, |_, _| true),
            "a refit that lands settles first"
        );
    }

    /// Against a real tmux (ignored in `cargo ut`; run with `--ignored`):
    /// the session killed and made again under its name, as a wake does,
    /// at tmux's default size: the running reader fits the new pane back
    /// to the zone's size without the zone asking again.
    #[test]
    #[ignore = "starts a tmux server"]
    fn a_woken_pane_is_fitted_again() {
        let sock = PathBuf::from(format!("/tmp/msmn-wake-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let script = "draw() { clear; printf '%s x %s\\n' $(tput cols) $(tput lines); }; \
                      trap draw WINCH; draw; while :; do sleep 0.02; done";
        let up = |s: &str| tmux(&sock, &["new-session", "-d", "-s", "wakeprobe", s]);
        // A second session keeps the server up across the kill.
        assert!(tmux(&sock, &["new-session", "-d", "-s", "keep", "sleep 30"]).is_some());
        assert!(up(script).is_some(), "tmux starts");
        std::thread::sleep(Duration::from_millis(300));
        let mut live =
            Live::start(sock.clone(), uuid::Uuid::nil(), "wakeprobe".into(), Some((60, 20)), None);
        let at = |live: &mut Live, cols: u16, secs: u64| {
            let deadline = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < deadline {
                live.poll();
                if live.shot.as_ref().is_some_and(|s| s.cols == cols) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        assert!(at(&mut live, 60, 3), "fitted at the start");
        let _ = tmux(&sock, &["kill-session", "-t", "wakeprobe"]);
        assert!(up(script).is_some(), "woken under its name");
        // The old pane's last picture is 60 wide too: only a capture of
        // the new pane counts.
        live.shot = None;
        let fitted = at(&mut live, 60, 4);
        let shot = live.shot.clone();
        drop(live);
        let _ = tmux(&sock, &["kill-server"]);
        let _ = std::fs::remove_file(&sock);
        let shot = shot.expect("a capture");
        assert!(fitted, "fitted again: {}x{}", shot.cols, shot.rows.len());
        assert!(shot.rows.iter().any(|r| r.text.contains("60 x 20")), "and drawn for it");
    }

    /// Against a real tmux (ignored in `cargo ut`; run with `--ignored`):
    /// a reader started with the zone's size fits the pane before its
    /// first capture, so the first screen the page gets is at that size
    /// and drawn for it — back from the pane (`ctrl-]`), never the
    /// attached terminal's layout first.
    #[test]
    #[ignore = "starts a tmux server"]
    fn the_first_capture_is_at_the_zone_size() {
        let sock = PathBuf::from(format!("/tmp/msmn-fit-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        // Redraws on a resize only, as an agent does.
        let script = "draw() { clear; printf '%s x %s\\n' $(tput cols) $(tput lines); }; \
                      trap draw WINCH; draw; while :; do sleep 0.02; done";
        let up =
            tmux(&sock, &["new-session", "-d", "-s", "fitprobe", "-x", "140", "-y", "41", script]);
        assert!(up.is_some(), "tmux starts");
        std::thread::sleep(Duration::from_millis(300));
        let mut live =
            Live::start(sock.clone(), uuid::Uuid::nil(), "fitprobe".into(), Some((60, 20)), None);
        let deadline = Instant::now() + Duration::from_secs(3);
        while live.shot.is_none() && Instant::now() < deadline {
            live.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        let shot = live.shot.clone().expect("a capture");
        let _ = tmux(&sock, &["kill-server"]);
        let _ = std::fs::remove_file(&sock);
        assert_eq!((shot.cols, shot.rows.len()), (60, 20), "the first capture is fitted");
        assert!(
            shot.rows.iter().any(|r| r.text.contains("60 x 20")),
            "and drawn for the fit: {:?}",
            shot.rows.iter().map(|r| &r.text).collect::<Vec<_>>()
        );
    }

    /// A framed dialog option under the cursor is not a composer.
    #[test]
    fn a_framed_option_under_the_cursor_is_not_a_prompt_row() {
        let mut lines: Vec<&str> = vec![
            "╭──────────────────────────────╮",
            "│ Do you want to run this?     │",
            "│ › 1. Yes                     │",
            "│   2. No                      │",
            "╰──────────────────────────────╯",
        ];
        while lines.len() < 24 {
            lines.insert(0, "");
        }
        let mut s = shot(&lines);
        s.cursor = Some((2, 21));
        assert_eq!(s.prompt_row(), None);
        assert_eq!(s.anchor(), 19, "the box's top corner");
    }

    #[test]
    fn sgr_runs_fold_and_reset() {
        let row =
            parse_row("\x1b[38;5;246mThought for \x1b[1m54s\x1b[0m\x1b[38;5;246m, read\x1b[39m x");
        assert_eq!(row.text, "Thought for 54s, read x");
        let grey = Style::default().fg(Color::Indexed(246));
        assert_eq!(row.runs[0], ("Thought for ".into(), grey));
        assert_eq!(row.runs[1], ("54s".into(), grey.add_modifier(Modifier::BOLD)));
        assert_eq!(row.runs[2], (", read".into(), grey));
        assert_eq!(row.runs[3], (" x".into(), Style::default()));
    }

    #[test]
    fn rgb_named_and_dim_colours() {
        let row = parse_row("\x1b[2m\x1b[34m~/x\x1b[0m \x1b[38;2;10;20;30mrgb\x1b[m plain");
        assert_eq!(
            row.runs[0],
            ("~/x".into(), Style::default().fg(Color::Blue).add_modifier(Modifier::DIM))
        );
        assert_eq!(row.runs[1], (" ".into(), Style::default()));
        assert_eq!(row.runs[2], ("rgb".into(), Style::default().fg(Color::Rgb(10, 20, 30))));
        assert_eq!(row.runs[3], (" plain".into(), Style::default()));
    }

    #[test]
    fn other_escapes_and_controls_are_dropped() {
        let row = parse_row("a\x1b]8;;http://x\x07link\x1b]8;;\x1b\\b\x1b[2Kc\u{200b}d ─│");
        assert_eq!(row.text, "alinkbcd ─│");
    }

    #[test]
    fn anchor_finds_the_composer_and_keeps_its_status_row() {
        let s = shot(&[
            "⏺ Reading the ticket first.",
            "",
            "· Vibing… (5m 55s)",
            RULE,
            "❯ ",
            RULE,
            "  ~/x | ⎇ main | Fable 5.1",
            "  ⏵⏵ auto mode on",
        ]);
        assert_eq!(s.anchor(), 2);
    }

    /// A dialog's heading is live, never in the fade (T-1143, the author
    /// asked whether questions and prompts show whole): from the first
    /// words under the transcript's mark, or from its frame's corner.
    #[test]
    fn anchor_takes_a_dialog_from_its_heading() {
        let s = shot(&[
            "⏺ Bash(cargo test)",
            "",
            "Do you want to run this command?",
            "  Run the tests",
            "",
            "❯ 1. Yes",
            "  2. No, and tell Claude what to do differently (esc)",
            "",
            "  ~/x | ⎇ main",
        ]);
        assert_eq!(s.anchor(), 2);
        let boxed = shot(&[
            "words",
            "",
            "╭──────────────╮",
            "│ Proceed?     │",
            "│ ❯ 1. Yes     │",
            "╰──────────────╯",
        ]);
        assert_eq!(boxed.anchor(), 2);
    }

    #[test]
    fn anchor_falls_back_to_the_last_rows() {
        let s = shot(&["a", "b", "c"]);
        assert_eq!(s.anchor(), 0);
        let lines: Vec<String> = (0..20).map(|i| format!("row {i}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        assert_eq!(shot(&refs).anchor(), 12);
    }

    #[test]
    fn fade_keeps_more_toward_the_seam() {
        assert_eq!(fade_keep(0, 4), 0.2);
        assert_eq!(fade_keep(3, 4), 0.8);
        assert_eq!(indexed_rgb(246), Color::Rgb(148, 148, 148));
        assert_eq!(indexed_rgb(196), Color::Rgb(255, 0, 0));
        assert_eq!(indexed_rgb(16), Color::Rgb(0, 0, 0));
    }

    /// The bottom of a Claude Code screen with the person's status line, as
    /// captured from a mesimon pane on 2026-11-07 (Claude Code 2.1.3xx, 152
    /// columns; the words shortened): the spinner and its tip, the box with
    /// the prompt mark and a no-break space, the status line, the mode row
    /// with its hint in the timer's grey.
    fn claude_working() -> Shot {
        let rule = format!("\x1b[38;5;244m{}", "─".repeat(60));
        let lines = [
            "\x1b[38;5;231m⏺\x1b[39m Measuring before answering.".to_string(),
            String::new(),
            "\x1b[38;5;174m✳\x1b[39m \x1b[38;5;216mPhilosophizing…\x1b[38;5;174m \x1b[38;5;246m(17s · ↓\x1b[39m \x1b[38;5;246m1.2k tokens · esc to interrupt)\x1b[39m".into(),
            "\x1b[38;5;246m  ⎿ \u{a0}Tip: Just want the answer? Run /focus now to hide the steps\x1b[39m".into(),
            String::new(),
            rule.clone(),
            "\x1b[38;5;246m❯\u{a0}\x1b[39m".into(),
            rule,
            "\x1b[39m  \x1b[2m\x1b[34m…/T-1-retry\x1b[38;5;246m |\x1b[0m \x1b[2m\x1b[32m⎇ msmn/T-1\x1b[38;5;246m |\x1b[0m \x1b[2m\x1b[36mOpus 5.5 (high)\x1b[0m".into(),
            "  \x1b[38;5;220m⏵⏵ auto mode on\x1b[38;5;246m (shift+tab to cycle) · \x1b[38;5;174m⧉\x1b[39m \x1b[38;5;246m2\x1b[39m".into(),
        ];
        let rows = parse(&lines.join("\n"));
        Shot { rows, cols: 60, cursor: Some((2, 6)) }
    }

    /// The page cuts everything under the box and takes the agent's key
    /// hints out of its status rows, keeping what they said (T-1156).
    #[test]
    fn the_page_cuts_the_footer_and_the_hints() {
        let shot = claude_working();
        assert_eq!(shot.box_rows(), Some((5, 7)));
        let page = shot.for_page();
        assert_eq!(page.rows.len(), 8, "nothing under the bottom rule");
        assert_eq!(page.rows[2].text, "✳ Philosophizing… (17s · ↓ 1.2k tokens)", "the timer stays");
        assert!(page.rows[3].blank(), "the tip goes");
        assert_eq!(page.rows[0], shot.rows[0], "prose is untouched");
        assert_eq!(page.page_box(), Some((5, 6, 7)));
        // The kept words keep their own inks.
        assert!(page.rows[2].runs.iter().any(|(t, _)| t.contains("Philosophizing")));
    }

    #[test]
    fn a_hint_is_a_key_and_what_it_does() {
        let strip = |t: &str| strip_hints(&parse_row(t)).text;
        assert_eq!(strip("  ⎿  Read 212 lines (ctrl+o to expand)"), "  ⎿  Read 212 lines");
        assert_eq!(strip("✻ Thinking… (12s · esc to interrupt)"), "✻ Thinking… (12s)");
        assert_eq!(strip("· Vibing… (esc to interrupt · 4s)"), "· Vibing… (4s)");
        assert_eq!(strip("✢ Baking… (esc to interrupt · ctrl+t to show todos)"), "✢ Baking…");
        assert_eq!(strip("✻ Churned for 28s · done 6:03 PM"), "✻ Churned for 28s · done 6:03 PM");
        assert_eq!(strip("✶ Waiting (? for shortcuts)"), "✶ Waiting");
        assert_eq!(strip("✶ Waiting (Press Esc again to clear)"), "✶ Waiting");
        // Prose and prompts never pass through it, whatever they say.
        let prose = "⏺ Press ctrl+c to stop it (esc to interrupt).";
        assert_eq!(strip(prose), prose);
        assert_eq!(strip("❯ (tab to complete)"), "❯ (tab to complete)");
        // Words that are not a key stay.
        assert_eq!(strip("  ⎿  (moved to trash)"), "  ⎿  (moved to trash)");
        assert_eq!(strip("  ⎿  (see docs for details)"), "  ⎿  (see docs for details)");
    }

    /// A placeholder is nothing typed: the cursor parked before it. Words
    /// with the cursor after them, or wrapped under the prompt, are held.
    #[test]
    fn the_box_holds_words_only_past_its_placeholder() {
        let boxed = |prompt: &str, wrapped: Option<&str>, cursor: (u16, u16)| {
            let mut lines = vec![RULE, prompt];
            lines.extend(wrapped);
            lines.push(RULE);
            Shot { rows: parse(&lines.join("\n")), cols: 80, cursor: Some(cursor) }
        };
        assert_eq!(boxed("❯\u{a0}", None, (2, 1)).box_holds(), Some(false), "empty");
        assert_eq!(
            boxed("❯\u{a0}build it, use recommended", None, (2, 1)).box_holds(),
            Some(false),
            "the agent's suggestion"
        );
        assert_eq!(boxed("❯ half a thought", None, (16, 1)).box_holds(), Some(true), "a draft");
        assert_eq!(
            boxed("❯ retry with backoff", Some("  cap it at 30 s"), (2, 1)).box_holds(),
            Some(true),
            "wrapped words are words wherever the cursor is"
        );
        assert_eq!(shot(&["⏺ no box here"]).box_holds(), None);
    }

    /// Codex's lone prompt row gets the page's own rules, over and under,
    /// so it closes and opens as Claude Code's box does (T-1156).
    #[test]
    fn codex_is_framed_by_the_page() {
        let mut lines: Vec<&str> = vec!["\x1b[1m•\x1b[0m done"];
        lines.extend(["", "", "\x1b[1m›\x1b[0m \x1b[2mAsk Codex to do anything\x1b[0m"]);
        lines.extend(["", "  GPT high · Context 2% used", "  ? for shortcuts"]);
        let shot = Shot { rows: parse(&lines.join("\n")), cols: 40, cursor: Some((2, 3)) };
        assert_eq!(shot.box_rows(), Some((3, 3)));
        assert_eq!(shot.box_holds(), Some(false), "its placeholder");
        let page = shot.for_page();
        assert_eq!(page.rows.len(), 5, "its status and hints rows go");
        assert_eq!(page.page_box(), Some((2, 3, 4)));
        assert_eq!(page.rows[4].text, "-".repeat(40));
        assert_eq!(page.prompt_row(), Some(3), "the field still finds its row");
    }
}
