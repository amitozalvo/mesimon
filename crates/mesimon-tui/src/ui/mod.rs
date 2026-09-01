//! Frame composition. The design laws that bind every draw fn here (06 §0):
//! L1 no drawn structure — structure is painted, never drawn (no U+2500–257F
//! anywhere); L2/L3 the one saturated colour appears only for needs-you;
//! SGR policy 06 §5.1 — no dim/italic/reverse outside Mono/Ansi8, bold only
//! in the five sanctioned places.

mod board;
mod card;
mod chrome;
pub(crate) mod diff;
mod help;
mod menu;
mod tagpicker;
#[cfg(test)]
mod tests;
mod themes;
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
        let msg = format!(
            "mesimon needs {MIN_W}x{MIN_H}; this terminal is {}x{}",
            area.width, area.height
        );
        let y = area.height / 2;
        let rect = ratatui::layout::Rect { x: 0, y, width: area.width, height: 1 };
        f.render_widget(Paragraph::new(Line::from(msg)).style(app.theme.dim1()).centered(), rect);
        return;
    }

    if let Screen::Ticket { ticket, rail_idx } = &app.screen {
        ticket::draw(f, app, *ticket, *rail_idx);
        // `^t` is bound on this screen too, so the panel has to reach it —
        // the footer already flips to the chord's hints, and a footer naming
        // keys over a grid that is not drawn is worse than no picker at all.
        if app.tag_armed.is_some() {
            let area = f.area();
            tagpicker::draw(f, area, app);
            chrome::draw_footer(
                f,
                ratatui::layout::Rect {
                    x: area.x,
                    y: area.y + area.height - 1,
                    width: area.width,
                    height: 1,
                },
                app,
            );
        }
        if app.help {
            help::draw(f, app);
        }
        return;
    }
    if let Screen::Diff { ticket } = &app.screen {
        diff::draw(f, app, *ticket);
        if app.help {
            help::draw(f, app);
        }
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
    chrome::draw_advisory(f, outer[3], app);
    chrome::draw_footer(f, outer[4], app);
    // Over the board, above the footer: the picker is a panel because the
    // vocabulary is the user's own and a one-line hint cannot show it.
    if app.tag_armed.is_some() {
        tagpicker::draw(f, f.area(), app);
        chrome::draw_footer(f, outer[4], app);
    }
    if let Mode::External { idx } = &app.mode {
        chrome::draw_drawer(f, app, *idx);
    }
    if let Mode::Archived { idx } = &app.mode {
        chrome::draw_archived(f, app, *idx);
    }
    if let Mode::Menu { idx } = &app.mode {
        menu::draw(f, app, *idx);
    }
    if let Mode::Theme { idx } = &app.mode {
        themes::draw(f, app, *idx);
    }
    // The overlay is the last thing drawn on every screen: it answers a
    // question about whatever is underneath it.
    if app.help {
        help::draw(f, app);
    }
}

/// The picker's visible group rows, and how many cells one holds. Both live
/// in the panel module (it owns the layout) and are re-exported here so the
/// key handler steers by exactly what is drawn.
pub(crate) fn tag_rows(app: &crate::app::App) -> Vec<u8> {
    tagpicker::visible_groups(app)
}

pub(crate) fn tag_row_len(app: &crate::app::App, group: u8) -> usize {
    tagpicker::row_len(app, group)
}
