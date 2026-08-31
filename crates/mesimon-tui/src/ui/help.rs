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

/// Widest key column across the groups, so the hints line up in one rail.
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

    for (group, rows) in groups {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!(" {}", group.title()),
            theme.dim2().add_modifier(Modifier::BOLD),
        )));
        for (key, hint) in rows {
            let pad = KEY_W.saturating_sub(key.width());
            lines.push(Line::from(vec![
                Span::styled(format!("   {key}"), theme.base()),
                Span::raw(" ".repeat(pad)),
                Span::styled(hint.to_string(), theme.dim1()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
}
