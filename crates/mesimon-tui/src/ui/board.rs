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

use crate::app::{App, InputPurpose, Mode};
use crate::layout::{self, Slot};
use crate::text::EditBuffer;

use super::card::{self, CardCtx};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn draw_columns(f: &mut Frame, area: Rect, app: &App) {
    let cols = app.columns();
    if cols.is_empty() {
        return;
    }
    let mut window = app.col_window.get();
    let geom = layout::board_geometry(area.width, cols.len(), app.cursor_col, &mut window);
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

    let ctx = CardCtx { theme, width: area.width, now_ms: now_ms(), spin: app.spin_frame() };

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
    let prompt_of = |t: &Ticket| -> Option<(&EditBuffer, bool)> {
        match editing {
            Some((InputPurpose::Prompt { ticket, queued, .. }, buf)) if *ticket == t.id => {
                Some((buf, *queued))
            }
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
            ranked.iter().find(|s| s.transcript_path.is_some()).and_then(|s| {
                let mut pk = app.peek_cache.peek(s.transcript_path.as_deref()?)?;
                // The activity row is a claim about NOW: a parked, finished
                // or waiting session's last tool call is history.
                if s.state != SessionState::Running {
                    pk.activity = None;
                }
                Some(pk)
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
            wt,
            selected,
            held,
            trail,
            mq,
            open,
            peek.as_ref(),
            &painted,
            app.doomed(t.id),
            unseen,
            app.snooze_row(t.id).as_deref(),
            app.owed(t.id),
            app.pending_row(t.id).as_deref(),
        );
        // The card is drawn WHOLE first — glyph, title, sessions, peek — and
        // the field is added under it. That order is the point: what you are
        // about to talk to stays legible while you type at it.
        let edit_cursor = prompt_of(t).map(|(buf, queued)| {
            let (line, x_off) = card::render_prompt(&ctx, buf, app.ticket_queued(t.id));
            lines.push(line);
            let at = lines.len() - 1;
            // The delivery row, where the ask can wait (2026-09-04): after
            // the field, so the cursor row is unchanged.
            if app.ask_queueable(t.id) {
                lines.push(card::render_ask_mode(&ctx, queued));
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
                    && app.cursor_row == i
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
            let selector = card::render_workspace_selector(&ctx, *workspace);
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

    // Scroll (cursor column only): keep the cursor card visible, scrolloff 2
    // — the ghost-peek row plus its blank must fit past the cursor card, so
    // the view scrolls one card early instead of parking the cursor flush
    // against a peek.
    let body_h = area.height.saturating_sub(2) as usize;
    let total = lines.len();
    let mut scroll =
        if is_cursor_col { app.scroll_row.get().min(total.saturating_sub(1)) } else { 0 };
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
        }
        if total <= body_h {
            scroll = 0;
        }
        app.scroll_row.set(scroll);
    }

    // Edge peeks (author 2026-08-30): when content continues past an edge,
    // that edge's row shows the next not-fully-visible card as a one-line
    // ghost — ghost bar + dim3 ink, the move-trail demotion — instead of
    // cutting a full-value card mid-flight. Cards that fit render whole; the
    // ghost line IS the "more here" affordance, both directions. The cursor
    // card is never the peek (scrolloff keeps it inside the content slice);
    // if it somehow is (a card taller than the window), the peek yields.
    let end_vis = (scroll + body_h).min(total);
    let not_cursor = |r: &&(usize, usize, bool)| Some((r.0, r.1)) != cursor_range;
    // Each peek costs TWO rows — the ghost line plus a blank keeping the
    // card rhythm — so a peek never sits flush against a full-value card.
    // Bottom: the first card that no longer fits once those rows are
    // reserved.
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
    // Content excludes the peeked cards and their separators; the blanks
    // around the ghosts are pushed explicitly at assembly.
    let content_start = top.map(|(_, end, _)| (end + 1).max(scroll + 2)).unwrap_or(scroll);
    let content_end = bottom
        .map(|(start, _, _)| start.saturating_sub(1).min(end_vis.saturating_sub(2)))
        .unwrap_or(end_vis)
        .max(content_start);
    // A peeked card is whole-card dim3 with the ghost bar (bar colour rides
    // the bg of a space cell in colour profiles — fg-only restyling would
    // blank it).
    let fade_line = |idx: usize| -> Line<'static> {
        let mut l = lines[idx].clone();
        l.style = Style::default();
        let (gch, gstyle) = theme.bar(crate::theme::BarWeight::Ghost);
        for (i, span) in l.spans.iter_mut().enumerate() {
            if i == 0 {
                // The bar loses its tag colour with everything else: a lit
                // tint would be the only thing at full value on a ghost.
                span.content = gch.to_string().into();
                span.style = gstyle;
            } else {
                span.style = theme.dim3();
            }
        }
        l
    };

    // Clipped-card badges (D33k's chevron replacement for the rail): counts of
    // cards fully above/below the viewport, and `!n` for scrolled-out waiting.
    // A peeked waiting card hands its needs-you signal to the badge — dim3
    // must not silently swallow attention.
    let mut above = 0usize;
    let mut below = 0usize;
    let mut attn_out = 0usize;
    for (start, end, waiting) in &card_ranges {
        if *end <= scroll {
            above += 1;
            if *waiting {
                attn_out += 1;
            }
        } else if *start >= scroll + body_h {
            below += 1;
            if *waiting {
                attn_out += 1;
            }
        }
    }
    if let Some((_, end, true)) = top {
        if end > scroll {
            attn_out += 1; // partially cut, so not in the `above` count
        }
    }
    if let Some((start, _, true)) = bottom {
        if start < scroll + body_h {
            attn_out += 1; // partially cut, so not in the `below` count
        }
    }

    // ---- header row (cursor-column treatment, reminted 2026-08-30) --------
    // The cursor column's header is a full-width painted band on the
    // `selected` surface — a 1-cell bar there read as another card. The name
    // still steps dim1 -> base. A value and a shape, never a hue.
    let quiet = if is_cursor_col { Style::default().fg(theme.sel.dim2) } else { theme.dim2() };
    let mut head: Vec<Span<'static>> = vec![Span::raw("  ")];
    if is_cursor_col {
        head.push(Span::styled(
            name.to_uppercase(),
            Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD),
        ));
    } else {
        head.push(Span::styled(name.to_uppercase(), theme.dim1().add_modifier(Modifier::BOLD)));
    }
    let mut right: Vec<Span<'static>> = Vec::new();
    if attn_out > 0 {
        right.push(Span::styled(format!("!{attn_out} "), theme.attn_text()));
    }
    if above > 0 {
        right.push(Span::styled(format!("▴{above} "), quiet));
    }
    if below > 0 {
        right.push(Span::styled(format!("▾{below} "), quiet));
    }
    right.push(Span::styled(format!("{count}"), quiet));
    let used: usize = head.iter().chain(right.iter()).map(|s| s.content.width()).sum();
    let fill = (area.width as usize).saturating_sub(used + 1);
    head.push(Span::raw(" ".repeat(fill)));
    head.extend(right);
    head.push(Span::raw(" "));
    let head_line =
        if is_cursor_col { Line::from(head).style(theme.selected_row()) } else { Line::from(head) };

    let mut out: Vec<Line<'static>> = vec![head_line, Line::default()];

    // ---- body -------------------------------------------------------------
    if lines.is_empty() {
        // Empty column: a legal cursor position (07 §16.2) — the hint shows
        // only under the cursor (author 2026-08-30).
        if is_cursor_col {
            let nudge =
                keymap::hint_for(keymap::Scope::Board, keymap::Verb::OpenTicket, &app.ctx())
                    .map(|(show, hint)| format!("  {show}  {hint}"))
                    .unwrap_or_default();
            out.push(Line::from(Span::styled(nudge, theme.dim3())));
        }
    } else {
        if let Some((_, end, _)) = top {
            out.push(fade_line(end - 1)); // the cut card's nearest line
            out.push(Line::default());
        }
        out.extend(lines[content_start..content_end].iter().cloned());
        if let Some((start, _, _)) = bottom {
            out.push(Line::default());
            out.push(fade_line(start));
        }
    }

    f.render_widget(Paragraph::new(out), area);

    // The hardware cursor sits in the edited title (06 §5.7: visible bar in
    // any text input — never a drawn glyph).
    let peek_rows = top.map(|_| 2usize).unwrap_or(0);
    if let Some((line, x)) = edit_at {
        if line >= content_start && line < content_end {
            f.set_cursor_position((
                area.x + x.min(area.width.saturating_sub(1)),
                area.y + 2 + (peek_rows + line - content_start) as u16,
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
            y: area.y + 2 + (peek_rows + cs - content_start) as u16,
            width: area.width,
            height: (ce - cs) as u16,
        });
        app.cursor_card.set(rect);
    }
}

/// A collapsed column (07 §3.1): 1 cell, reads vertically, codepoints
/// ⊆ [!A-Z0-9 space]. Header cell is `!` iff the column holds a waiting
/// session; then the name one letter per row; the count bottom-aligned,
/// never dropped — the name truncates.
fn draw_spine(f: &mut Frame, area: Rect, app: &App, name: &str) {
    let theme = &app.theme;
    let tickets = app.board.column_tickets(name);
    let waiting = tickets.iter().any(|t| {
        let sessions = ticket_sessions(app, t.id);
        card::needs_you(t, &sessions)
    });
    let count = tickets.len().to_string();

    let h = area.height as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    // Row 0 aligns with the column headers.
    lines.push(if waiting {
        Line::from(Span::styled("!", theme.attn_text()))
    } else {
        Line::default()
    });
    lines.push(Line::default()); // aligns with the blank under headers

    let body = h.saturating_sub(2);
    let digits: Vec<char> = count.chars().collect();
    let name_rows = body.saturating_sub(digits.len() + 1);
    let letters: Vec<char> =
        name.to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).take(name_rows).collect();
    for c in &letters {
        lines.push(Line::from(Span::styled(c.to_string(), theme.dim1())));
    }
    let used = lines.len() - 2;
    for _ in used..body.saturating_sub(digits.len()) {
        lines.push(Line::default());
    }
    for d in digits {
        lines.push(Line::from(Span::styled(d.to_string(), theme.dim2())));
    }

    f.render_widget(Paragraph::new(lines), area);
}
