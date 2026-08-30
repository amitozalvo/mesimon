//! The board frame: column geometry from `layout::board_geometry` (07 §2,
//! post-D33k arithmetic), expanded columns, 1-cell spines, the minted
//! cursor-column treatment, and vertical scroll with the D33k chevron badges.

use mesimon_core::board::{SessionRecord, Ticket};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, InputPurpose, Mode};
use crate::text::EditBuffer;
use crate::layout::{self, Slot};

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

    // Build card line-groups in display order.
    struct Group {
        lines: Vec<Line<'static>>,
        /// The cursor card (any height): scroll keeps it visible.
        cursor: bool,
        waiting: bool,
        /// Line offset (within the group) and x-offset of a live edit cursor.
        edit_cursor: Option<u16>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut push_card = |t: &Ticket, selected: bool, held: bool| {
        let sessions = ticket_sessions(app, t.id);
        let waiting = card::is_waiting(&sessions);
        if let Some(buf) = rename_of(t) {
            let (line, x_off) = card::render_edit(&ctx, buf);
            groups.push(Group { lines: vec![line], cursor: true, waiting, edit_cursor: Some(x_off) });
            return;
        }
        let trail = !held && moving == Some(t.id);
        let mq = if selected { Some(marquee_ms(t)) } else { None };
        // Transcript peek: the cursor card's highest-precedence session that
        // has a transcript (bash never does) — read through the draw cache.
        let peek = if selected && app.peek {
            let mut ranked: Vec<&&SessionRecord> = sessions.iter().collect();
            ranked.sort_by_key(|s| (mesimon_core::attention::rank(&s.state), s.id));
            ranked
                .iter()
                .find_map(|s| s.transcript_path.as_deref())
                .and_then(|p| app.peek_cache.text(p))
        } else {
            None
        };
        let wt = app.wt_item(t.id);
        let lines = card::render(&ctx, t, &sessions, wt, selected, held, trail, mq, peek.as_deref());
        groups.push(Group { lines, cursor: selected || held, waiting, edit_cursor: None });
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
                let selected = is_cursor_col && app.cursor_row == i && matches!(app.mode, Mode::Normal);
                push_card(t, selected, false);
            }
        }
    }
    // New-ticket entry: a phantom card at the column tail, edited in place.
    // The second line is the M4 workspace selector (Shift+Tab cycles it).
    if is_cursor_col {
        if let Some((InputPurpose::Create { workspace }, buf)) = editing {
            let (line, x_off) = card::render_edit(&ctx, buf);
            let selector = card::render_workspace_selector(&ctx, *workspace);
            groups.push(Group {
                lines: vec![line, selector],
                cursor: true,
                waiting: false,
                edit_cursor: Some(x_off),
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
        if let Some(x) = g.edit_cursor {
            edit_at = Some((start, x));
        }
        card_ranges.push((start, end, g.waiting));
    }
    lines.pop(); // no trailing blank after the last card

    // Scroll (cursor column only): keep the cursor card visible, scrolloff 1.
    let body_h = area.height.saturating_sub(2) as usize;
    let total = lines.len();
    let mut scroll = if is_cursor_col { app.scroll_row.get().min(total.saturating_sub(1)) } else { 0 };
    if is_cursor_col {
        if let Some((cs, ce)) = cursor_range {
            let lo = cs.saturating_sub(1);
            let hi = (ce + 1).min(total);
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

    // Clipped-card badges (D33k's chevron replacement for the rail): counts of
    // cards fully above/below the viewport, and `!n` for scrolled-out waiting.
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
        head.push(Span::styled(
            name.to_uppercase(),
            theme.dim1().add_modifier(Modifier::BOLD),
        ));
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
    let head_line = if is_cursor_col {
        Line::from(head).style(theme.selected_row())
    } else {
        Line::from(head)
    };

    let mut out: Vec<Line<'static>> = vec![head_line, Line::default()];

    // ---- body -------------------------------------------------------------
    if lines.is_empty() {
        // Empty column: a legal cursor position (07 §16.2) — the hint shows
        // only under the cursor (author 2026-08-30).
        if is_cursor_col {
            out.push(Line::from(Span::styled("  a  add here", theme.dim3())));
        }
    } else {
        let end = (scroll + body_h).min(total);
        out.extend(lines[scroll..end].iter().cloned());
    }

    f.render_widget(Paragraph::new(out), area);

    // The hardware cursor sits in the edited title (06 §5.7: visible bar in
    // any text input — never a drawn glyph).
    if let Some((line, x)) = edit_at {
        if line >= scroll && line < scroll + body_h {
            f.set_cursor_position((
                area.x + x.min(area.width.saturating_sub(1)),
                area.y + 2 + (line - scroll) as u16,
            ));
        }
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
        card::is_waiting(&sessions)
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
    let letters: Vec<char> = name
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(name_rows)
        .collect();
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
