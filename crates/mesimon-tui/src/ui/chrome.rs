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
/// `mesimon › project` with the project bold, and the needs-you `!N` beside
/// the project when anything waits. In `ink`'s ramp, so it reads on the
/// page ground and on a band alike.
pub(super) fn breadcrumb(app: &App, ink: &Ramp) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let repo = app.board_name();
    // Sessions waiting plus tickets a snooze woke lit (T-74) — the daemon's
    // tmux status line counts the same way.
    let needs_you = app.board.needs_you_count();
    let sep = format!(" {} ", crate::glyphs::crumb(theme.glyph_tier()));
    let mut spans = vec![
        Span::styled("mesimon".to_string(), Style::default().fg(ink.dim2)),
        Span::styled(sep, Style::default().fg(ink.dim3)),
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

/// The word for the screen the frame is drawn on — the header's chip. The
/// ticket page's carries the ticket's KEY (T-233, 2026-09-05): `TICKET
/// (T-12)` — the key is how a ticket is named in a prompt, a note or a
/// commit, and the page's own title row spells only the title.
fn screen_word(app: &App) -> String {
    match (&app.screen, &app.mode) {
        // The note editor covers its screen whole; the composer's editor
        // and the ask room (T-380) are dialogs over the board and the board
        // is still the room.
        (_, Mode::Editor(ed)) if !ed.composing() && !ed.asking() => "NOTE".into(),
        (Screen::Board, _) => "BOARD".into(),
        (Screen::Ticket { ticket, .. }, _) => match app.board.ticket(*ticket) {
            Some(t) => format!("TICKET ({})", t.short_key),
            None => "TICKET".into(),
        },
        (Screen::Diff, _) => "DIFF".into(),
        (Screen::Releases, _) => "RELEASES".into(),
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
    let mut spans = vec![chip(app, &word), Span::raw("  ".to_string())];
    spans.extend(breadcrumb(app, ink));
    let mut awake = Vec::new();
    // The preference reserves a fixed-width label; activity changes its text
    // and brightness. Focus preserves both geometry and the idle dimming.
    if app.prefs.keep_awake {
        let label = crate::glyphs::awake_label(theme.glyph_tier(), app.caffeinated);
        let mut style = if app.caffeinated { theme.base() } else { theme.dim3() };
        if app.header_focus && app.header_awake && matches!(app.screen, Screen::Board) {
            style = style.patch(theme.selected_row()).add_modifier(Modifier::BOLD);
            if theme.selected_bg.is_some() {
                style = style.fg(if app.caffeinated { theme.sel.base } else { theme.sel.dim3 });
            }
        }
        awake.push(Span::raw(" "));
        awake.push(Span::styled(label, style));
    }
    awake.extend(remote_mark(app));
    // Other screens have no ticket counter; retain their breadcrumb marker.
    if word != "BOARD" {
        spans.append(&mut awake);
    }
    if let Some(leaf) = leaf {
        let used: usize = super::spans_width(&spans);
        let sep = format!(" {} ", crate::glyphs::crumb(theme.glyph_tier()));
        spans.push(Span::styled(sep, Style::default().fg(ink.dim3)));
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
        spans.append(&mut awake);
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
        let used: usize = super::spans_width(&spans);
        // One cell of page padding at the right edge (06 §5.5), and never
        // less than a three-cell gap — a chip touching the state text reads
        // as part of it. Whatever is left is the chip's budget.
        let budget = (area.width as usize).saturating_sub(used + 4);
        let offer = suggestion_chip(app, budget);
        let offer_w: usize = super::spans_width(&offer);
        // The offer's cells and its gap are spoken for; the git clause takes
        // the rest, and gives its own parts up in order when that is tight.
        let reserved = if offer_w > 0 { offer_w + 4 } else { 1 };
        // The merge train has NO word here (author 2026-09-04, after one day
        // of ` ∙ train` and an hour of ` ∙ auto-merge`): the header speaks for
        // the whole board, and the train only ever touches ATTACHED worktree
        // tickets — a board-wide clause beside the checkout's own branch
        // claims it will merge everything. Where it is armed is a preference
        // (the Settings row) and what it will actually do is per ticket (the
        // card's `merge ∙ after T-3` row), so both halves already have a home.
        let room = (area.width as usize).saturating_sub(used + reserved);
        let git = git_clause(app, room, app.header_focus && !app.header_awake);
        let git_w: usize = super::spans_width(&git);
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

/// Remote Control's mark (T-641): ` ∙ remote` while this board is open to
/// paired browsers, and ` ∙ remote offline` while it is on but the relay is
/// not reached — a phone cannot see the board then. Off, nothing: the quiet
/// condition says nothing. It rides beside the awake mark on every screen,
/// because a board open to a phone is a fact about the board, not about the
/// page somebody sits on. Grey ramp only: reachable is `base`, unreachable
/// `dim3`, and `attn` stays needs-you's.
fn remote_mark(app: &App) -> Vec<Span<'static>> {
    if !app.control.enabled {
        return Vec::new();
    }
    let theme = &app.theme;
    let (word, style) = if app.control.connected {
        ("remote", theme.base())
    } else {
        ("remote offline", theme.dim3())
    };
    vec![Span::styled(" ∙ ".to_string(), theme.dim3()), Span::styled(word.to_string(), style)]
}

/// What the branch name keeps of itself while the row is tight: `main` whole,
/// or a key's worth of a slug (`msmn/T-12…`). The ticket page's `⎇` clause
/// keeps a floor for the same reason (`ticket.rs::WT_BRANCH_FLOOR`).
const GIT_BRANCH_FLOOR: usize = 10;

/// ` ↑2 ↓1` — what is ahead and behind, each only when it is not zero. One
/// spelling for the header and the diff screen's identity row.
pub(crate) fn arrows(tier: crate::glyphs::Tier, ahead: u32, behind: u32) -> String {
    let mut out = String::new();
    if ahead > 0 {
        out.push_str(&format!(" {}{ahead}", crate::glyphs::ahead_mark(tier)));
    }
    if behind > 0 {
        out.push_str(&format!(" {}{behind}", crate::glyphs::behind_mark(tier)));
    }
    out
}

/// The board's own checkout, as a clause after the breadcrumb (T-124):
/// ` ⎇ main ↑2 ↓1 ∙ 3 changed`. The glyph and the name are quiet identity
/// (`dim3`/`dim2` — the same weight as `mesimon` in the crumb), the arrows
/// are the calm register the card's `⎇↑` already uses for "there is
/// something to do here", and the change count is a fact in words, not a
/// star on the name. Nothing is drawn until a sample has landed.
///
/// It carried the key that reads the count — ` v diff`, beside the `∙ 3
/// changed` it opens — from T-221 until T-305 made the clause a place the
/// cursor can STAND: `k` off a column header lands on it, and a focused
/// section says what Enter does in the footer, which is one home for the
/// hint instead of two. The board's `v` still works and `?` still lists it.
///
/// `focused` is that cursor: the clause is painted on the elevated surface
/// the way a selected row is, leaving the surrounding gaps on the page
/// ground, and its greys step onto the `sel` ramp. The arrows keep the calm
/// register — being under the cursor does not change what they mean.
///
/// `room` is what the row can spare. The name gives first — truncating down
/// to `GIT_BRANCH_FLOOR` — and only once it is at that floor does the count
/// drop to buy it back; the arrows are never cut, and below the floor the
/// clause stands aside whole rather than lie.
fn git_clause(app: &App, room: usize, focused: bool) -> Vec<Span<'static>> {
    let g = &app.git;
    // The sampled branch and its arrows lead: the root's where the root is a
    // repository, the one nested repo's where a folder holds exactly one
    // (T-225). A workspace of SEVERAL adds its count as a clause, and a
    // folder of several — no branch of its own — is named by the count
    // alone. The change count is the board's: files across every repo.
    if !g.sampled || (g.branch.is_empty() && g.repos.is_empty()) {
        return Vec::new();
    }
    let name = if g.branch.is_empty() {
        mesimon_core::workspace::repos_word(g.repos.len())
    } else {
        g.branch.clone()
    };
    let theme = &app.theme;
    let tier = theme.glyph_tier();
    // The workspace's repos carry their own arrows (T-455), summed, after
    // the count that names them: on the name where a folder is named by its
    // count, on the `N repos` clause where the root's branch leads.
    let (na, nb) = g.nested_ahead_behind();
    let nested = arrows(tier, na, nb);
    let mut state = arrows(tier, g.ahead, g.behind);
    if g.branch.is_empty() {
        state.push_str(&nested);
    }
    let ink = if focused { &theme.sel } else { &theme.rest };
    // The clauses that give way first, whole, on a tight row.
    let mut tail: Vec<Span<'static>> = Vec::new();
    if g.repos.len() > 1 && !g.branch.is_empty() {
        let word = mesimon_core::workspace::repos_word(g.repos.len());
        tail.push(Span::styled(format!(" ∙ {word}"), Style::default().fg(ink.dim2)));
        if !nested.is_empty() {
            tail.push(Span::styled(nested, theme.calm_text()));
        }
    }
    if g.changed > 0 {
        let count = format!(" ∙ {} changed", g.changed);
        tail.push(Span::styled(count, Style::default().fg(ink.dim2)));
    }
    // The terminal's `!` (T-273) is NOT here: it drew ` ! terminal` after
    // ` v diff` for a day and was cut (T-277, user: "keep only on ? help
    // menu") — a standing key on every board is what `?` is for, and the
    // clause's job is the checkout's state.
    //
    // The gap belongs to the page, not the highlight. Focus changes only
    // style, so neither the branch nor the following count moves.
    let fixed = 3 + state.width();
    let floor = name.width().min(GIT_BRANCH_FLOOR);
    let mut name_room = room.saturating_sub(fixed + super::spans_width(&tail));
    if name_room < floor {
        tail.clear();
        name_room = room.saturating_sub(fixed);
    }
    if name_room < floor {
        return Vec::new();
    }
    let mut out = vec![
        Span::styled(
            format!("{} ", crate::glyphs::branch_mark(tier)),
            Style::default().fg(ink.dim3),
        ),
        Span::styled(truncate(&name, name_room), Style::default().fg(ink.dim2)),
    ];
    if !state.is_empty() {
        out.push(Span::styled(state, theme.calm_text()));
    }
    out.extend(tail);
    if focused {
        // Where the profile can paint no surface and may not reverse
        // (light-256, a phosphor at 16) the cursor would be invisible on a
        // row that has no bar cell to weight, so the clause takes the other
        // half of the header chip's treatment instead — bold.
        let mut surface = theme.selected_row();
        if surface == Style::default() {
            surface = surface.add_modifier(Modifier::BOLD);
        }
        for span in &mut out {
            span.style = span.style.patch(surface);
        }
    }
    out.insert(0, Span::raw(" "));
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
    let ctx = app.frame_ctx();
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
/// that will still be there next frame. With neither, it names the card
/// under the cursor (`hover_line`).
///
/// Notices live here rather than in the header because the header already
/// carries up to five optional `∙` clauses and a sixth pushes the update chip
/// off a 100-column terminal.
pub(super) fn draw_advisory(f: &mut Frame, area: Rect, app: &App) {
    let width = area.width as usize;
    // The subscription quota (T-327) takes the row's right side, one cell in
    // from the edge like the footer's own cluster: after a grace or a notice
    // in what they leave, before the hover row, which gives way to it.
    let compose = |left: Option<Line<'static>>, quota: Vec<Span<'static>>| {
        let mut spans: Vec<Span<'static>> = left.map(|l| l.spans).unwrap_or_default();
        let used: usize = super::spans_width(&spans);
        let qw: usize = super::spans_width(&quota);
        if qw > 0 {
            spans.push(Span::raw(" ".repeat(width.saturating_sub(used + qw + 1))));
            spans.extend(quota);
        }
        Line::from(spans)
    };
    let quota_after = |left: &Line<'static>| {
        let used: usize = super::spans_width(&left.spans);
        super::usage::line(app, width.saturating_sub(used + 3 + 1))
    };
    let Some(g) = app.grace.last() else {
        if let Some(n) = app.notices.first() {
            let more = app.notices.len();
            let tail = if more > 1 { format!(" ∙ +{} more", more - 1) } else { String::new() };
            // One cell in from each edge, the footer's inset, and the tail
            // is counted so a long notice never pushes it off the row.
            let room = width.saturating_sub(2 + tail.width());
            // The value step, not the accent: this is a warning, and the one
            // saturated colour stays reserved for needs-you (L3).
            let line = Line::from(Span::styled(
                format!(" {}{tail}", truncate(&n.text, room)),
                app.theme.base(),
            ));
            let quota = quota_after(&line);
            f.render_widget(Paragraph::new(compose(Some(line), quota)), area);
            return;
        }
        // The hover row is fitted second: the quota line is the scarcer
        // news, so it is fitted against what the row needs to name a card
        // at all, and the card's title takes what is left.
        let hover = hover_line(app, width);
        let reserve = if hover.is_some() { HOVER_ROW_FLOOR + 3 } else { 1 };
        let quota = super::usage::line(app, width.saturating_sub(reserve + 1));
        let qw: usize = super::spans_width(&quota);
        let hover = if qw > 0 && hover.is_some() {
            hover_line(app, width.saturating_sub(qw + 3))
        } else {
            hover
        };
        if hover.is_some() || qw > 0 {
            f.render_widget(Paragraph::new(compose(hover, quota)), area);
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
            " deleted \"{}\"{} ∙ u to undo ({}s)",
            truncate(&g.title, 30),
            sessions,
            g.expires_in_secs
        ),
        app.theme.dim1(),
    ));
    let quota = quota_after(&line);
    f.render_widget(Paragraph::new(compose(Some(line), quota)), area);
}

/// The fewest cells the hover row keeps beside the quota line: a key and a
/// title's floor.
const HOVER_ROW_FLOOR: usize = 8 + HOVER_TITLE_FLOOR;

/// What the hover row's title keeps before the created clause gives way.
const HOVER_TITLE_FLOOR: usize = 16;

/// The card under the cursor, named in full (T-471): `T-12 Title ∙ created
/// 2d ago by T-9`. A card cuts its title to its column and shows
/// its key only when opened (T-410); this row has the frame's width, so a
/// key a session quotes and a title the card cut both read without leaving
/// the board. The clause is the ticket page's own (`ticket::created_clause`).
///
/// The quietest thing the row says: a grace or a notice outranks it, since
/// those report something that happened and this repeats what the cursor
/// already points at. Only while the card is drawn as the cursor card — a
/// mode, a dialog or a header under the cursor leaves the row blank.
pub(super) fn hover_line(app: &App, width: usize) -> Option<Line<'static>> {
    let at_rest = matches!(app.screen, Screen::Board) && matches!(app.mode, Mode::Normal);
    if !at_rest || app.header_focus || app.tag_armed.is_some() {
        return None;
    }
    let ticket = app.selected_ticket()?;
    let theme = &app.theme;
    let key = format!(" {} ", ticket.short_key);
    let mut stats = super::ticket::created_clause(ticket, app, mesimon_core::clock::now_ms())
        .map(|c| format!(" ∙ {c}"))
        .unwrap_or_default();
    // One cell of page padding at the right edge. The title gives way
    // first; at its floor the clause goes whole, and the name keeps the row.
    let room = width.saturating_sub(key.width() + 1);
    if room.saturating_sub(stats.width()) < HOVER_TITLE_FLOOR.min(ticket.title.width()) {
        stats.clear();
    }
    let title = truncate(&ticket.title, room.saturating_sub(stats.width()));
    Some(Line::from(vec![
        Span::styled(key, theme.dim2()),
        Span::styled(title, theme.dim1()),
        Span::styled(stats, theme.dim2()),
    ]))
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
            Mode::Menu { .. }
                | Mode::Settings { .. }
                | Mode::Notifications { .. }
                | Mode::Usage { .. }
                | Mode::Theme { .. }
                | Mode::Archived { .. }
                | Mode::External { .. }
                | Mode::Brief { .. }
                | Mode::ColumnSettings { .. }
                | Mode::Search(_)
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
        Mode::Input {
            purpose: InputPurpose::Rename { .. } | InputPurpose::RenameColumn { .. },
            ..
        } => "RENAME",
        // Not "PROMPT": the mode word is what the text will DO, and every
        // other field here saves something to the board. This one leaves
        // mesimon entirely.
        Mode::Input { purpose: InputPurpose::Prompt { .. }, .. } => "ASK",
        // The editor is the composer in a bigger room, or a note — and the
        // note's editor is a screen of its own, named by the header.
        Mode::Editor(e) if e.composing() => "NEW",
        // The ask field in its bigger room is still the ask field (T-380).
        Mode::Editor(e) if e.asking() => "ASK",
        Mode::Editor(_) => return None,
        // The column dialog's name field (T-117).
        Mode::ColumnSettings { naming: Some(_), .. } => "NAME",
        Mode::ColumnSettings { describing: Some(_), .. } => "DESCRIPTION",
        Mode::Sharing { .. } if app.key_dialog => "LICENSE KEY",
        Mode::Sharing { .. } if app.mesophon_dialog => "MESOPHON",
        _ => scope.word(),
    };
    let resting = matches!(
        (&app.screen, scope),
        (Screen::Board, Scope::Board)
            | (Screen::Ticket { .. }, Scope::Ticket)
            | (Screen::Diff, Scope::Diff)
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
        let used: usize = super::spans_width(spans);
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    };
    // The layout pause outranks the status (T-458): it stands until it is
    // answered, and every key it drops would leave the same footer behind.
    // Each no flashes the row in the pending delete's treatment — the diff's
    // del tint under an `err` sentence — then it rests bold. Never the
    // attention hue: the one saturated colour is needs-you.
    if let Some(layout) = app.keys_paused() {
        let (band, text) = if app.layout_flash_lit() {
            (theme.delete_row(), theme.err_text())
        } else {
            (band, Style::default().fg(ink.base))
        };
        let mut spans =
            vec![Span::styled(format!(" {}", layout.warning()), text.add_modifier(Modifier::BOLD))];
        pad_to(&mut spans);
        return Line::from(spans).style(band);
    }
    if !app.status.is_empty() {
        let mut spans = vec![Span::styled(
            format!(" {}", crate::text::one_line(&app.status)),
            Style::default().fg(ink.base),
        )];
        pad_to(&mut spans);
        return Line::from(spans).style(band);
    }
    let scope = app.scope();
    let ctx = app.frame_ctx();
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
    let lead: usize = super::spans_width(&spans);
    // The right cluster is reserved first: it is how everything else is found.
    let mut right = hint_spans(&right, &ctx, ink, width.saturating_sub(lead + 1));
    // A team board's sync clause (T-335) sits at the right edge, past the
    // app keys: `Synced ∙ 3 members`, `Offline ∙ 2 drafts`, and for a
    // viewer the reason every edit key is missing. Only while the board
    // itself is on the screen — a dialog's frame carries its own word.
    if matches!(app.screen, Screen::Board) && matches!(app.mode, Mode::Normal | Mode::Move { .. }) {
        if let Some(clause) = app.sync_clause() {
            if !right.is_empty() {
                right.push(Span::styled("  ∙  ".to_string(), Style::default().fg(ink.dim3)));
            }
            right.push(Span::styled(clause, Style::default().fg(ink.dim2)));
        }
    }
    let right_w: usize = super::spans_width(&right);
    let reserved = if right_w == 0 { 1 } else { right_w + 4 };
    spans.extend(hint_spans(&own, &ctx, ink, width.saturating_sub(lead + reserved)));
    if right_w > 0 {
        let used: usize = super::spans_width(&spans);
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used + right_w + 1))));
        spans.extend(right);
    }
    pad_to(&mut spans);
    Line::from(spans).style(band)
}
