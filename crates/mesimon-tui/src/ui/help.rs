//! The `?` overlay — the answer to "what can I press here".
//!
//! Rendered entirely from `keymap::overlay`, so it lists exactly what works on
//! this screen in this state and nothing else: no key that would do nothing,
//! no key the footer had no room for left out. This is the surface the audit
//! found missing, and the reason the keymap became data. Framed (`dialog`),
//! and in TWO columns when one would not fit the terminal's height — the APP
//! group used to fall off the bottom of a 30-row terminal without a word.

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
/// a spelling wider than this (`option+hjkl`, `shift+enter`) pushes the rail
/// out rather than closing the gap, which is what `> <` used to hide — every
/// key was short enough that nothing tested the arithmetic.
const KEY_W: usize = 10;
/// One column's inner width, and the whole dialog's when there is one.
const COL_W: u16 = 52;
/// The gap between two columns.
const GAP: u16 = 2;

pub(super) fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let scope = app.scope();
    let groups = keymap::overlay(scope, &app.ctx());
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
    let total: usize = blocks.iter().map(|b| b.len() + 1).sum::<usize>() - 1;

    let screen = f.area();
    let room = screen.height.saturating_sub(4) as usize;
    let two = total > room && screen.width >= 2 * COL_W + GAP + 6;
    let columns: Vec<Vec<Line<'static>>> = if two {
        // Fill the left column to half the rows, whole groups only.
        let mut left: Vec<Line<'static>> = Vec::new();
        let mut right: Vec<Line<'static>> = Vec::new();
        for block in blocks {
            let target = if left.is_empty() || left.len() + block.len() <= total.div_ceil(2) {
                &mut left
            } else {
                &mut right
            };
            if !target.is_empty() {
                target.push(Line::default());
            }
            target.extend(block);
        }
        vec![left, right]
    } else {
        let mut one: Vec<Line<'static>> = Vec::new();
        for block in blocks {
            if !one.is_empty() {
                one.push(Line::default());
            }
            one.extend(block);
        }
        vec![one]
    };
    let rows = columns.iter().map(Vec::len).max().unwrap_or(0) as u16;
    let width = if two { 2 * COL_W + GAP } else { COL_W };
    let area = dialog::centred(screen, rows, width);
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        dialog::Edges {
            title: dialog::title(&theme.rest, format!("KEYS ∙ {}", scope.word().to_lowercase())),
            tail: Vec::new(),
        },
    );
    let col_w = if two { inner.width.saturating_sub(GAP) / 2 } else { inner.width };
    for (i, mut lines) in columns.into_iter().enumerate() {
        // What does not fit is cut, and the cut is marked: a list that ends
        // in a taller terminal must not look complete in a shorter one.
        let h = inner.height as usize;
        if lines.len() > h && h > 0 {
            lines.truncate(h);
            lines[h - 1] = Line::from(Span::styled("   ~", theme.dim2()));
        }
        let x = inner.x + i as u16 * (col_w + GAP);
        f.render_widget(
            Paragraph::new(lines),
            Rect { x, y: inner.y, width: col_w.min(inner.width), height: inner.height },
        );
    }
}
