//! One card (07 §4 owns the anatomy; 06 supplies glyphs and tokens).
//!
//! Frame per line: `[bar 2][pad 1][content T][pad 1]`, `T = width - 4`.
//! The fixed-width bar carries tags; its shape stays the same when opened.
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

use crate::glyphs::{self, ahead_mark, behind_mark, branch_mark, merged_mark, Register, Tier};
use crate::tags::BAR_WIDTH;
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
    /// Does an open card's meta row close with the ticket's short key
    /// (T-410)? True on the board, where nothing else names it; false where
    /// the frame around the card already does (the search preview).
    pub names_key: bool,
}

/// Render the in-place title editor as a card line (create + rename share it).
/// Returns the line and the cursor x-offset within the column rect.
pub(super) fn render_edit(
    ctx: &CardCtx,
    buffer: &EditBuffer,
    tags: &[crate::tags::Painted],
) -> (Line<'static>, u16) {
    let theme = ctx.theme;
    let t_cells = (ctx.width as usize).saturating_sub(BAR_WIDTH + 2);
    let (bar_ch, bar_style) = theme.bar(BarWeight::Cursor);
    // The tags picked with `^t` colour the phantom card's bar exactly as they
    // will colour the real one.
    // The composer's phantom card is the cursor card by construction.
    let mut spans = crate::tags::bar_spans(
        theme,
        bar_ch,
        bar_style,
        tags,
        crate::theme::TagLevel::Selected,
        theme.selected_bg,
    );
    // Scroll only as far as keeps the hardware cursor visible.
    let budget = t_cells.saturating_sub(1);
    let (shown, cx) = edit_window(buffer.as_str(), buffer.width_before_cursor(), budget);
    let x_off = BAR_WIDTH as u16 + 1 + cx;
    spans.extend([
        Span::raw(" "),
        Span::styled(shown, Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)),
    ]);
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
///
/// `placeholder` is what an EMPTY field says a blank Enter will do, and the
/// caller picks it because only the caller knows the seat: `enter drops` on a
/// field reopened over a WAITING ask (T-241), `start on the title` on an empty
/// agent seat (T-294) — both gestures the footer does not teach and the
/// delivery row under the field has no room for on a narrow column — and
/// `ask claude` or `ask codex` otherwise, matching the ticket's seated agent.
pub(super) fn render_prompt(
    ctx: &CardCtx,
    buffer: &EditBuffer,
    placeholder: &str,
) -> (Line<'static>, u16) {
    let theme = ctx.theme;
    // Bar-width indent, pad, caret, space. The caret is what an empty field has to
    // show; without it the state is an empty row.
    const LEAD: u16 = BAR_WIDTH as u16 + 3;
    let budget = (ctx.width as usize).saturating_sub(LEAD as usize + 2);
    let (shown, cx) = edit_window(buffer.as_str(), buffer.width_before_cursor(), budget);
    let mut spans = vec![
        Span::raw(" ".repeat(BAR_WIDTH + 1)),
        Span::styled("› ", Style::default().fg(theme.sel.dim2)),
    ];
    if buffer.as_str().is_empty() {
        // An empty field says what it is for, in the same words the key was
        // hinted with. The hardware cursor sits on the first letter of it,
        // which is how every placeholder has ever worked.
        spans
            .push(Span::styled(truncate(placeholder, budget), Style::default().fg(theme.sel.dim3)));
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
    column_default: Option<mesimon_core::board::WorkspaceStrategy>,
    plan: bool,
) -> Line<'static> {
    let theme = ctx.theme;
    let word = match workspace {
        Some(mesimon_core::board::WorkspaceStrategy::Worktree) => "worktree",
        Some(mesimon_core::board::WorkspaceStrategy::AdoptExisting) => "adopt",
        Some(mesimon_core::board::WorkspaceStrategy::SharedCheckout) | None => "shared",
    };
    // The column's own default, while the field still stands on it (T-117).
    let from_column = column_default.is_some() && workspace == column_default;
    let word = if from_column { format!("{word} (default)") } else { word.to_string() };
    let mut spans = vec![
        Span::raw(" ".repeat(BAR_WIDTH + 1)),
        Span::styled(format!("⎇ {word}"), Style::default().fg(theme.sel.dim1)),
    ];
    let hint = "  shift+tab";
    if super::spans_width(&spans) + hint.width() < ctx.width as usize {
        spans.push(Span::styled(hint, Style::default().fg(theme.sel.dim2)));
    }
    // `^p` armed (T-434): the claude Shift+Enter starts runs in plan mode.
    // A setting like the workspace pick, on the same row, in the same ink.
    if plan {
        spans.push(Span::styled(" ∙ plan mode", Style::default().fg(theme.sel.dim1)));
    }
    Line::from(spans).style(theme.selected_row())
}

/// The ask field's delivery row (2026-09-04): `now` or `queued`, Shift+Tab
/// cycles — the composer's workspace row, for the field under a card. It
/// said ` ∙ blank enter drops` on a reopened ask until T-241 (2026-09-05):
/// 39 cells, cut to `dr` on a narrow column. The emptied field's
/// placeholder carries that word now (`render_prompt`).
/// `cycles` is whether Shift+Tab is live on the row (T-420: not on a plan
/// dialog, where the one stop is `accept plan`); the hint goes with it.
pub(super) fn render_ask_mode(ctx: &CardCtx, word: &'static str, cycles: bool) -> Line<'static> {
    let theme = ctx.theme;
    let mut spans = vec![
        Span::raw(" ".repeat(BAR_WIDTH + 1)),
        Span::styled(word.to_string(), Style::default().fg(theme.sel.dim1)),
    ];
    if cycles {
        spans.push(Span::styled("  shift+tab".to_string(), Style::default().fg(theme.sel.dim2)));
    }
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
    /// Asked for, nothing cut yet — a step under quiet (T-309).
    Dormant,
}

fn worktree_mark(
    ticket: &Ticket,
    wt: Option<&mesimon_core::command::WorktreeItem>,
    ascii: bool,
) -> Option<(String, WtTone)> {
    let tier = if ascii { Tier::Ascii } else { Tier::Unicode };
    // The glyph and the arrows are the header's too (T-124): one home.
    let g = branch_mark(tier);
    let (up, down) = (ahead_mark(tier), behind_mark(tier));
    let dots = if ascii { '.' } else { '…' };
    let check = merged_mark(tier);
    let Some(w) = wt else {
        // Asked for, not cut yet (T-309). The workspace is a choice the
        // ticket carries from the moment it is minted and the worktree is
        // only cut at the first spawn, so between the two the card said
        // NOTHING — and shift+tab on the board, which is what put that
        // choice on this screen, had no answer to show for itself. One dot
        // against `queued`'s three, in the dormant register: less than
        // being provisioned, which is what it is.
        let planned =
            ticket.workspace_strategy() == mesimon_core::board::WorkspaceStrategy::Worktree;
        let dot = if ascii { '.' } else { '·' };
        return planned.then(|| (format!("{g}{dot}"), WtTone::Dormant));
    };
    Some(match w.status.as_str() {
        "queued" | "provisioning" => (format!("{g}{dots}"), WtTone::Quiet),
        "error" => (format!("{g}x"), WtTone::Err),
        "evicted" => (format!("{g}-"), WtTone::Quiet),
        _ if w.conflict => (format!("{g}!"), WtTone::Err),
        _ if w.merged => (format!("{g}{check}"), WtTone::Quiet),
        // Behind main — the m flow's rebase stage comes before merge.
        _ if w.needs_rebase => (format!("{g}{down}"), WtTone::Ready),
        _ if w.ahead > 0 => (format!("{g}{up}"), WtTone::Ready),
        // Nothing to report — and the trailing space is load-bearing: the
        // mark is right-aligned against the fixed age slot, so a one-cell
        // form would put THIS card's glyph a column right of every other
        // card's and hand the freed cell back to the title. The glyph's
        // column is the anchor; the state char sits in a slot beside it,
        // empty here.
        _ => (format!("{g} "), WtTone::Quiet),
    })
}

/// What the crown (T-411) has to say on a card. `Holder` is the one card
/// wearing it, `sweep` the ms since the crowning while its title still
/// sweeps (T-442); `Touched` is a card the crown just edited, lit with the
/// word for what was done;
/// `Residue` is the quiet mark that light leaves until the cursor rests on
/// the card, the unread done mark's rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrownMark<'a> {
    None,
    Holder { sweep: Option<u64> },
    Touched(&'a str),
    Residue,
}

/// Text under the crowning's sweep (T-442), one span a grapheme. `first` is
/// the column of the text's first cell in the swept run and `cells` the whole
/// run's width, so a run drawn in pieces sweeps as one.
pub(super) fn swept_spans(
    theme: &Theme,
    text: &str,
    first: usize,
    cells: usize,
    elapsed: u64,
    resting: Style,
    surface: Option<ratatui::style::Color>,
) -> Vec<Span<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut at = first;
    text.graphemes(true)
        .map(|g| {
            let style = theme.crown_sweep(at, cells, elapsed, resting, surface);
            at += g.width();
            Span::styled(g.to_string(), style)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)] // two call sites; a params struct would just rename the args
pub(super) fn render(
    ctx: &CardCtx,
    ticket: &Ticket,
    sessions: &[&SessionRecord],
    terminal_busy: bool,
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
    editor: Option<&str>,
    crown: CrownMark<'_>,
) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    let t_cells = (ctx.width as usize).saturating_sub(BAR_WIDTH + 2);
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
    // A ticket back from a snooze that asked to be lit (T-74), or one whose
    // agent raised its hand (T-107), wears the needs-you mark with no
    // session state behind it — the two ticket-level producers of the
    // saturated colour. It outranks the sessions' glyph the way a waiting
    // session outranks a working one: someone asked to be told. A raised
    // hand therefore covers the spinner while its agent works on, which is
    // right — the question is owed whatever the pane is doing, and the
    // ticket page still says what the session is at.
    let glyph = if ticket.is_woke() || ticket.hand_raised() {
        Some(('!', Register::Attn))
    } else {
        glyphs::card_glyph(sessions, terminal_busy, tier, ctx.spin).or_else(|| {
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
    // Selection only changes the neutral bar; tag identity stays full strength.
    let level =
        if cursorish { crate::theme::TagLevel::Selected } else { crate::theme::TagLevel::Rest };
    let bar_base = theme.bar(BarWeight::Dormant).1;
    // Keep the tag gutter on a contrast-checked surface even when the title
    // wears an attention/delete band; a yellow tag must not vanish into gold.
    let bar_surface = if cursorish { theme.selected_bg.or(theme.bg) } else { theme.bg };
    let bar_spans = || {
        if trail {
            vec![Span::styled(ladder_ch.to_string().repeat(BAR_WIDTH), state_style)]
        } else {
            crate::tags::bar_spans(theme, ladder_ch, bar_base, tags, level, bar_surface)
        }
    };

    // ---- line 1: [glyph sp?][crown sp?][title][fill][wt][crown word | age]
    let wt_mark = worktree_mark(ticket, wt, tier == crate::glyphs::Tier::Ascii);
    let glyph_cells = if glyph.is_some() { 2 } else { 0 };
    // The crown (T-411). Its holder wears the mark before the title — the
    // one card on the board that does, which is what makes it obvious. A
    // card the crown just touched says what was done to it where the age
    // goes, for a beat, and keeps a quiet mark there until the cursor has
    // rested on it. Never the saturated colour: the crown is status, and
    // needs-you is the one demand.
    let crown_glyph = glyphs::crown(tier);
    let crown_word = match crown {
        CrownMark::Touched(action) => Some(format!("{crown_glyph} {action}")),
        _ => None,
    };
    let holder_cells = match crown {
        CrownMark::Holder { .. } => crown_glyph.width() + 1,
        _ => 0,
    };
    let residue_cells = match crown {
        CrownMark::Residue => crown_glyph.width() + 1,
        _ => 0,
    };
    let age_cells = match (&crown_word, &age) {
        (Some(w), _) => w.width() + 1,
        (None, Some(_)) => 4, // sp + 3-cell slot
        (None, None) => 0,
    };
    let wt_cells = wt_mark.as_ref().map(|(m, _)| m.width() + 1).unwrap_or(0);
    // A teammate's initials (T-335): who made the card's last change, in the
    // quiet register beside the worktree mark's slot. Off the moment this
    // machine changes the ticket again.
    let editor_cells = editor.map(|e| e.width() + 1).unwrap_or(0);
    // The fixed bar budget is independent of how many tags the ticket wears.
    let title_budget = t_cells.saturating_sub(
        glyph_cells + holder_cells + residue_cells + age_cells + wt_cells + editor_cells,
    );
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
    } else if matches!(crown, CrownMark::Holder { .. } | CrownMark::Touched(_)) {
        theme.crown_text()
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

    let mut spans = bar_spans();
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
    // The crowning (T-442): a wavefront flows out of the mark and across
    // the title, letter by letter, on the row's own ground — under the
    // cursor too, which is where `^o` leaves it. One run, mark and title,
    // so the glow crosses the gap between them.
    let sweep = match crown {
        CrownMark::Holder { sweep: Some(ms) }
            if !(doomed || trail || attn_card || held || snooze.is_some()) =>
        {
            let surface = if cursorish { theme.selected_bg } else { theme.bg };
            Some((ms, surface, holder_cells + title.width()))
        }
        _ => None,
    };
    if let CrownMark::Holder { .. } = crown {
        // The mark keeps the tint under the cursor too — the cursor
        // surface recolours the title, and the crown is the one thing on
        // the card that must still read as itself there.
        let cs = if trail || attn_card { quiet_style } else { theme.crown_text() };
        let mark = format!("{crown_glyph} ");
        match sweep {
            Some((ms, surface, cells)) => {
                spans.extend(swept_spans(theme, &mark, 0, cells, ms, cs, surface));
            }
            None => spans.push(Span::styled(mark, cs)),
        }
    }
    match sweep {
        Some((ms, surface, cells)) => {
            spans.extend(swept_spans(theme, &title, holder_cells, cells, ms, title_style, surface));
        }
        None => spans.push(Span::styled(title, title_style)),
    }
    spans.push(Span::raw(" ".repeat(fill)));
    if let Some((m, tone)) = &wt_mark {
        // Trail/attn contexts demote the mark to the quiet tone with the row.
        let style = match tone {
            _ if attn_card || trail => quiet_style,
            WtTone::Err => theme.err_text(),
            WtTone::Ready => register_style(theme, Register::Calm),
            WtTone::Quiet => quiet_style,
            // One step under the quiet register — and under the cursor it is
            // the selected ramp's, like every other quiet thing on the card.
            WtTone::Dormant => {
                if cursorish {
                    Style::default().fg(theme.sel.dim3)
                } else {
                    theme.dim3()
                }
            }
        };
        spans.push(Span::styled(format!(" {m}"), style));
    }
    if let Some(e) = editor {
        spans.push(Span::styled(format!(" {e}"), quiet_style));
    }
    if let Some(w) = &crown_word {
        // The touched card's beat (T-411): the word for what the crown did
        // stands where the age does, in the crown's tint.
        let ws = if trail || attn_card { quiet_style } else { theme.crown_text() };
        spans.push(Span::styled(format!(" {w}"), ws));
    } else {
        if matches!(crown, CrownMark::Residue) {
            spans.push(Span::styled(format!(" {crown_glyph}"), quiet_style));
        }
        if let Some(a) = &age {
            spans.push(Span::styled(format!(" {a:>3}"), quiet_style));
        }
    }
    spans.push(Span::raw(" ".to_string()));
    let mut lines = vec![Line::from(spans).style(row_style)];

    if held {
        return lines;
    }

    // ---- accordion (07 §4.3) ---------------------------------------------
    //
    // The meta row is the TICKET's own metadata — its short key, and the
    // tags it wears — so an open card earns it whether or not there is a
    // transcript to peek: a backlog ticket has no session at all, and that
    // is exactly the card a quick-tag digit lands on. Before this the row
    // was gated on `peek.is_some()`, which is a claim about the AGENT, and
    // the commonest tagged card could never show it. The key rides the same
    // row (T-410): a session says "T-410" and a resting card never did, so
    // the id was one focus away on every card; with `p` on, the cursor card
    // names its own, and `P` names every card's.
    let meta_row = open && (ctx.names_key || !tags.is_empty());
    // The cursor card's accordion, or — under `P` (T-237) — a resting card
    // open on its own ground: the tag row and the reply, on the resting ramp,
    // no surface. The session list, the armed snooze and the owed row stay
    // the cursor card's: they are about the selection, not the ticket.
    // Why this card is lit, in the agent's own words (T-107). The cursor
    // card only: every other card wears the bare mark, because a card row is
    // the scarcest thing on the board and one open card at a time is what
    // the accordion is for.
    let raised_row = ticket.raised.as_ref().map(|r| r.reason.as_str());
    let accordion = selected
        && (!sessions.is_empty()
            || meta_row
            || snooze.is_some()
            || owed_row.is_some()
            || raised_row.is_some());
    let opened = !selected && meta_row;
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
            let mut all = bar_spans();
            all.push(Span::raw(" "));
            all.extend(spans);
            // Pad the interior so the surface paints the full card width.
            let used: usize = super::spans_width(&all);
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
        //
        // The short key closes the row, right-aligned under the age slot
        // (T-410). On the right rather than before the chips because the
        // chips are indented by the glyph column and the key is not about
        // the glyph: under `P` every card's key then lands in ONE column, the
        // way the ages do, and a reader holding "T-410" from a session scans
        // that column instead of every row. The key is never cut; the chips
        // get what it leaves and drop from the tail as they always did.
        // The armed snooze names its preset first: it is what the next key
        // does to this card, before what the card is.
        if let Some(words) = snooze.filter(|_| selected) {
            let words = truncate(words, t_cells.saturating_sub(glyph_cells));
            push(vec![Span::raw(" ".repeat(glyph_cells)), Span::styled(words, faint)]);
        }
        // Why the mark is up. A step brighter than the rows either side of
        // it, because this is the agent's own sentence rather than chrome —
        // and it is the answer to the question the `!` just asked.
        if let Some(words) = raised_row.filter(|_| selected) {
            let words = truncate(words, t_cells.saturating_sub(glyph_cells));
            push(vec![Span::raw(" ".repeat(glyph_cells)), Span::styled(words, dim)]);
        }
        // What mesimon will do to this card next, and what it waits on —
        // the same slot, the same voice: `queued ∙ after T-12`.
        if let Some(words) = owed_row.filter(|_| selected) {
            let words = truncate(words, t_cells.saturating_sub(glyph_cells));
            push(vec![Span::raw(" ".repeat(glyph_cells)), Span::styled(words, faint)]);
        }
        if meta_row {
            let inner = t_cells.saturating_sub(glyph_cells);
            let key =
                if ctx.names_key { truncate(&ticket.short_key, inner) } else { String::new() };
            let key_cells = key.width();
            let mut row = vec![Span::raw(" ".repeat(glyph_cells))];
            let chips = if tags.is_empty() {
                Vec::new()
            } else {
                let gap = if key_cells > 0 { 1 } else { 0 };
                crate::tags::chips(theme, tags, inner.saturating_sub(key_cells + gap))
            };
            let used = super::spans_width(&chips);
            row.extend(chips);
            row.push(Span::raw(" ".repeat(inner.saturating_sub(used + key_cells))));
            row.push(Span::styled(key, dim));
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
                SessionKind::Codex => "codex",
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

/// Does this card need you — a usable-confidence attention session, the
/// ticket itself back from a snooze that asked to be seen (T-74), or an
/// agent's raised hand (T-107)? The one predicate the off-screen `!N` badge
/// and the collapsed spine read, and it agrees with `Board::needs_you_count`
/// term for term.
pub(super) fn needs_you(ticket: &Ticket, sessions: &[&SessionRecord]) -> bool {
    ticket.is_woke() || ticket.hand_raised() || is_waiting(sessions)
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
