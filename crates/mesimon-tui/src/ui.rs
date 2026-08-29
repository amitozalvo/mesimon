//! Board renderer — the D19 baseline identity from day one: no card borders,
//! 1-char accent bar, a 4-step grey ramp, exactly one saturated colour reserved
//! for "needs you" (unused in M1 — no attention yet — but the slot exists).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;

use crate::app::{App, InputPurpose, Mode};

// Interim palette until the M6 design pass: body text rides the terminal's
// default foreground (readable on light AND dark backgrounds); only de-emphasis
// uses a named colour. Never hardcode white/grey for primary text before OSC-11
// background detection exists (06 §2.6).
const FG: Color = Color::Reset;
const DIM: Color = Color::DarkGray;
const ACCENT_IDLE: Color = Color::DarkGray;
const ACCENT_LIVE: Color = Color::Blue; // placeholder for the muted secondary
const GHOST: Color = Color::DarkGray;
/// THE one saturated colour (D19): needs-you, and nothing else, ever.
/// Named-ANSI until the M6 OSC-11 palette lands.
const ACCENT_ATTN: Color = Color::Red;

pub fn draw(f: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(3),    // columns
            Constraint::Length(1), // grace / ghost row
            Constraint::Length(1), // status / footer
        ])
        .split(f.area());

    draw_header(f, outer[0], app);
    draw_columns(f, outer[1], app);
    draw_grace(f, outer[2], app);
    draw_footer(f, outer[3], app);
    if let Mode::Pick { ticket, idx } = &app.mode {
        draw_picker(f, app, *ticket, *idx);
    }
    if let Mode::External { idx } = &app.mode {
        draw_drawer(f, app, *idx);
    }
}

/// The External drawer (19 §4): discovered foreign sessions, observe/resume.
fn draw_drawer(f: &mut Frame, app: &App, idx: usize) {
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

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(" external sessions — {}", app.external.len()),
        Style::default().fg(DIM).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::default());
    for (i, item) in app.external.iter().enumerate() {
        let name = item
            .name
            .clone()
            .unwrap_or_else(|| item.claude_session_id.to_string()[..8].to_string());
        let mut badges = String::new();
        if item.running_elsewhere {
            badges.push_str("  · running elsewhere");
        }
        let head = format!(
            " {} {}  {}{badges}",
            if i == idx { "▸" } else { " " },
            truncate(&name, 24),
            age_word(now, item.mtime_ms),
        );
        let style = if i == idx {
            Style::default().fg(FG).add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(FG)
        };
        lines.push(Line::from(Span::styled(head, style)));
        let preview = item.preview.as_deref().unwrap_or("");
        lines.push(Line::from(Span::styled(
            format!("     {}", truncate(preview, w as usize - 6)),
            Style::default().fg(DIM),
        )));
    }
    lines.push(Line::from(Span::styled(
        " jk · a import · R import + resume · Esc",
        Style::default().fg(DIM),
    )));
    f.render_widget(Paragraph::new(lines), area);
}

fn age_word(now_ms: u64, then_ms: u64) -> String {
    let secs = now_ms.saturating_sub(then_ms) / 1000;
    match secs {
        0..=119 => format!("{secs}s"),
        120..=7199 => format!("{}m", secs / 60),
        7200..=172_799 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// Small centered session picker (M1 stand-in for the ticket screen).
fn draw_picker(f: &mut Frame, app: &App, ticket: ulid::Ulid, idx: usize) {
    let live = app.live_sessions_of(ticket);
    if live.is_empty() {
        return;
    }
    let title = app.board.ticket(ticket).map(|t| t.title.clone()).unwrap_or_default();
    let w = 44.min(f.area().width.saturating_sub(4));
    let h = (live.len() as u16 + 4).min(f.area().height.saturating_sub(2));
    let area = Rect {
        x: (f.area().width.saturating_sub(w)) / 2,
        y: (f.area().height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(ratatui::widgets::Clear, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(" {} — sessions", truncate(&title, w as usize - 12)),
        Style::default().fg(DIM).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::default());
    for (i, s) in live.iter().enumerate() {
        let kind = match s.kind {
            mesimon_core::board::SessionKind::Claude => "claude",
            mesimon_core::board::SessionKind::Bash => "bash",
        };
        let mut badges = String::new();
        if matches!(s.state, mesimon_core::board::SessionState::Sleeping) {
            badges.push_str(" · asleep");
        }
        if s.provenance == mesimon_core::board::Provenance::Adopted && s.argv.is_empty() {
            badges.push_str(" · external");
        }
        if s.pinned_awake {
            badges.push_str(" · pinned");
        }
        let style = if i == idx {
            Style::default().fg(FG).add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(FG)
        };
        lines.push(Line::from(Span::styled(
            format!(
                "  {} {kind} · {}{badges}  ",
                if i == idx { "▸" } else { " " },
                &s.sid16()[..8]
            ),
            style,
        )));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        " jk · Enter focus · z sleep/wake · p pin · x kill · Esc",
        Style::default().fg(DIM),
    )));
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let repo = app
        .repo_root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // D33e: session count, RSS aggregate, PTY headroom — all grey. The one
    // saturated colour stays reserved for `needs you`.
    let r = &app.resources;
    let needs_you = mesimon_core::attention::attention_queue(&app.board).len();
    let mut spans = vec![
        Span::styled("  mesimon", Style::default().fg(FG).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {repo}"), Style::default().fg(DIM)),
        Span::styled(format!("   {} live", r.live), Style::default().fg(DIM)),
    ];
    if r.asleep > 0 {
        spans.push(Span::styled(format!(" · {} asleep", r.asleep), Style::default().fg(DIM)));
    }
    if r.pty_total > 0 {
        spans.push(Span::styled(
            format!("   ptys {}/{}", r.pty_used, r.pty_total),
            Style::default().fg(DIM),
        ));
    }
    if r.rss_measured > 0 {
        let gib = r.rss_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        spans.push(Span::styled(format!(" · {gib:.1}GiB"), Style::default().fg(DIM)));
    }
    // `needs you N`, no colon (07 §2.2) — never dropped, absent only at zero.
    if needs_you > 0 {
        spans.push(Span::styled(
            format!("   needs you {needs_you}"),
            Style::default().fg(ACCENT_ATTN).add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_columns(f: &mut Frame, area: Rect, app: &App) {
    let cols = app.columns();
    if cols.is_empty() {
        return;
    }
    let constraints: Vec<Constraint> =
        cols.iter().map(|_| Constraint::Ratio(1, cols.len() as u32)).collect();
    let rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (ci, (name, rect)) in cols.iter().zip(rects.iter()).enumerate() {
        draw_column(f, *rect, app, ci, name);
    }
}

fn draw_column(f: &mut Frame, area: Rect, app: &App, ci: usize, name: &str) {
    let tickets = app.board.column_tickets(name);
    let ghost = match &app.mode {
        Mode::Move { ticket, col, idx } if *col == ci => Some((*ticket, *idx)),
        _ => None,
    };

    let mut lines: Vec<Line> = Vec::new();
    // Column title: case rule — spine text is uppercase, dim (06 §3.3).
    let count = tickets.iter().filter(|t| ghost.map(|(g, _)| t.id != g).unwrap_or(true)).count()
        + ghost.map(|_| 1).unwrap_or(0);
    lines.push(Line::from(Span::styled(
        format!(" {} {}", name.to_uppercase(), count),
        Style::default().fg(DIM).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::default());

    // Rows: tickets minus the moving ghost, with the ghost re-inserted at its index.
    let rows: Vec<&mesimon_core::board::Ticket> = match ghost {
        Some((gid, _)) => tickets.iter().filter(|t| t.id != gid).copied().collect(),
        None => tickets,
    };

    // The ticket's card word: the reason of its min-rank attention session
    // (11 §11.7.2 — ticket state is the minimum rank; low/stale never counts).
    let attn_word = |tid: ulid::Ulid| -> Option<&'static str> {
        use mesimon_core::attention::{is_attention, rank, reason_word};
        use mesimon_core::board::{Confidence, SessionState};
        app.board
            .sessions
            .iter()
            .filter(|s| {
                s.ticket == tid
                    && is_attention(&s.state)
                    && matches!(s.confidence, Confidence::High | Confidence::Medium)
            })
            .min_by_key(|s| rank(&s.state))
            .and_then(|s| match &s.state {
                SessionState::RequiresAction { reason } => Some(reason_word(*reason)),
                _ => None,
            })
    };

    let render_row = |lines: &mut Vec<Line>, t: &mesimon_core::board::Ticket, selected: bool, ghosted: bool| {
        // Dots are panes; sleeping sessions render as a dim z-run instead.
        let live = app
            .board
            .sessions
            .iter()
            .filter(|s| s.ticket == t.id && s.state.has_pane())
            .count();
        let asleep = app
            .board
            .sessions
            .iter()
            .filter(|s| {
                s.ticket == t.id && matches!(s.state, mesimon_core::board::SessionState::Sleeping)
            })
            .count();
        let attn = if ghosted { None } else { attn_word(t.id) };
        let accent = if ghosted {
            GHOST
        } else if attn.is_some() {
            ACCENT_ATTN
        } else if live > 0 {
            ACCENT_LIVE
        } else {
            ACCENT_IDLE
        };
        // The reason replaces the dots (06 §3: the reason IS the state word).
        let mut suffix = match attn {
            Some(word) => format!(" ● {word}"),
            None if live > 0 => format!(" {}", "•".repeat(live.min(4))),
            None => String::new(),
        };
        if attn.is_none() && asleep > 0 {
            suffix.push_str(&format!(" {}", "z".repeat(asleep.min(4))));
        }
        let width = (area.width as usize).saturating_sub(3 + suffix.chars().count() + 1);
        let title = truncate(&t.title, width);
        let style = if selected {
            Style::default().fg(FG).add_modifier(Modifier::REVERSED)
        } else if ghosted {
            Style::default().fg(GHOST)
        } else {
            Style::default().fg(FG)
        };
        let suffix_style = if attn.is_some() {
            Style::default().fg(ACCENT_ATTN)
        } else {
            Style::default().fg(DIM)
        };
        lines.push(Line::from(vec![
            Span::styled("▌ ", Style::default().fg(accent)),
            Span::styled(title, style),
            Span::styled(suffix, suffix_style),
        ]));
        lines.push(Line::default()); // vertical rhythm: one blank row between cards (06 §5.5)
    };

    match ghost {
        Some((gid, gidx)) => {
            let ghost_ticket = app.board.ticket(gid);
            for (i, t) in rows.iter().enumerate() {
                if i == gidx {
                    if let Some(gt) = ghost_ticket {
                        render_row(&mut lines, gt, true, true);
                    }
                }
                render_row(&mut lines, t, false, false);
            }
            if gidx >= rows.len() {
                if let Some(gt) = ghost_ticket {
                    render_row(&mut lines, gt, true, true);
                }
            }
        }
        None => {
            for (i, t) in rows.iter().enumerate() {
                let selected =
                    app.cursor_col == ci && app.cursor_row == i && matches!(app.mode, Mode::Normal);
                render_row(&mut lines, t, selected, false);
            }
        }
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_grace(f: &mut Frame, area: Rect, app: &App) {
    let Some(g) = app.grace.last() else {
        return;
    };
    let sessions = if g.live_sessions > 0 {
        format!(" · {} session(s) detached, still running", g.live_sessions)
    } else {
        String::new()
    };
    let line = Line::from(Span::styled(
        format!(
            "  deleted {} \"{}\"{} · u to undo ({}s)",
            g.short_key,
            truncate(&g.title, 30),
            sessions,
            g.expires_in_secs
        ),
        Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
    ));
    f.render_widget(Paragraph::new(line), area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let line = match &app.mode {
        Mode::Input { purpose, buffer } => {
            let label = match purpose {
                InputPurpose::Create => "new ticket",
                InputPurpose::Rename { .. } => "rename",
            };
            Line::from(vec![
                Span::styled(format!(" {label}: "), Style::default().fg(DIM)),
                Span::styled(buffer.clone(), Style::default().fg(FG)),
                Span::styled("▏", Style::default().fg(FG)),
            ])
        }
        Mode::Move { .. } => Line::from(Span::styled(
            " MOVE  hjkl move · Enter drop · Esc cancel",
            Style::default().fg(FG).add_modifier(Modifier::REVERSED),
        )),
        Mode::Pick { .. } => Line::from(Span::styled(
            " PICK  jk select · Enter focus · z sleep/wake · p pin · x kill · Esc back",
            Style::default().fg(FG).add_modifier(Modifier::REVERSED),
        )),
        Mode::External { .. } => Line::from(Span::styled(
            " EXTERNAL  jk select · a import · R import + resume · Esc back",
            Style::default().fg(FG).add_modifier(Modifier::REVERSED),
        )),
        Mode::Normal => {
            if app.status.is_empty() {
                Line::from(Span::styled(
                    " BOARD  hjkl · Tab needs-you · o new · r rename · d delete · u undo · m move · s claude · S bash · e external · Z reclaim · Enter open · q quit",
                    Style::default().fg(DIM),
                ))
            } else {
                Line::from(Span::styled(format!(" {}", app.status), Style::default().fg(FG)))
            }
        }
    };
    f.render_widget(Paragraph::new(line), area);
}

/// Width-aware truncation with an ASCII ellipsis floor (06 §4.1: `~`).
fn truncate(s: &str, max: usize) -> String {
    let mut width = 0usize;
    let mut out = String::new();
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > max.saturating_sub(1) {
            out.push('~');
            return out;
        }
        width += w;
        out.push(ch);
    }
    out
}
