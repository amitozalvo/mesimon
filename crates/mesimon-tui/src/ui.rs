//! Board renderer — the D19 baseline identity from day one: no card borders,
//! 1-char accent bar, a 4-step grey ramp, exactly one saturated colour reserved
//! for "needs you" (unused in M1 — no attention yet — but the slot exists).

use mesimon_core::board::SessionState;
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
        let style = if i == idx {
            Style::default().fg(FG).add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(FG)
        };
        lines.push(Line::from(Span::styled(
            format!("  {} {kind} · {}  ", if i == idx { "▸" } else { " " }, &s.sid16()[..8]),
            style,
        )));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        " jk · Enter focus · x kill · Esc",
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
    let live = app
        .board
        .sessions
        .iter()
        .filter(|s| matches!(s.state, SessionState::Running | SessionState::Spawning))
        .count();
    let line = Line::from(vec![
        Span::styled("  mesimon", Style::default().fg(FG).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {repo}"), Style::default().fg(DIM)),
        Span::styled(format!("   {live} live"), Style::default().fg(DIM)),
    ]);
    f.render_widget(Paragraph::new(line), area);
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

    let render_row = |lines: &mut Vec<Line>, t: &mesimon_core::board::Ticket, selected: bool, ghosted: bool| {
        let live = app
            .board
            .sessions
            .iter()
            .filter(|s| s.ticket == t.id && matches!(s.state, SessionState::Running | SessionState::Spawning))
            .count();
        let accent = if ghosted {
            GHOST
        } else if live > 0 {
            ACCENT_LIVE
        } else {
            ACCENT_IDLE
        };
        let dots = if live > 0 { format!(" {}", "•".repeat(live.min(4))) } else { String::new() };
        let width = (area.width as usize).saturating_sub(3 + dots.len() + 1);
        let title = truncate(&t.title, width);
        let style = if selected {
            Style::default().fg(FG).add_modifier(Modifier::REVERSED)
        } else if ghosted {
            Style::default().fg(GHOST)
        } else {
            Style::default().fg(FG)
        };
        lines.push(Line::from(vec![
            Span::styled("▌ ", Style::default().fg(accent)),
            Span::styled(title, style),
            Span::styled(dots, Style::default().fg(DIM)),
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
            " PICK  jk select · Enter focus · x kill · Esc back",
            Style::default().fg(FG).add_modifier(Modifier::REVERSED),
        )),
        Mode::Normal => {
            if app.status.is_empty() {
                Line::from(Span::styled(
                    " BOARD  hjkl · o new · r rename · d delete · u undo · m move · s claude · S bash · Enter open · q quit",
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
