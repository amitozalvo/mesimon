//! A picked link's look (T-1192), one pass over the finished frame for every
//! surface that picks one: the ticket page's conversation, the transcript
//! dialog and the note reader (the author: "consolidate them"). The
//! renderers mark the pick's cells (`rich::PICK_MARK`) and know nothing
//! more; this pass finds the mark wherever it was drawn and paints it.
//!
//! While a pick is live its words are inverted under a slow crest
//! (`Theme::link_pick`) and every other cell but the footer's recedes, so
//! the state cannot be forgotten and the footer's keys are the ones that
//! read. Acted on (Enter or `c`), the pick flashes up to the brightest ink,
//! holds, and eases back while the frame comes forward again
//! (`Theme::link_ack`, `Theme::pick_recess`); then it is let go.

use ratatui::layout::Position;
use ratatui::style::{Color, Modifier};
use ratatui::Frame;

use crate::app::App;
use crate::theme::Theme;

pub(super) fn paint(f: &mut Frame, app: &App) {
    let area = f.area();
    let drawn = app.pick_drawn();
    let theme = &app.theme;
    let buf = f.buffer_mut();
    // The mark is stripped from every cell that wears it, drawn or not: it
    // is the renderers' word to this pass, never the terminal's.
    let mut picked: Vec<Position> = Vec::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if cell.modifier.contains(Modifier::UNDERLINED)
                && cell.underline_color == crate::rich::PICK_MARK
            {
                cell.modifier.remove(Modifier::UNDERLINED);
                cell.underline_color = Color::Reset;
                picked.push(Position { x, y });
            }
        }
    }
    let Some(ack) = drawn else { return };
    if picked.is_empty() {
        return;
    }
    // The footer row is the pick's keys: it stays as drawn. So does the
    // card a picked ticket link names (T-820): the board blinks it out of
    // its own recess behind the dialog, and that blink is how the board
    // says which card the link is.
    let named = app.link_hovered().and_then(|id| {
        app.spots.borrow().card(id).map(|c| ratatui::layout::Rect {
            x: c.x,
            y: c.y,
            width: c.width,
            height: c.rows,
        })
    });
    let recess = Theme::pick_recess(ack);
    if recess > 0.0 {
        let foot = area.bottom().saturating_sub(1);
        for y in area.top()..foot {
            for x in area.left()..area.right() {
                if picked.binary_search_by(|p| (p.y, p.x).cmp(&(y, x))).is_ok()
                    || named.is_some_and(|r| r.contains(Position { x, y }))
                {
                    continue;
                }
                let cell = &mut buf[(x, y)];
                cell.fg = theme.recede_by(cell.fg, recess);
                if cell.bg != Color::Reset && Some(cell.bg) != theme.bg {
                    cell.bg = theme.recede_ground_k(cell.bg, recess);
                }
                if recess >= 0.5 {
                    cell.modifier.remove(Modifier::BOLD);
                }
            }
        }
    }
    let now = (app.now)();
    let cells = picked.len();
    for (i, at) in picked.into_iter().enumerate() {
        let cell = &mut buf[at];
        let style = match ack {
            None => theme.link_pick(now, cells, i),
            Some(ms) => theme.link_ack(ms, cell.fg, cell.bg),
        };
        cell.set_style(style);
    }
}
