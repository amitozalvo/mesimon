//! The theme picker — a menu row's second level, and the only one.
//!
//! The list IS the preview: `App::nav` repaints `app.theme` as the cursor
//! moves, so the board behind this popup is already wearing the row under
//! the cursor, and the popup has to say only what the board cannot — the
//! name, one line about it, and which slot each pick is saved in. Words,
//! never a mark: `◦` belongs to the suggestion chip and nothing else.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap;

use crate::app::App;
use crate::text::truncate;
use crate::theme::{Flavor, Ground};

fn word(g: Ground) -> &'static str {
    match g {
        Ground::Dark => "dark",
        Ground::Light => "light",
    }
}

pub(super) fn draw(f: &mut Frame, app: &App, idx: usize) {
    let theme = &app.theme;
    let ctx = app.ctx();
    let idx = idx.min(Flavor::ALL.len() - 1);

    let w = 62.min(f.area().width.saturating_sub(4));
    let h = ((Flavor::ALL.len() as u16 * 2) + 4).min(f.area().height.saturating_sub(2));
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

    let inner = w as usize;
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            format!(" theme — for a {} terminal", ctx.theme_slot_word),
            theme.dim1().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
    ];
    for (i, flavor) in Flavor::ALL.into_iter().enumerate() {
        let selected = i == idx;
        // The flavor's own ground sits where the menu puts a key: it is the
        // one fact a preview cannot show while the popup covers the board.
        let tag = word(flavor.ground());
        let lead = "   ";
        let text = truncate(flavor.name(), inner.saturating_sub(tag.width() + lead.width() + 1));
        let pad = inner.saturating_sub(lead.width() + text.width() + tag.width() + 1);
        let style = if selected {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.base()
        };
        let row_style = if selected { theme.selected_row() } else { Style::default() };
        lines.push(
            Line::from(vec![
                Span::styled(lead, style),
                Span::styled(text, style),
                Span::raw(" ".repeat(pad)),
                Span::styled(tag, theme.dim2()),
                Span::raw(" "),
            ])
            .style(row_style),
        );
        let mut detail = flavor.blurb().to_string();
        for g in [Ground::Dark, Ground::Light] {
            if app.prefs.for_ground(g) == flavor {
                detail.push_str(&format!(" ∙ your pick for a {} terminal", word(g)));
            }
        }
        let text = format!("     {}", truncate(&detail, inner.saturating_sub(6)));
        let pad = inner.saturating_sub(text.width());
        lines.push(
            Line::from(vec![Span::styled(text, theme.dim3()), Span::raw(" ".repeat(pad))])
                .style(row_style),
        );
    }
    lines.push(Line::from(Span::styled(
        format!(" {}", keymap::footer(keymap::Scope::Theme, &ctx, inner.saturating_sub(2))),
        theme.dim2(),
    )));
    f.render_widget(Paragraph::new(lines), area);
}
