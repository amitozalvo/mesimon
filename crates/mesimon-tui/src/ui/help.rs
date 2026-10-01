//! The `?` overlay — the answer to "what can I press here".
//!
//! Rendered entirely from `keymap::overlay`, so it lists exactly what works on
//! this screen in this state and nothing else: no key that would do nothing,
//! no key the footer had no room for left out. This is the surface the audit
//! found missing, and the reason the keymap became data. Framed (`dialog`),
//! with its groups laid side by side: of every way to cut them into columns,
//! in order, it takes the shortest that fits the terminal's width (T-535). It
//! used to be one long column until that ran out of rows, so a short
//! terminal saw two columns and a tall one a ribbon down the middle.

use std::ops::Range;

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap;

use crate::app::App;

use super::dialog;

/// Floor for the key column, so the hints line up in one rail and the rail
/// sits in the same place on every screen. It is a floor and not the width:
/// a spelling wider than this (`shift+enter`, `shift+tab`) pushes the rail
/// out rather than closing the gap, which is what `> <` used to hide — every
/// key was short enough that nothing tested the arithmetic.
const KEY_W: usize = 10;
/// The gap between two columns: with a heading's own one-cell inset, a hint
/// ends four cells before the next column's heading starts.
const GAP: usize = 3;

pub(super) fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let scope = app.scope();
    let groups = keymap::overlay(scope, &app.frame_ctx());
    if groups.is_empty() {
        return;
    }

    // One rail for the whole overlay, wide enough for the widest spelling in
    // it plus a space — a key that fills the column exactly must still not
    // touch its hint.
    let rail = KEY_W.max(
        groups
            .iter()
            .flat_map(|(_, rows)| rows.iter())
            .map(|(key, _)| key.width() + 1)
            .max()
            .unwrap_or(0),
    );
    // Each group as a block: its heading, then its rows.
    let blocks: Vec<Vec<Line<'static>>> = groups
        .iter()
        .map(|(group, rows)| {
            let mut block = vec![Line::from(Span::styled(
                format!(" {}", group.title()),
                theme.dim2().add_modifier(Modifier::BOLD),
            ))];
            for (key, hint) in rows {
                let pad = rail.saturating_sub(key.width());
                block.push(Line::from(vec![
                    Span::styled(format!("   {key}"), theme.base()),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(hint.to_string(), theme.dim1()),
                ]));
            }
            block
        })
        .collect();
    let title = format!("KEYS ∙ {}", scope.word().to_lowercase());

    let screen = f.area();
    let runs = split(&blocks, screen.width as usize, title.width() + 4);
    let rows = runs.iter().map(|run| height(&blocks[run.clone()])).max().unwrap_or(0) as u16;
    let widths: Vec<usize> = runs.iter().map(|run| width(&blocks[run.clone()])).collect();
    let mut blocks = blocks.into_iter();
    let columns: Vec<Vec<Line<'static>>> = runs
        .iter()
        .map(|run| {
            let mut col: Vec<Line<'static>> = Vec::new();
            for block in blocks.by_ref().take(run.len()) {
                if !col.is_empty() {
                    col.push(Line::default());
                }
                col.extend(block);
            }
            col
        })
        .collect();
    let width = inner_width(&widths).max(title.width() + 4) as u16;
    let area = dialog::centred(screen, rows, width);
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        dialog::Edges { title: dialog::title(&theme.rest, title), tail: Vec::new() },
    );
    let mut x = inner.x;
    for (mut lines, w) in columns.into_iter().zip(widths) {
        // What does not fit is cut, and the cut is marked: a list that ends
        // in a taller terminal must not look complete in a shorter one.
        let h = inner.height as usize;
        if lines.len() > h && h > 0 {
            lines.truncate(h);
            lines[h - 1] = Line::from(Span::styled("   ~", theme.dim2()));
        }
        let right = inner.x + inner.width;
        if x >= right {
            break;
        }
        f.render_widget(
            Paragraph::new(lines),
            Rect { x, y: inner.y, width: (w as u16).min(right - x), height: inner.height },
        );
        x = x.saturating_add(w as u16 + GAP as u16);
    }
}

/// How the groups split into columns: contiguous runs, in the overlay's
/// order, so the groups read down one column and on into the next. Of the
/// splits whose dialog fits `screen_w`, the one with the fewest rows; a tie
/// goes to fewer columns (a column is added only when it makes the list
/// shorter) and then to the narrower dialog. When not even one column fits,
/// one column, clipped.
fn split(blocks: &[Vec<Line<'static>>], screen_w: usize, floor: usize) -> Vec<Range<usize>> {
    let n = blocks.len();
    // Six groups at most, so every split is cheap to try: bit i of the mask
    // cuts after group i.
    (0..1u32 << n.saturating_sub(1))
        .map(|mask| {
            let mut runs = Vec::new();
            let mut start = 0;
            for i in 0..n {
                if i + 1 == n || mask & (1 << i) != 0 {
                    runs.push(start..i + 1);
                    start = i + 1;
                }
            }
            runs
        })
        .filter(|runs| {
            let widths: Vec<usize> = runs.iter().map(|run| width(&blocks[run.clone()])).collect();
            // `dialog::centred` keeps two cells of screen beside each edge.
            inner_width(&widths).max(floor) + 2 + 4 <= screen_w
        })
        .min_by_key(|runs| {
            let rows = runs.iter().map(|run| height(&blocks[run.clone()])).max().unwrap_or(0);
            let w: usize = runs.iter().map(|run| width(&blocks[run.clone()])).sum();
            (rows, runs.len(), w)
        })
        .unwrap_or_else(|| std::iter::once(0..n).collect())
}

/// A column's rows: its groups, a blank row between two.
fn height(blocks: &[Vec<Line<'static>>]) -> usize {
    blocks.iter().map(|b| b.len() + 1).sum::<usize>().saturating_sub(1)
}

/// A column's width: its widest line.
fn width(blocks: &[Vec<Line<'static>>]) -> usize {
    blocks.iter().flatten().map(Line::width).max().unwrap_or(0)
}

/// The dialog's inner width for these columns: the gaps between them, and
/// one cell after the last to match the heading's inset on the left.
fn inner_width(widths: &[usize]) -> usize {
    widths.iter().sum::<usize>() + GAP * widths.len().saturating_sub(1) + 1
}
