//! Header, footer, grace row, and the External drawer. Footer copy follows
//! the author's dogfood direction: suggestions over an exhaustive shortcut
//! dump. Separator is `∙` U+2219 everywhere — `·` U+00B7 is EAW-Ambiguous
//! and banned (06 §4.1).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, InputPurpose, Mode};
use crate::text::truncate;

/// The breadcrumb — one component on every screen (author 2026-08-30):
/// ` mesimon > project` with the project bold, and the needs-you `!N`
/// beside the project when anything waits. Leading cell is the 1-cell page
/// padding (06 §5.5), aligned with the accent-bar column.
pub(super) fn breadcrumb(app: &App) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let repo = app
        .repo_root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
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
    // `board.tickets`; archived doesn't exist until v0.2.
    let n_tickets = app.board.tickets.len();
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
    // The sleep suggestion (suggestions over shortcuts, author 2026-08-29):
    // payoff first, the how in parens. Grey — it's an offer, not an alarm.
    // Below a tenth of a GiB the payoff would read "~0.0GiB"; stay quiet.
    if r.reclaim_sessions > 0 && r.reclaim_bytes >= 107_374_182 {
        let free = r.reclaim_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        spans.push(Span::styled(
            format!(" ∙ free ~{free:.1}GiB (Z sleeps {} in done)", r.reclaim_sessions),
            theme.dim2(),
        ));
    }
    // A newer binary sits at our own path (dev rebuild or upgrade). Grey
    // offer like the sleep suggestion — attn stays needs-you-only (L3).
    if app.update_ready() {
        spans.push(Span::styled(" ∙ update ready (U reloads)".to_string(), theme.dim2()));
    }
    // Needs-you lives in the breadcrumb's `!N` (07 §2.2's separate
    // `needs you N` word form superseded by the shared component).
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub(super) fn draw_grace(f: &mut Frame, area: Rect, app: &App) {
    let Some(g) = app.grace.last() else {
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

pub(super) fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let line = match &app.mode {
        // The text itself is edited in place (in the card / ticket title);
        // the footer only carries the verbs.
        Mode::Input { purpose, .. } => {
            let label = match purpose {
                InputPurpose::Create { .. } => "NEW",
                InputPurpose::Rename { .. } => "RENAME",
            };
            mode_line(app, label, "enter save ∙ esc cancel")
        }
        Mode::Move { .. } => mode_line(app, "MOVE", "hjkl move ∙ enter drop ∙ esc cancel"),
        Mode::External { .. } => {
            mode_line(app, "EXTERNAL", "jk ∙ a import ∙ R import + resume ∙ esc back")
        }
        Mode::Normal => {
            if app.status.is_empty() {
                mode_line(
                    app,
                    "BOARD",
                    "enter open ticket ∙ tab needs you ∙ a add ∙ m move ∙ p peek ∙ e external ∙ q quit",
                )
            } else {
                Line::from(Span::styled(format!(" {}", app.status), theme.base()))
            }
        }
    };
    f.render_widget(Paragraph::new(line), area);
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
        f.render_widget(
            ratatui::widgets::Block::default().style(Style::default().bg(bg)),
            area,
        );
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
        let head =
            format!(" {}  {}{badges}", truncate(&name, 24), crate::text::age_slot(now, item.mtime_ms, false));
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
        " jk ∙ a import ∙ R import + resume ∙ esc",
        theme.dim2(),
    )));
    f.render_widget(Paragraph::new(lines), area);
}
