//! The search picker (T-349) — telescope's shape in mesimon's register.
//!
//! What is borrowed from telescope is the *shape*, because it is the shape a
//! reader who types `/` already has in their hands: one floating surface, a
//! prompt at the top that never loses focus, a ranked list under it that
//! re-sorts on every keystroke, and a preview of whatever the cursor is on.
//! What is NOT borrowed is its colour. Telescope paints matched characters in
//! an accent; mesimon has exactly one saturated colour and it is spoken for
//! (L2/L3 — needs-you and nothing else), so a match is carried by the VALUE
//! ramp instead: the characters the query landed on come up to `base` and go
//! bold, and everything around them sits a step down at `dim1`. On the mono
//! and 8-colour profiles the ramp collapses and the bold carries it alone,
//! which is the same bargain every other surface here makes.
//!
//! The frame is `dialog::frame`, L1's one allowlisted drawn role, and it is
//! recorded like every other — so `test_no_drawn_structure` sees a perimeter
//! and not a box someone transcribed. There is no rule between the list and
//! the preview: the preview is a CARD, and a card's own accent bar is the
//! edge. That is also why the preview is a card at all rather than a detail
//! sheet — the picker previews the thing itself, in the vocabulary the board
//! already taught, and invents nothing.
//!
//! Narrow terminals drop the preview rather than squeeze it: two panes at 40
//! cells each are two unreadable panes, and the list is the half that answers
//! the question.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::board::SessionRecord;
use mesimon_core::clock::now_ms;
use mesimon_core::keymap::Scope;
use mesimon_core::search::{Field, Hit};

use crate::app::{App, Search};
use crate::text::{age_in_column, age_slot, created_at_epoch_ms, edit_window, truncate};
use crate::theme::Theme;

use super::card::{self, CardCtx};
use super::dialog::{self, Edges};

/// The surface's outer bounds: big, because a picker that shows four rows is
/// a picker you have to type blind into, and capped, because a 200-column
/// terminal does not want a 200-column list of 30-character titles.
const MAX_W: u16 = 112;
const MAX_H: u16 = 22;

/// Below this much OUTER width the preview is dropped. Two panes need a list
/// wide enough for `T-12  a real sentence of a title  COLUMN TAG` and a card
/// wide enough to be a card; under it, the list takes the room.
const PREVIEW_FLOOR: u16 = 94;

/// The preview frame's outer width: a board column's worth plus its two
/// edge cells, so the card inside is the size the card outside is.
const PREVIEW_W: u16 = 36;

/// The prompt row, then a blank (the subtitle's row, when the list is the
/// recently viewed pages), then the list.
const LIST_TOP: u16 = 2;

/// Where the hardware cursor belongs this frame, so the caller can put the
/// real one there (a picker whose cursor is painted is a picker you cannot
/// tell is focused).
pub(super) fn draw(f: &mut Frame, app: &App, s: &Search) -> Option<(u16, u16)> {
    let screen = f.area();
    let w = MAX_W.min(screen.width.saturating_sub(6)).max(4);
    // A FIXED height, never one that follows the list: a surface that grows
    // and shrinks on every keystroke is a surface whose rows move under the
    // finger that is typing at them.
    let h = MAX_H.min(screen.height.saturating_mul(3) / 4).max(5);
    let area = Rect {
        x: screen.x + screen.width.saturating_sub(w) / 2,
        y: screen.y + screen.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    // Two frames, side by side, the way telescope draws three: the list is
    // one surface and the preview is another, and the edge between them is
    // the one structure L1 lets a dialog draw. Below the floor — or with
    // nothing to preview — the list takes the whole surface, because two
    // panes at 40 cells each are two panes nobody can read.
    let preview = (area.width >= PREVIEW_FLOOR && s.selected().is_some()).then(|| Rect {
        x: area.x + area.width - PREVIEW_W,
        width: PREVIEW_W,
        ..area
    });
    let list = Rect { width: area.width - preview.map_or(0, |p| p.width), ..area };

    let cursor = draw_list(f, app, s, list);
    if let Some(rect) = preview {
        draw_preview(f, app, s, rect);
    }
    cursor
}

/// The left frame: the prompt, a breathing row, and the ranked rows.
fn draw_list(f: &mut Frame, app: &App, s: &Search, area: Rect) -> Option<(u16, u16)> {
    let theme = &app.theme;
    let inner_w = area.width.saturating_sub(2) as usize;
    // The title counts what the query means against what it was asked of, so
    // `3/47` says both "this is narrow" and "this board is big" — and the
    // second number moves when `tab` widens the list, which is the readout
    // that makes the toggle legible without a chip of its own.
    let total = app.board.tickets.iter().filter(|t| s.archived || !t.is_archived()).count();
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        Edges {
            title: dialog::title(&theme.rest, format!("SEARCH ∙ {}/{total}", s.hits.len())),
            tail: dialog::keys(app, Scope::Search, &theme.rest, inner_w.saturating_sub(4)),
        },
    );
    if inner.height < 1 {
        return None;
    }
    let cursor = draw_prompt(f, app, s, inner);
    if inner.height <= LIST_TOP {
        return Some(cursor);
    }
    // The breathing row under the prompt names the list when it is not the
    // board (T-355): the pages opened this run, newest first, and the one
    // thing that turns it back into the board.
    if s.recent {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate(" viewed recently ∙ type to search the whole board", inner.width as usize),
                theme.dim2(),
            ))),
            Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 },
        );
    }
    let body = Rect {
        x: inner.x,
        y: inner.y + LIST_TOP,
        width: inner.width,
        height: inner.height - LIST_TOP,
    };
    let w = body.width as usize;

    if s.hits.is_empty() {
        // A sentence, never an empty pane: the picker says what it looked
        // in, because "nothing" and "nothing HERE" are different answers and
        // the second one has a key that fixes it.
        let word = if s.archived {
            " no matches on this board"
        } else {
            " no matches among the live tickets ∙ tab looks in the archive too"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(truncate(word, w), theme.dim2()))),
            Rect { height: 1, ..body },
        );
        return Some(cursor);
    }

    // Keep the cursor row on screen: the window follows the cursor, never
    // the other way round, so `^n` at the bottom edge scrolls by one.
    let rows = body.height as usize;
    let mut top = s.top.get().min(s.hits.len().saturating_sub(1));
    if s.idx < top {
        top = s.idx;
    } else if rows > 0 && s.idx >= top + rows {
        top = s.idx + 1 - rows;
    }
    s.top.set(top);

    // One column geometry for the whole list, measured off the rows that
    // will actually be drawn: ragged columns are what make a list of short
    // strings unreadable, and every row here is mostly short strings.
    let shown = s.hits.iter().skip(top).take(rows);
    let key_w = shown.clone().map(|h| h.key.text.width()).max().unwrap_or(4).clamp(4, 8);
    let trail_cap = (w / 3).max(6);
    let trail_w = shown.clone().map(|h| h.trail.text.width()).max().unwrap_or(0).min(trail_cap);
    let title_w = w.saturating_sub(1 + key_w + 2 + 1 + trail_w + 1);

    let lines: Vec<Line> = shown
        .enumerate()
        .map(|(i, hit)| row(theme, hit, top + i == s.idx, w, key_w, title_w, trail_w))
        .collect();
    f.render_widget(Paragraph::new(lines), body);
    Some(cursor)
}

/// The prompt: telescope's `›`, the query, and the real cursor inside it.
fn draw_prompt(f: &mut Frame, app: &App, s: &Search, inner: Rect) -> (u16, u16) {
    let theme = &app.theme;
    let lead = crate::glyphs::prompt_mark(theme.glyph_tier());
    let budget = (inner.width as usize).saturating_sub(4);
    let (shown, cx) = edit_window(s.query.as_str(), s.query.width_before_cursor(), budget);
    let spans = vec![
        Span::styled(format!(" {lead} "), theme.dim2()),
        Span::styled(shown, Style::default().fg(theme.rest.base).add_modifier(Modifier::BOLD)),
    ];
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 },
    );
    (inner.x + 3 + cx, inner.y)
}

/// One row: the key, the title, and the words that say where the card is.
/// Every one of them was part of the haystack, so every highlight in it is a
/// character the query actually landed on — there is no hidden field that
/// explains why this row is here.
fn row(
    theme: &Theme,
    hit: &Hit,
    selected: bool,
    w: usize,
    key_w: usize,
    title_w: usize,
    trail_w: usize,
) -> Line<'static> {
    let ink = if selected { &theme.sel } else { &theme.rest };
    // The cursor row's own treatment, the archived list's and the rail's: a
    // painted surface, the selected ramp, and bold on what matters.
    let row_style = if selected { theme.selected_row() } else { Style::default() };
    let mut spans = vec![Span::styled(" ".to_string(), row_style)];
    spans.extend(painted(
        &hit.key,
        key_w,
        Style::default().fg(ink.dim2),
        Style::default().fg(ink.base).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled("  ".to_string(), row_style));
    spans.extend(painted(
        &hit.title,
        title_w,
        Style::default().fg(ink.dim1),
        Style::default().fg(ink.base).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" ".to_string(), row_style));
    spans.extend(painted(
        &hit.trail,
        trail_w,
        Style::default().fg(ink.dim3),
        Style::default().fg(ink.dim1).add_modifier(Modifier::BOLD),
    ));
    let used: usize = super::spans_width(&spans);
    spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
    Line::from(spans).style(row_style)
}

/// A field, cut to `width` cells, with the matched characters lifted onto
/// `lit` and the rest left on `rest`. Runs of the same style collapse into
/// one span, so a row is a handful of spans rather than one per character.
fn painted(field: &Field, width: usize, rest: Style, lit: Style) -> Vec<Span<'static>> {
    let text = truncate(&field.text, width);
    // A cut row ends in `truncate`'s `~`, which is not part of the haystack
    // and must never light: a highlight names a character the reader can see,
    // and this one stands for the ones they cannot. Everything before it is a
    // prefix of the original — `truncate` cuts on grapheme boundaries, and a
    // grapheme prefix is a character prefix — so the match indices still line
    // up character for character.
    let carried = text.chars().count().saturating_sub(usize::from(text != field.text));
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_lit = false;
    for (i, c) in text.chars().enumerate() {
        let is_lit = i < carried && field.is_match(i);
        if !run.is_empty() && is_lit != run_lit {
            spans.push(Span::styled(std::mem::take(&mut run), if run_lit { lit } else { rest }));
        }
        run_lit = is_lit;
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_lit { lit } else { rest }));
    }
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), rest));
    }
    spans
}

/// The right frame: the card itself, then what the ticket's notes are
/// called. Nothing here is new vocabulary — the card is the board's own, and
/// the note rows are the ticket page's rail rows with the same mark, label
/// and tail.
fn draw_preview(f: &mut Frame, app: &App, s: &Search, area: Rect) {
    let theme = &app.theme;
    let Some(hit) = s.selected() else { return };
    let Some(ticket) = app.board.ticket(hit.id) else { return };
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        Edges { title: dialog::title(&theme.rest, ticket.short_key.clone()), tail: Vec::new() },
    );
    let ctx = CardCtx {
        theme,
        width: inner.width,
        now_ms: now_ms(),
        spin: app.spin_frame(),
        names_key: false,
    };
    let sessions: Vec<&SessionRecord> =
        app.board.sessions.iter().filter(|r| r.ticket == ticket.id && r.state.is_live()).collect();
    let painted_tags = crate::tags::painted(&app.board, &ticket.tags);
    let mut lines = card::render(
        &ctx,
        ticket,
        &sessions,
        app.terminal_busy(ticket.id),
        app.wt_item(ticket.id),
        // Not the cursor card: the LIST carries the cursor, and two painted
        // cursors on one surface are two answers to "where am I".
        false,
        false,
        false,
        None,
        // Open, always: the agent's last reply is the most useful thing a
        // preview can hold, and here there is room for it.
        true,
        peek_of(app, &sessions).as_deref(),
        &painted_tags,
        false,
        false,
        None,
        app.owed(ticket.id),
        app.pending_row(ticket.id).as_deref(),
        app.remote_initials(ticket.id).as_deref(),
    );
    let w = inner.width as usize;
    let now = now_ms();
    // Where the card is and how long it has been there, in the ticket page's
    // own words (`age_in_column`) — the one fact the card itself does not
    // carry, and the one a search is usually asking after.
    if let Some(ms) = created_at_epoch_ms(ticket.column_since()) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            truncate(&format!(" {} {}", ticket.column.to_uppercase(), age_in_column(now, ms)), w),
            theme.dim2(),
        )));
    }
    if !ticket.notes.is_empty() && lines.len() + 3 <= inner.height as usize {
        let tier = theme.glyph_tier();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " NOTES".to_string(),
            theme.dim1().add_modifier(Modifier::BOLD),
        )));
        for (j, n) in ticket.notes.iter().enumerate() {
            if lines.len() >= inner.height as usize {
                break;
            }
            let who = super::ticket::author_word(&n.edited_by, app);
            let age = created_at_epoch_ms(&n.edited_at)
                .map(|ms| age_slot(now, ms, false))
                .unwrap_or_default();
            let tail = format!("{who} {age}");
            // `notes[0]` IS the description, and a note's name is its first
            // line — which the card above may already be showing. The rail
            // says the row's ROLE there for the same reason (T-344).
            let label = if j == 0 { "description" } else { n.name.as_str() };
            let budget = w.saturating_sub(4 + tail.width() + 1);
            let head = format!(" {} {}", crate::glyphs::note_mark(tier), truncate(label, budget));
            let used = head.width() + tail.width() + 1;
            lines.push(Line::from(vec![
                Span::styled(head, theme.dim1()),
                Span::raw(" ".repeat(w.saturating_sub(used))),
                Span::styled(tail, theme.dim3()),
            ]));
        }
    }
    lines.truncate(inner.height as usize);
    f.render_widget(Paragraph::new(lines), inner);
}

/// The card's reply, through the draw cache — `board::draw_column`'s rule:
/// the highest-ranked session that has a transcript, and a stale activity
/// row dropped on anything that is not running.
fn peek_of(app: &App, sessions: &[&SessionRecord]) -> Option<std::rc::Rc<crate::peek::Peek>> {
    let mut ranked: Vec<&&SessionRecord> = sessions.iter().collect();
    ranked.sort_by_key(|s| (mesimon_core::attention::rank(&s.state), s.id));
    ranked.iter().find(|s| crate::peek::preview_path(s).is_some()).and_then(|s| {
        let pk = app.peek_cache.peek_for(s.kind, crate::peek::preview_path(s)?)?;
        Some(if s.state == mesimon_core::board::SessionState::Running || pk.activity.is_none() {
            pk
        } else {
            std::rc::Rc::new(crate::peek::Peek { activity: None, ..(*pk).clone() })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::search::Field;

    fn rest() -> Style {
        Style::default().fg(ratatui::style::Color::Rgb(1, 1, 1))
    }

    fn lit() -> Style {
        Style::default().fg(ratatui::style::Color::Rgb(2, 2, 2)).add_modifier(Modifier::BOLD)
    }

    /// A cut row ends in `truncate`'s `~`, and that cell is never lit — a
    /// highlight names a character the reader can SEE, and the marker stands
    /// for the ones they cannot. The index that would have lit it is the one
    /// at the cut, which is why this is a unit test: a render test would have
    /// to guess the pane geometry that puts a match exactly there.
    #[test]
    fn the_truncation_marker_is_never_lit() {
        let text = "x".repeat(10) + "Q" + &"y".repeat(10);
        // The query landed on the `Q`, at character 10 — which is exactly
        // where an 11-cell budget puts the marker.
        let field = Field { text, matched: vec![10] };
        let spans = painted(&field, 11, rest(), lit());
        let drawn: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(drawn, "xxxxxxxxxx~", "ten characters and the marker");
        for span in &spans {
            assert!(
                !span.content.contains('~') || span.style == rest(),
                "the marker took the match's style: {span:?}",
            );
        }
        // One cell wider and the `Q` is really on the screen, so it lights.
        let spans = painted(&field, 12, rest(), lit());
        let q = spans.iter().find(|s| s.content.contains('Q')).expect("the Q is drawn");
        assert_eq!(q.style, lit(), "a character the reader can see does light");
    }

    /// Runs of the same style collapse, so a row is a handful of spans rather
    /// than one per character, and the field is padded to its column width.
    #[test]
    fn a_painted_field_collapses_its_runs_and_fills_its_column() {
        let field = Field { text: "abcdef".into(), matched: vec![2, 3] };
        let spans = painted(&field, 10, rest(), lit());
        let shape: Vec<(&str, bool)> =
            spans.iter().map(|s| (s.content.as_ref(), s.style == lit())).collect();
        assert_eq!(shape, vec![("ab", false), ("cd", true), ("ef", false), ("    ", false)]);
    }

    /// A width of zero is a degenerate column, not a panic.
    #[test]
    fn a_field_with_no_room_draws_nothing() {
        let field = Field { text: "abc".into(), matched: vec![0] };
        assert!(painted(&field, 0, rest(), lit()).is_empty());
    }
}
