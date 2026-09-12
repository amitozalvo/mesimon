//! The note editor (2026-09-02): a title row, one quiet line of context,
//! then the body. Two purposes share the shape, on two surfaces — and the
//! surface is the SCREEN's. Over the board it is a DIALOG: the cursor card,
//! grown — a room on the card's own surface, the card's accent bar down its
//! left edge wearing the tags (the picked ones composing, the ticket's own
//! on a description), the header, the column headers and a margin of the
//! board still in view around it — and it grows out of the card it stands
//! for (`Editor::grow`, 2026-09-03: the author wanted the bigger room to
//! read as the composer opening up, not as a different place, then wanted
//! it a dialog on top of the board rather than a panel taking the columns
//! zone edge to edge, and then — T-163 — wanted `Tab` on a card to open the
//! ticket's description the same way). Composing, the title is editable
//! and the mini composer's workspace and tags ride along; on a ticket that
//! exists the title is the ticket's, read-only, and the context row names
//! the ticket's workspace, which Shift+Tab still sets while nothing has
//! locked it. From the ticket page the same editor takes the whole screen
//! under a breadcrumb. No rules, no boxes (L1): the body is text on its
//! surface, the hardware cursor is the only cursor, and the footer is the
//! keymap's.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, Editor, EditorPurpose, Field};
use crate::layout::{self, Slot};
use crate::tags;
use crate::text::{age_slot, area_window, created_at_epoch_ms, edit_window, truncate};
use crate::theme::{BarWeight, Ramp, TagLevel, Theme};

use mesimon_core::keymap::Scope;

use super::chrome;
use super::dialog;

/// Rows above the body, full screen: title, breathing, context, breathing.
const HEAD_ROWS: u16 = 4;
/// The full-screen editor's left margin: one cell, the page's.
const PAGE_PAD: u16 = 1;
/// Rows above the body in the dialog: title, context, breathing. A card's
/// anatomy — the meta row sits directly under the title, the way a card's
/// session and chip rows do — which is also what lets frame zero of the
/// grow be the card itself, row for row.
const DIALOG_HEAD_ROWS: u16 = 3;
/// The dialog's frame is a card's: `[bar 2][pad 1][content][pad 1]`.
const DIALOG_LEAD: u16 = tags::BAR_WIDTH as u16 + 1;
const DIALOG_FRAME: u16 = DIALOG_LEAD + 1;
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

    // ---- row 0: the header — the NOTE chip, the breadcrumb, and the ticket
    // the note belongs to as its leaf (T-158: the editor never named its
    // ticket before). Row 2 is the context line; composing full screen (a
    // room the composer no longer opens, kept for the shape) the title is
    // typed on row 2 and the context sits under it.
    let leaf = match &ed.purpose {
        EditorPurpose::Note { .. } => Some(ed.title.as_str().to_string()),
        EditorPurpose::Compose { .. } => None,
    };
    chrome::draw_header(
        f,
        Rect { x: area.x, y: area.y, width: area.width, height: 1 },
        app,
        leaf.as_deref(),
    );
    let mut ctx = context_line(app, ed, &theme.rest, false);
    ctx.spans.insert(0, Span::raw(" ".repeat(PAGE_PAD as usize)));
    let mut cursor = None;
    let top = match &ed.purpose {
        EditorPurpose::Note { .. } => vec![Line::default(), Line::default(), ctx, Line::default()],
        EditorPurpose::Compose { .. } => {
            let (mut spans, cx) = title_spans(
                ed,
                (area.width as usize).saturating_sub(PAGE_PAD as usize + 1),
                &theme.rest,
            );
            spans.insert(0, Span::raw(" ".repeat(PAGE_PAD as usize)));
            cursor = cx.map(|cx| ((PAGE_PAD + cx).min(area.width.saturating_sub(1)), area.y + 2));
            vec![Line::default(), Line::default(), Line::from(spans), ctx]
        }
    };
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
    if let Some(c) = draw_body(f, ed, body, PAGE_PAD, &theme.rest, body_hint(app, ed)) {
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
    // Whether `dialog::frame` will draw one (its floor): the context row
    // says NEW TICKET itself when there is no edge to say it.
    let framed = area.width >= 4 && area.height >= 3;
    // Framed (T-158): the frame stands one cell outside the column run on
    // every side — in the gutters and the breathing rows the inset kept
    // clear — so the stripe still sits on the column's own bar cell and the
    // cards beside it stay whole. Below three rows `dialog::frame` draws no
    // frame and hands the area back, which is what keeps frame zero of the
    // grow the card itself.
    let area = dialog::frame(
        f,
        app,
        area,
        surface,
        ink,
        dialog::Edges {
            title: dialog::title(ink, heading(app, ed)),
            tail: dialog::keys(app, Scope::Editor, ink, (area.width as usize).saturating_sub(6)),
        },
    );
    if area.width <= DIALOG_FRAME || area.height == 0 {
        return;
    }

    // ---- the stripe: the card's two-cell bar, wearing the tags
    // exactly as the real card will (`render_edit` paints the phantom card's
    // the same way) --------------------------------------------------------
    // The picked tags composing; the ticket's own on a description, so the
    // dialog's stripe is the card's stripe at frame zero and after.
    let worn = match &ed.purpose {
        EditorPurpose::Compose { tags, .. } => tags::painted(&app.board, tags),
        EditorPurpose::Note { ticket, .. } => app
            .board
            .ticket(*ticket)
            .map(|t| tags::painted(&app.board, &t.tags))
            .unwrap_or_default(),
    };
    let (plain_ch, plain_style) = theme.bar(BarWeight::Cursor);
    let bar = tags::bar_spans(theme, plain_ch, plain_style, &worn, TagLevel::Selected, surface);
    let stripe: Vec<Line<'static>> = (0..area.height).map(|_| Line::from(bar.clone())).collect();
    f.render_widget(
        Paragraph::new(stripe),
        Rect { x: area.x, y: area.y, width: tags::BAR_WIDTH as u16, height: area.height },
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
    let head = vec![Line::from(spans), context_line(app, ed, ink, framed), Line::default()];
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
    if let Some(c) = draw_body(f, ed, body, 0, ink, body_hint(app, ed)) {
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
    let pinned: Vec<bool> =
        app.board.sorted_columns().iter().map(|c| c.settings.collapsed).collect();
    let geom = layout::board_geometry(
        cards.width,
        app.columns().len(),
        app.cursor_col,
        &pinned,
        &mut window,
    );
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
    // The frame's cell on every side: the gutter (or page pad) beside the
    // run, the breathing row over the cards, the row under them.
    let y = cards.y + DIALOG_INSET_Y.min(cards.height.saturating_sub(1));
    Rect {
        x: cards.x + x.saturating_sub(1),
        y: y.saturating_sub(1),
        width: width + 2,
        height: cards.height.saturating_sub(2 * DIALOG_INSET_Y).max(1) + 2,
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
fn surface(theme: &Theme) -> (Option<ratatui::style::Color>, &Ramp) {
    match theme.selected_bg {
        Some(bg) => (Some(bg), &theme.sel),
        None => (theme.bg, &theme.rest),
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
fn context_line(app: &App, ed: &Editor, ink: &Ramp, framed: bool) -> Line<'static> {
    let theme = &app.theme;
    let dim1 = Style::default().fg(ink.dim1);
    let dim2 = Style::default().fg(ink.dim2);
    let now = mesimon_core::clock::now_ms();
    let mut ctx_spans: Vec<Span<'static>> = Vec::new();
    let workspace_word = |workspace: Option<mesimon_core::board::WorkspaceStrategy>| match workspace
    {
        Some(mesimon_core::board::WorkspaceStrategy::Worktree) => "worktree",
        Some(mesimon_core::board::WorkspaceStrategy::AdoptExisting) => "adopt",
        Some(mesimon_core::board::WorkspaceStrategy::SharedCheckout) | None => "shared",
    };
    match &ed.purpose {
        EditorPurpose::Compose { workspace, tags } => {
            let word = workspace_word(*workspace);
            // Framed, the top edge already says NEW TICKET; the row starts
            // at the column. (Unframed — frame zero of the grow, a terminal
            // too short for a frame — it says it here.)
            if !framed {
                ctx_spans.push(Span::styled(format!("{} ∙ ", heading(app, ed)), dim2));
            }
            if let Some(col) = app.columns().get(app.cursor_col) {
                ctx_spans.push(Span::styled(col.to_uppercase(), dim1));
            }
            ctx_spans.push(Span::styled(format!(" ∙ ⎇ {word}"), dim1));
            // The key beside the pick it cycles, the way the one-line
            // composer's card spells it (`card::render_workspace_selector`)
            // — the dialog's bottom edge does not repeat it.
            ctx_spans.push(Span::styled("  shift+tab".to_string(), dim2));
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
            // WHICH note this is — the description or another, existing or
            // new — is the heading: the frame's top edge says it on the
            // dialog, and the row says it itself where there is no edge
            // (frame zero of the grow, the full-screen editor under its NOTE
            // chip). Then the ticket's workspace, where the composer shows
            // its pick — Shift+Tab sets it here too while nothing has locked
            // it, and a toggle with no readout is a coin flip — then who last
            // wrote it (T-158). The heading leads so the row fits a
            // two-column dialog: "∙ the description" on the end was what the
            // dialog's width cut first.
            let t = app.board.ticket(*ticket);
            let meta = note.and_then(|id| t.and_then(|t| t.note(id)));
            let mut parts: Vec<Span<'static>> = Vec::new();
            if let Some(t) = t {
                parts.push(Span::styled(format!("⎇ {}", workspace_word(t.workspace)), dim1));
                // Spelled only while Shift+Tab can still change it — the
                // choice locks with the first session or worktree.
                if app.ctx().workspace_open {
                    parts.push(Span::styled("shift+tab".to_string(), dim2));
                }
            }
            if let Some(m) = meta {
                let who = super::ticket::author_word(&m.edited_by, app);
                let when = created_at_epoch_ms(&m.edited_at)
                    .map(|ms| format!(" {} ago", age_slot(now, ms, false).trim()))
                    .unwrap_or_default();
                parts.push(Span::styled(format!("edited by {who}{when}"), dim2));
            }
            if !framed {
                parts.insert(0, Span::styled(heading(app, ed).to_string(), dim2));
            }
            for (i, part) in parts.into_iter().enumerate() {
                if i > 0 {
                    ctx_spans.push(Span::styled(" ∙ ".to_string(), dim2));
                }
                ctx_spans.push(part);
            }
        }
    }
    if ed.dirty() {
        ctx_spans.push(Span::styled(" ∙ unsaved".to_string(), dim2));
    }
    // A teammate wrote the note since it was opened here (T-335): the
    // draft stays, the row says so in the full register, and a save is
    // still last writer wins — which is what "overwrites" warns.
    if let Some(who) = app.note_changed_elsewhere(ed) {
        ctx_spans.push(Span::styled(
            format!(" ∙ changed elsewhere by {who} ∙ saving overwrites"),
            Style::default().fg(ink.base),
        ));
    }
    Line::from(ctx_spans)
}

/// The dialog's name, set into its frame's top edge (and said on the
/// context row where there is no edge): what the text IS — a new ticket,
/// the description (`notes[0]`, or the fresh note that becomes it), or
/// another note.
fn heading(app: &App, ed: &Editor) -> &'static str {
    match &ed.purpose {
        EditorPurpose::Compose { .. } => "NEW TICKET",
        EditorPurpose::Note { ticket, note } => {
            let t = app.board.ticket(*ticket);
            let exists = note.is_some_and(|id| t.is_some_and(|t| t.note(id).is_some()));
            let first = t.is_some_and(|t| t.description().map(|d| d.id) == *note);
            match (exists, first) {
                (true, true) => "DESCRIPTION",
                (true, false) => "NOTE",
                (false, true) => "NEW DESCRIPTION",
                (false, false) => "NEW NOTE",
            }
        }
    }
}

/// What an empty body is for, in the footer's own word: `describe it` when
/// the text will be the ticket's description — composing, or a note that
/// is (or would become) `notes[0]` — and `write the note` otherwise.
fn body_hint(app: &App, ed: &Editor) -> &'static str {
    let describes = match &ed.purpose {
        EditorPurpose::Compose { .. } => true,
        EditorPurpose::Note { ticket, note } => {
            app.board.ticket(*ticket).is_some_and(|t| t.description().map(|d| d.id) == *note)
        }
    };
    if describes {
        "describe it"
    } else {
        "write the note"
    }
}

/// The body in `area`, `pad` cells in from its left edge: the text (or,
/// empty, `hint`), scrolled under the cursor, in `ink`. Returns the cursor
/// cell when the body is the focused field.
fn draw_body(
    f: &mut Frame,
    ed: &Editor,
    area: Rect,
    pad: u16,
    ink: &Ramp,
    hint: &str,
) -> Option<(u16, u16)> {
    let body_h = area.height as usize;
    if body_h == 0 {
        return None;
    }
    let pad_s = " ".repeat(pad as usize);
    let width = (area.width as usize).saturating_sub(pad as usize + 1);
    ed.body_width.set(width);
    let (top_line, rows, (cy, cx)) = area_window(&ed.body, ed.top.get(), body_h, width);
    ed.top.set(top_line);
    let mut lines: Vec<Line<'static>> = Vec::new();
    if ed.body.is_empty() {
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
