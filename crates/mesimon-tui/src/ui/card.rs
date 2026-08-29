//! One card (07 §4 owns the anatomy; 06 supplies glyphs and tokens).
//!
//! Frame per line: `[bar 1][pad 1][content T][pad 1]`, `T = width - 3`.
//! Line 1: `[glyph+sp when abnormal][title][fill][age 3]` — a normal card's
//! title starts at T[0]. The meta strip (line 2) carries only the session
//! dots in M3.5: the tag and stage zones collapse to zero width (no tags
//! field yet; stages are tags per D33g), and a session-less card is a single
//! line with no age (07 §4.4). The cursor card expands in place — accordion,
//! never an overlay (07 §4.3).

use mesimon_core::attention::rank;
use mesimon_core::board::{SessionKind, SessionRecord, SessionState, Ticket};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::glyphs::{self, Register};
use crate::text::{age_slot, marquee_offset, marquee_window, truncate};
use crate::theme::{BarWeight, Theme};

pub(super) struct CardCtx<'a> {
    pub theme: &'a Theme,
    /// Column width including the accent bar and both pads.
    pub width: u16,
    pub now_ms: u64,
}

/// Render the in-place title editor as a card line (create + rename share it).
/// Returns the line and the cursor x-offset within the column rect.
pub(super) fn render_edit(ctx: &CardCtx, buffer: &str) -> (Line<'static>, u16) {
    let theme = ctx.theme;
    let t_cells = (ctx.width as usize).saturating_sub(3);
    let (bar_ch, bar_style) = theme.bar(BarWeight::Cursor);
    // Show the tail while typing: the hardware cursor must stay visible.
    let budget = t_cells.saturating_sub(1);
    let bw = buffer.width();
    let shown = if bw > budget {
        marquee_window(buffer, budget, bw - budget)
    } else {
        buffer.to_string()
    };
    let x_off = 2 + shown.width() as u16;
    let spans = vec![
        Span::styled(bar_ch.to_string(), bar_style),
        Span::raw(" "),
        Span::styled(
            shown,
            Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD),
        ),
    ];
    (Line::from(spans).style(theme.selected_row()), x_off)
}

fn register_style(theme: &Theme, reg: Register) -> Style {
    match reg {
        Register::Attn => theme.attn_text(),
        Register::Err => theme.err_text(),
        Register::Calm => theme.calm_text(),
        Register::Grey => theme.dim2(),
    }
}

/// Render one card. `selected` is the cursor card (accordion), `held` is the
/// MOVE ghost — the held card carries `bar.cursor` in the target column
/// (06 §10.13). `marquee_ms` is ms since the cursor landed on this card
/// (cursor card only) — it drives the truncated-title reveal.
pub(super) fn render(
    ctx: &CardCtx,
    ticket: &Ticket,
    sessions: &[&SessionRecord],
    selected: bool,
    held: bool,
    marquee_ms: Option<u64>,
) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    let t_cells = (ctx.width as usize).saturating_sub(3);
    let tier = theme.glyph_tier();
    let glyph = glyphs::card_glyph(sessions, tier);
    let attn_card = matches!(glyph, Some((_, Register::Attn)));
    let cursorish = selected || held;

    // Age: newest state change across the ticket's sessions; suppressed on a
    // session-less card (07 §4.4 — created_at staleness handling is deferred).
    let age = sessions.iter().filter_map(|s| s.state_changed_at).max().map(|ms| age_slot(ctx.now_ms, ms));

    // Accent bar weight (06 §2.4a). An alarm card never demotes to
    // dormant/ghost — it holds its state hue in every de-emphasis context.
    let bar = if cursorish {
        BarWeight::Cursor
    } else {
        match glyph {
            Some((_, Register::Attn)) => BarWeight::Live(Register::Attn),
            Some((_, Register::Err)) => BarWeight::Live(Register::Err),
            Some((_, Register::Calm)) => BarWeight::Live(Register::Calm),
            Some(('z', _)) => BarWeight::Dormant,
            _ if sessions.iter().any(|s| s.state.has_pane()) => BarWeight::Live(Register::Grey),
            _ => BarWeight::Dormant,
        }
    };
    let (bar_ch, bar_style) = theme.bar(bar);

    // ---- line 1: [glyph sp?][title][fill][age] ----------------------------
    let glyph_cells = if glyph.is_some() { 2 } else { 0 };
    let age_cells = age.as_ref().map(|_| 4).unwrap_or(0); // sp + 3-cell slot
    let title_budget = t_cells.saturating_sub(glyph_cells + age_cells);
    // A truncated title on the cursor card reveals itself marquee-style.
    let overflow = ticket.title.width().saturating_sub(title_budget);
    let scroll = match (marquee_ms, overflow) {
        (Some(ms), o) if o > 0 => marquee_offset(ms, o),
        _ => 0,
    };
    let title = if scroll > 0 {
        marquee_window(&ticket.title, title_budget, scroll)
    } else {
        truncate(&ticket.title, title_budget)
    };
    let fill = title_budget.saturating_sub(title.width());

    // Row surface: the inverted needs-you title row beats the cursor surface
    // (06 §2.4b — one row only, `err` never gets a band).
    let row_style = if attn_card {
        theme.attn_row()
    } else if cursorish {
        theme.selected_row()
    } else {
        Style::default()
    };

    let title_style = if attn_card {
        Style::default() // inherits attn_ink from the row
    } else if cursorish {
        Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.rest.base)
    };
    let quiet_style = if attn_card {
        Style::default()
    } else if cursorish {
        Style::default().fg(theme.sel.dim2)
    } else {
        theme.dim2()
    };

    let mut spans: Vec<Span<'static>> = vec![Span::styled(bar_ch.to_string(), bar_style)];
    spans.push(Span::styled(" ".to_string(), Style::default()));
    if let Some((g, reg)) = glyph {
        let gs = if attn_card { Style::default() } else { register_style(theme, reg) };
        spans.push(Span::styled(format!("{g} "), gs));
    }
    spans.push(Span::styled(title, title_style));
    spans.push(Span::raw(" ".repeat(fill)));
    if let Some(a) = &age {
        spans.push(Span::styled(format!(" {a:>3}"), quiet_style));
    }
    spans.push(Span::raw(" "));
    let mut lines = vec![Line::from(spans).style(row_style)];

    if held {
        return lines;
    }

    // ---- accordion (07 §4.3; session rows only — short keys are hidden
    // from the UI for now, author 2026-08-30) -------------------------------
    if selected && !sessions.is_empty() {
        let acc_style = theme.selected_row();
        let dim = Style::default().fg(theme.sel.dim1);
        let quiet = Style::default().fg(theme.sel.dim2);
        let mut push = |spans: Vec<Span<'static>>| {
            let mut all = vec![
                Span::styled(bar_ch.to_string(), bar_style),
                Span::styled(" ".to_string(), Style::default()),
            ];
            all.extend(spans);
            // Pad the interior so the surface paints the full card width.
            let used: usize = all.iter().map(|s| s.content.width()).sum();
            let pad = (ctx.width as usize).saturating_sub(used);
            all.push(Span::raw(" ".repeat(pad)));
            lines.push(Line::from(all).style(acc_style));
        };
        let mut ranked: Vec<&&SessionRecord> = sessions.iter().collect();
        ranked.sort_by_key(|s| (rank(&s.state), s.id));
        for s in ranked.iter().take(2) {
            let mark = glyphs::kind_mark(s.kind, tier);
            let word = match s.kind {
                SessionKind::Claude => "claude",
                SessionKind::Bash => "bash",
            };
            let (g, reg) = glyphs::session_glyph(&s.state, tier);
            let a = s.state_changed_at.map(|ms| age_slot(ctx.now_ms, ms)).unwrap_or_default();
            // Right-align glyph + age into the same columns the resting
            // card uses (dots under the age slot) — selection must not make
            // the state glyph or the time jump sideways.
            let left = format!("  {mark} {word}");
            let fill = t_cells.saturating_sub(left.width() + 1 + 4);
            push(vec![
                Span::styled(left, dim),
                Span::raw(" ".repeat(fill)),
                Span::styled(g.to_string(), register_style(theme, reg)),
                Span::styled(format!(" {a:>3}"), quiet),
            ]);
        }
        if ranked.len() > 2 {
            push(vec![Span::styled(format!("  +{} more", ranked.len() - 2), quiet)]);
        }
        return lines;
    }
    if selected {
        return lines; // session-less card: nothing to expand, nothing shifts
    }

    // No meta strip at rest (author 2026-08-30): the session dots repeated
    // the aggregate glyph — a resting card is always ONE line. Per-session
    // detail lives in the accordion and the ticket rail.
    lines
}

/// Does this ticket currently hold a usable-confidence attention session?
pub(super) fn is_waiting(sessions: &[&SessionRecord]) -> bool {
    use mesimon_core::board::Confidence;
    sessions.iter().any(|s| {
        mesimon_core::attention::is_attention(&s.state)
            && matches!(s.confidence, Confidence::High | Confidence::Medium)
            && !matches!(s.state, SessionState::Sleeping)
    })
}
