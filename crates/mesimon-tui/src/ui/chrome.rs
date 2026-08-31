//! Header, footer, grace row, and the External drawer. Footer copy follows
//! the author's dogfood direction: suggestions over an exhaustive shortcut
//! dump. Separator is `∙` U+2219 everywhere — `·` U+00B7 is EAW-Ambiguous
//! and banned (06 §4.1).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::{self, Scope};

use crate::app::{App, InputPurpose, Mode};
use crate::text::truncate;

/// The breadcrumb — one component on every screen (author 2026-08-30):
/// ` mesimon > project` with the project bold, and the needs-you `!N`
/// beside the project when anything waits. Leading cell is the 1-cell page
/// padding (06 §5.5), aligned with the accent-bar column.
pub(super) fn breadcrumb(app: &App) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let repo =
        app.repo_root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let needs_you = mesimon_core::attention::attention_queue(&app.board).len();
    let mut spans = vec![
        Span::styled(" mesimon".to_string(), theme.dim2()),
        Span::styled(" > ".to_string(), Style::default().fg(app.theme.rest.dim3)),
        Span::styled(repo, theme.base().add_modifier(Modifier::BOLD)),
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

pub(super) fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    // D33e: session count, RSS aggregate, PTY headroom — all grey. The one
    // saturated colour stays reserved for `needs you`.
    let r = &app.resources;
    // Board contents, not process stats (author 2026-08-30): the count is
    // tickets on the board. Grace-band deletions are already out of
    // `board.tickets`; archived tickets stay in it but are off the board.
    let n_tickets = app.board.tickets.iter().filter(|t| !t.is_archived()).count();
    let noun = if n_tickets == 1 { "ticket" } else { "tickets" };
    let mut spans = breadcrumb(app);
    spans.push(Span::styled(format!("   {n_tickets} {noun}"), theme.dim2()));
    // Asleep count cut from the header (author 2026-08-30): sleeping is the
    // quiet, correct condition — the card's own state word carries it; the
    // header only speaks when something is spendable (the offer) or scarce.
    // PTY headroom is machine-wide noise until it isn't: surface it only past
    // 80% of the OS cap, as a warning (author 2026-08-30: suggestions over
    // dashboards). Grey ramp, not the accent — attn stays needs-you-only (L3).
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
    // One cell of page padding at the right edge (06 §5.5), and never less
    // than a three-cell gap — a chip touching the state text reads as part of
    // it. Whatever is left is the chip's budget.
    let budget = (area.width as usize).saturating_sub(used + 4);
    let chip = suggestion_chip(app, budget);
    let chip_w: usize = chip.iter().map(|s| s.content.width()).sum();
    if chip_w > 0 {
        let pad = (area.width as usize).saturating_sub(used + chip_w + 1);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend(chip);
    }
    // Needs-you lives in the breadcrumb's `!N` (07 §2.2's separate
    // `needs you N` word form superseded by the shared component).
    f.render_widget(Paragraph::new(Line::from(spans)), area);
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
    // The tag tail owns this row while it is open: the vocabulary is the
    // user's own and mesimon seeds none, so the only way a digit can mean
    // anything is for the row to say what each one currently holds.
    if let Some(line) = tag_line(app, area.width) {
        f.render_widget(Paragraph::new(line), area);
        return;
    }
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

/// A footer mode line: bold mode word, quiet suggestions.
pub(super) fn mode_line(app: &App, word: &str, hint: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {word}"), app.theme.dim1().add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {hint}"), app.theme.dim2()),
    ])
}

/// THE footer, for every screen. A pending status outranks the hints — it is
/// the answer to the key just pressed — and otherwise the line is rendered
/// from the keymap, filtered to what this screen can actually do right now.
/// There are no hint literals anywhere in `ui/`: a key that is hinted works,
/// and a key that works is hinted, because one table decides both.
pub(super) fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Paragraph::new(footer_line(app, area.width)), area);
}

pub(super) fn footer_line(app: &App, width: u16) -> Line<'static> {
    if !app.status.is_empty() {
        return Line::from(Span::styled(
            format!(" {}", crate::text::one_line(&app.status)),
            app.theme.base(),
        ));
    }
    let scope = app.scope();
    // A text field says which field it is; "INPUT" would be true and useless.
    let word = match &app.mode {
        // The tag tail outranks the composer: while it is open it owns the
        // keys, so the word must name the scope the hints came from.
        _ if app.tag_armed.is_some() => Scope::TagChord.word(),
        Mode::Input { purpose: InputPurpose::Create { .. }, .. } => "NEW",
        Mode::Input { purpose: InputPurpose::Rename { .. }, .. } => "RENAME",
        _ => scope.word(),
    };
    // The mode word plus its two-space gutter and the leading pad.
    let budget = (width as usize).saturating_sub(word.len() + 4);
    mode_line(app, word, &keymap::footer(scope, &app.ctx(), budget))
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
    let w = 64.min(f.area().width.saturating_sub(4));
    let h = ((app.external.len() as u16 * 2) + 4).min(f.area().height.saturating_sub(2));
    let area = Rect {
        x: (f.area().width.saturating_sub(w)) / 2,
        y: (f.area().height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(ratatui::widgets::Clear, area);
    if let Some(bg) = theme.bg {
        f.render_widget(ratatui::widgets::Block::default().style(Style::default().bg(bg)), area);
    }

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(" external sessions — {}", app.external.len()),
        theme.dim1().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::default());
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
        let style = if i == idx {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.base()
        };
        lines.push(Line::from(Span::styled(head, style)));
        let preview = item.preview.as_deref().unwrap_or("");
        lines.push(Line::from(Span::styled(
            format!("     {}", truncate(preview, w as usize - 6)),
            theme.dim2(),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!(" {}", keymap::footer(keymap::Scope::Drawer, &app.ctx(), w as usize - 2)),
        theme.dim2(),
    )));
    f.render_widget(Paragraph::new(lines), area);
}

/// The archived-tickets dialog (V): restore or open. Same popup treatment as
/// the External drawer — no drawn structure, grey ramp only.
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
    let w = 64.min(f.area().width.saturating_sub(4));
    let h = ((archived.len() as u16) + 4).min(f.area().height.saturating_sub(2));
    let area = Rect {
        x: (f.area().width.saturating_sub(w)) / 2,
        y: (f.area().height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(ratatui::widgets::Clear, area);
    if let Some(bg) = theme.bg {
        f.render_widget(ratatui::widgets::Block::default().style(Style::default().bg(bg)), area);
    }
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(" archived — {}", archived.len()),
        theme.dim1().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::default());
    for (i, t) in archived.iter().enumerate() {
        // Archive age from the `@<secs>` stamp; unparsable stamps show no age.
        let age = t
            .archived
            .as_ref()
            .and_then(|a| a.at.strip_prefix('@'))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|secs| crate::text::age_slot(now, secs * 1000, false))
            .unwrap_or_default();
        let head = format!(" {}  {} ∙ {} ∙ {}", t.short_key, truncate(&t.title, 28), t.column, age);
        let style = if i == idx {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.base()
        };
        lines.push(Line::from(Span::styled(head, style)));
    }
    lines.push(Line::from(Span::styled(
        format!(" {}", keymap::footer(keymap::Scope::Archived, &app.ctx(), w as usize - 2)),
        theme.dim2(),
    )));
    f.render_widget(Paragraph::new(lines), area);
}

/// The `^t` tail's row: every axis that holds something, plus the one being
/// typed. Groups with no vocabulary are omitted rather than shown empty — a
/// board that has never used group 7 should not advertise it.
fn tag_line(app: &App, width: u16) -> Option<Line<'static>> {
    let arm = app.tag_armed.as_ref()?;
    let theme = &app.theme;
    let worn = app.tag_subject().unwrap_or(&[]);
    let mut spans =
        vec![Span::styled(" TAG".to_string(), theme.dim1().add_modifier(Modifier::BOLD))];

    if let Some(buf) = arm.naming.as_ref() {
        let g = arm.group.unwrap_or(0);
        spans.push(Span::styled(format!("  {g} #"), theme.dim2()));
        spans.push(Span::styled(buf.as_str().to_string(), theme.base()));
        // A block, not the hardware cursor: the real cursor may be parked in
        // the composer's title field one row away.
        spans.push(Span::styled("\u{2588}".to_string(), theme.dim2()));
        return Some(Line::from(spans));
    }

    let mut shown = 0usize;
    for g in 1..=9u8 {
        let vocab = app.board.group_tags(g);
        let wearing = worn.iter().find(|t| t.group == g);
        if vocab.is_empty() && wearing.is_none() {
            continue;
        }
        let picked = arm.group == Some(g);
        let key = if picked { theme.base().add_modifier(Modifier::BOLD) } else { theme.dim2() };
        spans.push(Span::styled(format!("  {g} "), key));
        match wearing {
            Some(t) => {
                spans.push(Span::styled(
                    crate::tags::pip_char(&t.name).to_string(),
                    Style::default().fg(theme.pip(crate::tags::tint_index(&t.name))),
                ));
                spans.push(Span::styled(format!(" {}", t.name), theme.dim1()));
            }
            None => spans.push(Span::styled("\u{2013}".to_string(), theme.dim3())),
        }
        shown += 1;
    }
    if shown == 0 {
        spans.push(Span::styled("  no tags yet \u{2014} press a digit".to_string(), theme.dim2()));
    }
    // Never overflow the row: drop whole entries from the right.
    let mut used: usize = spans.iter().map(|s| s.content.width()).sum();
    while used > width as usize && spans.len() > 1 {
        let s = spans.pop().expect("non-empty");
        used -= s.content.width();
    }
    Some(Line::from(spans))
}
