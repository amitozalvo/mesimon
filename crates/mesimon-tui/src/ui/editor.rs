//! The note editor (2026-09-02): a title row, one quiet line of context,
//! then the body. Two purposes share the shape, on two surfaces. Composing
//! a new ticket (the title is editable and the mini composer's workspace and
//! tags ride along) it is a PANEL over the board's cards — the header, the
//! column headers and the footer stay in view, the context line names the
//! column the ticket will land in, and the panel grows out of the phantom
//! card it replaced (`Editor::grow`, 2026-09-03: the author wanted the
//! bigger room to read as the composer opening up, not as a different
//! place). Writing a note on a ticket that exists (the title is the
//! ticket's, read-only) it takes the whole screen under a breadcrumb. No
//! rules, no boxes (L1): the body is text on the page ground, the hardware
//! cursor is the only cursor, and the footer is the keymap's.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Editor, EditorPurpose, Field};
use crate::text::{age_slot, area_window, created_at_epoch_ms, edit_window, truncate};

use super::chrome;

/// Rows above the body: title, breathing, context, breathing.
const HEAD_ROWS: u16 = 4;
/// The full-screen editor's left margin: one cell, the page's.
const PAGE_PAD: u16 = 1;
/// The panel's left margin: three cells, so its text stands where a card's
/// text stood (`LPAD` + bar + pad) and under the column headers' names.
const PANEL_PAD: u16 = 3;

/// The note editor, full screen: breadcrumb + title, context, body, footer.
pub(super) fn draw(f: &mut Frame, app: &App, ed: &Editor) {
    let theme = &app.theme;
    let area = f.area();

    // ---- line 0: breadcrumb + the title -----------------------------------
    let mut head = chrome::breadcrumb(app);
    head.push(Span::styled(" > ".to_string(), Style::default().fg(theme.rest.dim3)));
    let prefix_w: usize = head.iter().map(|s| s.content.width()).sum();
    let (spans, cx) = title_spans(app, ed, (area.width as usize).saturating_sub(prefix_w + 1));
    head.extend(spans);
    let mut cursor =
        cx.map(|cx| ((prefix_w as u16 + cx).min(area.width.saturating_sub(1)), area.y));

    let top = vec![Line::from(head), Line::default(), context_line(app, ed), Line::default()];
    f.render_widget(
        Paragraph::new(top),
        Rect { x: area.x, y: area.y, width: area.width, height: HEAD_ROWS.min(area.height) },
    );

    // ---- the body, then the footer ----------------------------------------
    let body = Rect {
        x: area.x,
        y: area.y + HEAD_ROWS,
        width: area.width,
        height: area.height.saturating_sub(HEAD_ROWS + 1),
    };
    if let Some(c) = draw_body(f, app, ed, body, PAGE_PAD) {
        cursor = Some(c);
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position((x, y));
    }
    let footer = chrome::footer_line(app, area.width);
    f.render_widget(
        Paragraph::new(footer),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
}

/// The composer, grown: a panel over the board's cards (`cards` is the
/// columns zone). At rest it covers the card rows under the column headers
/// edge to edge; for the first `GROW` after Tab it is drawn between the
/// phantom card's rectangle and that, so the panel visibly comes out of the
/// card. The board's own footer stays under it and already speaks for the
/// editor's scope.
pub(super) fn draw_panel(f: &mut Frame, app: &App, ed: &Editor, cards: Rect) {
    let theme = &app.theme;
    let rest = resting_rect(cards);
    let area = match (ed.grow, ed.grow_progress()) {
        (Some((from, _)), Some(t)) => lerp_rect(from, rest, t),
        _ => rest,
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    f.render_widget(Clear, area);
    if let Some(bg) = theme.bg {
        f.render_widget(Block::default().style(Style::default().bg(bg)), area);
    }

    // ---- line 0: the title, where the card's title row was -----------------
    let pad = " ".repeat(PANEL_PAD as usize);
    let mut head: Vec<Span<'static>> = vec![Span::raw(pad.clone())];
    let (spans, cx) =
        title_spans(app, ed, (area.width as usize).saturating_sub(PANEL_PAD as usize + 1));
    head.extend(spans);
    let mut cursor = cx.map(|cx| ((area.x + PANEL_PAD + cx).min(area.x + area.width - 1), area.y));

    let mut ctx = context_line(app, ed);
    ctx.spans.insert(0, Span::raw(pad[1..].to_string()));
    let top = vec![Line::from(head), Line::default(), ctx, Line::default()];
    f.render_widget(
        Paragraph::new(top),
        Rect { x: area.x, y: area.y, width: area.width, height: HEAD_ROWS.min(area.height) },
    );

    // ---- the body -----------------------------------------------------------
    let body = Rect {
        x: area.x,
        y: area.y + HEAD_ROWS.min(area.height),
        width: area.width,
        height: area.height.saturating_sub(HEAD_ROWS),
    };
    if let Some(c) = draw_body(f, app, ed, body, PANEL_PAD) {
        cursor = Some(c);
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position((x, y));
    }
}

/// Where the composer panel rests: the card rows of the columns zone, edge
/// to edge, under the column headers and their breathing row.
fn resting_rect(cards: Rect) -> Rect {
    Rect { x: cards.x, y: cards.y + 2, width: cards.width, height: cards.height.saturating_sub(2) }
}

/// A rectangle `t` of the way from `from` to `to`, every edge moving.
fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    let mix = |a: u16, b: u16| -> u16 {
        let v = a as f32 + (b as f32 - a as f32) * t;
        v.round().max(0.0) as u16
    };
    Rect {
        x: mix(from.x, to.x),
        y: mix(from.y, to.y),
        width: mix(from.width, to.width).max(1),
        height: mix(from.height, to.height).max(1),
    }
}

/// The title as spans, and the cursor's cell offset inside it while it is
/// the field being typed in (composing, `Field::Title`).
fn title_spans(app: &App, ed: &Editor, budget: usize) -> (Vec<Span<'static>>, Option<u16>) {
    let theme = &app.theme;
    match (ed.focus, ed.composing()) {
        (Field::Title, true) => {
            let budget = budget.saturating_sub(1);
            let (shown, cx) =
                edit_window(ed.title.as_str(), ed.title.width_before_cursor(), budget);
            let span = if shown.is_empty() {
                Span::styled("title".to_string(), theme.dim3())
            } else {
                Span::styled(
                    shown,
                    Style::default().fg(theme.rest.base).add_modifier(Modifier::BOLD),
                )
            };
            (vec![span], Some(cx))
        }
        _ => {
            let title = ed.title.as_str();
            let (text, style) = if title.is_empty() {
                ("title".to_string(), theme.dim3())
            } else {
                (truncate(title, budget), Style::default().fg(theme.rest.base))
            };
            (vec![Span::styled(text, style)], None)
        }
    }
}

/// The quiet line under the title: what this is, and what rides along.
/// Composing, it names the COLUMN the ticket lands in — the panel covers
/// the cards, so the cursor column's band alone no longer says it.
fn context_line(app: &App, ed: &Editor) -> Line<'static> {
    let theme = &app.theme;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut ctx_spans: Vec<Span<'static>> = Vec::new();
    match &ed.purpose {
        EditorPurpose::Compose { workspace, tags } => {
            let word = match workspace {
                Some(mesimon_core::board::WorkspaceStrategy::Worktree) => "worktree",
                Some(mesimon_core::board::WorkspaceStrategy::AdoptExisting) => "adopt",
                Some(mesimon_core::board::WorkspaceStrategy::SharedCheckout) | None => "shared",
            };
            ctx_spans.push(Span::styled(" NEW TICKET".to_string(), theme.dim2()));
            if let Some(col) = app.columns().get(app.cursor_col) {
                ctx_spans.push(Span::styled(" ∙ ".to_string(), theme.dim2()));
                ctx_spans.push(Span::styled(col.to_uppercase(), theme.dim1()));
                ctx_spans.push(Span::styled(" column".to_string(), theme.dim2()));
            }
            ctx_spans.push(Span::styled(format!(" ∙ ⎇ {word}"), theme.dim1()));
            if !tags.is_empty() {
                ctx_spans.push(Span::styled(" ∙".to_string(), theme.dim2()));
                for t in tags {
                    ctx_spans.push(Span::raw(" "));
                    let text = format!(" {} ", t.name);
                    let tint = theme.pip(app.board.tint_of(t) as usize);
                    if theme.paints_tags() {
                        ctx_spans.push(Span::styled(
                            text,
                            Style::default().bg(tint).fg(theme.tag_ink()),
                        ));
                    } else {
                        ctx_spans.push(Span::styled(text, Style::default().fg(theme.rest.dim1)));
                    }
                }
            }
        }
        EditorPurpose::Note { ticket, note } => {
            let meta = note.and_then(|id| app.board.ticket(*ticket).and_then(|t| t.note(id)));
            match meta {
                Some(m) => {
                    let who = super::ticket::author_word(&m.edited_by);
                    let when = created_at_epoch_ms(&m.edited_at)
                        .map(|ms| format!(" {} ago", age_slot(now, ms, false)))
                        .unwrap_or_default();
                    ctx_spans.push(Span::styled(" NOTE".to_string(), theme.dim2()));
                    ctx_spans.push(Span::styled(format!(" ∙ edited by {who}{when}"), theme.dim2()));
                    let first = app
                        .board
                        .ticket(*ticket)
                        .and_then(|t| t.description())
                        .is_some_and(|d| d.id == m.id);
                    if first {
                        ctx_spans
                            .push(Span::styled(" ∙ the description".to_string(), theme.dim2()));
                    }
                }
                None => {
                    let first =
                        app.board.ticket(*ticket).is_some_and(|t| t.description().is_none());
                    let what =
                        if first { " NEW NOTE ∙ becomes the description" } else { " NEW NOTE" };
                    ctx_spans.push(Span::styled(what.to_string(), theme.dim2()));
                }
            }
        }
    }
    if ed.dirty() {
        ctx_spans.push(Span::styled(" ∙ unsaved".to_string(), theme.dim2()));
    }
    Line::from(ctx_spans)
}

/// The body in `area`, `pad` cells in from its left edge: the text (or,
/// empty, what it is for), scrolled under the cursor. Returns the cursor
/// cell when the body is the focused field.
fn draw_body(f: &mut Frame, app: &App, ed: &Editor, area: Rect, pad: u16) -> Option<(u16, u16)> {
    let theme = &app.theme;
    let body_h = area.height as usize;
    if body_h == 0 {
        return None;
    }
    let pad_s = " ".repeat(pad as usize);
    let width = (area.width as usize).saturating_sub(pad as usize + 1);
    let (top_line, rows, (cy, cx)) = area_window(&ed.body, ed.top.get(), body_h, width);
    ed.top.set(top_line);
    let mut lines: Vec<Line<'static>> = Vec::new();
    if ed.body.is_empty() {
        // An empty body says what it is for, in the footer's own word.
        let hint = if ed.composing() { "describe it" } else { "write the note" };
        lines.push(Line::from(Span::styled(format!("{pad_s}{hint}"), theme.dim3())));
    } else {
        for row in rows {
            lines.push(Line::from(Span::styled(format!("{pad_s}{row}"), theme.base())));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
    (ed.focus == Field::Body).then(|| {
        (
            (area.x + pad + cx).min(area.x + area.width.saturating_sub(1)),
            area.y + cy.min(body_h.saturating_sub(1) as u16),
        )
    })
}
