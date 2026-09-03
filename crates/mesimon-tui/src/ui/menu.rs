//! The Esc menu — everything that acts on the board as a whole, plus the two
//! lists that are not the board.
//!
//! These actions deliberately have no key of their own: they are rare, they
//! are not about the selection, and a menu row has room to say what it will do
//! in full words before you commit to it. Rows come from `keymap::menu_items`,
//! filtered to what applies right now, so the menu is never a list of things
//! that would do nothing. Framed (`dialog`): its name in the top edge, its
//! keys in the bottom one.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::{self, Scope};

use crate::app::App;
use crate::text::truncate;

use super::dialog;

pub(super) fn draw(f: &mut Frame, app: &App, idx: usize) {
    let theme = &app.theme;
    let ctx = app.ctx();
    let items = keymap::menu_items(&ctx);
    if items.is_empty() {
        return;
    }
    let idx = idx.min(items.len() - 1);

    let area = dialog::centred(f.area(), items.len() as u16 * 2, dialog::MAX_W);
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        dialog::Edges {
            title: dialog::title(&theme.rest, "MENU"),
            tail: dialog::keys(app, Scope::Menu, &theme.rest, inner_w.saturating_sub(4)),
        },
    );

    let mut lines: Vec<Line<'static>> = Vec::new();
    // The mark the header's chips wear. A marked row is the thing a chip is
    // offering — same glyph, same words, same order, and they sort first, so
    // the offer the header names is the row already under the cursor.
    let mark = crate::glyphs::suggest_mark(theme.glyph_tier());
    for (i, item) in items.iter().enumerate() {
        let selected = i == idx;
        let label = (item.label)(&ctx);
        let suggested = keymap::is_suggested(item.verb, &ctx);
        // A row that also has a key teaches it at the right edge — the menu is
        // a discovery surface, not a second way to hide the keymap.
        let key = item.key;
        let lead = if suggested { format!(" {mark} ") } else { "   ".to_string() };
        let text = truncate(&label, inner_w.saturating_sub(key.width() + lead.width() + 1));
        let pad = inner_w.saturating_sub(lead.width() + text.width() + key.width() + 1);
        let style = if selected {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.base()
        };
        let row_style = if selected { theme.selected_row() } else { Style::default() };
        // The mark stays a step quieter than the words it introduces, so the
        // row reads as a label with a mark, never as a bulleted list.
        let mark_style = if selected { style } else { theme.dim3() };
        lines.push(
            Line::from(vec![
                Span::styled(lead, mark_style),
                Span::styled(text, style),
                Span::raw(" ".repeat(pad)),
                Span::styled(key.to_string(), theme.dim2()),
                Span::raw(" "),
            ])
            .style(row_style),
        );
        let detail = (item.detail)(&ctx);
        let text = format!("     {}", truncate(&detail, inner_w.saturating_sub(6)));
        let pad = inner_w.saturating_sub(text.width());
        lines.push(
            Line::from(vec![Span::styled(text, theme.dim3()), Span::raw(" ".repeat(pad))])
                .style(row_style),
        );
    }
    f.render_widget(Paragraph::new(lines), inner);
}
