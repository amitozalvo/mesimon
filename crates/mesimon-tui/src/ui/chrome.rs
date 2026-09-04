//! Header, footer and the advisory row. Footer copy follows the author's
//! dogfood direction: suggestions over an exhaustive shortcut dump. Separator
//! is `∙` U+2219 everywhere — `·` U+00B7 is EAW-Ambiguous and banned (06 §4.1).
//!
//! T-158 (2026-09-03): the header opens with a CHIP naming the screen
//! (`BOARD`, `TICKET`, `DIFF`, `NOTE`) and the footer is a painted band whose
//! keys are set apart from their words, with the app-level keys (`esc menu`,
//! `? keys`) in a right-hand cluster of their own. A hint lives in ONE place:
//! a dialog's keys are in its frame, so the footer under an open dialog
//! carries only its mode chip and the right cluster.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::{self, Binding, Ctx, Scope};

use crate::app::{App, InputPurpose, Mode, Screen};
use crate::text::truncate;
use crate::theme::Ramp;

/// The breadcrumb — one component on every screen (author 2026-08-30):
/// `mesimon > project` with the project bold, and the needs-you `!N` beside
/// the project when anything waits. In `ink`'s ramp, so it reads on the
/// page ground and on a band alike.
pub(super) fn breadcrumb(app: &App, ink: &Ramp) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let repo =
        app.repo_root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // Sessions waiting plus tickets a snooze woke lit (T-74) — the daemon's
    // tmux status line counts the same way.
    let needs_you = app.board.needs_you_count();
    let mut spans = vec![
        Span::styled("mesimon".to_string(), Style::default().fg(ink.dim2)),
        Span::styled(" > ".to_string(), Style::default().fg(ink.dim3)),
        Span::styled(repo, Style::default().fg(ink.base).add_modifier(Modifier::BOLD)),
    ];
    if needs_you > 0 {
        // Inverted chip (06 §2.4b treatment): attn ground, attn_ink text.
        // Bare fg text vanished against dark terminal grounds inside tmux.
        spans.push(Span::raw(" ".to_string()));
        spans.push(Span::styled(
            format!(" !{needs_you} "),
            theme.attn_row().add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

/// The word for the screen the frame is drawn on — the header's chip.
fn screen_word(app: &App) -> &'static str {
    match (&app.screen, &app.mode) {
        // The note editor covers its screen whole; the composer's editor is
        // a dialog over the board and the board is still the room.
        (_, Mode::Editor(ed)) if !ed.composing() => "NOTE",
        (Screen::Board, _) => "BOARD",
        (Screen::Ticket { .. }, _) => "TICKET",
        (Screen::Diff { .. }, _) => "DIFF",
        (Screen::Releases, _) => "RELEASES",
    }
}

/// The chip: one bold word on the elevated surface, ` WORD `. The header's
/// is the only painted cell run on its row, which is what makes it read as
/// a label rather than a band.
fn chip(app: &App, word: &str) -> Span<'static> {
    let theme = &app.theme;
    let mut style = theme.selected_row().add_modifier(Modifier::BOLD);
    if theme.selected_bg.is_some() {
        style = style.fg(theme.sel.base);
    }
    Span::styled(format!(" {word} "), style)
}

/// THE header, for every screen: the screen chip, the breadcrumb, an
/// optional leaf (the ticket a diff or a note belongs to — context, not the
/// subject), and on the board the ticket count on the left and the standing
/// offer on the right. One row, on the page ground.
pub(super) fn draw_header(f: &mut Frame, area: Rect, app: &App, leaf: Option<&str>) {
    let theme = &app.theme;
    let ink = &theme.rest;
    let word = screen_word(app);
    let mut spans = vec![chip(app, word), Span::raw("  ".to_string())];
    spans.extend(breadcrumb(app, ink));
    if let Some(leaf) = leaf {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        spans.push(Span::styled(" > ".to_string(), Style::default().fg(ink.dim3)));
        let budget = (area.width as usize).saturating_sub(used + 4);
        spans.push(Span::styled(truncate(leaf, budget), Style::default().fg(ink.base)));
    }
    // The board's own facts and offer — on the board, not on a note editor
    // that happens to have been opened from it.
    if word == "BOARD" {
        // The checkout's own state (T-124) hangs off the breadcrumb — where
        // you are is also which branch — but it is FITTED last, into what the
        // offer leaves: the offer is the next thing to do, the branch is where
        // you already are. `git_at` is where it goes.
        let git_at = spans.len();
        // D33e: session count, RSS aggregate, PTY headroom — all grey. The one
        // saturated colour stays reserved for `needs you`.
        let r = &app.resources;
        // Board contents, not process stats (author 2026-08-30): the count is
        // tickets on the board. Grace-band deletions are already out of
        // `board.tickets`; archived tickets stay in it but are off the board.
        let n_tickets = app.board.tickets.iter().filter(|t| !t.is_archived()).count();
        let noun = if n_tickets == 1 { "ticket" } else { "tickets" };
        spans.push(Span::styled(format!("   {n_tickets} {noun}"), theme.dim2()));
        // Asleep count cut from the header (author 2026-08-30): sleeping is
        // the quiet, correct condition — the card's own state word carries
        // it; the header only speaks when something is spendable (the offer)
        // or scarce. PTY headroom is machine-wide noise until it isn't:
        // surface it only past 80% of the OS cap, as a warning (author
        // 2026-08-30: suggestions over dashboards). Grey ramp, not the
        // accent — attn stays needs-you-only (L3).
        if r.pty_total > 0 && r.pty_used * 5 >= r.pty_total * 4 {
            // Full-value text, no bold — 06 §5.1's bold allowlist doesn't
            // include the header, and the value step is the warning.
            spans.push(Span::styled(
                format!("   ptys {}/{} ∙ close to the limit", r.pty_used, r.pty_total),
                theme.base(),
            ));
        }
        if r.rss_measured > 0 {
            let gib = r.rss_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            spans.push(Span::styled(format!(" ∙ {gib:.1}GiB"), theme.dim2()));
        }
        // What the board is, on the left. What it offers, on the right.
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        // One cell of page padding at the right edge (06 §5.5), and never
        // less than a three-cell gap — a chip touching the state text reads
        // as part of it. Whatever is left is the chip's budget.
        let budget = (area.width as usize).saturating_sub(used + 4);
        let offer = suggestion_chip(app, budget);
        let offer_w: usize = offer.iter().map(|s| s.content.width()).sum();
        // The offer's cells and its gap are spoken for; the git clause takes
        // the rest, and gives its own parts up in order when that is tight.
        let reserved = if offer_w > 0 { offer_w + 4 } else { 1 };
        let git = git_clause(app, (area.width as usize).saturating_sub(used + reserved));
        let git_w: usize = git.iter().map(|s| s.content.width()).sum();
        spans.splice(git_at..git_at, git);
        let used = used + git_w;
        if offer_w > 0 {
            let pad = (area.width as usize).saturating_sub(used + offer_w + 1);
            spans.push(Span::raw(" ".repeat(pad)));
            spans.extend(offer);
        }
    }
    // Needs-you lives in the breadcrumb's `!N` (07 §2.2's separate
    // `needs you N` word form superseded by the shared component).
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// What the branch name keeps of itself while the row is tight: `main` whole,
/// or a key's worth of a slug (`msmn/T-12…`). The ticket page's `⎇` clause
/// keeps a floor for the same reason (`ticket.rs::WT_BRANCH_FLOOR`).
const GIT_BRANCH_FLOOR: usize = 10;

/// The board's own checkout, as a clause after the breadcrumb (T-124):
/// ` ⎇ main ↑2 ↓1 ∙ 3 changed`. The glyph and the name are quiet identity
/// (`dim3`/`dim2` — the same weight as `mesimon` in the crumb), the arrows
/// are the calm register the card's `⎇↑` already uses for "there is
/// something to do here", and the change count is a fact in words, not a
/// star on the name. Nothing is drawn until a sample has landed.
///
/// `room` is what the row can spare. The parts give way in order: the count
/// drops first, then the name truncates to its floor, and the arrows are
/// never cut — below that the clause stands aside whole rather than lie.
fn git_clause(app: &App, room: usize) -> Vec<Span<'static>> {
    let g = &app.git;
    if !g.sampled || g.branch.is_empty() {
        return Vec::new();
    }
    let theme = &app.theme;
    let tier = theme.glyph_tier();
    let mut state = String::new();
    if g.ahead > 0 {
        state.push_str(&format!(" {}{}", crate::glyphs::ahead_mark(tier), g.ahead));
    }
    if g.behind > 0 {
        state.push_str(&format!(" {}{}", crate::glyphs::behind_mark(tier), g.behind));
    }
    let mut changed =
        if g.changed > 0 { format!(" ∙ {} changed", g.changed) } else { String::new() };
    // ` ⎇ ` is three cells; the arrows ride on the name.
    let fixed = 3 + state.width();
    let floor = g.branch.width().min(GIT_BRANCH_FLOOR);
    let mut name_room = room.saturating_sub(fixed + changed.width());
    if name_room < floor {
        changed.clear();
        name_room = room.saturating_sub(fixed);
    }
    if name_room < floor {
        return Vec::new();
    }
    let mut out = vec![
        Span::styled(format!(" {} ", crate::glyphs::branch_mark(tier)), theme.dim3()),
        Span::styled(truncate(&g.branch, name_room), theme.dim2()),
    ];
    if !state.is_empty() {
        out.push(Span::styled(state, theme.calm_text()));
    }
    if !changed.is_empty() {
        out.push(Span::styled(changed, theme.dim2()));
    }
    out
}

/// The standing offer, right-aligned: the highest-priority one, in full words,
/// with the key that takes it. One at a time, and no count of the ones behind
/// it — a header that says how much is queued is a dashboard, and the point of
/// a suggestion is that it is the next thing. The rest are one Esc away, marked
/// with the same `◦` and in the same order, which is where they are read.
///
/// Grey: an offer is never an alarm, and attn stays needs-you-only (L3).
fn suggestion_chip(app: &App, budget: usize) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let ctx = app.ctx();
    let offers = keymap::suggestions(&ctx);
    let Some(top) = offers.first() else {
        return Vec::new();
    };
    let mut text = (top.headline)(&ctx);
    // Every offer is taken from the menu, so every chip names `esc`. One that
    // also has a key of its own names that first — a key that works right now
    // should not need a menu to be found.
    if top.key.is_empty() {
        text.push_str(" (esc)");
    } else {
        text.push_str(&format!(" ({} ∙ esc)", top.key));
    }
    let mark = crate::glyphs::suggest_mark(theme.glyph_tier());
    if text.width() + 2 > budget {
        return Vec::new();
    }
    vec![Span::styled(format!("{mark} "), theme.dim3()), Span::styled(text, theme.dim2())]
}

/// The one-line advisory row. Grace wins when both are present: it is a 9 s
/// countdown with an undo behind it, while a notice is a standing condition
/// that will still be there next frame.
///
/// Notices live here rather than in the header because the header already
/// carries up to five optional `∙` clauses and a sixth pushes the update chip
/// off a 100-column terminal. The row early-returns when there is nothing to
/// say, which is why adding this drifted no existing golden.
pub(super) fn draw_advisory(f: &mut Frame, area: Rect, app: &App) {
    let Some(g) = app.grace.last() else {
        if let Some(n) = app.notices.first() {
            let more = app.notices.len();
            let tail = if more > 1 { format!(" ∙ +{} more", more - 1) } else { String::new() };
            // The value step, not the accent: this is a warning, and the one
            // saturated colour stays reserved for needs-you (L3).
            let line = Line::from(Span::styled(
                format!("  {}{tail}", truncate(&n.text, area.width.saturating_sub(4) as usize)),
                app.theme.base(),
            ));
            f.render_widget(Paragraph::new(line), area);
        }
        return;
    };
    let sessions = if g.live_sessions > 0 {
        format!(" ∙ {} session(s) detached, still running", g.live_sessions)
    } else {
        String::new()
    };
    let line = Line::from(Span::styled(
        format!(
            "  deleted \"{}\"{} ∙ u to undo ({}s)",
            truncate(&g.title, 30),
            sessions,
            g.expires_in_secs
        ),
        app.theme.dim1(),
    ));
    f.render_widget(Paragraph::new(line), area);
}

/// Bindings as hint spans — `key` in `ink.base` + bold (06 §5.1 clause 3:
/// bold is sanctioned on the footer's key names), its word in `ink.dim2`,
/// `∙` between in `ink.dim3` — filled greedily to `budget` cells in the
/// order given: an item that does not fit is skipped, never cut, and a
/// shorter one after it may still land. The footer, a dialog's bottom edge
/// and the rail's trailer rows all spell their keys through this.
pub(super) fn hint_spans(
    items: &[&Binding],
    ctx: &Ctx,
    ink: &Ramp,
    budget: usize,
) -> Vec<Span<'static>> {
    let key = Style::default().fg(ink.base).add_modifier(Modifier::BOLD);
    let word = Style::default().fg(ink.dim2);
    let sep = Style::default().fg(ink.dim3);
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for b in items {
        let hint = (b.hint)(ctx);
        let w = b.show.width() + 1 + hint.width();
        let add = if out.is_empty() { w } else { w + 3 };
        if used + add > budget {
            continue;
        }
        if !out.is_empty() {
            out.push(Span::styled(" ∙ ".to_string(), sep));
        }
        out.push(Span::styled(b.show.to_string(), key));
        out.push(Span::styled(format!(" {hint}"), word));
        used += add;
    }
    out
}

/// Is a framed dialog open? Its keys are in its frame; the footer under it
/// then carries only the mode chip and the right cluster.
fn dialog_open(app: &App) -> bool {
    app.tag_armed.is_some()
        || matches!(
            app.mode,
            Mode::Menu { .. } | Mode::Theme { .. } | Mode::Archived { .. } | Mode::External { .. }
        )
        || (matches!(app.screen, Screen::Board)
            && matches!(&app.mode, Mode::Editor(ed) if ed.composing()))
}

/// The footer's mode chip, or `None` while the screen is at rest — the
/// header already names the screen, and BOARD twice on one frame said
/// nothing the second time. A mode that has taken the keys over (a chord
/// tail, a text field, a move, a dialog) is named here, where the hints
/// it changed are.
fn mode_word(app: &App) -> Option<&'static str> {
    let scope = app.scope();
    // A text field says which field it is; "INPUT" would be true and useless.
    let word = match &app.mode {
        // The tag tail outranks the composer: while it is open it owns the
        // keys, so the word must name the scope the hints came from.
        _ if app.tag_armed.is_some() => Scope::TagChord.word(),
        Mode::Input { purpose: InputPurpose::Create { .. }, .. } => "NEW",
        Mode::Input { purpose: InputPurpose::Rename { .. }, .. } => "RENAME",
        // Not "PROMPT": the mode word is what the text will DO, and every
        // other field here saves something to the board. This one leaves
        // mesimon entirely.
        Mode::Input { purpose: InputPurpose::Prompt { .. }, .. } => "ASK",
        // The editor is the composer in a bigger room, or a note — and the
        // note's editor is a screen of its own, named by the header.
        Mode::Editor(e) if e.composing() => "NEW",
        Mode::Editor(_) => return None,
        _ => scope.word(),
    };
    let resting = matches!(
        (&app.screen, scope),
        (Screen::Board, Scope::Board)
            | (Screen::Ticket { .. }, Scope::Ticket)
            | (Screen::Diff { .. }, Scope::Diff)
            | (Screen::Releases, Scope::Releases)
    );
    (!resting).then_some(word)
}

/// THE footer, for every screen. A pending status outranks the hints — it is
/// the answer to the key just pressed — and otherwise the line is rendered
/// from the keymap, filtered to what this screen can actually do right now.
/// There are no hint literals anywhere in `ui/`: a key that is hinted works,
/// and a key that works is hinted, because one table decides both.
pub(super) fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Paragraph::new(footer_line(app, area.width)), area);
}

/// The footer as a painted band: the mode chip (when a mode is on), the
/// screen's own keys filling left, the app keys right-aligned, every cell
/// of the row on the elevated surface where the profile paints one.
pub(super) fn footer_line(app: &App, width: u16) -> Line<'static> {
    let theme = &app.theme;
    let (band, ink) = match theme.selected_bg {
        Some(bg) => (Style::default().bg(bg), &theme.sel),
        None => (Style::default(), &theme.rest),
    };
    let width = width as usize;
    let pad_to = |spans: &mut Vec<Span<'static>>| {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    };
    if !app.status.is_empty() {
        let mut spans = vec![Span::styled(
            format!(" {}", crate::text::one_line(&app.status)),
            Style::default().fg(ink.base),
        )];
        pad_to(&mut spans);
        return Line::from(spans).style(band);
    }
    let scope = app.scope();
    let ctx = app.ctx();
    let (own, right) = keymap::footer_split(scope, &ctx);
    let own: Vec<&Binding> = if dialog_open(app) { Vec::new() } else { own };

    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".to_string())];
    if let Some(word) = mode_word(app) {
        spans.push(Span::styled(
            word.to_string(),
            Style::default().fg(ink.base).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("  ".to_string()));
    }
    let lead: usize = spans.iter().map(|s| s.content.width()).sum();
    // The right cluster is reserved first: it is how everything else is found.
    let right = hint_spans(&right, &ctx, ink, width.saturating_sub(lead + 1));
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let reserved = if right_w == 0 { 1 } else { right_w + 4 };
    spans.extend(hint_spans(&own, &ctx, ink, width.saturating_sub(lead + reserved)));
    if right_w > 0 {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used + right_w + 1))));
        spans.extend(right);
    }
    pad_to(&mut spans);
    Line::from(spans).style(band)
}
