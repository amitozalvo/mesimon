//! The note editor (2026-09-02): a title row, one quiet line of context,
//! then the body. Two purposes share the shape, on two surfaces. Composing
//! a new ticket (the title is editable and the mini composer's workspace and
//! tags ride along) it is a DIALOG over the board — the cursor card, grown:
//! a centred room on the card's own surface, the card's accent bar down its
//! left edge wearing the picked tags, the header, the column headers and a
//! margin of the board still in view around it — and it grows out of the
//! phantom card it replaced (`Editor::grow`, 2026-09-03: the author wanted
//! the bigger room to read as the composer opening up, not as a different
//! place, and then wanted it a dialog on top of the board rather than a
//! panel taking the columns zone edge to edge). Writing a note on a ticket
//! that exists (the title is the ticket's, read-only) it takes the whole
//! screen under a breadcrumb. No rules, no boxes (L1): the body is text on
//! its surface, the hardware cursor is the only cursor, and the footer is
//! the keymap's.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Editor, EditorPurpose, Field};
use crate::layout::{self, Slot};
use crate::tags;
use crate::text::{age_slot, area_window, created_at_epoch_ms, edit_window, truncate};
use crate::theme::{BarWeight, Ramp, TagLevel, Theme};

use super::chrome;

/// Rows above the body, full screen: title, breathing, context, breathing.
const HEAD_ROWS: u16 = 4;
/// The full-screen editor's left margin: one cell, the page's.
const PAGE_PAD: u16 = 1;
/// Rows above the body in the dialog: title, context, breathing. A card's
/// anatomy — the meta row sits directly under the title, the way a card's
/// session and chip rows do — which is also what lets frame zero of the
/// grow be the card itself, row for row.
const DIALOG_HEAD_ROWS: u16 = 3;
/// The dialog's frame is a card's: `[bar 1][pad 1][content][pad 1]`.
const DIALOG_LEAD: u16 = 2;
const DIALOG_FRAME: u16 = 3;
/// The dialog covers WHOLE columns (`dialog_rect`): its edges fall on the
/// board's gutters, so the cards beside it show complete and never as a
/// sliver cut mid-word — which is what a free-floating centred box left on
/// both sides, and a sliver is drawn structure by another name. This is the
/// least it settles at: two columns at their narrowest and the gutter
/// between them, so a board at `MIN_W` still gives it two.
const DIALOG_MIN_W: u16 = 2 * layout::MIN_COL + layout::GUT;
/// And past this a narrower run is preferred to a better-centred one: a
/// wide terminal with five columns at `MAX_COL` would otherwise hand the
/// dialog three of them, and prose in a 120-cell line is not a kindness.
const DIALOG_MAX_W: u16 = 96;
/// Rows of the columns zone kept above and below the dialog: the column
/// headers and their breathing row stay in view, its top edge is the cards'
/// top edge, and it never touches the advisory row.
const DIALOG_INSET_Y: u16 = 2;
/// The margin of a centred box, for the one case with no columns to snap to.
const DIALOG_INSET_X: u16 = 4;

/// The note editor, full screen: breadcrumb + title, context, body, footer.
pub(super) fn draw(f: &mut Frame, app: &App, ed: &Editor) {
    let theme = &app.theme;
    let area = f.area();

    // ---- line 0: breadcrumb + the title -----------------------------------
    let mut head = chrome::breadcrumb(app);
    head.push(Span::styled(" > ".to_string(), Style::default().fg(theme.rest.dim3)));
    let prefix_w: usize = head.iter().map(|s| s.content.width()).sum();
    let (spans, cx) =
        title_spans(ed, (area.width as usize).saturating_sub(prefix_w + 1), &theme.rest);
    head.extend(spans);
    let mut cursor =
        cx.map(|cx| ((prefix_w as u16 + cx).min(area.width.saturating_sub(1)), area.y));

    let mut ctx = context_line(app, ed, &theme.rest);
    ctx.spans.insert(0, Span::raw(" ".repeat(PAGE_PAD as usize)));
    let top = vec![Line::from(head), Line::default(), ctx, Line::default()];
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
    if let Some(c) = draw_body(f, ed, body, PAGE_PAD, &theme.rest) {
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

/// The composer, grown: a dialog over the board. It IS the cursor card,
/// bigger — the same surface, the same accent bar down its left edge (with
/// the picked tags on it, split the way an open card's stripe splits), the
/// same `sel` ink, the title on the first row and the meta row under it. At
/// rest it is `dialog_rect`: centred in the columns zone (`cards`), with the
/// column headers above it and a margin of board around it. For the first
/// `GROW` after Tab it is drawn between the phantom card's rectangle and
/// that, so the dialog visibly comes out of the card — and since the
/// anatomy is the card's, frame zero is the card, row for row and cell for
/// cell. The board's own footer stays under it and already speaks for the
/// editor's scope.
pub(super) fn draw_dialog(f: &mut Frame, app: &App, ed: &Editor, cards: Rect) {
    let theme = &app.theme;
    let rest = dialog_rect(app, cards);
    let area = match (ed.grow, ed.grow_progress()) {
        (Some((from, _)), Some(t)) => lerp_rect(from, rest, t),
        _ => rest,
    };
    if area.width <= DIALOG_FRAME || area.height == 0 {
        return;
    }
    let (surface, ink) = surface(theme);
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(surface), area);

    // ---- the stripe: the card's bar, one cell per row, wearing the tags
    // exactly as the real card will (`render_edit` paints the phantom card's
    // the same way) --------------------------------------------------------
    let worn = match &ed.purpose {
        EditorPurpose::Compose { tags, .. } => tags::painted(&app.board, tags),
        EditorPurpose::Note { .. } => Vec::new(),
    };
    let (plain_ch, plain_style) = theme.bar(BarWeight::Cursor);
    let (bar_ch, bar_style) =
        tags::bar_cell(theme, plain_ch, plain_style, &worn, TagLevel::Selected);
    let mut stripe: Vec<Line<'static>> =
        (0..area.height).map(|_| Line::from(Span::styled(bar_ch.clone(), bar_style))).collect();
    tags::stack_full(theme, &mut stripe, plain_ch, plain_style, &worn, TagLevel::Selected);
    f.render_widget(
        Paragraph::new(stripe),
        Rect { x: area.x, y: area.y, width: 1, height: area.height },
    );

    // ---- the head: the title where the card's title row was, the context
    // under it where the card's meta row was ----------------------------------
    let inner = Rect {
        x: area.x + DIALOG_LEAD,
        y: area.y,
        width: area.width - DIALOG_FRAME,
        height: area.height,
    };
    let (spans, cx) = title_spans(ed, inner.width as usize, ink);
    let mut cursor = cx.map(|cx| ((inner.x + cx).min(area.x + area.width - 1), inner.y));
    let head = vec![Line::from(spans), context_line(app, ed, ink), Line::default()];
    f.render_widget(
        Paragraph::new(head),
        Rect { height: DIALOG_HEAD_ROWS.min(inner.height), ..inner },
    );

    // ---- the body, with one breathing row under it ---------------------------
    let head_h = DIALOG_HEAD_ROWS.min(inner.height);
    let body = Rect {
        x: inner.x,
        y: inner.y + head_h,
        width: inner.width,
        height: inner.height.saturating_sub(DIALOG_HEAD_ROWS + 1),
    };
    if let Some(c) = draw_body(f, ed, body, 0, ink) {
        cursor = Some(c);
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position((x, y));
    }
}

/// Where the composer dialog rests: over a run of whole columns
/// (`snap_to_columns`, on the same geometry the board just drew), `DIALOG_INSET_Y`
/// rows in from the zone's top and bottom.
fn dialog_rect(app: &App, cards: Rect) -> Rect {
    let mut window = app.col_window.get();
    let geom =
        layout::board_geometry(cards.width, app.columns().len(), app.cursor_col, &mut window);
    let cols: Vec<(u16, u16)> = geom
        .slots
        .iter()
        .filter_map(|s| match s {
            Slot::Expanded { x, width } => Some((*x, *width)),
            Slot::Spine { .. } => None,
        })
        .collect();
    let (x, width) = snap_to_columns(&cols, cards.width)
        .unwrap_or((DIALOG_INSET_X, cards.width.saturating_sub(2 * DIALOG_INSET_X).max(1)));
    Rect {
        x: cards.x + x,
        y: cards.y + DIALOG_INSET_Y.min(cards.height.saturating_sub(1)),
        width,
        height: cards.height.saturating_sub(2 * DIALOG_INSET_Y).max(1),
    }
}

/// The run of whole columns the dialog covers, as `(x, width)` in the
/// zone's cells — `cols` are the expanded columns' `(x, width)` in board
/// order. Among the runs at least `DIALOG_MIN_W` wide: the ones within
/// `DIALOG_MAX_W` first, then the one whose centre is nearest the zone's,
/// then the narrowest. Every column when no run reaches the floor; `None`
/// with no columns at all.
fn snap_to_columns(cols: &[(u16, u16)], zone_w: u16) -> Option<(u16, u16)> {
    let (&(first_x, _), &(last_x, last_w)) = (cols.first()?, cols.last()?);
    let runs = cols.iter().enumerate().flat_map(|(i, &(x, _))| {
        cols[i..].iter().map(move |&(end_x, end_w)| (x, end_x + end_w - x))
    });
    let best = runs.filter(|&(_, w)| w >= DIALOG_MIN_W).min_by_key(|&(x, w)| {
        // Twice the distance between the run's centre and the zone's, so
        // the comparison stays in whole cells.
        (w > DIALOG_MAX_W, (2 * x + w).abs_diff(zone_w), w)
    });
    Some(best.unwrap_or((first_x, last_x + last_w - first_x)))
}

/// The dialog's surface and the ink that reads on it: the cursor card's
/// where the profile paints one, and the page's where it does not (light-256
/// and mono carry the cursor structurally, and a whole dialog in reverse
/// video would be a fourth SGR-7 use, not one of the three sanctioned).
fn surface(theme: &Theme) -> (Style, &Ramp) {
    match theme.selected_bg {
        Some(bg) => (Style::default().bg(bg), &theme.sel),
        None => (theme.bg.map(|bg| Style::default().bg(bg)).unwrap_or_default(), &theme.rest),
    }
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
fn title_spans(ed: &Editor, budget: usize, ink: &Ramp) -> (Vec<Span<'static>>, Option<u16>) {
    match (ed.focus, ed.composing()) {
        (Field::Title, true) => {
            let budget = budget.saturating_sub(1);
            let (shown, cx) =
                edit_window(ed.title.as_str(), ed.title.width_before_cursor(), budget);
            let span = if shown.is_empty() {
                Span::styled("title".to_string(), Style::default().fg(ink.dim3))
            } else {
                Span::styled(shown, Style::default().fg(ink.base).add_modifier(Modifier::BOLD))
            };
            (vec![span], Some(cx))
        }
        _ => {
            let title = ed.title.as_str();
            let (text, style) = if title.is_empty() {
                ("title".to_string(), Style::default().fg(ink.dim3))
            } else {
                (truncate(title, budget), Style::default().fg(ink.base))
            };
            (vec![Span::styled(text, style)], None)
        }
    }
}

/// The quiet line under the title: what this is, and what rides along.
/// Composing, it names the COLUMN the ticket lands in — the panel covers
/// the cards, so the cursor column's band alone no longer says it.
fn context_line(app: &App, ed: &Editor, ink: &Ramp) -> Line<'static> {
    let theme = &app.theme;
    let dim1 = Style::default().fg(ink.dim1);
    let dim2 = Style::default().fg(ink.dim2);
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
            ctx_spans.push(Span::styled("NEW TICKET".to_string(), dim2));
            if let Some(col) = app.columns().get(app.cursor_col) {
                ctx_spans.push(Span::styled(" ∙ ".to_string(), dim2));
                ctx_spans.push(Span::styled(col.to_uppercase(), dim1));
                ctx_spans.push(Span::styled(" column".to_string(), dim2));
            }
            ctx_spans.push(Span::styled(format!(" ∙ ⎇ {word}"), dim1));
            if !tags.is_empty() {
                ctx_spans.push(Span::styled(" ∙".to_string(), dim2));
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
                        ctx_spans.push(Span::styled(text, dim1));
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
                    ctx_spans.push(Span::styled("NOTE".to_string(), dim2));
                    ctx_spans.push(Span::styled(format!(" ∙ edited by {who}{when}"), dim2));
                    let first = app
                        .board
                        .ticket(*ticket)
                        .and_then(|t| t.description())
                        .is_some_and(|d| d.id == m.id);
                    if first {
                        ctx_spans.push(Span::styled(" ∙ the description".to_string(), dim2));
                    }
                }
                None => {
                    let first =
                        app.board.ticket(*ticket).is_some_and(|t| t.description().is_none());
                    let what =
                        if first { "NEW NOTE ∙ becomes the description" } else { "NEW NOTE" };
                    ctx_spans.push(Span::styled(what.to_string(), dim2));
                }
            }
        }
    }
    if ed.dirty() {
        ctx_spans.push(Span::styled(" ∙ unsaved".to_string(), dim2));
    }
    Line::from(ctx_spans)
}

/// The body in `area`, `pad` cells in from its left edge: the text (or,
/// empty, what it is for), scrolled under the cursor, in `ink`. Returns the
/// cursor cell when the body is the focused field.
fn draw_body(f: &mut Frame, ed: &Editor, area: Rect, pad: u16, ink: &Ramp) -> Option<(u16, u16)> {
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
        lines.push(Line::from(Span::styled(
            format!("{pad_s}{hint}"),
            Style::default().fg(ink.dim3),
        )));
    } else {
        for row in rows {
            lines.push(Line::from(Span::styled(
                format!("{pad_s}{row}"),
                Style::default().fg(ink.base),
            )));
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
