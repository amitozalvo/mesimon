//! One card (07 §4 owns the anatomy; 06 supplies glyphs and tokens).
//!
//! Frame per line: `[bar B][pad 1][content T][pad 1]`, `T = width - B - 2`.
//! The bar carries tags, one cell or two across the whole board
//! (`tags::bar_width`); its shape stays the same when opened.
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
use crate::text::{
    age_slot, created_at_epoch_ms, edit_window, marquee_offset, marquee_window, truncate,
    EditBuffer,
};
use crate::theme::{BarWeight, CrownSweep, LandKind, Landing, Theme};

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
    /// The tag bar's width on this board (`App::bar_width`, T-821).
    pub bar: usize,
}

/// Render the in-place title editor as a card line (create + rename share it).
/// Returns the line and the cursor x-offset within the column rect.
pub(super) fn render_edit(
    ctx: &CardCtx,
    buffer: &EditBuffer,
    tags: &[crate::tags::Painted],
) -> (Line<'static>, u16) {
    let theme = ctx.theme;
    let t_cells = (ctx.width as usize).saturating_sub(ctx.bar + 2);
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
        ctx.bar,
    );
    // Scroll only as far as keeps the hardware cursor visible.
    let budget = t_cells.saturating_sub(1);
    let (shown, cx) = edit_window(buffer.as_str(), buffer.width_before_cursor(), budget);
    let x_off = ctx.bar as u16 + 1 + cx;
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
    let lead = ctx.bar as u16 + 3;
    let budget = (ctx.width as usize).saturating_sub(lead as usize + 2);
    let (shown, cx) = edit_window(buffer.as_str(), buffer.width_before_cursor(), budget);
    let mut spans = vec![
        Span::raw(" ".repeat(ctx.bar + 1)),
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
    (Line::from(spans).style(theme.selected_row()), lead + cx)
}

/// The whole ask field for one ticket's seat — the prompt row and, when the
/// seat has a delivery to name, the row under it — with the cursor's x
/// offset in the prompt row. The board card (under the card) and the ticket
/// page (under the zone's content, T-476) draw the same rows from here, so
/// the two surfaces cannot say different things about one seat.
///
/// The placeholder is what a blank Enter would do, in the seat's own words,
/// and by the same rule `commit_input` judges it: drop the entry that is
/// waiting, start the agent on the title where the seat is empty and the
/// toggle says now (T-294), send the brief where the seat never took a
/// prompt (T-603), accept the plan at `accept plan` (T-420), or nothing at
/// all.
#[allow(clippy::too_many_arguments)] // the field's state, spelled out; two call sites
pub(super) fn render_ask_field(
    ctx: &CardCtx,
    app: &crate::app::App,
    ticket: ulid::Ulid,
    buffer: &EditBuffer,
    (queued, immediately): (bool, bool),
    accept_plan: bool,
    plan: bool,
    tier: Option<&str>,
) -> (Vec<Line<'static>>, u16) {
    let starts = app.board.live_agent(ticket).is_none();
    let placeholder = if accept_plan {
        "enter accepts the plan"
    } else if app.ticket_queued(ticket) && !(starts && !queued) {
        "enter drops"
    } else if starts {
        "start on the title"
    } else if app.ticket_unprompted(ticket) {
        "send the brief"
    } else {
        "ask agent"
    };
    let (line, x_off) = render_prompt(ctx, buffer, placeholder);
    let mut rows = vec![line];
    // The delivery row, where the ask can wait (2026-09-04): after the
    // field, so the cursor row is unchanged. Plan mode (T-434) shows on the
    // same row, so a seat that cannot wait — a worktree ticket's empty seat
    // — still gets the row while the flag is on.
    let tier_word = app.field_tier_word(Some(ticket), tier);
    if app.ask_queueable(ticket) || plan || tier_word.is_some() {
        rows.push(render_ask_mode(
            ctx,
            crate::app::App::ask_mode_word(accept_plan, queued, immediately, plan),
            app.ask_queueable(ticket) && !app.ticket_plan_ready(ticket),
            tier_word.as_deref(),
        ));
    }
    (rows, x_off)
}

/// The composer's workspace row (M4): git glyph + one word, Shift+Tab cycles.
/// Rendered on the phantom card's selected surface, dim — a setting, not text.
pub(super) fn render_workspace_selector(
    ctx: &CardCtx,
    workspace: Option<mesimon_core::board::WorkspaceStrategy>,
    plan: bool,
    tier_word: Option<&str>,
) -> Line<'static> {
    let theme = ctx.theme;
    let word = match workspace {
        Some(mesimon_core::board::WorkspaceStrategy::Worktree) => "worktree",
        Some(mesimon_core::board::WorkspaceStrategy::AdoptExisting) => "adopt",
        Some(mesimon_core::board::WorkspaceStrategy::SharedCheckout) | None => "shared",
    };
    // No `(default)` tail when the pick is the column's own (T-766): the
    // word is the workspace the ticket gets, wherever it came from.
    let mut spans = vec![
        Span::raw(" ".repeat(ctx.bar + 1)),
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
    // `^n` (T-443): the tier the new ticket starts on, where it is not the
    // default — the other launch pick, beside plan mode.
    if let Some(word) = tier_word {
        spans.push(Span::styled(format!(" ∙ {word}"), Style::default().fg(theme.sel.dim1)));
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
pub(super) fn render_ask_mode(
    ctx: &CardCtx,
    word: &'static str,
    cycles: bool,
    tier_word: Option<&str>,
) -> Line<'static> {
    let theme = ctx.theme;
    let mut spans = vec![
        Span::raw(" ".repeat(ctx.bar + 1)),
        Span::styled(word.to_string(), Style::default().fg(theme.sel.dim1)),
    ];
    // `^n` (T-443): the tier the words switch the agent to.
    if let Some(tier) = tier_word {
        spans.push(Span::styled(format!(" ∙ {tier}"), Style::default().fg(theme.sel.dim1)));
    }
    if cycles {
        spans.push(Span::styled("  shift+tab".to_string(), Style::default().fg(theme.sel.dim2)));
    }
    Line::from(spans).style(theme.selected_row())
}

pub(super) fn register_style(theme: &Theme, reg: Register) -> Style {
    match reg {
        Register::Attn => theme.attn_text(),
        Register::Err => theme.err_text(),
        Register::Calm => theme.calm_text(),
        Register::Grey => theme.dim2(),
        Register::Dormant => theme.dim3(),
    }
}

/// The card's pull-request brief (T-739) as drawn on line 2: one glyph and
/// number per PR, newest first, in at most half of `t_cells`; past that the
/// oldest fold into a count. Empty with no PRs. `board.rs` matches these
/// spans on the drawn line to give the brief its click (T-759).
pub(super) fn brief_spans(
    theme: &Theme,
    prs: &[(String, Register)],
    t_cells: usize,
) -> Vec<Span<'static>> {
    if prs.is_empty() {
        return Vec::new();
    }
    let budget = t_cells / 2;
    let mut keep = prs.len();
    let width_of = |n: usize| -> usize {
        let items: usize = prs[..n].iter().map(|(s, _)| s.width()).sum();
        let more = if n < prs.len() { format!(" +{}", prs.len() - n).width() } else { 0 };
        items + n.saturating_sub(1) + more
    };
    while keep > 1 && width_of(keep) > budget {
        keep -= 1;
    }
    let mut spans = Vec::new();
    for (i, (text, reg)) in prs[..keep].iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let mut graphemes = text.chars();
        let mark = graphemes.next().map(|c| c.to_string()).unwrap_or_default();
        let label: String = graphemes.collect();
        spans.push(Span::styled(mark, register_style(theme, *reg)));
        spans.push(Span::styled(label, theme.dim2()));
    }
    if keep < prs.len() {
        spans.push(Span::styled(format!(" +{}", prs.len() - keep), theme.dim2()));
    }
    spans
}

/// What the card's pull-request line says in the room left of its marks
/// (T-794), about the newest open PR, else the newest — the leftmost mark:
/// the page's clauses (`ticket::pr_clauses`) in fewer words, since the mark
/// beside them says it is a pipeline, and `draft` last, the one state no
/// mark spells. A running pipeline is its job's name alone, or `running`,
/// and the flag says so: the line shimmers for it. `passed` is said only
/// while `passed_news` holds (T-803): a few seconds after it turned, then
/// the mark says it alone. The merge's environments are the last it
/// reached, `in production`. A card whose PRs are all closed
/// (`App::card_prs`, T-797) says `closed`. `None` with nothing to say.
pub(super) fn pr_words(
    prs: &[&mesimon_core::command::PrItem],
    passed_news: impl Fn(&mesimon_core::command::PrItem) -> bool,
) -> Option<(String, bool)> {
    use mesimon_core::text::scrub_cells;
    let p = prs.iter().find(|p| matches!(p.state.as_str(), "open" | "draft")).or(prs.first())?;
    if p.state == "closed" {
        return Some(("closed".into(), false));
    }
    let mut words = Vec::new();
    let job = p.job.as_deref().map(|j| scrub_cells(j, false)).filter(|j| !j.is_empty());
    match (p.pipeline.as_str(), job) {
        ("running", Some(job)) => words.push(job),
        ("running", None) => words.push("running".into()),
        (word @ ("failed" | "waiting" | "cancelled"), Some(job)) => {
            words.push(format!("{word}: {job}"))
        }
        (word @ ("failed" | "waiting" | "cancelled"), _) => words.push(word.into()),
        ("passed", _) if passed_news(p) => words.push("passed".into()),
        _ => {}
    }
    words.extend(super::ticket::pr_state_clauses(p, true));
    if p.state == "draft" {
        words.push(p.state.clone());
    }
    if words.is_empty() {
        return None;
    }
    Some((crate::text::one_line(&words.join(" ∙ ")), p.pipeline == "running"))
}

/// The fewest cells the PR line's words are drawn in (T-794); in less, the
/// marks stand alone.
const PR_WORDS_MIN: usize = 6;

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
/// word for what was done — and `land`, while the board's lightning is on
/// (T-544), where the bolt's landing is on its title;
/// `Residue` is the quiet mark that light leaves until the cursor rests on
/// the card, the unread done mark's rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrownMark<'a> {
    None,
    Holder { sweep: Option<u64> },
    Touched { action: &'a str, land: Option<Land> },
    Residue,
}

/// The bolt's landing on a touched card (T-544): what it does to the
/// title, and how long ago it landed — negative while the bolt is still on
/// its way, when the card looks as it did before the touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Land {
    pub kind: LandKind,
    pub ms: i64,
}

/// A title under the bolt's landing (T-544), one span a grapheme: a letter
/// the landing has not written, or has burnt away, is a blank of its width
/// on the row's own ground.
fn landed_spans(theme: &Theme, run: &Landing, text: &str) -> Vec<Span<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut at = 0;
    text.graphemes(true)
        .map(|g| {
            let cell = at;
            at += g.width();
            match theme.crown_land(run, cell) {
                Some(style) => Span::styled(g.to_string(), style),
                None => Span::raw(" ".repeat(g.width())),
            }
        })
        .collect()
}

/// Text under the crowning's sweep (T-442), one span a grapheme. `first` is
/// the column of the text's first cell in the swept run, so a run drawn in
/// pieces sweeps as one.
pub(super) fn swept_spans(
    theme: &Theme,
    run: &CrownSweep,
    text: &str,
    first: usize,
) -> Vec<Span<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut at = first;
    text.graphemes(true)
        .map(|g| {
            let style = theme.crown_sweep(run, at);
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
    archiving: bool,
    unseen: bool,
    snooze: Option<&str>,
    owed: bool,
    owed_row: Option<&str>,
    ship_row: Option<&str>,
    owed_keys: Option<(&[&'static mesimon_core::keymap::Binding], &mesimon_core::keymap::Ctx)>,
    editor: Option<&str>,
    crown: CrownMark<'_>,
    tier_word: Option<&str>,
    corner: Option<String>,
    summary: Option<&crate::app::TicketSummary>,
    summary_keys: Option<&[&'static mesimon_core::keymap::Binding]>,
    summary_pulse: Option<(u64, Option<mesimon_core::summary::Count>)>,
    summary_underline: bool,
    fresh: Option<(&str, u64, bool)>,
    prs: &[(String, Register)],
    pr_words: Option<(&str, bool)>,
    moved: bool,
    ghost: bool,
) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    let t_cells = (ctx.width as usize).saturating_sub(ctx.bar + 2);
    let tier = theme.glyph_tier();
    // The `z` chord is armed on this card: it opens and names the preset on
    // a row of its own, the way a quick tag opens the card it tagged.
    let open = open || snooze.is_some();
    // The `d` chord is armed on this card: it flashes as a deletion — the
    // diff's del tint under an `err` title — until the second `d` or the
    // cancel, on the MOVE ghost's cadence (`Theme::delete_lit`).
    let doomed = doomed && theme.delete_lit(ctx.spin);
    // The `a` chord is armed on this card (T-545): it fades to the move
    // trail's semi-transparent look and back (`Theme::archive_faded`). The
    // faded phase IS a trail, and it gives up the cursor surface — but not
    // the accordion, which `selected` keeps open so the column never jumps.
    let fading = archiving && theme.archive_faded(ctx.spin);
    let trail = trail || fading;
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
    // Words a launch never delivered (T-570) are the third: the agent sits
    // at an empty prompt and the person is the only way forward.
    let glyph = if ticket.is_woke() || ticket.hand_raised() || is_unsent(sessions) {
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
    // A moving card's ghost (T-817) keeps the accordion `selected` gave it
    // under the cursor, which rode the card away: the rows, not the cursor.
    let cursorish = (selected || held) && !fading && !ghost;

    // Age: time in COLUMN (author 2026-09-01) — it counts from the ticket's
    // `entered_at`, so only a column move restarts it; a session changing
    // state does not, and neither does a reorder. It was the newest session
    // state change, which reset on every hook and said nothing about how
    // long the work had sat where it is. The ticket's own stamp means a
    // session-less card carries it too. Seconds tick only while an agent is
    // working — a shell's `Running` is not work (`glyphs::is_working`), so a
    // settled card counts in minutes.
    let age = match corner {
        // `$` (T-327): the ticket's estimated cost holds the slot instead —
        // blank where nothing priced was counted.
        Some(word) => (!word.is_empty()).then_some(word),
        None => created_at_epoch_ms(ticket.column_since())
            .map(|ms| age_slot(ctx.now_ms, ms, sessions.iter().any(|s| glyphs::is_working(s)))),
    };

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
            vec![Span::styled(ladder_ch.to_string().repeat(ctx.bar), state_style)]
        } else {
            crate::tags::bar_spans(theme, ladder_ch, bar_base, tags, level, bar_surface, ctx.bar)
        }
    };

    // ---- line 1: [glyph sp?][crown sp?][title][fill][residue][wt][crown word | age]
    let wt_mark = worktree_mark(ticket, wt, tier == crate::glyphs::Tier::Ascii);
    let glyph_cells = if glyph.is_some() { 2 } else { 0 };
    // The crown (T-411). Its holder wears the mark before the title — the
    // one card on the board that does, which is what makes it obvious. A
    // card the crown just touched says what was done to it where the age
    // goes, for a beat, and keeps a quiet mark there until the cursor has
    // rested on it. Never the saturated colour: the crown is status, and
    // needs-you is the one demand.
    let crown_glyph = glyphs::crown(tier);
    // While the bolt is on its way the card is as it was (T-544): the word
    // arrives with it.
    let land = match crown {
        CrownMark::Touched { land, .. } => land,
        _ => None,
    };
    let landed = land.is_none_or(|l| l.ms >= 0);
    let crown_word = match crown {
        CrownMark::Touched { action, .. } if landed => Some(format!("{crown_glyph} {action}")),
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
        // sp + the 3-cell slot, wider for a cost (`$1.2k`).
        (None, Some(a)) => a.width().max(3) + 1,
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
    } else if matches!(crown, CrownMark::Holder { .. }) && (!cursorish || theme.paints_tags()) {
        // The holder's title keeps the crown's tint under the cursor too
        // (T-442, "it should stay blue as if not hovered"); the cursor adds
        // only its weight. Below TrueColor the tint is the quiet grey, and
        // the cursor's own ink reads better there.
        let tint = theme.crown_text();
        if cursorish {
            tint.add_modifier(Modifier::BOLD)
        } else {
            tint
        }
    } else if cursorish {
        Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)
    } else if matches!(crown, CrownMark::Touched { .. }) && landed {
        theme.crown_text()
    } else if moved && glyph.is_none() {
        // A moved card the cursor has not rested on (T-817), with no glyph
        // to carry the mark: its title does.
        Style::default().fg(theme.rest.base).add_modifier(Modifier::BOLD)
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
        // A moved card the cursor has not rested on (T-817): its glyph is
        // bold until it does. Still, so it never reads as working.
        let gs = if moved && !trail { gs.add_modifier(Modifier::BOLD) } else { gs };
        spans.push(Span::styled(format!("{g} "), gs));
    }
    // The crowning (T-442): a wavefront flows out of the mark and across
    // the title, letter by letter, on the row's own ground — under the
    // cursor too, which is where `^o` leaves it — turning each letter from
    // the title's plain ink to the holder's tint as it passes. One run, mark
    // and title, so the glow crosses the gap between them.
    let sweep = match crown {
        CrownMark::Holder { sweep: Some(elapsed) }
            if !(doomed || trail || attn_card || held || snooze.is_some()) =>
        {
            Some(CrownSweep {
                elapsed,
                cells: holder_cells + title.width(),
                before: if cursorish {
                    Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.rest.base)
                },
                after: title_style,
                surface: if cursorish { theme.selected_bg } else { theme.bg },
            })
        }
        _ => None,
    };
    if let CrownMark::Holder { .. } = crown {
        // The mark keeps the tint under the cursor too — the cursor
        // surface recolours the title, and the crown is the one thing on
        // the card that must still read as itself there.
        let cs = if trail || attn_card { quiet_style } else { theme.crown_text() };
        let mark = format!("{crown_glyph} ");
        match &sweep {
            Some(run) => {
                let run = CrownSweep { before: cs, after: cs, ..*run };
                spans.extend(swept_spans(theme, &run, &mark, 0));
            }
            None => spans.push(Span::styled(mark, cs)),
        }
    }
    // The bolt's landing (T-544): a front runs out of the title's first
    // letter, where the bolt struck, on the row's own ground — under the
    // cursor too — and leaves the title lit, dimmed, written in or burnt
    // away by what the crown did. The cursor's ink stays the cursor's.
    let landing = match land {
        Some(l) if !(doomed || trail || attn_card || held || snooze.is_some()) => {
            let plain = if cursorish {
                Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.rest.base)
            };
            let held = match l.kind {
                LandKind::Dim if cursorish => Style::default().fg(theme.sel.dim2),
                LandKind::Dim => theme.dim2(),
                _ if cursorish => plain,
                _ => theme.crown_text(),
            };
            Some(Landing {
                kind: l.kind,
                ms: l.ms,
                cells: title.width(),
                plain,
                held,
                surface: if cursorish { theme.selected_bg } else { theme.bg },
            })
        }
        _ => None,
    };
    // The summary (T-696). Open with the peek on, the cursor card lists
    // its rows under the reply; at rest, and under `P` on every other open
    // card, the row wears the boxes as an underline — from the cell after
    // the bar's pad to the age, the done share as the run and the rest as a
    // fainter track (author: "start from 1 cell after the tag cell and
    // also include the age cell"). Applied to the finished row below. Never
    // on a row that is already saying something louder: the inverted
    // needs-you band, a trail, the delete flash, the move ghost.
    let rows_shown = selected && open && summary.is_some_and(|s| !s.rows.is_empty());
    // A change plays on the underline even while the rows show (the tick
    // from the dialog lands on the cursor card, which is usually open):
    // the wave crosses, and the row is bare again when it is done.
    // Every box ticked with no agent at work — parked, or gone — the job
    // is told and the row is bare (T-860, "too verbose on the board"),
    // once the last tick's wave has crossed. A shell keeps no job.
    let finished = |s: &&crate::app::TicketSummary| {
        s.count.done == s.count.total
            && !sessions.iter().any(|r| r.kind.is_agent() && r.state.has_pane())
    };
    let under = summary
        .filter(|s| summary_underline && s.count.total > 0)
        .filter(|s| !finished(s) || summary_pulse.is_some())
        .filter(|_| !rows_shown || summary_pulse.is_some())
        .filter(|_| !(doomed || trail || attn_card || held || snooze.is_some()))
        .map(|s| s.count);
    match (&sweep, &landing) {
        (Some(run), _) => spans.extend(swept_spans(theme, run, &title, holder_cells)),
        (None, Some(run)) => spans.extend(landed_spans(theme, run, &title)),
        (None, None) => spans.push(Span::styled(title, title_style)),
    }
    spans.push(Span::raw(" ".repeat(fill)));
    // The residue sits before the worktree mark (T-807), so the marks and
    // the age keep the columns every other card's do.
    if matches!(crown, CrownMark::Residue) {
        spans.push(Span::styled(format!(" {crown_glyph}"), quiet_style));
    }
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
    } else if let Some(a) = &age {
        spans.push(Span::styled(format!(" {a:>3}"), quiet_style));
    }
    if let Some(count) = under {
        let from = ctx.bar + 1;
        let to = super::spans_width(&spans);
        spans = underline_row(theme, spans, from, to, cursorish, count, summary_pulse);
    }
    spans.push(Span::raw(" ".to_string()));
    let mut lines = vec![Line::from(spans).style(row_style)];
    // The rows under the title wear the card's surface (T-788): the cursor
    // card's band, never the needs-you row's — that is one row only.
    let acc_style = if doomed {
        theme.delete_row()
    } else if cursorish {
        theme.selected_row()
    } else {
        Style::default()
    };

    // A resting card's new reply (T-720): what its agent said that the
    // person has not been on the card for, one line under the title in the
    // quiet ink, fading in from the ground left to right as it arrives
    // (`Theme::reply_reveal`). Closed cards only — an open one shows the
    // reply whole — and never on the cursor, the move ghost, a trail or the
    // delete flash: the first two are being looked at, the others are
    // saying something else.
    // The ticket's pull requests (T-739): one glyph and number per PR,
    // newest first, right-aligned under the age slot the way the worktree
    // mark is on line 1. At most half the content width; past that the
    // oldest fold into a count. It shares line 2 with a new reply: the
    // reply keeps the left and is cut against the brief, never a third
    // line. Not on a trail or the delete flash, which say something else.
    let brief: Vec<Span<'static>> =
        if trail || doomed { Vec::new() } else { brief_spans(theme, prs, t_cells) };
    let brief_w = super::spans_width(&brief);
    let brief_room = if brief_w > 0 { brief_w + 1 } else { 0 };
    // An open card draws the brief on its own row instead (T-789), under
    // the reply, the boxes and the running step and over the tags (T-829):
    // the card reads title, then what the agent said, then where the work
    // went.
    let brief_last = open && !held && brief_w > 0;
    // Where no reply holds the line (T-794), the room left of the marks
    // says what the newest PR is at — the running job, the failing one, a
    // conflict, a review — in the marks' quiet ink, a running pipeline's
    // words under a slow shimmer. Cut against the marks like a reply.
    // `lead` is what the merge train owes the card (T-829), said first and
    // a step brighter: it is the next thing to happen to the branch.
    let words_row = |row: &mut Vec<Span<'static>>, lead: Option<&str>| {
        if lead.is_none() && pr_words.is_none() {
            return;
        }
        let gap = if brief_room > 0 { brief_room + 1 } else { 0 };
        let mut room = t_cells.saturating_sub(glyph_cells + gap);
        if room < PR_WORDS_MIN {
            return;
        }
        row.push(Span::raw(" ".repeat(glyph_cells)));
        let ramp = if cursorish { &theme.sel } else { &theme.rest };
        if let Some(lead) = lead {
            let lead = truncate(lead, room);
            room = room.saturating_sub(lead.width());
            row.push(Span::styled(lead, Style::default().fg(ramp.dim1)));
            if pr_words.is_none() || room < 3 + PR_WORDS_MIN {
                return;
            }
            row.push(Span::styled(" ∙ ", Style::default().fg(ramp.dim2)));
            room -= 3;
        }
        let Some((words, running)) = pr_words else { return };
        let words = truncate(words, room);
        if !running {
            row.push(Span::styled(words, Style::default().fg(ramp.dim2)));
            return;
        }
        use unicode_segmentation::UnicodeSegmentation;
        let cells = words.width();
        let mut at = 0;
        for g in words.graphemes(true) {
            row.push(Span::styled(
                g.to_string(),
                theme.pr_shimmer(ctx.now_ms, cells, at, ramp.dim2, ramp.base),
            ));
            at += g.width();
        }
    };
    let brief_row = |brief: Vec<Span<'static>>| {
        let mut row = bar_spans();
        row.push(Span::raw(" "));
        words_row(&mut row, None);
        let used = super::spans_width(&row);
        row.push(Span::raw(" ".repeat((ctx.width as usize).saturating_sub(used + brief_w + 1))));
        row.extend(brief);
        row.push(Span::raw(" "));
        Line::from(row).style(acc_style)
    };
    let mut drew_line2 = brief_last;
    // Why the mark is up (T-107), in the agent's own words, under the title
    // of every card that wears it (T-865, the author: "right now user has to
    // go inside the agent to find out what's the matter"): pinned until the
    // hand comes down, never faded, and in the needs-you colour, the one
    // demand on the board speaking for itself. It holds line 2 over a new
    // reply, which is the transcript's and waits; the PR marks keep their
    // place beside it. Not on a trail, the delete flash or the move ghost,
    // which say something else.
    let raised = ticket.raised.as_ref().filter(|_| !(trail || doomed || held));
    if let Some(r) = raised {
        let room = if brief_last { 0 } else { brief_room };
        let words = truncate(&r.reason, t_cells.saturating_sub(glyph_cells + room));
        let mut row = bar_spans();
        row.push(Span::raw(" ".repeat(1 + glyph_cells)));
        row.push(Span::styled(words, theme.attn_text()));
        let shown = if brief_last { Vec::new() } else { brief.clone() };
        let used = super::spans_width(&row) + super::spans_width(&shown);
        row.push(Span::raw(" ".repeat((ctx.width as usize).saturating_sub(used + 1))));
        row.extend(shown);
        row.push(Span::raw(" "));
        lines.push(Line::from(row).style(acc_style));
        drew_line2 = true;
    }
    if let Some((words, ago, crown)) =
        fresh.filter(|_| !(open || cursorish || trail || doomed || raised.is_some()))
    {
        use unicode_segmentation::UnicodeSegmentation;
        // The crown's words (T-783) settle in its tint, a reply in the
        // quiet ink.
        let reveal = if crown { Theme::crown_reveal } else { Theme::reply_reveal };
        let words = truncate(words, t_cells.saturating_sub(glyph_cells + brief_room));
        let cells = words.width();
        let mut row = bar_spans();
        row.push(Span::raw(" ".repeat(1 + glyph_cells)));
        let mut at = 0;
        for g in words.graphemes(true) {
            let w = g.width();
            row.push(match reveal(theme, ago, cells, at) {
                Some(style) => Span::styled(g.to_string(), style),
                None => Span::raw(" ".repeat(w)),
            });
            at += w;
        }
        let used = super::spans_width(&row);
        row.push(Span::raw(" ".repeat((ctx.width as usize).saturating_sub(used + brief_w + 1))));
        row.extend(brief.iter().cloned());
        row.push(Span::raw(" "));
        lines.push(Line::from(row).style(acc_style));
        drew_line2 = true;
    }
    if !drew_line2 && brief_w > 0 {
        lines.push(brief_row(brief.clone()));
    }

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
    // The ticket's agent tier (T-443) is one of its own facts, and rides
    // the same row where it is not the default, or for a moment after `^n`
    // (`App::card_tier_word`).
    let meta_row = open && (ctx.names_key || !tags.is_empty() || tier_word.is_some());
    // The cursor card's accordion, or — under `P` (T-237) — a resting card
    // open on its own ground: the tag row and the reply, on the resting ramp,
    // no surface. The session list, the armed snooze and the owed row stay
    // the cursor card's: they are about the selection, not the ticket.
    // The crown answered this card's question (T-569): `answered by T-411:
    // Okta`, the claude's `detail` past its stop, which the daemon keeps
    // until the next state edge. A claude reaches `Running` or `Idle` only
    // through a commit that clears every other detail; a stop's own words
    // are the question, said by the needs-you mark, and a restart's
    // `Unknown` may still hold them.
    let answered_row = sessions
        .iter()
        .filter(|s| s.kind == SessionKind::Claude)
        .filter(|s| matches!(s.state, SessionState::Running | SessionState::Idle { .. }))
        .find_map(|s| s.detail.as_deref());
    // What the merge train owes the card (T-829): the cursor card's, like
    // the owed row, and said on the pull requests' row.
    let ship_words = ship_row.filter(|_| selected);
    let accordion = selected
        && (!sessions.is_empty()
            || meta_row
            || snooze.is_some()
            || owed_row.is_some()
            || ship_words.is_some()
            || answered_row.is_some()
            || rows_shown);
    let opened = !selected && meta_row;
    let mut ship_drawn = false;
    if accordion || opened {
        let ramp = if cursorish { &theme.sel } else { &theme.rest };
        // A trail is semi-transparent everything, its open rows included.
        let (dim, quiet, faint) = if trail {
            (theme.dim3(), theme.dim3(), theme.dim3())
        } else {
            (
                Style::default().fg(ramp.dim1),
                Style::default().fg(ramp.dim2),
                Style::default().fg(ramp.dim3),
            )
        };
        let mut push = |spans: Vec<Span<'static>>| {
            let mut all = bar_spans();
            all.push(Span::raw(" "));
            all.extend(spans);
            // Pad the interior so the surface paints the full card width.
            let used: usize = super::spans_width(&all);
            all.push(Span::raw(" ".repeat((ctx.width as usize).saturating_sub(used))));
            lines.push(Line::from(all).style(acc_style));
        };

        // The rows read in the order a person asks of a card (T-829, the
        // author: "make sure everything is in the RIGHT place"): what it
        // needs from you, what the agent said, how far along it is, what it
        // is doing now, where the work goes, and last what the ticket is.
        // Each kind has one place, so a card that grows a row grows it where
        // that kind always stands.

        // The armed snooze names its preset first: it is what the next key
        // does to this card, before what the card is.
        if let Some(words) = snooze.filter(|_| selected) {
            let words = truncate(words, t_cells.saturating_sub(glyph_cells));
            push(vec![Span::raw(" ".repeat(glyph_cells)), Span::styled(words, faint)]);
        }
        // What the crown chose for this card's question (T-569), in the
        // board's quiet voice: a decision the person reads, not the agent's
        // own sentence.
        if let Some(words) = answered_row.filter(|_| selected) {
            // An option label is an agent's words: scrubbed before it is drawn.
            let words = mesimon_core::text::scrub_cells(words, false);
            let words = truncate(&words, t_cells.saturating_sub(glyph_cells));
            push(vec![Span::raw(" ".repeat(glyph_cells)), Span::styled(words, quiet)]);
        }
        // What mesimon will do to this card next, and what it waits on —
        // the same slot, the same voice: `queued ∙ after T-12`.
        if let Some(words) = owed_row.filter(|_| selected) {
            let inner = t_cells.saturating_sub(glyph_cells);
            // A held ask's keys ride the same row, right-aligned under the
            // age (T-551, user: "it's not obvious that user needs to press
            // ^y … should be next to the queued message"; T-829: one row):
            // the keymap's words through the footer's own span builder, so a
            // key drawn here is a key that works, and a key that does not
            // fit is dropped whole rather than cut. One too wide for the
            // words' row (`shift+enter resend`) takes the row under them.
            let keys_in = |budget: usize| match owed_keys {
                Some((keys, kctx)) => {
                    let mut spans = super::chrome::hint_spans(keys, kctx, ramp, budget);
                    if trail {
                        for s in &mut spans {
                            s.style = faint;
                        }
                    }
                    spans
                }
                None => Vec::new(),
            };
            let keys = keys_in(inner.saturating_sub(words.width().min(OWED_WORDS_MIN) + 1));
            let under = if keys.is_empty() { keys_in(inner) } else { Vec::new() };
            let keys_w = super::spans_width(&keys);
            // `^y send` beside the words says whose the send is, so their
            // own `∙ you send` would say it twice in the cells the key needs.
            let words = if keys_w > 0 {
                words.strip_suffix(" ∙ you send").unwrap_or(words)
            } else {
                words
            };
            let words =
                truncate(words, inner.saturating_sub(if keys_w > 0 { keys_w + 1 } else { 0 }));
            let fill = inner.saturating_sub(words.width() + keys_w);
            let mut row = vec![Span::raw(" ".repeat(glyph_cells)), Span::styled(words, faint)];
            if keys_w > 0 {
                row.push(Span::raw(" ".repeat(fill)));
                row.extend(keys);
            }
            push(row);
            if !under.is_empty() {
                let mut row = vec![Span::raw(" ".repeat(glyph_cells))];
                row.extend(under);
                push(row);
            }
        }

        // Peek rows: the latest assistant reply, wrapped inside the card's
        // interior. Two rows (T-829): the gist, with the transcript one
        // focus away.
        if let Some(text) = peek.and_then(|pk| pk.text.as_deref()) {
            let words = crate::peek::card_words(text);
            for row in crate::peek::wrap(&words, t_cells.saturating_sub(2), PEEK_ROWS) {
                push(vec![Span::styled(format!("  {row}"), dim)]);
            }
        }
        // The summary rows (T-696), under the reply: the open boxes in
        // their order (ticked ones after, when few are open), then a fold
        // row counting the rest with their state — every box ticked, none,
        // or some — and, right after the count, the key that lists them all
        // (T-819: `∙ ^j` in the count's ink, its word the count's), through
        // the keymap so a key drawn here is a key that works. Boxes alone
        // (T-829): a plain line is the note's. The rows start on the glyph
        // column, two cells before the reply's words (author: "to indicate
        // it's not the transcript").
        if let Some(s) = summary.filter(|_| rows_shown) {
            let (shown, fold) = s.card_rows(SUMMARY_ROWS);
            for r in shown {
                let words =
                    mesimon_core::text::scrub_cells(&crate::rich::plain_inline(&r.text), false);
                let row = format!("{} {words}", box_mark(tier, Some(r.done)));
                push(vec![Span::styled(truncate(&row, t_cells), if r.done { faint } else { dim })]);
            }
            if let Some((n, state)) = fold {
                let words = format!("{} {n} more", box_mark(tier, state));
                let mut row = vec![Span::styled(words.clone(), faint)];
                if let Some(keys) = summary_keys {
                    let budget = t_cells.saturating_sub(words.width());
                    row.extend(super::chrome::key_spans(keys, faint, budget));
                }
                push(row);
            }
        }
        // The step running now, under the boxes (author: "put the
        // checklists above the command line") — the spinner marks it as
        // now, and quiet keeps the reply the loudest thing in the accordion.
        // A compaction in flight (T-894) is the step running now, and the
        // peek's last call is history until it ends; the row says so in
        // the step's own place, pulse and all.
        let compacting = sessions.iter().any(|s| glyphs::is_compacting(s));
        let act = peek.and_then(|pk| pk.activity.as_ref());
        if compacting || act.is_some() {
            let word = match act {
                _ if compacting => glyphs::COMPACTING_WORDS,
                Some(crate::peek::Doing::Tool(t)) => t.as_str(),
                Some(crate::peek::Doing::Thinking) | None => "thinking",
            };
            let row = crate::text::truncate(word, t_cells.saturating_sub(4));
            let mark = if glyphs::pulse_lit(ctx.spin) { quiet } else { faint };
            push(vec![
                Span::styled(format!("  {} ", glyphs::pulse(tier)), mark),
                Span::styled(row, quiet),
            ]);
        }
        // The ticket's sessions, beside what is running (T-829: one row). A
        // single session duplicates line 1 — the aggregate glyph IS that
        // session — so only a card with more lists them: each its mark, its
        // name (the OSC-0 title, else the kind word) and its glyph, in rank
        // order, and what does not fit counted. Their ages are the page's.
        let mut ranked: Vec<&&SessionRecord> = sessions.iter().collect();
        ranked.sort_by_key(|s| (rank(&s.state), s.id));
        if selected && ranked.len() > 1 {
            let mut row = vec![Span::raw("  ")];
            let mut used = 2;
            for (i, s) in ranked.iter().enumerate() {
                let mark = glyphs::kind_mark(s.kind, tier);
                let word = truncate(s.title.as_deref().unwrap_or(s.kind.word()), SESSION_WORD);
                let entry = format!("{mark} {word} ");
                let (g, reg) = glyphs::session_glyph(s, tier, ctx.spin);
                let gap = if i == 0 { 0 } else { 2 };
                let w = gap + entry.width() + 1;
                let left = ranked.len() - i - 1;
                let more = if left > 0 { format!(" +{left}").width() } else { 0 };
                if i > 0 && used + w + more > t_cells {
                    row.push(Span::styled(format!(" +{}", ranked.len() - i), quiet));
                    break;
                }
                if gap > 0 {
                    row.push(Span::raw(" ".repeat(gap)));
                }
                row.push(Span::styled(entry, dim));
                row.push(Span::styled(
                    g.to_string(),
                    if trail { faint } else { register_style(theme, reg) },
                ));
                used += w;
            }
            push(row);
        }
        // Where the work goes (T-829): the pull requests' marks under the
        // age and their words, after what the merge train owes the branch.
        // On the closed card the marks hold line 2 instead.
        if brief_last || ship_words.is_some() {
            let brief = if brief_last { brief.clone() } else { Vec::new() };
            let mut row = Vec::new();
            words_row(&mut row, ship_words);
            let used = ctx.bar + 1 + super::spans_width(&row);
            let bw = super::spans_width(&brief);
            row.push(Span::raw(" ".repeat((ctx.width as usize).saturating_sub(used + bw + 1))));
            row.extend(brief);
            push(row);
            ship_drawn = true;
        }

        // The meta row, last (T-829): the bar already wears the tags'
        // colours, and what the ticket is reads after what is happening to
        // it. The chips sit under the title's FIRST CHARACTER: two cells in
        // where a glyph precedes the title, flush where none does. The
        // short key closes the row, right-aligned under the age slot, so
        // under `P` every card's key lands in ONE column, the way the ages
        // do. The key is never cut; the chips get what it leaves and drop
        // from the tail as they always did.
        if meta_row {
            let inner = t_cells.saturating_sub(glyph_cells);
            let key = if ctx.names_key { ticket.short_key.clone() } else { String::new() };
            // The tier word before the key (T-443), the quiet ink the key
            // wears: `coder ∙ T-443`.
            let key = match tier_word {
                Some(t) if key.is_empty() => t.to_string(),
                Some(t) => format!("{t} ∙ {key}"),
                None => key,
            };
            let key = truncate(&key, inner);
            let key_cells = key.width();
            let mut row = vec![Span::raw(" ".repeat(glyph_cells))];
            let mut chips = if tags.is_empty() {
                Vec::new()
            } else {
                let gap = if key_cells > 0 { 1 } else { 0 };
                crate::tags::chips(theme, tags, inner.saturating_sub(key_cells + gap))
            };
            if trail {
                // The names stay; their tints go with the bar's.
                for c in &mut chips {
                    c.style = faint;
                }
            }
            let used = super::spans_width(&chips);
            row.extend(chips);
            row.push(Span::raw(" ".repeat(inner.saturating_sub(used + key_cells))));
            row.push(Span::styled(key, dim));
            push(row);
        }
    }
    if brief_last && !ship_drawn {
        lines.push(brief_row(brief));
    }

    // No meta strip at rest (author 2026-08-30): the session dots repeated
    // the aggregate glyph — a resting card is one line, plus its stripe.
    // Per-session detail lives in the accordion and the ticket rail.
    lines
}

/// How many rows the open card gives the summary's boxes, the fold row
/// included (T-696, T-829).
const SUMMARY_ROWS: usize = 3;

/// How many rows of the agent's reply an open card shows (T-829: two, the
/// author's pick).
const PEEK_ROWS: usize = 2;

/// The fewest cells a held ask's words keep beside its keys (T-829).
const OWED_WORDS_MIN: usize = 10;

/// The most cells a session's name takes on the card's sessions row.
const SESSION_WORD: usize = 10;

/// A box as the card and the dialog draw it: ticked wears the board's own
/// check (the note draws it the same, `rich.rs`), open is empty, and the
/// fold row's "some of them" is a dash. ASCII keeps `[x]`.
pub(super) fn box_mark(tier: Tier, state: Option<bool>) -> &'static str {
    match state {
        Some(true) if tier == Tier::Ascii => "[x]",
        Some(true) => "[\u{2713}]",
        Some(false) => "[ ]",
        None => "[-]",
    }
}

/// The row's cells from `from` to `to` underlined as the summary's progress:
/// the done share of that span as the run, the rest as the track, each in
/// its own ink (`Theme::summary_under_at`), or — while a change plays
/// (`pulse`: how long ago, and the count before) — the wave that reveals
/// it (`Theme::summary_wave`). A span that straddles a boundary is split by
/// grapheme; every other span keeps its words and its style and gains the
/// stroke. Nothing changes where the tier draws no underline.
fn underline_row(
    theme: &Theme,
    spans: Vec<Span<'static>>,
    from: usize,
    to: usize,
    cursorish: bool,
    count: mesimon_core::summary::Count,
    pulse: Option<(u64, Option<mesimon_core::summary::Count>)>,
) -> Vec<Span<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    if theme.summary_under(cursorish, true).is_none() {
        return spans;
    }
    let width = to.saturating_sub(from);
    let ink_at = |cell: usize| -> Option<Style> {
        if cell < from || cell >= to {
            return None;
        }
        let at = cell - from;
        match pulse {
            Some((elapsed, before)) => {
                theme.summary_wave(elapsed, width, at, cursorish, before, count)
            }
            None => theme.summary_under_at(cursorish, width, at, count),
        }
    };
    let mut out = Vec::with_capacity(spans.len() + 2);
    let mut at = 0usize;
    for span in spans {
        let style = span.style;
        let mut piece = String::new();
        let mut piece_ink: Option<Option<Style>> = None;
        for g in span.content.graphemes(true) {
            let ink = ink_at(at);
            if piece_ink.is_some_and(|p| p != ink) {
                let p = piece_ink.take().flatten();
                out.push(Span::styled(
                    std::mem::take(&mut piece),
                    p.map_or(style, |u| style.patch(u)),
                ));
            }
            piece_ink = Some(ink);
            piece.push_str(g);
            at += g.width();
        }
        if !piece.is_empty() {
            let p = piece_ink.flatten();
            out.push(Span::styled(piece, p.map_or(style, |u| style.patch(u))));
        }
    }
    out
}

/// Does this card need you — a usable-confidence attention session, the
/// ticket itself back from a snooze that asked to be seen (T-74), an
/// agent's raised hand (T-107), or a launch's words its agent never got
/// (T-570)? The one predicate the off-screen `!N` badge and the collapsed
/// spine read, and it agrees with `Board::needs_you_count` term for term.
pub(super) fn needs_you(ticket: &Ticket, sessions: &[&SessionRecord]) -> bool {
    ticket.is_woke() || ticket.hand_raised() || is_waiting(sessions) || is_unsent(sessions)
}

/// Does a seat of this ticket hold words its launch never delivered
/// (T-570, `Board::unsent_tickets`' term)?
pub(super) fn is_unsent(sessions: &[&SessionRecord]) -> bool {
    sessions.iter().any(|s| s.unsent_words().is_some())
}

/// Does this ticket currently hold a usable-confidence attention session?
pub(super) fn is_waiting(sessions: &[&SessionRecord]) -> bool {
    use mesimon_core::board::Confidence;
    sessions.iter().any(|s| {
        (mesimon_core::attention::is_attention(&s.state)
            && matches!(s.confidence, Confidence::High | Confidence::Medium)
            && !matches!(s.state, SessionState::Sleeping))
            || (s.kind == SessionKind::Bash
                && s.state.has_pane()
                && s.shell_close_request
                    .as_ref()
                    .is_some_and(|q| q.expires_ms > mesimon_core::clock::now_ms()))
    })
}
