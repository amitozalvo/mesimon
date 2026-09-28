//! Frame composition. The design laws that bind every draw fn here (06 §0):
//! L1 no drawn structure — structure is painted, never drawn (no U+2500–257F
//! anywhere); L2/L3 the one saturated colour appears only for needs-you;
//! SGR policy 06 §5.1 — no dim/italic/reverse outside Mono/Ansi8, bold only
//! in the five sanctioned places.

mod board;
mod brief;
mod card;
mod chrome;
mod dialog;
pub(crate) mod diff;
mod editor;
mod help;
mod menu;
pub(crate) mod releases;
mod search;
mod tagpicker;
#[cfg(test)]
mod tests;
mod themes;
mod ticket;

pub(crate) use card::CrownMark;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::{App, Mode, Screen};
use crate::layout::{MIN_H, MIN_W};

/// The cells a run of spans occupies.
pub(crate) fn spans_width(spans: &[ratatui::text::Span<'_>]) -> usize {
    use unicode_width::UnicodeWidthStr;
    spans.iter().map(|s| s.content.width()).sum()
}

/// One rendered markdown document, kept across frames (`ticket::rendered`).
pub(crate) struct RichCache {
    pub key: u64,
    pub width: usize,
    pub flavor: crate::theme::Flavor,
    pub rows: std::rc::Rc<Vec<Line<'static>>>,
}

pub fn draw(f: &mut Frame, app: &App) {
    // Paint the page ground first (a transparent ground would ride the
    // terminal's own theme under a mismatched palette).
    if let Some(bg) = app.theme.bg {
        f.render_widget(Block::default().style(Style::default().bg(bg)), f.area());
    }
    // This frame's dialog frames, recorded as they are drawn (`dialog::frame`).
    app.frames.borrow_mut().clear();
    *app.mascot.borrow_mut() = None;
    // And this frame's `Ctx` (`App::frame_ctx`): built on first read.
    app.ctx_dirty();

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

    // The editor's surface is the screen's: over the board it is a dialog,
    // drawn with the board below (the composer's, and since T-163 the
    // description's too — `Tab` on a card); from the ticket page it takes
    // the screen. The picker reaches it either way.
    if let (Mode::Editor(ed), false) = (&app.mode, matches!(app.screen, Screen::Board)) {
        editor::draw(f, app, ed);
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
        if let Mode::Links { links, idx, ticket } = &app.mode {
            dialog::draw_links(f, app, *ticket, links, *idx);
        }
        // The merge dialog (T-431): the m flow's question, wait and answer,
        // over the page whose branch it is about.
        if let Some(d) = &app.merge_dialog {
            dialog::draw_merge(f, app, d);
        }
        if app.help {
            help::draw(f, app);
        }
        return;
    }
    if matches!(app.screen, Screen::Diff) {
        diff::draw(f, app);
        if app.help {
            help::draw(f, app);
        }
        return;
    }
    if let Screen::Releases = &app.screen {
        releases::draw(f, app);
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

    chrome::draw_header(f, outer[0], app, None);
    board::draw_columns(f, outer[2], app);
    chrome::draw_advisory(f, outer[3], app);
    chrome::draw_footer(f, outer[4], app);
    // The composer, grown: a dialog over the cards. The column headers stay
    // above it, a margin of board around it and the footer under it, so the
    // board is still the room.
    if let Mode::Editor(ed) = &app.mode {
        editor::draw_dialog(f, app, ed, outer[2]);
    }
    // Over the board, above the footer: the picker is a panel because the
    // vocabulary is the user's own and a one-line hint cannot show it.
    if app.tag_armed.is_some() {
        tagpicker::draw(f, f.area(), app);
        chrome::draw_footer(f, outer[4], app);
    }
    if let Mode::External { idx } = &app.mode {
        dialog::draw_drawer(f, app, *idx);
    }
    if let Mode::Archived { idx } = &app.mode {
        dialog::draw_archived(f, app, *idx);
    }
    if let Mode::Links { links, idx, ticket } = &app.mode {
        dialog::draw_links(f, app, *ticket, links, *idx);
    }
    if let Mode::Menu { idx } = &app.mode {
        menu::draw(f, app, *idx);
    }
    if matches!(app.mode, Mode::Brief { .. }) {
        brief::draw(f, app);
    }
    if let Mode::Settings { idx } = &app.mode {
        menu::draw_settings(f, app, *idx);
    }
    if let Mode::Notifications { idx } = &app.mode {
        menu::draw_notify(f, app, *idx);
    }
    if matches!(app.mode, Mode::Prompts { .. }) {
        menu::draw_prompts(f, app);
    }
    if matches!(app.mode, Mode::Sharing { .. }) {
        menu::draw_sharing(f, app);
    }
    if matches!(app.mode, Mode::Tiers { .. }) {
        menu::draw_tiers(f, app);
    }
    if matches!(app.mode, Mode::TierEdit { .. }) {
        menu::draw_tier_edit(f, app);
    }
    if matches!(app.mode, Mode::ColumnSettings { .. }) {
        menu::draw_column(f, app);
    }
    if let Mode::Theme { idx, slot } = &app.mode {
        themes::draw(f, app, *idx, *slot);
    }
    // The search picker, over the board it is a view of (T-349). Last of the
    // dialogs and before the overlay: it is the biggest surface here, and
    // nothing else may be open at the same time.
    if let Mode::Search(s) = &app.mode {
        if let Some((x, y)) = search::draw(f, app, s) {
            f.set_cursor_position((x.min(f.area().width.saturating_sub(1)), y));
        }
    }
    // The overlay is the last thing drawn on every screen: it answers a
    // question about whatever is underneath it.
    if app.help {
        help::draw(f, app);
    }
}

/// The footer as plain text at `width` cells, for tests that read what the
/// board offers without rendering a frame.
#[cfg(test)]
pub(crate) fn footer_text(app: &crate::app::App, width: u16) -> String {
    chrome::footer_line(app, width)
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>()
        .trim_end()
        .to_string()
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
