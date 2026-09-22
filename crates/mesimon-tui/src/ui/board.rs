//! The board frame: column geometry from `layout::board_geometry` (07 §2,
//! post-D33k arithmetic), expanded columns, 1-cell spines, the minted
//! cursor-column treatment, and vertical scroll with the D33k chevron badges.

use mesimon_core::board::{SessionRecord, SessionState, Ticket};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap;

use crate::app::{App, AskTarget, InputPurpose, Mode};
use crate::layout::{self, Slot};
use crate::text::{edit_window, EditBuffer};

use super::card::{self, CardCtx};

use mesimon_core::clock::now_ms;

pub(super) fn draw_columns(f: &mut Frame, area: Rect, app: &App) {
    let cols = app.columns();
    if cols.is_empty() {
        return;
    }
    let mut window = app.col_window.get();
    // A column pinned collapsed (T-117) is a spine unless the cursor is in it.
    let pinned: Vec<bool> =
        app.board.sorted_columns().iter().map(|c| c.settings.collapsed).collect();
    let geom = layout::board_geometry(area.width, cols.len(), app.cursor_col, &pinned, &mut window);
    app.col_window.set(window);

    for (ci, name) in cols.iter().enumerate() {
        match geom.slots[ci] {
            Slot::Expanded { x, width } => {
                let rect = Rect { x: area.x + x, y: area.y, width, height: area.height };
                draw_column(f, rect, app, ci, name);
            }
            Slot::Spine { x } => {
                let rect = Rect { x: area.x + x, y: area.y, width: 1, height: area.height };
                draw_spine(f, rect, app, name);
            }
        }
    }
}

fn ticket_sessions(app: &App, ticket: ulid::Ulid) -> Vec<&SessionRecord> {
    app.board.sessions.iter().filter(|s| s.ticket == ticket && s.state.is_live()).collect()
}

fn draw_column(f: &mut Frame, area: Rect, app: &App, ci: usize, name: &str) {
    let theme = &app.theme;
    let tickets = app.board.column_tickets(name);
    let is_cursor_col = app.cursor_col == ci;
    let ghost = match &app.mode {
        Mode::Move { ticket, col, idx, .. } if *col == ci => Some((*ticket, *idx)),
        _ => None,
    };
    // While a move is pending, the card at its ORIGINAL spot stays visible
    // but semi-transparent — the trail under the blinking ghost.
    let moving: Option<ulid::Ulid> = match &app.mode {
        Mode::Move { ticket, .. } => Some(*ticket),
        _ => None,
    };

    // Rows: tickets minus the moving ghost, with the ghost re-inserted at its index.
    let rows: Vec<&Ticket> = match ghost {
        Some((gid, _)) => tickets.iter().filter(|t| t.id != gid).copied().collect(),
        None => tickets,
    };
    let count = rows.len() + ghost.map(|_| 1).unwrap_or(0);

    let ctx = CardCtx {
        theme,
        width: area.width,
        now_ms: now_ms(),
        spin: app.spin_frame(),
        names_key: true,
    };

    // Marquee clock: reset when the cursor lands on a different ticket.
    let marquee_ms = |t: &Ticket| -> u64 {
        match app.marquee.get() {
            Some((id, epoch)) if id == t.id => epoch.elapsed().as_millis() as u64,
            _ => {
                app.marquee.set(Some((t.id, std::time::Instant::now())));
                0
            }
        }
    };

    // In-place title edit (rename on the card itself; create as a phantom
    // card at the column tail — where the daemon will append it).
    let editing: Option<(&InputPurpose, &EditBuffer)> = match &app.mode {
        Mode::Input { purpose, buffer } => Some((purpose, buffer)),
        _ => None,
    };
    let rename_of = |t: &Ticket| -> Option<&EditBuffer> {
        match editing {
            Some((InputPurpose::Rename { id }, buf)) if *id == t.id => Some(buf),
            _ => None,
        }
    };
    // The prompt field (Shift+Enter) hangs UNDER its card instead of taking
    // the title line the way a rename does: the ticket is not what is being
    // edited here, it is who the text is going to — so it has to stay whole
    // and stay on screen while the sentence is typed.
    let prompt_of = |t: &Ticket| -> Option<(&EditBuffer, bool, bool)> {
        match editing {
            Some((
                InputPurpose::Prompt {
                    target: AskTarget::Ticket(ticket), queued, accept_plan, ..
                },
                buf,
            )) if *ticket == t.id => Some((buf, *queued, *accept_plan)),
            _ => None,
        }
    };

    // Build card line-groups in display order.
    struct Group {
        lines: Vec<Line<'static>>,
        /// The cursor card (any height): scroll keeps it visible.
        cursor: bool,
        waiting: bool,
        /// Line offset (within the group) and x-offset of a live edit cursor.
        /// The offset is not always 0: a rename edits the card's first line,
        /// a prompt edits a row appended under the whole card.
        edit_cursor: Option<(usize, u16)>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut push_card = |t: &Ticket, selected: bool, held: bool| {
        let sessions = ticket_sessions(app, t.id);
        let waiting = card::needs_you(t, &sessions);
        // The registry lives on the board, so colours resolve here rather
        // than inside the card, which never sees it.
        let painted = crate::tags::painted(&app.board, &t.tags);
        if let Some(buf) = rename_of(t) {
            let (line, x_off) = card::render_edit(&ctx, buf, &painted);
            groups.push(Group {
                lines: vec![line],
                cursor: true,
                waiting,
                edit_cursor: Some((0, x_off)),
            });
            return;
        }
        let trail = !held && moving == Some(t.id);
        let mq = if selected { Some(marquee_ms(t)) } else { None };
        // Is this card open? The `p` preference, or a quick-tag digit still
        // inside its reveal — on the cursor card; `P` opens every card
        // (T-237). The card is open on this alone — the transcript below may
        // or may not exist, and the tag row does not depend on it.
        let open = app.peek_all || (selected && app.peek_showing(t.id));
        // Transcript peek: the cursor card's highest-precedence session that
        // has a transcript (bash never does) — read through the draw cache.
        let peek = if open {
            let mut ranked: Vec<&&SessionRecord> = sessions.iter().collect();
            ranked.sort_by_key(|s| (mesimon_core::attention::rank(&s.state), s.id));
            ranked.iter().find(|s| crate::peek::preview_path(s).is_some()).and_then(|s| {
                let pk = app.peek_cache.peek_for(s.kind, crate::peek::preview_path(s)?)?;
                // The activity row is a claim about NOW: a parked, finished
                // or waiting session's last tool call is history.
                Some(if s.state == SessionState::Running || pk.activity.is_none() {
                    pk
                } else {
                    std::rc::Rc::new(crate::peek::Peek { activity: None, ..(*pk).clone() })
                })
            })
        } else {
            None
        };
        let wt = app.wt_item(t.id);
        // Has the agent said something the cursor has not been here for?
        // The done mark decays on it. Never for the cursor card or the move
        // ghost (both `cursorish`, and their ticket is the subject being
        // acked), so the frame between a cursor move and the ack agrees.
        let unseen = !(selected || held) && app.spoke_unseen(t.id);
        let mut lines = card::render(
            &ctx,
            t,
            &sessions,
            app.terminal_busy(t.id),
            wt,
            selected,
            held,
            trail,
            mq,
            open,
            peek.as_deref(),
            &painted,
            app.doomed(t.id),
            unseen,
            app.snooze_row(t.id).as_deref(),
            app.owed(t.id),
            app.pending_row(t.id).as_deref(),
            app.remote_initials(t.id).as_deref(),
            app.crown_mark(t.id),
        );
        // The card is drawn WHOLE first — glyph, title, sessions, peek — and
        // the field is added under it. That order is the point: what you are
        // about to talk to stays legible while you type at it.
        let edit_cursor = prompt_of(t).map(|(buf, queued, accept_plan)| {
            // What a blank Enter would do, in the seat's own words, and by
            // the same rule `commit_input` judges it: drop the entry that is
            // waiting, start the agent on the title where the seat is empty and
            // the toggle says now (T-294), or nothing at all.
            let starts = app.board.live_agent(t.id).is_none();
            // At `accept plan` (T-420) a blank Enter IS the accept.
            let placeholder = if accept_plan {
                "enter accepts the plan"
            } else if app.ticket_queued(t.id) && !(starts && !queued) {
                "enter drops"
            } else if starts {
                "start on the title"
            } else {
                "ask agent"
            };
            let (line, x_off) = card::render_prompt(&ctx, buf, placeholder);
            lines.push(line);
            let at = lines.len() - 1;
            // The delivery row, where the ask can wait (2026-09-04): after
            // the field, so the cursor row is unchanged.
            if app.ask_queueable(t.id) {
                lines.push(card::render_ask_mode(
                    &ctx,
                    crate::app::App::ask_mode_word(accept_plan, queued),
                ));
            }
            (at, x_off)
        });
        groups.push(Group { lines, cursor: selected || held, waiting, edit_cursor });
    };
    match ghost {
        Some((gid, gidx)) => {
            let ghost_ticket = app.board.ticket(gid);
            for (i, t) in rows.iter().enumerate() {
                if i == gidx {
                    if let Some(gt) = ghost_ticket {
                        push_card(gt, false, true);
                    }
                }
                push_card(t, false, false);
            }
            if gidx >= rows.len() {
                if let Some(gt) = ghost_ticket {
                    push_card(gt, false, true);
                }
            }
        }
        None => {
            for (i, t) in rows.iter().enumerate() {
                // A prompted card stays the cursor card. Every other text
                // field drops the selection (the composer's phantom card
                // becomes the cursor card instead), but this one is anchored
                // to a real ticket, and collapsing it mid-prompt would take
                // the agent's own state off screen while you type at it.
                let selected = is_cursor_col
                    && app.cursor_row == Some(i)
                    && (matches!(app.mode, Mode::Normal) || prompt_of(t).is_some());
                push_card(t, selected, false);
            }
        }
    }
    // New-ticket entry: a phantom card at the column tail, edited in place.
    // The second line is the M4 workspace selector (Shift+Tab cycles it).
    if is_cursor_col {
        if let Some((InputPurpose::Create { workspace, tags, .. }, buf)) = editing {
            // The tags picked with `^t` stripe the phantom card exactly as
            // they will stripe the real one — otherwise you are picking
            // blind until the ticket exists.
            let painted = crate::tags::painted(&app.board, tags);
            let (line, x_off) = card::render_edit(&ctx, buf, &painted);
            let column_default = app.board.column(name).and_then(|c| c.settings.workspace);
            let selector = card::render_workspace_selector(&ctx, *workspace, column_default);
            groups.push(Group {
                lines: vec![line, selector],
                cursor: true,
                waiting: false,
                edit_cursor: Some((0, x_off)),
            });
        }
    }

    // Flatten with rhythm: one blank row between cards. Track the cursor
    // card's line range and each card's, for scroll + badges.
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut card_ranges: Vec<(usize, usize, bool)> = Vec::new(); // (start, end, waiting)
    let mut cursor_range: Option<(usize, usize)> = None;
    let mut edit_at: Option<(usize, u16)> = None; // (flat line idx, x offset)
    for g in &groups {
        let start = lines.len();
        lines.extend(g.lines.iter().cloned());
        let end = lines.len();
        // One blank row between cards (06 §5.5 — reinstated after dogfood:
        // contiguous one-line cards were hard to separate by eye).
        lines.push(Line::default());
        if g.cursor {
            cursor_range = Some((start, end));
        }
        if let Some((off, x)) = g.edit_cursor {
            edit_at = Some((start + off, x));
        }
        card_ranges.push((start, end, g.waiting));
    }
    lines.pop(); // no trailing blank after the last card

    // The column's ask field (T-378) hangs UNDER THE HEADER the way a
    // ticket's hangs under its card: the header is what names where the
    // words go, so it stays whole and the field takes rows from the body.
    let column_ask: Option<(&EditBuffer, bool)> = match editing {
        Some((InputPurpose::Prompt { target: AskTarget::Column(n), queued, .. }, buf))
            if n == name =>
        {
            Some((buf, *queued))
        }
        _ => None,
    };
    let column_ask_toggle = column_ask.is_some() && app.column_ask_queueable(name);
    // Header, blank — plus the field and its delivery row while it is open.
    let head_rows = 2 + usize::from(column_ask.is_some()) + usize::from(column_ask_toggle);
    // Keep two blank rows inside the column above the board's footer gap.
    // Overflow cues also reserve a row plus a blank next to visible cards.
    let body_h = area.height.saturating_sub(2 + head_rows as u16) as usize;
    let total = lines.len();
    let mut scroll =
        if is_cursor_col { app.scroll_row.get().min(total.saturating_sub(1)) } else { 0 };
    // On the header the column shows its top, so `j` lands on a visible card
    // — including while the cursor has stepped one row further up, onto the
    // board's own top row (T-305), which is where `j` comes back from.
    let at_header = is_cursor_col
        && app.at_column_header()
        && !matches!(app.mode, Mode::Input { purpose: InputPurpose::Create { .. }, .. });
    // The cursor ITSELF is here — the bar cell — only while the top row does
    // not hold it. A focused header leaves the column its painted band, which
    // is what says where `j` returns to, and takes the bar with it.
    let col_header = at_header && !app.header_focus;
    if at_header {
        scroll = 0;
    }
    if is_cursor_col {
        if let Some((cs, ce)) = cursor_range {
            let lo = cs.saturating_sub(2);
            let hi = (ce + 2).min(total);
            if hi > scroll + body_h {
                scroll = hi - body_h;
            }
            if lo < scroll {
                scroll = lo;
            }
            // Scroll margins must yield before they cut the cursor card.
            // An oversized group instead keeps its live input row visible.
            if ce - cs <= body_h {
                scroll = scroll.clamp(ce.saturating_sub(body_h), cs);
            } else {
                let focus = edit_at.map(|(line, _)| line).unwrap_or(cs);
                scroll = focus.saturating_sub(body_h.saturating_sub(1)).max(cs);
            }
        }
        if total <= body_h {
            scroll = 0;
        }
        app.scroll_row.set(scroll);
    }

    // Show complete cards between explicit overflow cues. The cursor card
    // takes priority over a cue when it is taller than the available slice.
    let end_vis = (scroll + body_h).min(total);
    let not_cursor = |r: &&(usize, usize, bool)| Some((r.0, r.1)) != cursor_range;
    // Each cue costs two rows: its text and a blank separating it from cards.
    // Bottom: the first card that no longer fits after reserving the cue.
    let bottom: Option<(usize, usize, bool)> = if end_vis < total {
        card_ranges
            .iter()
            .find(|(_, end, _)| *end > end_vis.saturating_sub(2))
            .filter(not_cursor)
            .copied()
    } else {
        None
    };
    // Top: the last card still cut once the top rows are reserved.
    let top: Option<(usize, usize, bool)> = if scroll > 0 {
        card_ranges
            .iter()
            .rev()
            .find(|(start, _, _)| *start < scroll + 2)
            .filter(not_cursor)
            .copied()
    } else {
        None
    };
    // Content excludes hidden cards and their separators. Cues are their
    // own rows; never restyle a card or change its tag bar at the edge.
    let content_start = top.map(|(_, end, _)| (end + 1).max(scroll + 2)).unwrap_or(scroll);
    let content_end = bottom
        .map(|(start, _, _)| start.saturating_sub(1).min(end_vis.saturating_sub(2)))
        .unwrap_or(end_vis)
        .max(content_start);
    // Count everything actually omitted, including the boundary cards
    // removed to make room for a cue. Hidden attention stays in the header.
    let above = card_ranges.iter().filter(|(_, end, _)| *end <= content_start).count();
    let below = card_ranges.iter().filter(|(start, _, _)| *start >= content_end).count();
    let attn_out = card_ranges
        .iter()
        .filter(|(start, end, waiting)| *waiting && (*start < content_start || *end > content_end))
        .count();
    let overflow_arrow = |up| match (theme.glyph_tier(), up) {
        (crate::glyphs::Tier::Ascii, true) => '^',
        (crate::glyphs::Tier::Ascii, false) => 'v',
        (_, true) => '↑',
        (_, false) => '↓',
    };
    let overflow_line = |count: usize, up: bool| {
        let arrow = overflow_arrow(up);
        let direction = if up { "above" } else { "below" };
        Line::from(vec![
            Span::raw(" ".repeat(crate::tags::BAR_WIDTH + 1)),
            Span::styled(format!("{arrow} {count} {direction}"), theme.dim2()),
        ])
    };

    // ---- header row (cursor-column treatment, reminted 2026-08-30) --------
    // The cursor column's header is a full-width painted band on the
    // `selected` surface — a 1-cell bar there read as another card. The name
    // still steps dim1 -> base. A value and a shape, never a hue. With the
    // cursor ON the header (T-117) the band's first cell is the cursor BAR,
    // the way a selected card's is: the same shape one row up, still no hue.
    let quiet = if is_cursor_col { Style::default().fg(theme.sel.dim2) } else { theme.dim2() };
    let header_edit: Option<&EditBuffer> = match editing {
        Some((InputPurpose::RenameColumn { name: n }, buf)) if n == name => Some(buf),
        _ => None,
    };
    let mut head: Vec<Span<'static>> = Vec::new();
    let mut header_cursor_x: Option<u16> = None;
    if col_header || header_edit.is_some() {
        let (bar_ch, bar_style) = theme.bar(crate::theme::BarWeight::Cursor);
        head.push(Span::styled(bar_ch.to_string().repeat(crate::tags::BAR_WIDTH), bar_style));
        head.push(Span::raw(" "));
    } else {
        head.push(Span::raw(" ".repeat(crate::tags::BAR_WIDTH + 1)));
    }
    if let Some(buf) = header_edit {
        // The name edited in place, with the hardware cursor (06 §5.7),
        // the badges standing down for the field.
        let budget = (area.width as usize).saturating_sub(crate::tags::BAR_WIDTH + 2);
        let (shown, cx) = edit_window(buf.as_str(), buf.width_before_cursor(), budget);
        header_cursor_x = Some(crate::tags::BAR_WIDTH as u16 + 1 + cx);
        head.push(Span::styled(
            shown,
            Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD),
        ));
    } else if is_cursor_col {
        head.push(Span::styled(
            name.to_uppercase(),
            Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD),
        ));
    } else {
        head.push(Span::styled(name.to_uppercase(), theme.dim1().add_modifier(Modifier::BOLD)));
    }
    let mut right: Vec<Span<'static>> = Vec::new();
    if header_edit.is_none() {
        if attn_out > 0 {
            right.push(Span::styled(format!("!{attn_out} "), theme.attn_text()));
        }
        // A tall cursor card may need the cue rows. Keep its hidden counts
        // in the header in that case, without sacrificing the input field.
        if above > 0 && top.is_none() {
            right.push(Span::styled(format!("{}{above} ", overflow_arrow(true)), quiet));
        }
        if below > 0 && bottom.is_none() {
            right.push(Span::styled(format!("{}{below} ", overflow_arrow(false)), quiet));
        }
        right.push(Span::styled(format!("{count}"), quiet));
    }
    let used: usize = head.iter().chain(right.iter()).map(|s| s.content.width()).sum();
    let mut fill = (area.width as usize).saturating_sub(used + 1);
    // The column's one optional mark (T-117): it does something to a ticket
    // — a move on an edge, a claude on creation, the train. After the count,
    // in the quiet register, and the first thing to go when the row is tight.
    let automated =
        header_edit.is_none() && app.board.column(name).is_some_and(|c| c.settings.automated());
    if automated && fill >= 3 {
        right.push(Span::styled(
            format!(" {}", crate::glyphs::auto_mark(theme.glyph_tier())),
            quiet,
        ));
        fill -= 2;
    }
    head.push(Span::raw(" ".repeat(fill)));
    head.extend(right);
    head.push(Span::raw(" "));
    let head_line =
        if is_cursor_col { Line::from(head).style(theme.selected_row()) } else { Line::from(head) };

    let mut out: Vec<Line<'static>> = vec![head_line];
    let mut header_cursor_y: u16 = 0;
    if let Some((buf, queued)) = column_ask {
        // The empty field says what it is for in the key's own words — or,
        // where the column holds a seat a blank Enter would fill, what that
        // Enter does (T-405), the plural of the card's `start on the title`
        // and by the same rule `commit_input` judges it.
        let placeholder =
            if app.column_starts(name) { "start on the titles" } else { "ask every agent" };
        let (line, x_off) = card::render_prompt(&ctx, buf, placeholder);
        out.push(line);
        header_cursor_x = Some(x_off);
        header_cursor_y = 1;
        if column_ask_toggle {
            out.push(card::render_ask_mode(&ctx, crate::app::App::ask_mode_word(false, queued)));
        }
    }
    out.push(Line::default());
    debug_assert_eq!(out.len(), head_rows);

    // ---- body -------------------------------------------------------------
    if lines.is_empty() {
        // Empty column: a legal cursor position (07 §16.2) — the hint shows
        // only under the cursor (author 2026-08-30).
        if is_cursor_col {
            let nudge =
                keymap::hint_for(keymap::Scope::Board, keymap::Verb::OpenTicket, &app.frame_ctx())
                    .map(|(show, hint)| format!("  {show}  {hint}"))
                    .unwrap_or_default();
            out.push(Line::from(Span::styled(nudge, theme.dim3())));
        }
    } else {
        if top.is_some() {
            out.push(overflow_line(above, true));
            out.push(Line::default());
        }
        out.extend(lines[content_start..content_end].iter().cloned());
        if bottom.is_some() {
            // Keep the cue at the viewport edge even when a tall hidden card
            // leaves unused rows after the final complete visible card.
            out.resize(head_rows + body_h.saturating_sub(1), Line::default());
            out.push(overflow_line(below, false));
        }
    }

    f.render_widget(Paragraph::new(out), area);

    // The hardware cursor sits in the edited title (06 §5.7: visible bar in
    // any text input — never a drawn glyph).
    let top_cue_rows = top.map(|_| 2usize).unwrap_or(0);
    if let Some((line, x)) = edit_at {
        if line >= content_start && line < content_end {
            f.set_cursor_position((
                area.x + x.min(area.width.saturating_sub(1)),
                area.y + head_rows as u16 + (top_cue_rows + line - content_start) as u16,
            ));
        }
    }
    // The cursor card, where it landed on screen — the composer's phantom
    // card or the ticket under the cursor: Tab's dialog grows out of this
    // rectangle. Only the whole card counts — a card cut by the window's
    // edge would put the dialog's origin off screen. The card's own
    // rectangle, bar cell included: the dialog's frame is the card's, so at
    // frame zero its bar and title stand in the card's cells.
    if let Some((cs, ce)) = cursor_range {
        let rect = (cs >= content_start && ce <= content_end).then(|| Rect {
            x: area.x,
            y: area.y + head_rows as u16 + (top_cue_rows + cs - content_start) as u16,
            width: area.width,
            height: (ce - cs) as u16,
        });
        app.cursor_card.set(rect);
    } else if at_header {
        // No card is the cursor card on a header: nothing may grow out of
        // last frame's rectangle.
        app.cursor_card.set(None);
    }
    if let Some(x) = header_cursor_x {
        f.set_cursor_position((
            area.x + x.min(area.width.saturating_sub(1)),
            area.y + header_cursor_y,
        ));
    }
}

/// A collapsed column (07 §3.1): 1 cell, reads vertically, codepoints
/// ⊆ [!A-Z0-9⁺ space]. The TOP carries what an expanded column's header
/// row carries — the `!` iff the column holds a waiting session, then the
/// count, ONE digit — and the name runs down from under them, one letter
/// per row, truncating when the column is short.
///
/// The count is one cell, never two (T-359): ten or more reads `9` with a
/// `⁺` in the row under it, the row a smaller count leaves blank, so the
/// name starts on the same row whatever the column holds. Stacking the
/// digits (`1` over `0`) read as two counts and pushed the name down.
///
/// The count sat at the FOOT until T-302 (2026-09-07, user: "bottom too
/// far"), which put a 1-cell column's only number twenty rows away from
/// every other count on the board. With nothing waiting its first digit
/// lands on row 0, the very row the expanded columns write their own count
/// on; a `!` claims that cell and pushes it one row down, because the mark
/// is what the folded column is standing in for.
///
/// The `!` is INVERTED — `theme.attn_row()`, the needs-you title row's own
/// treatment, bold like the header's `!N` chip (T-271, 2026-09-06, user:
/// "more aggressiveness"). One cell of attn-coloured glyph is the smallest
/// mark the board can make and it was reading as quiet next to the folded
/// column it stands for; painting the cell spends the same one colour on
/// the whole cell instead of on a stroke.
fn draw_spine(f: &mut Frame, area: Rect, app: &App, name: &str) {
    let theme = &app.theme;
    let tickets = app.board.column_tickets(name);
    let waiting = tickets.iter().any(|t| {
        let sessions = ticket_sessions(app, t.id);
        card::needs_you(t, &sessions)
    });

    let h = area.height as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    // Row 0 aligns with the column headers.
    if waiting {
        lines.push(Line::from(Span::styled("!", theme.attn_row().add_modifier(Modifier::BOLD))));
    }
    let (digit, more) = spine_count(tickets.len());
    lines.push(Line::from(Span::styled(digit.to_string(), theme.dim2())));
    // The row under the count is blank under headers; `⁺` when it overflows.
    lines.push(Line::from(Span::styled(more.to_string(), theme.dim2())));

    let name_rows = h.saturating_sub(lines.len());
    let upper = name.to_uppercase();
    let letters = upper.chars().filter(|c| c.is_ascii_alphanumeric()).take(name_rows);
    for c in letters {
        lines.push(Line::from(Span::styled(c.to_string(), theme.dim1())));
    }

    f.render_widget(Paragraph::new(lines), area);
}

/// The spine's count block: the digit, and the cell under it — a space, or
/// `⁺` when the column holds more than the one digit can say.
fn spine_count(n: usize) -> (char, char) {
    match n {
        0..=9 => (char::from(b'0' + n as u8), ' '),
        _ => ('9', '⁺'),
    }
}
