//! One card (07 §4 owns the anatomy; 06 supplies glyphs and tokens).
//!
//! Frame per line: `[bar 1][pad 1][content T][pad 1]`, `T = width - 3`. The
//! bar is both the state ladder and the tag mark: `tags::bar_cell` repaints
//! it in the ticket's colours and `tags::stack_full` runs them down an open
//! card's stripe, and tags cost the card no cell at all.
//! Line 1: `[glyph+sp when stateful][title][fill][age 3]` — a card with no
//! aggregate glyph (quiet-idle only) starts its title at T[0]; spawning left
//! that set when Shift+Enter made the launch window something a user watches. The meta strip (line 2) carries only the session
//! dots in M3.5: the tag and stage zones collapse to zero width (no tags
//! field yet; stages are tags per D33g). The age is the ticket's time in its
//! column (`Ticket::column_since`), so every card carries one, session or
//! not — 07 §4.4's session-less "no age" is superseded. The cursor card
//! expands in place — accordion, never an overlay (07 §4.3).

use mesimon_core::attention::rank;
use mesimon_core::board::{SessionKind, SessionRecord, SessionState, Ticket};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::glyphs::{self, ahead_mark, behind_mark, branch_mark, Register, Tier};
use crate::text::{
    age_slot, created_at_epoch_ms, edit_window, marquee_offset, marquee_window, truncate,
    EditBuffer,
};
use crate::theme::{BarWeight, Theme};

pub(super) struct CardCtx<'a> {
    pub theme: &'a Theme,
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
    let (bar_ch, bar_style) =
        crate::tags::bar_cell(theme, bar_ch, bar_style, tags, crate::theme::TagLevel::Selected);
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

/// The board's prompt row (Shift+Enter): a line hanging under the selected
/// card whose text is bound for that ticket's live agent, not for the board.
///
/// It hangs off the CARD rather than sitting in a command line at the foot of
/// the screen, and that is the whole reason it is legible: a prompt has a
/// destination, and the only thing on this screen that can name the
/// destination is the card. The card is also where the agent's answer comes
/// back — the peek row is two lines up — so the question and the reply share
/// a place. The cost is width: ~26 cells of a sentence are visible and the
/// rest scrolls, which is the trade a board makes for never leaving the board.
///
/// No bar on span 0. `render_workspace_selector` set that shape first: a row
/// hanging under a card belongs to it and must not read as a second card.
/// It also keeps the row clear of `tags::stack_full`, which paints the stripe
/// of the lines above and knows nothing about this one.
///
/// `reopened` is a field opened on a WAITING ask (T-241): emptied, its
/// placeholder says what a blank Enter does — `enter drops` — because the
/// gesture is not one the footer teaches and the delivery row under the
/// field has no room to say it on a narrow column.
pub(super) fn render_prompt(
    ctx: &CardCtx,
    buffer: &EditBuffer,
    reopened: bool,
) -> (Line<'static>, u16) {
    let theme = ctx.theme;
    // `  › ` — indent, caret, space. The caret is what an empty field has to
    // show; without it the state is an empty row.
    const LEAD: u16 = 4;
    let t_cells = (ctx.width as usize).saturating_sub(3);
    let budget = t_cells.saturating_sub(LEAD as usize - 1);
    let (shown, cx) = edit_window(buffer.as_str(), buffer.width_before_cursor(), budget);
    let mut spans = vec![Span::raw("  "), Span::styled("› ", Style::default().fg(theme.sel.dim2))];
    if buffer.as_str().is_empty() {
        // An empty field says what it is for, in the same words the key was
        // hinted with. The hardware cursor sits on the first letter of it,
        // which is how every placeholder has ever worked.
        let word = if reopened { "enter drops" } else { "ask claude" };
        spans.push(Span::styled(truncate(word, budget), Style::default().fg(theme.sel.dim3)));
    } else {
        spans.push(Span::styled(
            shown,
            Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD),
        ));
    }
    (Line::from(spans).style(theme.selected_row()), LEAD + cx)
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

/// The ask field's delivery row (2026-09-04): `now` or `queued`, Shift+Tab
/// cycles — the composer's workspace row, for the field under a card. It
/// said ` ∙ blank enter drops` on a reopened ask until T-241 (2026-09-05):
/// 39 cells, cut to `dr` on a narrow column. The emptied field's
/// placeholder carries that word now (`render_prompt`).
pub(super) fn render_ask_mode(ctx: &CardCtx, queued: bool) -> Line<'static> {
    let theme = ctx.theme;
    let word = if queued { "queued" } else { "now" };
    let spans = vec![
        Span::raw("  "),
        Span::styled(word.to_string(), Style::default().fg(theme.sel.dim1)),
        Span::styled("  shift+tab".to_string(), Style::default().fg(theme.sel.dim2)),
    ];
    Line::from(spans).style(theme.selected_row())
}

fn register_style(theme: &Theme, reg: Register) -> Style {
    match reg {
        Register::Attn => theme.attn_text(),
        Register::Err => theme.err_text(),
        Register::Calm => theme.calm_text(),
        Register::Grey => theme.dim2(),
        Register::Dormant => theme.dim3(),
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
    let tier = if ascii { Tier::Ascii } else { Tier::Unicode };
    // The glyph and the arrows are the header's too (T-124): one home.
    let g = branch_mark(tier);
    let (up, down) = (ahead_mark(tier), behind_mark(tier));
    let (dots, check) = if ascii { ('.', '+') } else { ('…', '✓') };
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
    open: bool,
    peek: Option<&crate::peek::Peek>,
    tags: &[crate::tags::Painted],
    doomed: bool,
    unseen: bool,
    snooze: Option<&str>,
    owed: bool,
    owed_row: Option<&str>,
) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    let t_cells = (ctx.width as usize).saturating_sub(3);
    let tier = theme.glyph_tier();
    // The `z` chord is armed on this card: it opens and names the preset on
    // a row of its own, the way a quick tag opens the card it tagged.
    let open = open || snooze.is_some();
    // The `d` chord is armed on this card: it flashes as a deletion — the
    // diff's del tint under an `err` title — until the second `d` or the
    // cancel, on the MOVE ghost's cadence (`Theme::delete_lit`).
    let doomed = doomed && theme.delete_lit(ctx.spin);
    // The launch window starts at the keypress, not at the session record.
    // A worktree ticket's first spawn is PARKED while the worktree is cut
    // (~2 s of git, sometimes more), and provisioning is lazy — a queued or
    // in-flight binding IS a parked spawn, and nothing else queues one. So
    // the card carries the same slow arc it will carry a moment later, and
    // Shift+Enter is answered on the frame after the press either way.
    // A ticket back from a snooze that asked to be lit (T-74) wears the
    // needs-you mark with no session behind it — the one ticket-level
    // producer of the saturated colour, cleared by the keypress that lands
    // the cursor on it. It outranks the sessions' glyph the way a waiting
    // session outranks a working one: someone asked to be told.
    let glyph = if ticket.is_woke() {
        Some(('!', Register::Attn))
    } else {
        glyphs::card_glyph(sessions, tier, ctx.spin).or_else(|| {
            let launching =
                wt.is_some_and(|w| matches!(w.status.as_str(), "queued" | "provisioning"));
            launching.then(|| (glyphs::launching(tier, ctx.spin), Register::Grey))
        })
    };
    // The done mark decays once seen (T-173, the D19 decay 06 §3.6 parked
    // for M6): while the reply it stands for is one the cursor has not been
    // on the card for it is the HEAVY check in the calm register, and the
    // moment it has it is the thin check on the grey ramp — shape and
    // loudness both step down (author 2026-09-04: "play with the glyph
    // itself"). `Calm` was always defined as "done-UNSEEN"; this is the half
    // that makes the word true. No cell is spent: a `◊` beside the title
    // shipped for an hour and was cut ("too big, and with the worktree mark
    // it takes too much space"). The caller passes `unseen` false for the
    // cursor card and the move ghost, whose ticket is being acked as it is
    // drawn.
    let glyph = match glyph {
        Some((_, Register::Calm)) if unseen => Some((glyphs::done_unread(tier), Register::Calm)),
        Some((g, Register::Calm)) => Some((g, Register::Grey)),
        other => other,
    };
    // Mesimon owes this ticket an action — a queued ask, a train merge or
    // rebase ask (2026-09-04) — and the card says so on every card, not only
    // the cursor's: the slow owed mark over a still or empty glyph slot,
    // never over a moving or a loud one (`glyphs::queued_over`).
    let glyph = if owed { glyphs::queued_over(glyph, tier, ctx.spin) } else { glyph };
    // A pending move's trail is semi-transparent everything — even an attn
    // card demotes while its ghost is in hand (the ghost carries the weight).
    let attn_card = !trail && matches!(glyph, Some((_, Register::Attn)));
    let cursorish = selected || held;

    // Age: time in COLUMN (author 2026-09-01) — it counts from the ticket's
    // `entered_at`, so only a column move restarts it; a session changing
    // state does not, and neither does a reorder. It was the newest session
    // state change, which reset on every hook and said nothing about how
    // long the work had sat where it is. The ticket's own stamp means a
    // session-less card carries it too. Seconds tick only while an agent is
    // working — a shell's `Running` is not work (`glyphs::is_working`), so a
    // settled card counts in minutes.
    let age = created_at_epoch_ms(ticket.column_since())
        .map(|ms| age_slot(ctx.now_ms, ms, sessions.iter().any(|s| glyphs::is_working(s))));

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
            Some((_, Register::Dormant)) => BarWeight::Dormant,
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
    // Two loudnesses, and the cursor picks: the cursor card wears its block
    // at full strength, every other card one small step down — which is
    // where nearly every tag on the board is read. A third, quieter level
    // for a parked ticket lived here for a day (read off the sessions, not
    // the glyph) and was cut as too muted (author 2026-09-02): the glyph
    // already says asleep, and the block's one job is "which tag".
    let level =
        if cursorish { crate::theme::TagLevel::Selected } else { crate::theme::TagLevel::Rest };
    let bar_base = theme.bar(BarWeight::Dormant).1;
    let (bar_ch, bar_style) = if trail {
        (ladder_ch.to_string(), state_style)
    } else {
        crate::tags::bar_cell(theme, ladder_ch, bar_base, tags, level)
    };

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
    let row_style = if doomed {
        theme.delete_row()
    } else if attn_card {
        theme.attn_row()
    } else if cursorish {
        theme.selected_row()
    } else {
        Style::default()
    };

    let title_style = if doomed {
        theme.err_text().add_modifier(Modifier::BOLD)
    } else if trail {
        // The original spot of a pending move: semi-transparent.
        theme.dim3()
    } else if attn_card {
        Style::default() // inherits attn_ink from the row
    } else if held || snooze.is_some() {
        // The MOVE ghost blinks in place until dropped or cancelled — the
        // grabbed card must read as "in hand" (STALE-MAP 2026-08-30). The
        // armed SNOOZE borrows it (2026-09-04, "flash while in snooze not
        // confirmed yet"): until Enter or the cancel the card is in hand the
        // same way, about to leave, and the delete's red is a deletion's.
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
    spans.push(Span::raw(" ".to_string()));
    let mut lines = vec![Line::from(spans).style(row_style)];

    if held {
        return lines;
    }

    // ---- accordion (07 §4.3; session rows only — short keys are hidden
    // from the UI for now, author 2026-08-30) -------------------------------
    //
    // The tag row is the TICKET's own metadata, so an open card earns it
    // whether or not there is a transcript to peek — a backlog ticket has no
    // session at all, and that is exactly the card a quick-tag digit lands
    // on. Before this the row was gated on `peek.is_some()`, which is a claim
    // about the AGENT, and the commonest tagged card could never show it.
    let tag_row = open && !tags.is_empty();
    // The cursor card's accordion, or — under `P` (T-237) — a resting card
    // open on its own ground: the tag row and the reply, on the resting ramp,
    // no surface. The session list, the armed snooze and the owed row stay
    // the cursor card's: they are about the selection, not the ticket.
    let accordion =
        selected && (!sessions.is_empty() || tag_row || snooze.is_some() || owed_row.is_some());
    let opened = !selected && open && (tag_row || peek.is_some());
    if accordion || opened {
        let acc_style = if doomed {
            theme.delete_row()
        } else if selected {
            theme.selected_row()
        } else {
            Style::default()
        };
        let ramp = if selected { &theme.sel } else { &theme.rest };
        let dim = Style::default().fg(ramp.dim1);
        let quiet = Style::default().fg(ramp.dim2);
        let faint = Style::default().fg(ramp.dim3);
        let mut push = |spans: Vec<Span<'static>>| {
            let mut all = vec![
                Span::styled(bar_ch.clone(), bar_style),
                Span::styled(" ".to_string(), Style::default()),
            ];
            all.extend(spans);
            // Pad the interior so the surface paints the full card width.
            let used: usize = all.iter().map(|s| s.content.width()).sum();
            all.push(Span::raw(" ".repeat((ctx.width as usize).saturating_sub(used))));
            lines.push(Line::from(all).style(acc_style));
        };

        // The peek names the tags. The mark under the card says how many and
        // in which colours; only words say WHICH, and a card open far enough
        // to show a sentence can afford the row. It sits directly under the
        // title, above the reply — the ticket's own metadata before the
        // agent's. It sits under the title's FIRST CHARACTER: two cells in
        // where a glyph precedes the title, flush where none does — a
        // session-less card has no glyph column, and indenting its chips
        // past a title that starts at the bar hung them in the air.
        // The armed snooze names its preset first: it is what the next key
        // does to this card, before what the card is.
        if let Some(words) = snooze.filter(|_| selected) {
            let words = truncate(words, t_cells.saturating_sub(glyph_cells));
            push(vec![Span::raw(" ".repeat(glyph_cells)), Span::styled(words, faint)]);
        }
        // What mesimon will do to this card next, and what it waits on —
        // the same slot, the same voice: `queued ∙ after T-12`.
        if let Some(words) = owed_row.filter(|_| selected) {
            let words = truncate(words, t_cells.saturating_sub(glyph_cells));
            push(vec![Span::raw(" ".repeat(glyph_cells)), Span::styled(words, faint)]);
        }
        if tag_row {
            let mut row = vec![Span::raw(" ".repeat(glyph_cells))];
            row.extend(crate::tags::chips(theme, tags, t_cells.saturating_sub(glyph_cells)));
            push(row);
        }

        let mut ranked: Vec<&&SessionRecord> = sessions.iter().collect();
        ranked.sort_by_key(|s| (rank(&s.state), s.id));
        // A single session duplicates line 1 (the aggregate glyph IS that
        // session, and its own age is the ticket page's) — its row adds
        // nothing, so only multi-session cards list.
        let listed: &[&&SessionRecord] = if selected && ranked.len() > 1 { &ranked } else { &[] };
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
        if selected && ranked.len() > 2 {
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
                let mark = if glyphs::pulse_lit(ctx.spin) { quiet } else { faint };
                push(vec![
                    Span::styled(format!("  {} ", glyphs::pulse(tier)), mark),
                    Span::styled(row, quiet),
                ]);
            }
        }
        // An open card's stripe is five or six cells tall, so the two tags
        // run down it as full blocks — ~70% the first, ~30% the second —
        // instead of sharing one cell across a half-block. It repaints span 0
        // and nothing else, so no text moves (`tags::stack_full`).
        if !trail {
            crate::tags::stack_full(theme, &mut lines, ladder_ch, bar_base, tags, level);
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

/// Does this card need you — a usable-confidence attention session, or the
/// ticket itself back from a snooze that asked to be seen (T-74)? The one
/// predicate the off-screen `!N` badge and the collapsed spine read.
pub(super) fn needs_you(ticket: &Ticket, sessions: &[&SessionRecord]) -> bool {
    ticket.is_woke() || is_waiting(sessions)
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
