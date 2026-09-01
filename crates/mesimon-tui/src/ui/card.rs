//! One card (07 §4 owns the anatomy; 06 supplies glyphs and tokens).
//!
//! Frame per line: `[bar 1][pad 1][content T][pad 1]`, `T = width - 3`. The
//! bar is both the state ladder and the tag mark: `tags::tint_bar` repaints
//! it in the ticket's colours, and tags cost the card no cell at all.
//! Line 1: `[glyph+sp when stateful][title][fill][age 3]` — a card with no
//! aggregate glyph (spawning/quiet-idle only) starts its title at T[0]. The meta strip (line 2) carries only the session
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
use crate::text::{age_slot, edit_window, marquee_offset, marquee_window, truncate, EditBuffer};
use crate::theme::{BarWeight, Theme};

pub(super) struct CardCtx<'a> {
    pub theme: &'a Theme,
    /// Where the second tag goes (`w` cycles it).
    pub second: crate::tags::Second,
    /// Column width including the accent bar and both pads.
    pub width: u16,
    pub now_ms: u64,
    /// Redraw-clock frame for the working spinner (`App::spin_frame`).
    pub spin: usize,
}

/// Render the in-place title editor as a card line (create + rename share it).
/// Returns the line and the cursor x-offset within the column rect.
pub(super) fn render_edit(
    ctx: &CardCtx,
    buffer: &EditBuffer,
    tags: &[crate::tags::Painted],
) -> (Line<'static>, u16) {
    let theme = ctx.theme;
    let t_cells = (ctx.width as usize).saturating_sub(3);
    let (bar_ch, bar_style) = theme.bar(BarWeight::Cursor);
    // The tags picked with `^t` colour the phantom card's bar exactly as they
    // will colour the real one.
    // The composer's phantom card is the cursor card by construction.
    let (bar_ch, bar_style) = crate::tags::bar_cell(
        theme,
        bar_ch,
        bar_style,
        tags,
        ctx.second,
        crate::theme::TagLevel::Selected,
    );
    // Scroll only as far as keeps the hardware cursor visible.
    let budget = t_cells.saturating_sub(1);
    let (shown, cx) = edit_window(buffer.as_str(), buffer.width_before_cursor(), budget);
    let x_off = 2 + cx;
    let spans = vec![
        Span::styled(bar_ch, bar_style),
        Span::raw(" "),
        Span::styled(shown, Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)),
    ];
    (Line::from(spans).style(theme.selected_row()), x_off)
}

/// The composer's workspace row (M4): git glyph + one word, Shift+Tab cycles.
/// Rendered on the phantom card's selected surface, dim — a setting, not text.
pub(super) fn render_workspace_selector(
    ctx: &CardCtx,
    workspace: Option<mesimon_core::board::WorkspaceStrategy>,
) -> Line<'static> {
    let theme = ctx.theme;
    let word = match workspace {
        Some(mesimon_core::board::WorkspaceStrategy::Worktree) => "worktree",
        Some(mesimon_core::board::WorkspaceStrategy::AdoptExisting) => "adopt",
        Some(mesimon_core::board::WorkspaceStrategy::SharedCheckout) | None => "shared",
    };
    let spans = vec![
        Span::raw("  "),
        Span::styled(format!("⎇ {word}"), Style::default().fg(theme.sel.dim1)),
        Span::styled("  shift+tab", Style::default().fg(theme.sel.dim2)),
    ];
    Line::from(spans).style(theme.selected_row())
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
/// (cursor card only) — it drives the truncated-title reveal. `peek` is the
/// sanitized latest assistant reply (cursor card, peek toggle on): wrapped
/// under the session rows, a deliberate sentence in the accordion — 07 §4.3's
/// "never sentences" is amended for this opt-in toggle (STALE-MAP).
/// The card's right-side worktree mark (M4): branch glyph + one state char.
enum WtTone {
    Quiet,
    /// Conflict/error — something is wrong.
    Err,
    /// Commits waiting — merge available (a suggestion, calm register).
    Ready,
}

fn worktree_mark(
    wt: Option<&mesimon_core::command::WorktreeItem>,
    ascii: bool,
) -> Option<(String, WtTone)> {
    let w = wt?;
    let g = if ascii { '&' } else { '⎇' };
    let (dots, check, up, down) =
        if ascii { ('.', '+', '^', 'v') } else { ('…', '✓', '↑', '↓') };
    Some(match w.status.as_str() {
        "queued" | "provisioning" => (format!("{g}{dots}"), WtTone::Quiet),
        "error" => (format!("{g}x"), WtTone::Err),
        "evicted" => (format!("{g}-"), WtTone::Quiet),
        _ if w.conflict => (format!("{g}!"), WtTone::Err),
        _ if w.merged => (format!("{g}{check}"), WtTone::Quiet),
        // Behind main — the m flow's rebase stage comes before merge.
        _ if w.needs_rebase => (format!("{g}{down}"), WtTone::Ready),
        _ if w.ahead > 0 => (format!("{g}{up}"), WtTone::Ready),
        _ => (g.to_string(), WtTone::Quiet),
    })
}

#[allow(clippy::too_many_arguments)] // one call site; a params struct would just rename the args
pub(super) fn render(
    ctx: &CardCtx,
    ticket: &Ticket,
    sessions: &[&SessionRecord],
    wt: Option<&mesimon_core::command::WorktreeItem>,
    selected: bool,
    held: bool,
    trail: bool,
    marquee_ms: Option<u64>,
    peek: Option<&crate::peek::Peek>,
    tags: &[crate::tags::Painted],
) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    let t_cells = (ctx.width as usize).saturating_sub(3);
    let tier = theme.glyph_tier();
    let glyph = glyphs::card_glyph(sessions, tier, ctx.spin);
    // A pending move's trail is semi-transparent everything — even an attn
    // card demotes while its ghost is in hand (the ghost carries the weight).
    let attn_card = !trail && matches!(glyph, Some((_, Register::Attn)));
    let cursorish = selected || held;

    // Age: newest state change across the ticket's sessions; suppressed on a
    // session-less card (07 §4.4 — created_at staleness handling is deferred).
    // Seconds tick only while that session is working — a shell's `Running`
    // is not work (`glyphs::is_working`), so its age counts in minutes like
    // any other settled row.
    let age = sessions
        .iter()
        .filter_map(|s| s.state_changed_at.map(|ms| (ms, glyphs::is_working(s))))
        .max_by_key(|(ms, _)| *ms)
        .map(|(ms, running)| age_slot(ctx.now_ms, ms, running));

    // Accent bar weight (06 §2.4a). An alarm card never demotes to
    // dormant/ghost — it holds its state hue in every de-emphasis context
    // (except as a move trail, which is the one whole-card demotion).
    let bar = if trail {
        BarWeight::Ghost
    } else if cursorish {
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
    // The bar is the TAG channel, and nothing else (author 2026-09-01): state
    // is the glyph's job, and needs-you also has the inverted title row, so a
    // second colour ladder on the bar was saying it twice. Untagged means
    // neutral. The ASCII tiers keep their `: | #` ladder, which is a shape
    // and not a colour, and a move trail still goes ghost with the card.
    let (ladder_ch, state_style) = theme.bar(bar);
    // Three loudnesses, and the card's own state picks one: the cursor card
    // wears its tags at full strength, a sleeping ticket at a level that
    // still answers "which tag", everything else one small step down from
    // the cursor — which is where nearly every tag on the board is read.
    // Sleeping is read off the SESSIONS, not off the aggregate glyph: a
    // ticket whose parked session sits behind any other glyph still has
    // nothing running, and the glyph check missed exactly those (dogfood
    // 2026-09-01, "sleeping vs not sleeping looks the same").
    let parked = !sessions.is_empty()
        && sessions.iter().any(|s| s.state == SessionState::Sleeping)
        && !sessions.iter().any(|s| s.state.has_pane());
    let level = if cursorish {
        crate::theme::TagLevel::Selected
    } else if parked {
        crate::theme::TagLevel::Sleeping
    } else {
        crate::theme::TagLevel::Rest
    };
    let (bar_ch, bar_style) = if trail {
        (ladder_ch.to_string(), state_style)
    } else {
        crate::tags::bar_cell(
            theme,
            ladder_ch,
            theme.bar(BarWeight::Dormant).1,
            tags,
            ctx.second,
            level,
        )
    };
    // `Second::Edge` puts the second tag on the card's last cell, which was
    // trailing pad — so it costs no width and never touches the bar.
    let edge = if trail { None } else { crate::tags::edge_cell(theme, tags, ctx.second, level) };

    // ---- line 1: [glyph sp?][title][fill][wt][age] ------------------------
    let wt_mark = worktree_mark(wt, tier == crate::glyphs::Tier::Ascii);
    let glyph_cells = if glyph.is_some() { 2 } else { 0 };
    let age_cells = age.as_ref().map(|_| 4).unwrap_or(0); // sp + 3-cell slot
    let wt_cells = wt_mark.as_ref().map(|(m, _)| m.width() + 1).unwrap_or(0);
    // Tags cost the title NOTHING: they are bands under the block, not a zone
    // on this line. That is the point of moving them off it.
    let title_budget = t_cells.saturating_sub(glyph_cells + age_cells + wt_cells);
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

    let title_style = if trail {
        // The original spot of a pending move: semi-transparent.
        theme.dim3()
    } else if attn_card {
        Style::default() // inherits attn_ink from the row
    } else if held {
        // The MOVE ghost blinks in place until dropped or cancelled — the
        // grabbed card must read as "in hand" (STALE-MAP 2026-08-30).
        theme.move_blink(ctx.spin)
    } else if cursorish {
        Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.rest.base)
    };
    let quiet_style = if trail {
        theme.dim3()
    } else if attn_card {
        Style::default()
    } else if cursorish {
        Style::default().fg(theme.sel.dim2)
    } else {
        theme.dim2()
    };

    let mut spans: Vec<Span<'static>> = vec![Span::styled(bar_ch.clone(), bar_style)];
    spans.push(Span::styled(" ".to_string(), Style::default()));
    if let Some((g, reg)) = glyph {
        let gs = if trail {
            theme.dim3()
        } else if attn_card {
            Style::default()
        } else {
            register_style(theme, reg)
        };
        spans.push(Span::styled(format!("{g} "), gs));
    }
    spans.push(Span::styled(title, title_style));
    spans.push(Span::raw(" ".repeat(fill)));
    if let Some((m, tone)) = &wt_mark {
        // Trail/attn contexts demote the mark to the quiet tone with the row.
        let style = match tone {
            _ if attn_card || trail => quiet_style,
            WtTone::Err => theme.err_text(),
            WtTone::Ready => register_style(theme, Register::Calm),
            WtTone::Quiet => quiet_style,
        };
        spans.push(Span::styled(format!(" {m}"), style));
    }
    if let Some(a) = &age {
        spans.push(Span::styled(format!(" {a:>3}"), quiet_style));
    }
    spans.push(Span::styled(" ".to_string(), edge.unwrap_or_default()));
    let mut lines = vec![Line::from(spans).style(row_style)];

    if held {
        return lines;
    }

    // ---- accordion (07 §4.3; session rows only — short keys are hidden
    // from the UI for now, author 2026-08-30) -------------------------------
    //
    if selected && !sessions.is_empty() {
        let acc_style = theme.selected_row();
        let dim = Style::default().fg(theme.sel.dim1);
        let quiet = Style::default().fg(theme.sel.dim2);
        let mut push = |spans: Vec<Span<'static>>| {
            let mut all = vec![
                Span::styled(bar_ch.clone(), bar_style),
                Span::styled(" ".to_string(), Style::default()),
            ];
            all.extend(spans);
            // Pad the interior so the surface paints the full card width,
            // keeping the last cell for the edge tag when it wants one.
            let used: usize = all.iter().map(|s| s.content.width()).sum();
            let pad = (ctx.width as usize).saturating_sub(used);
            match edge {
                Some(style) if pad > 0 => {
                    all.push(Span::raw(" ".repeat(pad - 1)));
                    all.push(Span::styled(" ".to_string(), style));
                }
                _ => all.push(Span::raw(" ".repeat(pad))),
            }
            lines.push(Line::from(all).style(acc_style));
        };

        // The peek names the tags. The mark under the card says how many and
        // in which colours; only words say WHICH, and a card open far enough
        // to show a sentence can afford the row. It sits directly under the
        // title, above the reply — the ticket's own metadata before the
        // agent's.
        if peek.is_some() && !tags.is_empty() {
            let mut row = vec![Span::raw("  ".to_string())];
            row.extend(crate::tags::chips(theme, tags, t_cells.saturating_sub(2)));
            push(row);
        }

        let mut ranked: Vec<&&SessionRecord> = sessions.iter().collect();
        ranked.sort_by_key(|s| (rank(&s.state), s.id));
        // A single session duplicates line 1 (aggregate glyph + age ARE that
        // session) — its row adds nothing, so only multi-session cards list.
        let listed: &[&&SessionRecord] = if ranked.len() > 1 { &ranked } else { &[] };
        for s in listed.iter().take(2) {
            let mark = glyphs::kind_mark(s.kind, tier);
            // The session's own name (OSC-0 title, same as the tmux status
            // bar's breadcrumb leaf) when it set one, else the kind word.
            let word = s.title.as_deref().unwrap_or(match s.kind {
                SessionKind::Claude => "claude",
                SessionKind::Bash => "bash",
            });
            // Row budget: "  {mark} {word}" + ≥1 fill + glyph + " {age:>3}".
            // An overflowing name reveals itself marquee-style on the same
            // clock as the card title (accordion rows exist on the cursor
            // card only, so the clock is always live here).
            let budget = t_cells.saturating_sub(10);
            let overflow = word.width().saturating_sub(budget);
            let scroll = match (marquee_ms, overflow) {
                (Some(ms), o) if o > 0 => marquee_offset(ms, o),
                _ => 0,
            };
            let word = if scroll > 0 {
                marquee_window(word, budget, scroll)
            } else {
                truncate(word, budget)
            };
            let (g, reg) = glyphs::session_glyph(s, tier, ctx.spin);
            let a = s
                .state_changed_at
                .map(|ms| age_slot(ctx.now_ms, ms, glyphs::is_working(s)))
                .unwrap_or_default();
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
        // Peek rows: the latest assistant reply, wrapped inside the card's
        // interior. Capped at 4 rows — the transcript itself is one focus away.
        if let Some(pk) = peek {
            if let Some(text) = pk.text.as_deref() {
                for row in crate::peek::wrap(text, t_cells.saturating_sub(2), 4) {
                    push(vec![Span::styled(format!("  {row}"), dim)]);
                }
            }
            // One more row for the step running under those words — the
            // spinner marks it as now, and quiet keeps the reply the loudest
            // thing in the accordion.
            if let Some(act) = pk.activity.as_ref() {
                let word = match act {
                    crate::peek::Doing::Tool(t) => t.as_str(),
                    crate::peek::Doing::Thinking => "thinking",
                };
                let row = crate::text::truncate(word, t_cells.saturating_sub(4));
                let mark = if glyphs::pulse_lit(ctx.spin) {
                    quiet
                } else {
                    Style::default().fg(theme.sel.dim3)
                };
                push(vec![
                    Span::styled(format!("  {} ", glyphs::pulse(tier)), mark),
                    Span::styled(row, quiet),
                ]);
            }
        }
        return lines;
    }
    if selected {
        // Session-less card: nothing to expand, nothing shifts.
        return lines;
    }

    // No meta strip at rest (author 2026-08-30): the session dots repeated
    // the aggregate glyph — a resting card is one line, plus its stripe.
    // Per-session detail lives in the accordion and the ticket rail.
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
