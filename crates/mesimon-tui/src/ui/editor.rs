//! The note editor (2026-09-02): a title row, one quiet line of context,
//! then the body. Two purposes share the shape, on two surfaces — and the
//! surface is the SCREEN's. Over the board it is a DIALOG: the cursor card,
//! grown — a room on the card's own surface, with no accent bar down its
//! left edge (T-1151: the card beside it wears the tags),
//! the header, the column headers and a margin of the board still in view
//! around it — and it grows out of the card it stands
//! for (`Editor::grow`, 2026-09-03: the author wanted the bigger room to
//! read as the composer opening up, not as a different place, then wanted
//! it a dialog on top of the board rather than a panel taking the columns
//! zone edge to edge, and then — T-163 — wanted `Tab` on a card to open the
//! ticket's description the same way). Composing, the title is editable
//! and the mini composer's workspace and tags ride along; on a ticket that
//! exists the title is the ticket's, read-only, and the context row names
//! the ticket's workspace, which Shift+Tab still sets while nothing has
//! locked it. From the ticket page the same editor takes the whole screen
//! under a breadcrumb. On a ticket with more than one note, `Tab` walks
//! them in the same room (T-440), and the heading counts where it stands —
//! `NOTE 2/3`. No rules, no boxes (L1): the body is text on its surface, the
//! hardware cursor is the only cursor, and the footer is the keymap's.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, AskTarget, Editor, EditorPurpose, Field};
use crate::layout::{self, Slot};
use crate::text::{age_ago, area_window, created_at_epoch_ms, edit_window, truncate};
use crate::theme::{Ramp, Theme};

use mesimon_core::keymap::{self, Scope};

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
        EditorPurpose::Note { .. } | EditorPurpose::Ask { .. } => {
            Some(ed.title.as_str().to_string())
        }
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
        EditorPurpose::Note { .. } | EditorPurpose::Ask { .. } => {
            vec![Line::default(), Line::default(), ctx, Line::default()]
        }
        EditorPurpose::Compose { .. } => {
            let (mut spans, cx) = title_spans(
                ed,
                (area.width as usize).saturating_sub(PAGE_PAD as usize + 1),
                &theme.rest,
            );
            // A refused `^s` shakes the title within its pad.
            let pad = (i32::from(PAGE_PAD) + i32::from(ed.title_dx())).max(0) as u16;
            spans.insert(0, Span::raw(" ".repeat(pad as usize)));
            cursor = cx.map(|cx| ((pad + cx).min(area.width.saturating_sub(1)), area.y + 2));
            title_spot(app, ed, Rect { x: area.x + PAGE_PAD, y: area.y + 2, ..area }, cx);
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
    if ed.reading {
        let lines = reader_lines(app, ed, body, PAGE_PAD, crate::rich::Surface::Ground);
        f.render_widget(Paragraph::new(lines), body);
    } else {
        if let Some(c) = draw_body(f, ed, body, PAGE_PAD, &theme.rest, body_hint(app, ed)) {
            cursor = Some(c);
        }
        body_spot(app, ed, body, PAGE_PAD);
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
/// bigger — the same surface (no accent bar down its left edge since
/// T-1151: the card beside it wears the tags), the same `sel` ink, the title on the first row and the meta row under it. At
/// rest it is `dialog_rect`: centred in the columns zone (`cards`), with the
/// column headers above it and a margin of board around it — and beside the
/// card it came from, which stays lit while the rest recedes (T-1144). For the first
/// `GROW` after Tab it is drawn between the phantom card's rectangle and
/// that, so the dialog visibly comes out of the card — and since the
/// anatomy is the card's, frame zero is the card, row for row (its bar
/// aside). The board's own footer stays under it and already speaks for the
/// editor's scope.
pub(super) fn draw_dialog(f: &mut Frame, app: &App, ed: &Editor, cards: Rect) {
    if ed.reading {
        return draw_reader_dialog(f, app, ed);
    }
    let theme = &app.theme;
    // Inside the frame: `[pad 2][content][pad 2]`, the reader dialog's
    // measure. No stripe down the left edge (T-1151, author: "the big block
    // on the left side, remove it"): the card the dialog came from stays in
    // view wearing its own.
    let (lead, chrome) = (2, 4);
    let rest = dialog_rect(app, cards);
    let area = match (ed.grow, ed.grow_progress()) {
        (Some((from, _)), Some(t)) => lerp_rect(from, rest, t),
        _ => rest,
    };
    if area.width <= chrome || area.height == 0 {
        return;
    }
    let (surface, ink) = surface(theme);
    // Whether `dialog::frame` will draw one (its floor): the context row
    // says NEW TICKET itself when there is no edge to say it.
    let framed = area.width >= 4 && area.height >= 3;
    // Framed (T-158): the frame stands one cell outside the column run on
    // every side — in the gutters and the breathing rows the inset kept
    // clear — so the cards beside it stay whole. Below three rows `dialog::frame` draws no
    // frame and hands the area back, which is what keeps frame zero of the
    // grow the card itself.
    let outer = area;
    let area = dialog::frame(
        f,
        app,
        area,
        surface,
        ink,
        dialog::Edges {
            title: edge_heading(app, ed, ink),
            tail: dialog::keys(app, Scope::Editor, ink, (area.width as usize).saturating_sub(6)),
        },
    );
    // Stood above or below its card (`clear_of`), the edge points at it.
    if framed {
        dialog::notch(f, app, outer);
    }
    if area.width <= chrome || area.height == 0 {
        return;
    }

    // ---- the head: the title where the card's title row was, the context
    // under it where the card's meta row was ----------------------------------
    let inner =
        Rect { x: area.x + lead, y: area.y, width: area.width - chrome, height: area.height };
    let (spans, cx) = title_spans(ed, inner.width as usize, ink);
    title_spot(app, ed, inner, cx);
    let head = vec![Line::default(), context_line(app, ed, ink, framed), Line::default()];
    f.render_widget(
        Paragraph::new(head),
        Rect { height: DIALOG_HEAD_ROWS.min(inner.height), ..inner },
    );
    // A refused `^s` shakes the title row, cursor and all, a cell either
    // way: into the pads at either edge, both the dialog's own surface.
    let tx = (i32::from(inner.x) + i32::from(ed.title_dx())).max(i32::from(area.x)) as u16;
    let mut cursor = cx.map(|cx| ((tx + cx).min(area.x + area.width - 1), inner.y));
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect { x: tx, y: inner.y, width: inner.width, height: 1.min(inner.height) },
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
    body_spot(app, ed, body, 0);
    if let Some((x, y)) = cursor {
        f.set_cursor_position((x, y));
    }
}

/// Where the composer dialog rests: over a run of whole columns
/// (`snap_to_columns`, on the same geometry the board just drew), `DIALOG_INSET_Y`
/// rows in from the zone's top and bottom.
fn dialog_rect(app: &App, cards: Rect) -> Rect {
    let mut window = app.col_window.get();
    let pinned = app.pinned_columns();
    let geom = layout::board_geometry(
        cards.width,
        app.columns().len(),
        app.cursor_col,
        app.move_target(),
        pinned,
        &app.column_sizes(),
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
    // The card the dialog came from (T-1144), in the zone's cells: a run
    // that leaves its column out keeps it in view.
    let card = app.dialog_anchor();
    let avoid = card.map(|c| (c.x.saturating_sub(cards.x), c.width));
    let beside = avoid.and_then(|a| snap_to_columns(&cols, cards.width, Some(a)));
    let (x, width) = beside
        .or_else(|| snap_to_columns(&cols, cards.width, None))
        .unwrap_or((DIALOG_INSET_X, cards.width.saturating_sub(2 * DIALOG_INSET_X).max(1)));
    // The frame's cell on every side: the gutter (or page pad) beside the
    // run, the breathing row over the cards, the row under them.
    let y = cards.y + DIALOG_INSET_Y.min(cards.height.saturating_sub(1));
    let rest = Rect {
        x: cards.x + x.saturating_sub(1),
        y: y.saturating_sub(1),
        width: width + 2,
        height: cards.height.saturating_sub(2 * DIALOG_INSET_Y).max(1) + 2,
    };
    match card {
        Some(card) if beside.is_none() => clear_of(rest, card),
        _ => rest,
    }
}

/// The fewest rows a dialog stood above or below its card settles for:
/// frame, head and breathing rows, and a body of three.
const DIALOG_MIN_H: u16 = 2 + DIALOG_HEAD_ROWS + 1 + 3;

/// `rest` cut to the side of `card` with more rows, a row clear of it,
/// where no run of columns leaves the card out (T-1144): a narrow board
/// still shows the card the dialog is about. `rest` as it is when neither
/// side has `DIALOG_MIN_H` rows, or the card is not under it.
fn clear_of(rest: Rect, card: Rect) -> Rect {
    if !rest.intersects(card) {
        return rest;
    }
    let below_y = card.bottom() + 1;
    let below = rest.bottom().saturating_sub(below_y);
    let above = card.y.saturating_sub(1).saturating_sub(rest.y);
    match (above, below) {
        (a, b) if b >= a && b >= DIALOG_MIN_H => Rect { y: below_y, height: b, ..rest },
        (a, _) if a >= DIALOG_MIN_H => Rect { height: a, ..rest },
        _ => rest,
    }
}

/// The run of whole columns the dialog covers, as `(x, width)` in the
/// zone's cells — `cols` are the expanded columns' `(x, width)` in board
/// order. Among the runs at least `DIALOG_MIN_W` wide and clear of
/// `avoid` (a span `(x, width)` the run must not cross, T-1144): the ones
/// within `DIALOG_MAX_W` first, then the one whose centre is nearest the
/// zone's, then the narrowest. Every column when no run reaches the floor
/// and nothing is avoided; `None` with no columns at all, or no run clear
/// of `avoid`.
fn snap_to_columns(
    cols: &[(u16, u16)],
    zone_w: u16,
    avoid: Option<(u16, u16)>,
) -> Option<(u16, u16)> {
    let (&(first_x, _), &(last_x, last_w)) = (cols.first()?, cols.last()?);
    let runs = cols.iter().enumerate().flat_map(|(i, &(x, _))| {
        cols[i..].iter().map(move |&(end_x, end_w)| (x, end_x + end_w - x))
    });
    let clear = |&(x, w): &(u16, u16)| avoid.is_none_or(|(ax, aw)| x + w <= ax || ax + aw <= x);
    let best = runs.filter(|&(_, w)| w >= DIALOG_MIN_W).filter(clear).min_by_key(|&(x, w)| {
        // Twice the distance between the run's centre and the zone's, so
        // the comparison stays in whole cells.
        (w > DIALOG_MAX_W, (2 * x + w).abs_diff(zone_w), w)
    });
    match (best, avoid) {
        (Some(run), _) => Some(run),
        (None, Some(_)) => None,
        (None, None) => Some((first_x, last_x + last_w - first_x)),
    }
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

/// The composer's title row from `row.x` on, for a click to put the
/// cursor in it (T-716): `cx` is the cursor's cell while the title has it,
/// which is how far the field's window has scrolled. A note's title is the
/// ticket's, not the editor's, and takes no click.
fn title_spot(app: &App, ed: &Editor, row: Rect, cx: Option<u16>) {
    if !ed.composing() {
        return;
    }
    let skip = cx.map_or(0, |cx| (ed.title.width_before_cursor() as u16).saturating_sub(cx));
    app.hits.borrow_mut().record(
        Rect { height: 1.min(row.height), ..row },
        crate::mouse::Target::Title { x0: row.x, skip },
    );
}

/// The body's rows, for a click to put the cursor in them (T-716).
fn body_spot(app: &App, ed: &Editor, body: Rect, pad: u16) {
    let top = u16::try_from(ed.top.get()).unwrap_or(u16::MAX);
    app.hits
        .borrow_mut()
        .record(body, crate::mouse::Target::Body { x0: body.x + pad, y0: body.y, top });
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
        EditorPurpose::Compose { workspace, tags, plan, tier } => {
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
            // `^p` armed (T-434): the launch runs in plan mode, said where
            // the one-line composer's card says it.
            if *plan {
                ctx_spans.push(Span::styled(" ∙ plan mode".to_string(), dim1));
            }
            // `^n` (T-443): the tier the new ticket starts on, where it is
            // not the default — said beside `plan mode`, the other launch pick.
            if let Some(word) = app.field_tier_word(None, tier.as_deref()) {
                ctx_spans.push(Span::styled(format!(" ∙ {word}"), dim1));
            }
            if !tags.is_empty() {
                ctx_spans.push(Span::styled(" ∙".to_string(), dim2));
                for t in tags {
                    ctx_spans.push(Span::raw(" "));
                    let text = format!(" {} ", t.name);
                    let style =
                        crate::tags::chip_style(theme, app.board.tint_of(t)).unwrap_or(dim1);
                    ctx_spans.push(Span::styled(text, style));
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
            // Not in the reader (T-1139, author: "why is workspace shown on
            // the note popup subtitle? it's not relevant there"): it reads
            // the note, sets nothing, and hangs under the card that already
            // wears the workspace.
            if let Some(t) = t.filter(|_| !ed.reading) {
                parts.push(Span::styled(format!("⎇ {}", workspace_word(t.workspace)), dim1));
                // Spelled only while Shift+Tab can still change it — the
                // choice locks with the first session or worktree.
                let cx = app.frame_ctx();
                if cx.workspace_open {
                    parts.push(Span::styled("shift+tab".to_string(), dim2));
                }
            }
            if let Some(m) = meta {
                let who = super::ticket::author_word(&m.edited_by, app);
                let when = created_at_epoch_ms(&m.edited_at)
                    .map(|ms| format!(" {}", age_ago(now, ms)))
                    .unwrap_or_default();
                parts.push(Span::styled(format!("edited by {who}{when}"), dim2));
            }
            if !framed {
                parts.insert(0, Span::styled(heading(app, ed), dim2));
            }
            for (i, part) in parts.into_iter().enumerate() {
                if i > 0 {
                    ctx_spans.push(Span::styled(" ∙ ".to_string(), dim2));
                }
                ctx_spans.push(part);
            }
        }
        // The ask room (T-380): the field's delivery row, where the
        // composer shows its workspace pick — `now` or `queued`, and the key
        // that flips it while the checkout is shared, spelled here because
        // the dialog's bottom edge does not repeat it. A ticket's ask names
        // the ticket by its key; a column's names the seats the words reach
        // — every seat in it, since T-405 starts the empty ones.
        EditorPurpose::Ask { target, queued, immediately, accept_plan, plan, tier } => {
            if !framed {
                ctx_spans.push(Span::styled(format!("{} ∙ ", heading(app, ed)), dim2));
            }
            match target {
                AskTarget::Ticket(ticket) => {
                    if let Some(t) = app.board.ticket(*ticket) {
                        ctx_spans.push(Span::styled(t.short_key.clone(), dim1));
                        ctx_spans.push(Span::styled(" ∙ ".to_string(), dim2));
                    }
                }
                AskTarget::Column(name) => {
                    let seats = app.column_reach(name);
                    ctx_spans.push(Span::styled(
                        mesimon_core::text::plural(seats, mesimon_core::keymap::AGENT_WORD),
                        dim1,
                    ));
                    ctx_spans.push(Span::styled(" ∙ ".to_string(), dim2));
                }
            }
            ctx_spans.push(Span::styled(
                crate::app::App::ask_mode_word(*accept_plan, *queued, *immediately, *plan),
                dim1,
            ));
            // `^n` (T-443): the tier the words switch the agent to.
            let ticket = match target {
                AskTarget::Ticket(t) => Some(*t),
                AskTarget::Column(_) => None,
            };
            if let Some(word) = app.field_tier_word(ticket, tier.as_deref()) {
                ctx_spans.push(Span::styled(format!(" ∙ {word}"), dim1));
            }
            let fc = app.frame_ctx();
            if fc.ask_queueable && !fc.ask_plan_ready {
                ctx_spans.push(Span::styled("  shift+tab".to_string(), dim2));
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
/// another note — and, where `tab` walks the ticket's notes (T-440), which
/// of them: `NOTE 2/3`.
fn heading(app: &App, ed: &Editor) -> String {
    let word = heading_word(app, ed);
    match app.note_ring(ed) {
        Some((at, n)) => format!("{word} {}/{n}", at + 1),
        None => word.to_string(),
    }
}

/// The heading in a dialog's top edge: the word, then where the note
/// stands in the ring in the quiet ink, `NOTE ∙ 2 of 3` — the transcript's
/// count, so the two dialogs count alike (T-1152).
fn edge_heading(app: &App, ed: &Editor, ink: &Ramp) -> Vec<Span<'static>> {
    let mut spans = dialog::title(ink, heading_word(app, ed));
    if let Some((at, n)) = app.note_ring(ed) {
        spans.push(Span::styled(format!(" ∙ {} of {n}", at + 1), Style::default().fg(ink.dim2)));
    }
    spans
}

fn heading_word(app: &App, ed: &Editor) -> &'static str {
    match &ed.purpose {
        EditorPurpose::Compose { .. } => "NEW TICKET",
        // Who the words reach — the one-line field's own hint, in capitals.
        EditorPurpose::Ask { target: AskTarget::Ticket(_), .. } => "ASK AGENT",
        EditorPurpose::Ask { target: AskTarget::Column(_), .. } => "ASK EVERY AGENT",
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
        // The empty room says what it is for in the field's own words —
        // and at `accept plan` (T-420) what the send actually does, so the
        // grown field does not read as an ordinary ask (user 2026-09-23:
        // "not to distract the user from the actual thing that's going to
        // happen").
        EditorPurpose::Ask { target: AskTarget::Ticket(_), accept_plan: true, .. } => {
            return "accept the plan ∙ words go right after"
        }
        EditorPurpose::Ask { target: AskTarget::Ticket(_), .. } => return "ask agent",
        EditorPurpose::Ask { target: AskTarget::Column(_), .. } => return "ask every agent",
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

/// The note reader over the board (T-1139): a card's dialog, hung one row
/// under the card with its notch while the rest of the board recedes — the
/// transcript's place and measure (T-817, T-921), so the card stays in
/// sight (author: "why is the popup hiding the ticket? it should come
/// below it"). The heading counts the notes in the top edge, the context
/// row leads, the note follows; the reading keys sit in the bottom edge
/// and Esc is the footer's. It stands on the card's surface, the editor's
/// (T-1152): the transcript hangs in the same place on the board's ground,
/// and the ground is what tells a note from a conversation at a glance.
fn draw_reader_dialog(f: &mut Frame, app: &App, ed: &Editor) {
    let theme = &app.theme;
    let (surface, ink) = surface(theme);
    let screen = f.area();
    let mut area = dialog::placed(app, screen, super::transcript::ROWS, super::transcript::WIDTH);
    // The row above the footer stays the screen's, as a list's does.
    let whole_y = screen.bottom().saturating_sub(2);
    area.height = area.height.min(whole_y.saturating_sub(area.y)).max(3);
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let body = Rect { x: inner.x + 2, width: inner.width.saturating_sub(4), ..inner };
    let fits = body.width >= 8 && body.height >= 3;
    // The note is measured before the edge is drawn, so the edge's `jk`
    // offers a scroll on the very frame that has one.
    let words = Rect { y: body.y + 2, height: body.height.saturating_sub(2), ..body };
    // Elevated reads as the ground where the profile paints no card.
    let lines = fits.then(|| reader_lines(app, ed, words, 0, crate::rich::Surface::Elevated));
    app.preview_measured();
    let ctx = app.frame_ctx();
    let (own, _) = keymap::footer_split(Scope::Reader, &ctx);
    let own: Vec<_> = own.into_iter().filter(|b| b.verb != keymap::Verb::Back).collect();
    let tail = super::chrome::hint_spans(&own, &ctx, ink, inner.width.saturating_sub(4).into());
    let title = edge_heading(app, ed, ink);
    dialog::frame(f, app, area, surface, ink, dialog::Edges { title, tail });
    dialog::notch(f, app, area);
    let Some(lines) = lines else { return };
    f.render_widget(Paragraph::new(context_line(app, ed, ink, true)), Rect { height: 1, ..body });
    f.render_widget(Paragraph::new(lines), words);
}

/// The note reader's body (T-1139): the note drawn as markdown, the way the
/// ticket page's preview draws one, in the window `jk` and `{ }` move. With
/// the wrap off (`w`) the lines are laid out as wide as the widest needs
/// and the window shows `x` cells in from their left (`hl`). A window that
/// stops short of the last row ends in the `~` cut mark, as the preview's.
fn reader_lines(
    app: &App,
    ed: &Editor,
    area: Rect,
    pad: u16,
    surface: crate::rich::Surface,
) -> Vec<Line<'static>> {
    let Some(r) = ed.reader.as_ref() else { return Vec::new() };
    let theme = &app.theme;
    let budget = area.height as usize;
    let width = (area.width as usize).saturating_sub(pad as usize + 1);
    if budget == 0 || width == 0 {
        return Vec::new();
    }
    if ed.body.as_str().trim().is_empty() {
        return vec![Line::from(vec![
            Span::raw(" ".repeat(pad as usize)),
            Span::styled("empty", theme.dim3()),
        ])];
    }
    let unwrapped = app.reader_unwrapped;
    let key = crate::app::reader_key(ed);
    let selected = app.reader_cursor_source(ed);
    // Unwrapped, as wide as the longest source line plus room for the
    // indents a list or a quote adds, never narrower than the window; the
    // cap keeps a pasted log from laying out a mile of slab.
    let lay = if unwrapped {
        let longest =
            ed.body.as_str().lines().map(unicode_width::UnicodeWidthStr::width).max().unwrap_or(0);
        (longest + 8).clamp(width, 4096)
    } else {
        width
    };
    let newline = match &ed.purpose {
        EditorPurpose::Note { ticket, note: Some(id) } => app
            .board
            .ticket(*ticket)
            .and_then(|t| t.note(*id))
            .map_or(crate::rich::Newline::Break, crate::rich::Newline::of_note),
        _ => crate::rich::Newline::Break,
    };
    let (rows, widest) = {
        let mut slot = r.rows.borrow_mut();
        let fresh = slot.as_ref().is_some_and(|c| {
            c.key == key
                && c.width == lay
                && c.unwrapped == unwrapped
                && c.flavor == theme.flavor
                && c.surface == surface
                && c.selected == selected
        });
        if !fresh {
            // A note's words are a person's or an agent's: scrubbed as the
            // ticket page's preview scrubs them before they are drawn.
            let text = crate::peek::sanitize(ed.body.as_str());
            let rows =
                crate::rich::render_all_on(&text, lay, theme, surface, newline, selected.as_ref());
            let widest = rows.iter().map(|l| super::spans_width(&l.spans)).max().unwrap_or(0);
            *slot = Some(crate::app::ReaderRows {
                key,
                width: lay,
                unwrapped,
                flavor: theme.flavor,
                surface,
                rows: std::rc::Rc::new(rows),
                widest,
                selected: selected.clone(),
            });
        }
        let c = slot.as_ref().expect("laid out above");
        (std::rc::Rc::clone(&c.rows), c.widest)
    };
    let mut at = r.pager.window(Some(key), rows.len(), budget, false);
    // A link just selected is brought into the window, clear of the cut
    // mark, as the transcript's is; sideways too, unwrapped.
    if r.reveal.replace(false) {
        let underlined = |s: &Span<'_>| s.style.add_modifier.contains(Modifier::UNDERLINED);
        if let Some(row) = rows.iter().position(|l| l.spans.iter().any(underlined)) {
            if row < at || row >= at + budget.saturating_sub(1).max(1) {
                r.pager.jump(row);
                at = r.pager.window(Some(key), rows.len(), budget, false);
            }
            if unwrapped {
                let mut cell = 0;
                for s in &rows[row].spans {
                    if underlined(s) {
                        break;
                    }
                    cell += unicode_width::UnicodeWidthStr::width(s.content.as_ref());
                }
                if cell < r.x.get() || cell >= r.x.get() + width {
                    r.x.set(cell.saturating_sub(width / 4));
                }
            }
        }
    }
    let max_x = if unwrapped { widest.saturating_sub(width) } else { 0 };
    r.max_x.set(max_x);
    let x = r.x.get().min(max_x);
    r.x.set(x);
    let mut shown: Vec<Line<'static>> = rows
        .iter()
        .skip(at)
        .take(budget)
        .map(|l| if unwrapped { Line::from(cells(&l.spans, x, width)) } else { l.clone() })
        .collect();
    if at + shown.len() < rows.len() {
        crate::rich::mark_cut(&mut shown, width, theme);
    }
    if pad > 0 {
        for line in &mut shown {
            line.spans.insert(0, Span::raw(" ".repeat(pad as usize)));
        }
    }
    shown
}

/// The `take` cells of `spans` from cell `skip` on, styles kept. A wide
/// character cut by either edge becomes a space, so a row never draws
/// wider than it was asked.
fn cells(spans: &[Span<'static>], skip: usize, take: usize) -> Vec<Span<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let (start, end) = (skip, skip + take);
    let mut out = Vec::new();
    let mut at = 0usize;
    for span in spans {
        let mut kept = String::new();
        for g in span.content.graphemes(true) {
            let w = g.width();
            let (from, to) = (at, at + w);
            at = to;
            if to <= start || w == 0 && from < start {
                continue;
            }
            if from >= end {
                break;
            }
            if from < start || to > end {
                kept.push_str(&" ".repeat(to.min(end) - from.max(start)));
            } else {
                kept.push_str(g);
            }
        }
        if !kept.is_empty() {
            out.push(Span::styled(kept, span.style));
        }
        if at >= end {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-1144: the run the dialog takes leaves the card's column out, and
    /// with no such run wide enough it says so rather than cover the card.
    #[test]
    fn the_dialog_snaps_to_columns_clear_of_its_card() {
        // Four columns of 27 with a gutter between, as at 120 cells.
        let cols = [(3, 27), (31, 27), (59, 27), (87, 27)];
        assert_eq!(snap_to_columns(&cols, 117, None), Some((31, 55)), "the middle two");
        assert_eq!(snap_to_columns(&cols, 117, Some((31, 27))), Some((59, 55)), "right of it");
        assert_eq!(snap_to_columns(&cols, 117, Some((59, 27))), Some((3, 55)), "left of it");
        assert_eq!(snap_to_columns(&cols, 117, Some((3, 27))), Some((31, 55)));
        // Two columns: any run of the floor's width crosses the card.
        let two = [(3, 27), (31, 27)];
        assert_eq!(snap_to_columns(&two, 61, Some((3, 27))), None);
        assert_eq!(snap_to_columns(&two, 61, None), Some((3, 55)));
    }

    /// With no run beside the card, the dialog stands on the side of it
    /// with more rows, a row clear; covering it only when neither side
    /// holds `DIALOG_MIN_H`.
    #[test]
    fn with_no_run_beside_it_the_dialog_stands_above_or_below_its_card() {
        let rest = Rect { x: 2, y: 4, width: 57, height: 24 };
        let top = Rect { x: 3, y: 5, width: 27, height: 3 };
        assert_eq!(clear_of(rest, top), Rect { y: 9, height: 19, ..rest }, "below");
        let low = Rect { y: 20, ..top };
        assert_eq!(clear_of(rest, low), Rect { height: 15, ..rest }, "above");
        let mid = Rect { y: 12, height: 6, ..top };
        assert_eq!(clear_of(Rect { height: 20, ..rest }, mid), Rect { height: 20, ..rest });
    }
}
