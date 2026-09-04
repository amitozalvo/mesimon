//! Framed dialogs — the one place structure is DRAWN (author 2026-09-03,
//! T-158). A floating surface over the board or a page gets a frame with its
//! title set into the top edge and its keys set into the bottom edge, so a
//! dialog names itself and teaches itself in the same cells, and the footer
//! under it has nothing to repeat. Every frame is recorded on `App::frames`
//! for the frame it was drawn in: `test_no_drawn_structure` admits a box
//! glyph on a recorded perimeter and nowhere else, which is what keeps L1
//! ("no drawn structure") a law with one allowlisted role rather than a
//! preference. The board, its cards and the ticket page stay painted.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::{self, Scope};

use crate::app::App;
use crate::text::truncate;
use crate::theme::Ramp;

use super::chrome;

/// The one width every centred dialog settles at, terminal permitting —
/// the menu, the theme picker, the archived list and the drawer were 54, 62
/// and 64 wide before, for no reason anyone could name.
pub(super) const MAX_W: u16 = 64;

/// A centred rectangle holding `rows` inner rows inside a frame, at most
/// `max_w` inner cells wide (plus the frame's two), never wider than the
/// screen less a two-cell margin a side.
pub(super) fn centred(screen: Rect, rows: u16, max_w: u16) -> Rect {
    let w = (max_w + 2).min(screen.width.saturating_sub(4)).max(4);
    let h = (rows + 2).min(screen.height.saturating_sub(2)).max(3);
    Rect {
        x: screen.x + screen.width.saturating_sub(w) / 2,
        y: screen.y + screen.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// What the frame carries: the title in its top edge, the keys in its
/// bottom edge. Either may be empty; the edge is then a plain rule.
pub(super) struct Edges {
    pub title: Vec<Span<'static>>,
    pub tail: Vec<Span<'static>>,
}

/// The dialog's name, in the frame's own register: `dim1` + bold, the
/// weight every section heading carries.
pub(super) fn title(ink: &Ramp, text: impl Into<String>) -> Vec<Span<'static>> {
    vec![Span::styled(text.into(), Style::default().fg(ink.dim1).add_modifier(Modifier::BOLD))]
}

/// The dialog's own keys for its bottom edge — the left cluster of its
/// scope's footer (the app-level keys stay in the footer under it, which is
/// where `? keys` lives).
pub(super) fn keys(app: &App, scope: Scope, ink: &Ramp, budget: usize) -> Vec<Span<'static>> {
    let ctx = app.ctx();
    let (own, _) = keymap::footer_split(scope, &ctx);
    chrome::hint_spans(&own, &ctx, ink, budget)
}

/// Clears `area`, paints it `surface` (the page ground when `None`), draws
/// the frame in `ink.dim3` with `edges` set into it, records the frame, and
/// returns the inner rectangle. Below three rows or four columns there is
/// no frame to draw and the whole area is returned as it is.
pub(super) fn frame(
    f: &mut Frame,
    app: &App,
    area: Rect,
    surface: Option<Color>,
    ink: &Ramp,
    edges: Edges,
) -> Rect {
    let theme = &app.theme;
    let ground = surface.or(theme.bg);
    f.render_widget(Clear, area);
    if let Some(bg) = ground {
        f.render_widget(Block::default().style(Style::default().bg(bg)), area);
    }
    if area.width < 4 || area.height < 3 {
        return area;
    }
    let g = crate::glyphs::frame_set(theme.glyph_tier());
    let rule = Style::default().fg(ink.dim3);
    let w = area.width as usize;
    // Corners, one rule cell, a space each side of the words, one rule cell.
    let budget = w.saturating_sub(6);

    let mut top: Vec<Span<'static>> = vec![Span::styled(g.tl.to_string(), rule)];
    let words = fit(edges.title, budget);
    if words.is_empty() {
        top.push(Span::styled(g.h.to_string().repeat(w - 2), rule));
    } else {
        let used: usize = words.iter().map(|s| s.content.width()).sum();
        top.push(Span::styled(format!("{} ", g.h), rule));
        top.extend(words);
        top.push(Span::styled(format!(" {}", g.h.to_string().repeat(w - 5 - used)), rule));
    }
    top.push(Span::styled(g.tr.to_string(), rule));

    let mut bottom: Vec<Span<'static>> = vec![Span::styled(g.bl.to_string(), rule)];
    let words = fit(edges.tail, budget);
    if words.is_empty() {
        bottom.push(Span::styled(g.h.to_string().repeat(w - 2), rule));
    } else {
        let used: usize = words.iter().map(|s| s.content.width()).sum();
        bottom.push(Span::styled(format!("{} ", g.h.to_string().repeat(w - 5 - used)), rule));
        bottom.extend(words);
        bottom.push(Span::styled(format!(" {}", g.h), rule));
    }
    bottom.push(Span::styled(g.br.to_string(), rule));

    f.render_widget(
        Paragraph::new(Line::from(top)),
        Rect { x: area.x, y: area.y, width: area.width, height: 1 },
    );
    f.render_widget(
        Paragraph::new(Line::from(bottom)),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
    let side: Vec<Line<'static>> =
        (0..area.height - 2).map(|_| Line::from(Span::styled(g.v.to_string(), rule))).collect();
    f.render_widget(
        Paragraph::new(side.clone()),
        Rect { x: area.x, y: area.y + 1, width: 1, height: area.height - 2 },
    );
    f.render_widget(
        Paragraph::new(side),
        Rect { x: area.x + area.width - 1, y: area.y + 1, width: 1, height: area.height - 2 },
    );
    app.frames.borrow_mut().push(area);
    Rect { x: area.x + 1, y: area.y + 1, width: area.width - 2, height: area.height - 2 }
}

/// The spans that fit in `budget` cells, in order; the first that does not
/// is cut to the room left and ends the run.
fn fit(spans: Vec<Span<'static>>, budget: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for s in spans {
        let w = s.content.width();
        if used + w <= budget {
            used += w;
            out.push(s);
            continue;
        }
        let room = budget.saturating_sub(used);
        if room > 0 {
            out.push(Span::styled(truncate(&s.content, room), s.style));
        }
        break;
    }
    out
}

/// The archived-tickets dialog: restore or open.
pub(super) fn draw_archived(f: &mut Frame, app: &App, idx: usize) {
    let theme = &app.theme;
    let archived = app.board.archived_tickets();
    if archived.is_empty() {
        return;
    }
    let idx = idx.min(archived.len() - 1);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let area = centred(f.area(), archived.len() as u16, MAX_W);
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner = frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        Edges {
            title: title(&theme.rest, format!("ARCHIVED ∙ {}", archived.len())),
            tail: keys(app, Scope::Archived, &theme.rest, inner_w.saturating_sub(4)),
        },
    );
    let mut lines: Vec<Line> = Vec::new();
    for (i, t) in archived.iter().enumerate() {
        // A snoozed ticket says when it comes back; a plain archive says how
        // long it has been gone. Unparsable stamps show nothing.
        let age = match t.snooze_until_secs() {
            Some(until) => format!("wakes {}", crate::text::until_word(now, until * 1000)),
            None => t
                .archived
                .as_ref()
                .and_then(|a| a.at.strip_prefix('@'))
                .and_then(|s| s.parse::<u64>().ok())
                .map(|secs| crate::text::age_slot(now, secs * 1000, false))
                .unwrap_or_default(),
        };
        let head = format!(" {}  {} ∙ {} ∙ {}", t.short_key, truncate(&t.title, 28), t.column, age);
        let pad = inner_w.saturating_sub(head.width());
        let style = if i == idx {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.base()
        };
        let row_style = if i == idx { theme.selected_row() } else { Style::default() };
        lines.push(
            Line::from(vec![Span::styled(head, style), Span::raw(" ".repeat(pad))])
                .style(row_style),
        );
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// The External drawer (19 §4): discovered foreign sessions, observe/resume.
pub(super) fn draw_drawer(f: &mut Frame, app: &App, idx: usize) {
    let theme = &app.theme;
    if app.external.is_empty() {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let area = centred(f.area(), app.external.len() as u16 * 2, MAX_W);
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner = frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        Edges {
            title: title(&theme.rest, format!("EXTERNAL ∙ {}", app.external.len())),
            tail: keys(app, Scope::Drawer, &theme.rest, inner_w.saturating_sub(4)),
        },
    );
    let mut lines: Vec<Line> = Vec::new();
    for (i, item) in app.external.iter().enumerate() {
        let name = item
            .name
            .clone()
            .unwrap_or_else(|| item.claude_session_id.to_string()[..8].to_string());
        let mut badges = String::new();
        if item.running_elsewhere {
            badges.push_str("  ∙ running elsewhere");
        }
        let head = format!(
            " {}  {}{badges}",
            truncate(&name, 24),
            crate::text::age_slot(now, item.mtime_ms, false)
        );
        let selected = i == idx;
        let style = if selected {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.base()
        };
        let row_style = if selected { theme.selected_row() } else { Style::default() };
        let pad = inner_w.saturating_sub(head.width());
        lines.push(
            Line::from(vec![Span::styled(head, style), Span::raw(" ".repeat(pad))])
                .style(row_style),
        );
        let preview = item.preview.as_deref().unwrap_or("");
        let text = format!("     {}", truncate(preview, inner_w.saturating_sub(6)));
        let pad = inner_w.saturating_sub(text.width());
        lines.push(
            Line::from(vec![Span::styled(text, theme.dim2()), Span::raw(" ".repeat(pad))])
                .style(row_style),
        );
    }
    f.render_widget(Paragraph::new(lines), inner);
}
