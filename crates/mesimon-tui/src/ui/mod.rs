//! Frame composition. The design laws that bind every draw fn here (06 §0):
//! L1 no drawn structure — structure is painted, never drawn (no U+2500–257F
//! anywhere); L2/L3 the one saturated colour appears only for needs-you;
//! SGR policy 06 §5.1 — no dim/italic/reverse outside Mono/Ansi8, bold only
//! in the five sanctioned places.

mod board;
mod card;
mod chrome;
mod diff;
#[cfg(test)]
mod tests;
mod ticket;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::{App, Mode, Screen};
use crate::layout::{MIN_H, MIN_W};

pub fn draw(f: &mut Frame, app: &App) {
    // Paint the page ground first (a transparent ground would ride the
    // terminal's own theme under a mismatched palette).
    if let Some(bg) = app.theme.bg {
        f.render_widget(Block::default().style(Style::default().bg(bg)), f.area());
    }

    // Live-resize floor (07 §2.4): a notice, never a broken layout.
    let area = f.area();
    if area.width < MIN_W || area.height < MIN_H {
        let msg = format!("mesimon needs {MIN_W}x{MIN_H}; this terminal is {}x{}", area.width, area.height);
        let y = area.height / 2;
        let rect = ratatui::layout::Rect { x: 0, y, width: area.width, height: 1 };
        f.render_widget(
            Paragraph::new(Line::from(msg)).style(app.theme.dim1()).centered(),
            rect,
        );
        return;
    }

    if let Screen::Ticket { ticket, rail_idx } = &app.screen {
        ticket::draw(f, app, *ticket, *rail_idx);
        return;
    }
    if let Screen::Diff { ticket } = &app.screen {
        diff::draw(f, app, *ticket);
        return;
    }

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Length(1), // breathing row (06 §5.5 — chrome never touches content)
            Constraint::Min(3),    // columns
            Constraint::Length(1), // grace / ghost row
            Constraint::Length(1), // status / footer
        ])
        .split(f.area());

    chrome::draw_header(f, outer[0], app);
    board::draw_columns(f, outer[2], app);
    chrome::draw_grace(f, outer[3], app);
    chrome::draw_footer(f, outer[4], app);
    if let Mode::External { idx } = &app.mode {
        chrome::draw_drawer(f, app, *idx);
    }
}
