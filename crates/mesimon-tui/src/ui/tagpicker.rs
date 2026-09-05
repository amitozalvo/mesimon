//! The `^t` picker: every tag on the board, laid out as a grid you steer.
//!
//! One row per group, the tags of that group across it, and a `+` cell at the
//! end of each row for making a new one. The cursor is a cell; `hjkl` walks
//! it, a digit jumps to that group's row and cycles along it, and the actions
//! all apply to the cell under the cursor.
//!
//! It is a panel rather than a one-line hint because the vocabulary is the
//! user's own and mesimon seeds none: the only way a digit can mean anything
//! is for the panel to show what each one currently holds, in the colour it
//! will paint.
//!
//! An axis holds up to `MAX_TAGS_PER_GROUP` names and a row is one terminal
//! line, so a full row does not always fit. It is WINDOWED, not truncated:
//! `window` picks the run of cells around the cursor that fits, and a `~`
//! marks the side that has more behind it — the same marker `text::truncate`
//! uses, for the same reason.

use std::ops::Range;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use mesimon_core::board::MAX_TAGS_PER_GROUP;

use crate::app::App;
use crate::text::EditBuffer;

/// Groups the picker shows. Ten, addressed by `1`–`9` and `0`.
pub(crate) const GROUPS: u8 = 10;

/// What stands in for the cells a row could not fit. `~` is what
/// `text::truncate` already spends on "there is more here than you see".
const MORE: &str = "~";

/// A group's row is shown when it holds something, and the first empty group
/// is shown too so there is always somewhere to make the next tag. Beyond
/// that the rows stay hidden — a board that has never used group 7 should not
/// advertise it.
pub(crate) fn visible_groups(app: &App) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut spare = false;
    for g in 1..=GROUPS {
        if !app.board.group_entries(g).is_empty() {
            out.push(g);
        } else if !spare {
            out.push(g);
            spare = true;
        }
    }
    out
}

/// Cells in a group's row: its tags, plus the trailing `+` unless it is full.
pub(crate) fn row_len(app: &App, group: u8) -> usize {
    let n = app.board.group_entries(group).len();
    if n >= MAX_TAGS_PER_GROUP {
        n
    } else {
        n + 1
    }
}

/// How tall the panel wants to be: one row per visible group inside the frame.
pub(crate) fn height(app: &App) -> u16 {
    visible_groups(app).len() as u16 + 2
}

fn digit_of(group: u8) -> char {
    if group == 10 {
        '0'
    } else {
        char::from(b'0' + group)
    }
}

/// One cell of a row, built before anything is placed: which cells are drawn
/// depends on the total, and the total is not known until they all exist.
#[derive(Default)]
struct Cell {
    spans: Vec<Span<'static>>,
    /// Where the hardware cursor sits inside this cell, while it is being
    /// named. An offset, not a column: the cell does not know yet where the
    /// window will put it.
    cursor: Option<usize>,
}

impl Cell {
    fn width(&self) -> usize {
        super::spans_width(&self.spans)
    }

    fn push(&mut self, span: Span<'static>) {
        self.spans.push(span);
    }
}

/// Which cells of a row get drawn when the row is wider than the panel.
///
/// The cursor's own cell is never the one dropped. The window grows LEFT from
/// it first — the cells already walked past are the ones the eye can most
/// afford to lose, and it puts the cursor at the right edge exactly the way a
/// one-line field scrolls — then right with whatever is left over. It is a
/// function of the cursor and nothing else, so there is no remembered scroll
/// offset that can fall out of step with the board.
fn window(widths: &[usize], cursor: usize, avail: usize) -> Range<usize> {
    let n = widths.len();
    if n == 0 {
        return 0..0;
    }
    if widths.iter().sum::<usize>() <= avail {
        return 0..n;
    }
    let cursor = cursor.min(n - 1);
    // A `~` costs one cell on whichever side still has cells behind it.
    let fits = |first: usize, end: usize| -> bool {
        let body: usize = widths[first..end].iter().sum();
        body + usize::from(first > 0) + usize::from(end < n) <= avail
    };
    let (mut first, mut end) = (cursor, cursor + 1);
    if !fits(first, end) {
        // A single cell too wide for the panel: draw it and let the panel
        // clip it. There is nothing better, and it beats an empty row.
        return first..end;
    }
    while first > 0 && fits(first - 1, end) {
        first -= 1;
    }
    while end < n && fits(first, end + 1) {
        end += 1;
    }
    first..end
}

pub(super) fn draw(f: &mut Frame, area: Rect, app: &App) {
    let Some(arm) = app.tag_armed.as_ref() else { return };
    let theme = &app.theme;
    let worn = app.tag_subject().unwrap_or(&[]);
    let groups = visible_groups(app);

    // Above the footer row, not over it: the footer keeps saying which mode
    // is on while the sheet's own bottom edge names the chord's keys.
    let h = height(app).min(area.height.saturating_sub(1));
    let panel = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(h + 1),
        width: area.width,
        height: h,
    };
    // A bottom sheet, framed: the subject in its top edge — so the panel is
    // never ambiguous about what is being tagged, especially in the composer,
    // where the ticket has no name on the board yet — and its keys in the
    // bottom one.
    let subject = match arm.ticket.and_then(|id| app.board.ticket(id)) {
        Some(t) => crate::text::truncate(&t.title, area.width.saturating_sub(14) as usize),
        None => "new ticket".to_string(),
    };
    let inner = super::dialog::frame(
        f,
        app,
        panel,
        None,
        &theme.rest,
        super::dialog::Edges {
            title: super::dialog::title(&theme.rest, format!("TAGS ∙ {subject}")),
            tail: super::dialog::keys(
                app,
                mesimon_core::keymap::Scope::TagChord,
                &theme.rest,
                (panel.width as usize).saturating_sub(6),
            ),
        },
    );
    let area = inner;

    let mut lines: Vec<Line<'static>> = Vec::new();
    // Where the hardware cursor goes while a name is being typed — the real
    // cursor, not a glyph. The block character that stood in for it was
    // inside the range the L1 law bans, and Ambiguous-width besides.
    let mut cursor: Option<usize> = None;

    for (row, g) in groups.iter().enumerate() {
        let entries = app.board.group_entries(*g);
        // The cell being named, if it is on this row.
        let naming_here = |col: usize| -> Option<&EditBuffer> {
            let (_, buf) = arm.naming.as_ref()?;
            (arm.row == row && arm.col == col).then_some(buf)
        };
        let mut cells: Vec<Cell> = Vec::new();
        for (col, def) in entries.iter().enumerate() {
            let here = arm.row == row && arm.col == col;
            let is_worn = worn.iter().any(|w| w.group == *g && w.name == def.name);
            // The swatch is the tag's own colour — the same paint the card
            // band will use, so the picker is a preview and not a legend.
            let swatch = if theme.paints_tags() {
                Style::default().bg(theme.pip(def.tint() as usize))
            } else {
                Style::default().fg(theme.rest.dim2).add_modifier(Modifier::REVERSED)
            };
            // A worn tag is MARKED, not just coloured: whether this ticket
            // has it is the one thing here that must not be colour-only.
            let mark = if is_worn { '\u{2022}' } else { ' ' };
            let label = if here {
                theme.base().add_modifier(Modifier::BOLD)
            } else if is_worn {
                theme.dim1()
            } else {
                theme.dim2()
            };
            // The cursor is a bracket pair, never a colour: the cells are
            // already spending colour on the tag's own tint.
            let (open, close) = if here { ('[', ']') } else { (' ', ' ') };
            let mut cell = Cell::default();
            cell.push(Span::styled(open.to_string(), theme.base()));
            cell.push(Span::styled("  ".to_string(), swatch));
            // A rename edits the cell WHERE IT SITS: the row keeps its shape
            // and the rest of the vocabulary stays readable beside it. The
            // old form replaced the whole row with a field, which is the
            // "edit mode" this is not supposed to have.
            match naming_here(col) {
                Some(buf) => {
                    cell.cursor = Some(cell.width() + 1 + buf.width_before_cursor());
                    cell.push(Span::styled(format!(" {}", buf.as_str()), theme.base()));
                }
                None => cell.push(Span::styled(format!("{mark}{}", def.name), label)),
            }
            cell.push(Span::styled(close.to_string(), theme.base()));
            cell.push(Span::raw(" ".to_string()));
            cells.push(cell);
        }
        if entries.len() < MAX_TAGS_PER_GROUP {
            let here = arm.row == row && arm.col == entries.len();
            let style = if here { theme.base().add_modifier(Modifier::BOLD) } else { theme.dim3() };
            let mut cell = Cell::default();
            if let Some(buf) = naming_here(entries.len()) {
                // A new tag is typed in the `+ new` slot it will occupy.
                cell.push(Span::styled("[".to_string(), theme.base()));
                cell.cursor = Some(cell.width() + buf.width_before_cursor());
                cell.push(Span::styled(buf.as_str().to_string(), theme.base()));
                cell.push(Span::styled("]".to_string(), theme.base()));
            } else {
                cell.push(Span::styled(
                    if here { "[+ new]".to_string() } else { " + new ".to_string() },
                    style,
                ));
            }
            cells.push(cell);
        }

        let mut spans = vec![Span::styled(
            format!("  {}  ", digit_of(*g)),
            if arm.row == row { theme.base().add_modifier(Modifier::BOLD) } else { theme.dim3() },
        )];
        let widths: Vec<usize> = cells.iter().map(Cell::width).collect();
        let avail = (area.width as usize).saturating_sub(super::spans_width(&spans));
        // A row the cursor is not on has no cell to keep in view, so it is
        // anchored at its start: the digit's first tags are the ones a jump
        // will land on.
        let focus = if arm.row == row { arm.col } else { 0 };
        let win = window(&widths, focus, avail);
        if win.start > 0 {
            spans.push(Span::styled(MORE.to_string(), theme.dim3()));
        }
        for cell in &cells[win.start..win.end] {
            if let Some(off) = cell.cursor {
                cursor = Some(super::spans_width(&spans) + off);
            }
            spans.extend(cell.spans.iter().cloned());
        }
        if win.end < cells.len() {
            spans.push(Span::styled(MORE.to_string(), theme.dim3()));
        }
        let used = super::spans_width(&spans);
        let pad = (area.width as usize).saturating_sub(used);
        spans.push(Span::raw(" ".repeat(pad)));
        lines.push(Line::from(spans));
    }

    let rows = lines.len();
    f.render_widget(Paragraph::new(lines), area);
    if let Some(x) = cursor {
        // The naming row is the group row at `arm.row`, inside the frame.
        let y = area.y + arm.row as u16;
        if arm.row < rows {
            f.set_cursor_position((area.x + (x as u16).min(area.width.saturating_sub(1)), y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::window;

    /// The common case: an axis that fits is drawn whole, wherever the
    /// cursor is, and spends no `~` on either side.
    #[test]
    fn a_row_that_fits_is_drawn_whole() {
        assert_eq!(window(&[9; 4], 0, 80), 0..4);
        assert_eq!(window(&[9; 4], 3, 36), 0..4);
        assert_eq!(window(&[], 0, 80), 0..0);
    }

    /// The point of the window: at ten tags a row can outrun the panel, and
    /// the cell under the cursor is the one thing that may never be the cell
    /// that got dropped.
    #[test]
    fn the_cursor_cell_is_always_drawn() {
        let widths = [11usize, 9, 14, 8, 10, 13, 7, 11, 9, 12];
        for avail in [20usize, 33, 55, 75] {
            for cursor in 0..widths.len() {
                let win = window(&widths, cursor, avail);
                assert!(win.contains(&cursor), "avail {avail}, cursor {cursor} fell off: {win:?}");
            }
        }
    }

    /// What is drawn, plus the `~` each clipped side costs, stays inside the
    /// panel — the window is what keeps the row from overrunning it.
    #[test]
    fn the_window_and_its_markers_fit() {
        let widths = [11usize, 9, 14, 8, 10, 13, 7, 11, 9, 12];
        for avail in [20usize, 33, 55, 75] {
            for cursor in 0..widths.len() {
                let win = window(&widths, cursor, avail);
                let body: usize = widths[win.start..win.end].iter().sum();
                let used = body + usize::from(win.start > 0) + usize::from(win.end < widths.len());
                assert!(used <= avail, "avail {avail}, cursor {cursor}: {win:?} spends {used}");
            }
        }
    }

    /// The row is anchored at its start until the cursor walks past the edge,
    /// and then follows it one cell at a time — a field's scroll, not a jump
    /// that re-centres the row under the eye on every `l`.
    #[test]
    fn the_row_scrolls_by_the_cell() {
        let widths = [10usize; 10];
        // 30 cells of room: three tags plus a `~` on the side with more.
        assert_eq!(window(&widths, 0, 30), 0..2);
        assert_eq!(window(&widths, 1, 30), 0..2);
        assert_eq!(window(&widths, 2, 30), 1..3);
        assert_eq!(window(&widths, 3, 30), 2..4);
        // At the far end the right-hand `~` is not needed, so the two cells
        // that fit are the last two rather than a cell and a stub.
        assert_eq!(window(&widths, 9, 30), 8..10);
    }

    /// A single cell wider than the whole panel still gets drawn: clipped is
    /// bad, blank is worse.
    #[test]
    fn one_oversized_cell_is_still_drawn() {
        assert_eq!(window(&[8, 90, 8], 1, 20), 1..2);
    }
}
