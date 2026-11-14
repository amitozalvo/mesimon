//! The board turning into its table and back, played (T-1240). `b` used to
//! be one frame: the columns gone and the rows already there, with nothing
//! to say where a card went. Now each card's title row slides from where it
//! stood to where it lands, a column a little after the one before it and a
//! card a little after the one above it, and the headers with them. While
//! they travel nothing else is drawn under the board's top row: the open
//! card's rows, a field, the cues come once everything has landed.
//!
//! A card in flight is its own title row as the new view draws it, cut to
//! the width it has reached: from a column to a row the title is revealed as
//! the row widens, and back it is cut as the row narrows. A card that had no
//! place on screen before (scrolled out) or has none after is simply where
//! the new view put it at the end. `Prefs::card_moves` turns it off, with
//! the other cards' travel (T-817).

use std::collections::HashMap;

use ratatui::buffer::Cell;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::App;

/// One card's or header's flight, at most.
pub(crate) const EACH_MS: u64 = 300;
/// Each column starts this long after the one before it.
const COLUMN_STAGGER_MS: u64 = 55;
/// And each card this long after the one above it in its column.
const ROW_STAGGER_MS: u64 = 16;
/// The column stagger stops growing here, so a wide board still lands fast.
const MAX_COLUMN_DELAY_MS: u64 = 220;
/// The row stagger counts this many cards, the rest leave with the last.
const MAX_ROW_STAGGER: u64 = 8;
/// The whole switch, start to every card landed.
pub(crate) const SHIFT_MS: u64 = MAX_COLUMN_DELAY_MS + MAX_ROW_STAGGER * ROW_STAGGER_MS + EACH_MS;

/// Where every card's title row and every column's header row stood on the
/// frame before the switch, and when the switch started.
#[derive(Debug, Clone, Default)]
pub(crate) struct ViewShift {
    pub at: u64,
    pub cards: HashMap<ulid::Ulid, Rect>,
    pub heads: HashMap<String, Rect>,
}

impl ViewShift {
    /// The last frame's places, read off the spots it recorded.
    pub(crate) fn from_spots(spots: &crate::strike::Spots, at: u64) -> Self {
        ViewShift {
            at,
            cards: spots
                .cards
                .iter()
                .map(|c| (c.id, Rect { x: c.x, y: c.y, width: c.width, height: 1 }))
                .collect(),
            heads: spots.head_rows.iter().cloned().collect(),
        }
    }

    pub(crate) fn playing(&self, now: u64) -> bool {
        now.saturating_sub(self.at) < SHIFT_MS
    }
}

/// When the item at column `ci`, row `ri` (`None` for the header) starts.
pub(crate) fn delay(ci: usize, ri: Option<usize>) -> u64 {
    let col = (ci as u64 * COLUMN_STAGGER_MS).min(MAX_COLUMN_DELAY_MS);
    let row = ri.map_or(0, |r| (r as u64 + 1).min(MAX_ROW_STAGGER) * ROW_STAGGER_MS);
    col + row
}

/// How far along its flight an item is, eased, 0 to 1.
pub(crate) fn progress(elapsed: u64, delay: u64) -> f32 {
    let t = (elapsed.saturating_sub(delay) as f32 / EACH_MS as f32).clamp(0.0, 1.0);
    // Ease in and out, cubic.
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// The rectangle between `from` and `to` at `p`, a row tall.
pub(crate) fn between(from: Rect, to: Rect, p: f32) -> Rect {
    let lerp = |a: u16, b: u16| (f32::from(a) + (f32::from(b) - f32::from(a)) * p).round() as u16;
    Rect {
        x: lerp(from.x, to.x),
        y: lerp(from.y, to.y),
        width: lerp(from.width, to.width),
        height: 1,
    }
}

/// Over the view just drawn in `area`, the switch's frame: every row the
/// view drew is taken off, and each card's and header's row is put back
/// where its flight has it. Drawn after the board, before the bolts.
pub(crate) fn draw(f: &mut Frame, app: &App, area: Rect) {
    let shift = app.view_shift.borrow();
    let Some(shift) = shift.as_ref() else { return };
    let now = (app.now)();
    if !shift.playing(now) {
        return;
    }
    let elapsed = now.saturating_sub(shift.at);
    let cols = app.columns();
    // Every card's column and row, read off the board once for the frame.
    let places: HashMap<ulid::Ulid, (usize, usize)> = cols
        .iter()
        .enumerate()
        .flat_map(|(ci, name)| {
            let tickets = app.board.column_tickets(name);
            tickets.into_iter().enumerate().map(move |(ri, t)| (t.id, (ci, ri)))
        })
        .collect();
    // What this frame drew, and where each item flies from.
    let mut items: Vec<(Rect, Rect, u64)> = Vec::new();
    {
        let spots = app.spots.borrow();
        for c in &spots.cards {
            let Some(&from) = shift.cards.get(&c.id) else { continue };
            let to = Rect { x: c.x, y: c.y, width: c.width, height: 1 };
            let (ci, ri) = places.get(&c.id).copied().unwrap_or((0, 0));
            items.push((from, to, delay(ci, Some(ri))));
        }
        for (name, to) in &spots.head_rows {
            let Some(&from) = shift.heads.get(name) else { continue };
            let ci = cols.iter().position(|c| c == name).unwrap_or(0);
            items.push((from, *to, delay(ci, None)));
        }
    }
    let buf = f.buffer_mut();
    let rows: Vec<Vec<Cell>> = items
        .iter()
        .map(|(_, to, _)| {
            (to.x..to.right().min(buf.area.right()))
                .map(|x| buf[(x, to.y.min(buf.area.bottom().saturating_sub(1)))].clone())
                .collect()
        })
        .collect();
    let ground = app.theme.bg;
    let clear = |cell: &mut Cell| {
        cell.reset();
        if let Some(bg) = ground {
            cell.set_bg(bg);
        }
    };
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            clear(&mut buf[(x, y)]);
        }
    }
    for ((from, to, d), row) in items.iter().zip(rows) {
        let at = between(*from, *to, progress(elapsed, *d));
        if at.y < area.y || at.y >= area.bottom() {
            continue;
        }
        let width = usize::from(at.width).min(row.len());
        for (i, cell) in row.into_iter().take(width).enumerate() {
            let x = at.x + i as u16;
            if x >= area.x && x < area.right() {
                buf[(x, at.y)] = cell;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_leaves_after_the_one_before_it_and_a_card_after_the_one_above() {
        assert!(delay(1, None) > delay(0, None));
        assert!(delay(0, Some(1)) > delay(0, Some(0)));
        assert!(delay(0, Some(0)) > delay(0, None), "the header leads its cards");
        assert!(delay(40, Some(40)) + EACH_MS <= SHIFT_MS, "a wide board lands inside the switch");
    }

    #[test]
    fn the_flight_starts_at_the_old_place_and_lands_on_the_new() {
        let from = Rect { x: 3, y: 4, width: 25, height: 1 };
        let to = Rect { x: 1, y: 10, width: 110, height: 1 };
        assert_eq!(between(from, to, progress(0, 0)), from);
        assert_eq!(between(from, to, progress(EACH_MS, 0)), to);
        let mid = between(from, to, progress(EACH_MS / 2, 0));
        assert!(mid.y > from.y && mid.y < to.y && mid.width > 25 && mid.width < 110, "{mid:?}");
        assert_eq!(between(from, to, progress(EACH_MS, EACH_MS)), from, "not yet started");
    }
}
