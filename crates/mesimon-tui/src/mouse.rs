//! The pointer (T-716): what the last frame put where, so a click, the
//! wheel and a hover land on exactly what the person sees.
//!
//! The draw records a [`Spot`] for everything a pointer can stand on — a
//! card, a column's header, a row of the open list — and `ui::draw` adds
//! every key hint left visible on the finished frame (`scrape_hints`), so
//! a hint that is drawn is a hint that clicks, on any surface that spells
//! one through `chrome::hint_spans`. Nothing here acts: `App::on_mouse`
//! turns a spot into the keypress it stands for, through the same keymap a
//! key goes through, so a click can do nothing a key could not.
//!
//! Dialogs stack: each frame `ui::dialog::frame` draws opens a layer, and
//! only the top layer's spots answer. A click off the top dialog is that
//! dialog's Esc — except on the footer's own keys, which stay reachable
//! under any dialog.

use std::cell::RefCell;

use mesimon_core::keymap::{HeaderChip, Key};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// A cell on screen, `(x, y)`.
pub(crate) type At = (u16, u16);

/// What a spot stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    /// A key hint: a click presses the key.
    Key(Key),
    /// A row of the open list dialog, by the list's own index.
    Row(usize),
    /// A board card.
    Card(ulid::Ulid),
    /// A board column's header (or the whole spine of a folded column).
    Column(usize),
    /// A board column's whole body, for the wheel.
    Lane(usize),
    /// A chip on the board's top row.
    Chip(HeaderChip),
    /// A ticket page's rail row, by `rail_idx`.
    Rail(usize),
    /// A tag picker cell, by its row and its cell along it.
    Tag(usize, usize),
    /// A diff's file row, by `file_idx`.
    File(usize),
    /// The row of the one-line field being typed in, and the cell its
    /// cursor stands on: a click moves the cursor by the cells between.
    Field { cursor_x: u16 },
    /// The composer's title: cell `x0` shows the title from `skip` cells in.
    Title { x0: u16, skip: u16 },
    /// A note body: cell `(x0, y0)` is displayed row `top`'s first cell.
    Body { x0: u16, y0: u16, top: u16 },
}

impl Target {
    /// Whether a hover lights it. A lane is only where the wheel turns, and
    /// text being typed is not a button.
    fn lit(self) -> bool {
        !matches!(
            self,
            Target::Lane(_) | Target::Field { .. } | Target::Title { .. } | Target::Body { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Spot {
    pub rect: Rect,
    pub target: Target,
    /// How many dialogs were open when it was drawn.
    pub layer: usize,
}

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    Spot(Spot),
    /// Off the top dialog: a click there closes it.
    Outside,
    Nothing,
}

/// One frame's spots and dialog layers. `ui::draw` empties it first.
#[derive(Debug, Default)]
pub(crate) struct Map {
    /// The frame's whole area: its last row is the footer.
    area: Rect,
    spots: Vec<Spot>,
    /// Each open dialog's frames, bottom first.
    layers: Vec<Vec<Rect>>,
}

impl Map {
    pub fn clear(&mut self, area: Rect) {
        self.area = area;
        self.spots.clear();
        self.layers.clear();
    }

    /// A dialog frame was drawn: a new layer over everything before it.
    pub fn open(&mut self, frame: Rect) {
        self.layers.push(vec![frame]);
    }

    /// The last frame drawn is part of the dialog under it, not a dialog
    /// of its own — the search picker's preview beside its list.
    pub fn join_top(&mut self) {
        if self.layers.len() >= 2 {
            let top = self.layers.pop().unwrap_or_default();
            if let Some(below) = self.layers.last_mut() {
                below.extend(top);
            }
        }
    }

    /// `target` covers `rect`, on whatever layer is open now.
    pub fn record(&mut self, rect: Rect, target: Target) {
        if rect.width > 0 && rect.height > 0 {
            self.spots.push(Spot { rect, target, layer: self.layers.len() });
        }
    }

    /// The topmost layer whose frames hold `p`, or 0 for the page.
    fn layer_at(&self, p: Position) -> usize {
        self.layers.iter().rposition(|l| l.iter().any(|r| r.contains(p))).map_or(0, |i| i + 1)
    }

    /// `target` covers `rect`, on the layer drawn there — for a spot found
    /// on the finished frame, after every layer is open.
    fn record_found(&mut self, rect: Rect, target: Target) {
        let layer = self.layer_at(rect.as_position());
        self.spots.push(Spot { rect, target, layer });
    }

    /// What `(x, y)` is over. The screen's last row is the footer, whose
    /// keys answer under any dialog.
    pub fn at(&self, x: u16, y: u16) -> Hit {
        let footer_y = self.area.bottom().saturating_sub(1);
        let p = Position { x, y };
        let top = self.layers.len();
        let find = |ok: &dyn Fn(&Spot) -> bool| {
            self.spots.iter().rev().find(|s| s.rect.contains(p) && ok(s)).copied()
        };
        if top == 0 {
            return find(&|_| true).map_or(Hit::Nothing, Hit::Spot);
        }
        if self.layers[top - 1].iter().any(|r| r.contains(p)) {
            return find(&|s| s.layer == top).map_or(Hit::Nothing, Hit::Spot);
        }
        if y == footer_y {
            if let Some(s) = find(&|s| s.layer == 0 && matches!(s.target, Target::Key(_))) {
                return Hit::Spot(s);
            }
        }
        Hit::Outside
    }

    /// Whether any dialog is open.
    pub fn dialog(&self) -> bool {
        !self.layers.is_empty()
    }
}

thread_local! {
    /// The hints this frame's draw spelled (`note_hint`), each as the text
    /// it put on screen and the key a click on it presses.
    static HINTS: RefCell<Vec<(String, Key)>> = const { RefCell::new(Vec::new()) };
}

/// A hint was spelled: `show` in bold, then ` word`. `chrome::hint_spans`
/// calls this for every binding whose hint a click can press.
pub(crate) fn note_hint(show: &str, word: &str, key: Key) {
    HINTS.with(|h| h.borrow_mut().push((format!("{show} {word}"), key)));
}

/// Forget the hints of the frame before.
pub(crate) fn forget_hints() {
    HINTS.with(|h| h.borrow_mut().clear());
}

/// Every hint this frame spelled that is still whole on `buf` — not cut by
/// a frame's edge, not drawn over by a dialog — recorded as a spot over
/// its key and its word. A hint's key is bold (06 §5.1 clause 3), which is
/// what tells it from the same words in running text.
pub(crate) fn scrape_hints(buf: &Buffer, map: &mut Map) {
    let mut hints = HINTS.with(|h| std::mem::take(&mut *h.borrow_mut()));
    hints.sort_by(|a, b| a.0.cmp(&b.0));
    hints.dedup_by(|a, b| a.0 == b.0);
    if hints.is_empty() {
        return;
    }
    let area = buf.area;
    for y in area.top()..area.bottom() {
        // The row as text, and the cell each char starts at.
        let mut text = String::new();
        let mut cells: Vec<(usize, u16)> = Vec::new();
        for x in area.left()..area.right() {
            let sym = buf[(x, y)].symbol();
            if sym.is_empty() {
                continue;
            }
            cells.push((text.len(), x));
            text.push_str(sym);
        }
        let cell_at = |byte: usize| cells.iter().find(|(b, _)| *b == byte).map(|(_, x)| *x);
        for (words, key) in &hints {
            let mut from = 0;
            while let Some(i) = text[from..].find(words.as_str()) {
                let start = from + i;
                let end = start + words.len();
                from = end;
                let before = text[..start].chars().next_back();
                let after = text[end..].chars().next();
                // A whole hint: nothing word-like either side of it.
                if before.is_some_and(char::is_alphanumeric)
                    || after.is_some_and(char::is_alphanumeric)
                {
                    continue;
                }
                let Some(x) = cell_at(start) else { continue };
                if !buf[(x, y)].modifier.contains(Modifier::BOLD) {
                    continue;
                }
                let width = u16::try_from(words.width()).unwrap_or(u16::MAX);
                let rect = Rect { x, y, width, height: 1 }.intersection(area);
                map.record_found(rect, Target::Key(*key));
            }
        }
    }
}

/// The hover (T-716): the spot under the pointer, its ink one step up the
/// ramp and — where the theme paints a page ground in true colour — its
/// ground halfway to the cursor card's surface, so the row reads as
/// reachable without reading as selected. Cells on a surface of their own
/// (the cursor's row, a tag's bar, the needs-you row) keep it.
pub(crate) fn paint_hover(buf: &mut Buffer, map: &Map, at: (u16, u16), theme: &Theme) {
    let Hit::Spot(spot) = map.at(at.0, at.1) else { return };
    if !spot.target.lit() {
        return;
    }
    let ground = theme.hover_bg();
    let rect = spot.rect.intersection(buf.area);
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            let cell = &mut buf[(x, y)];
            if let Some(fg) = theme.lift(cell.fg) {
                cell.fg = fg;
            }
            if let Some(g) = ground {
                if cell.bg == Color::Reset || Some(cell.bg) == theme.bg {
                    cell.bg = g;
                }
            }
        }
    }
}

/// The text selection (T-716): the cells from `a` to `b` in reading order,
/// the way a terminal selects — the first row from `a` on, the rows between
/// whole, the last row up to `b`. Painted on the cursor's surface (bold
/// where the profile paints none), and returned as the text it covers, one
/// line a row, trailing blanks trimmed.
pub(crate) fn paint_selection(
    buf: &mut Buffer,
    a: (u16, u16),
    b: (u16, u16),
    theme: &Theme,
) -> String {
    let area = buf.area;
    let (from, to) = if (a.1, a.0) <= (b.1, b.0) { (a, b) } else { (b, a) };
    let mut style = theme.selected_row();
    if style == ratatui::style::Style::default() {
        style = style.add_modifier(Modifier::BOLD);
    }
    let mut rows: Vec<String> = Vec::new();
    for y in from.1.max(area.top())..=to.1.min(area.bottom().saturating_sub(1)) {
        let first = if y == from.1 { from.0 } else { area.left() };
        let last = if y == to.1 { to.0 } else { area.right().saturating_sub(1) };
        let mut row = String::new();
        for x in first.max(area.left())..=last.min(area.right().saturating_sub(1)) {
            let cell = &mut buf[(x, y)];
            row.push_str(cell.symbol());
            cell.set_style(style);
        }
        rows.push(row.trim_end().to_string());
    }
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, width: w, height: h }
    }

    #[test]
    fn the_top_dialog_answers_and_a_click_off_it_is_outside() {
        let mut m = Map::default();
        m.clear(rect(0, 0, 30, 10));
        m.record(rect(0, 0, 10, 1), Target::Card(ulid::Ulid::nil()));
        m.record(rect(0, 9, 10, 1), Target::Key(Key::Char('?')));
        m.open(rect(2, 2, 6, 5));
        m.record(rect(3, 3, 4, 1), Target::Row(0));
        assert_eq!(
            m.at(4, 3),
            Hit::Spot(Spot { rect: rect(3, 3, 4, 1), target: Target::Row(0), layer: 1 })
        );
        assert_eq!(m.at(4, 5), Hit::Nothing, "inside the frame, on nothing");
        assert_eq!(m.at(0, 0), Hit::Outside, "a card under a dialog is not there");
        assert!(
            matches!(m.at(1, 9), Hit::Spot(Spot { target: Target::Key(_), .. })),
            "the footer's keys stay"
        );
        m.clear(rect(0, 0, 30, 10));
        assert!(matches!(m.at(0, 0), Hit::Nothing));
    }

    #[test]
    fn a_joined_frame_is_part_of_the_dialog_under_it() {
        let mut m = Map::default();
        m.clear(rect(0, 0, 30, 10));
        m.open(rect(0, 0, 5, 5));
        m.record(rect(1, 1, 3, 1), Target::Row(2));
        m.open(rect(6, 0, 5, 5));
        m.join_top();
        assert!(matches!(m.at(2, 1), Hit::Spot(Spot { target: Target::Row(2), .. })));
        assert_eq!(m.at(7, 1), Hit::Nothing, "the preview is inside the dialog");
        assert_eq!(m.at(12, 1), Hit::Outside);
    }

    #[test]
    fn a_hint_clicks_only_where_it_is_drawn_whole_and_bold() {
        use ratatui::style::Style;
        let mut buf = Buffer::empty(rect(0, 0, 30, 2));
        buf.set_string(1, 0, "e", Style::default().add_modifier(Modifier::BOLD));
        buf.set_string(2, 0, " edit ∙ ", Style::default());
        // The same words in running text, not bold: not a hint.
        buf.set_string(1, 1, "e edit", Style::default());
        forget_hints();
        note_hint("e", "edit", Key::Char('e'));
        let mut m = Map::default();
        m.clear(rect(0, 0, 30, 10));
        scrape_hints(&buf, &mut m);
        assert!(matches!(
            m.at(3, 0),
            Hit::Spot(Spot {
                target: Target::Key(Key::Char('e')),
                rect: Rect { x: 1, width: 6, .. },
                ..
            })
        ));
        assert_eq!(m.at(3, 1), Hit::Nothing);
    }
}
