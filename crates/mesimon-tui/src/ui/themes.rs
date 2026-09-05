//! The theme picker — a menu row's second level, and the only one.
//!
//! The list IS the preview: `App::nav` repaints `app.theme` as the cursor
//! moves, so the board behind this popup is already wearing the row under
//! the cursor, and the popup has to say only what the board cannot — the
//! name, one line about it, and which slot each pick is saved in. Words,
//! never a mark: `◦` belongs to the suggestion chip and nothing else.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::Scope;

use crate::app::App;
use crate::text::truncate;
use crate::theme::Flavor;

use super::dialog;

pub(super) fn draw(f: &mut Frame, app: &App, idx: usize) {
    let theme = &app.theme;
    let ctx = app.ctx();
    let idx = idx.min(Flavor::ALL.len() - 1);

    let area = dialog::centred(f.area(), Flavor::ALL.len() as u16 * 2, dialog::MAX_W);
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        dialog::Edges {
            title: dialog::title(
                &theme.rest,
                format!("THEME ∙ for a {} terminal", ctx.theme_slot_word),
            ),
            tail: dialog::keys(app, Scope::Theme, &theme.rest, inner_w.saturating_sub(4)),
        },
    );

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, flavor) in Flavor::ALL.into_iter().enumerate() {
        let selected = i == idx;
        // The flavor's own ground sits where the menu puts a key: it is the
        // one fact a preview cannot show while the popup covers the board.
        let tag = flavor.ground().word();
        let lead = "   ";
        let text = truncate(flavor.name(), inner_w.saturating_sub(tag.width() + lead.width() + 1));
        let pad = inner_w.saturating_sub(lead.width() + text.width() + tag.width() + 1);
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
        let text = format!("     {}", truncate(flavor.blurb(), inner_w.saturating_sub(6)));
        let pad = inner_w.saturating_sub(text.width());
        lines.push(
            Line::from(vec![Span::styled(text, theme.dim3()), Span::raw(" ".repeat(pad))])
                .style(row_style),
        );
    }
    f.render_widget(Paragraph::new(lines), inner);
}
