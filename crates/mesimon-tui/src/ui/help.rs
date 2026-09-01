//! The `?` overlay — the answer to "what can I press here".
//!
//! Rendered entirely from `keymap::overlay`, so it lists exactly what works on
//! this screen in this state and nothing else: no key that would do nothing,
//! no key the footer had no room for left out. This is the surface the audit
//! found missing, and the reason the keymap became data.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap;

use crate::app::App;

/// Floor for the key column, so the hints line up in one rail and the rail
/// sits in the same place on every screen. It is a floor and not the width:
/// a spelling wider than this (`option+hjkl`, `shift+enter`) pushes the rail
/// out rather than closing the gap, which is what `> <` used to hide — every
/// key was short enough that nothing tested the arithmetic.
const KEY_W: usize = 10;

pub(super) fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let scope = app.scope();
    let groups = keymap::overlay(scope, &app.ctx());
    if groups.is_empty() {
        return;
    }

    // One blank between groups, one heading each, plus the title block.
    let rows: usize = groups.iter().map(|(_, r)| r.len() + 2).sum::<usize>() + 2;
    let w = 54.min(f.area().width.saturating_sub(4));
    let h = (rows as u16 + 2).min(f.area().height.saturating_sub(2));
    let area = Rect {
        x: (f.area().width.saturating_sub(w)) / 2,
        y: (f.area().height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(ratatui::widgets::Clear, area);
    if let Some(bg) = theme.bg {
        f.render_widget(ratatui::widgets::Block::default().style(Style::default().bg(bg)), area);
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let head = format!(" {} — everything you can press here", scope.word().to_lowercase());
    lines.push(Line::from(Span::styled(head, theme.dim1().add_modifier(Modifier::BOLD))));

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
    for (group, rows) in groups {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!(" {}", group.title()),
            theme.dim2().add_modifier(Modifier::BOLD),
        )));
        for (key, hint) in rows {
            let pad = rail.saturating_sub(key.width());
            lines.push(Line::from(vec![
                Span::styled(format!("   {key}"), theme.base()),
                Span::raw(" ".repeat(pad)),
                Span::styled(hint.to_string(), theme.dim1()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
}
