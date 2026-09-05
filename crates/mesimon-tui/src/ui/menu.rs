//! The Esc menu — everything that acts on the board as a whole, plus the two
//! lists that are not the board — and the settings list one level under it,
//! drawn by the same function: the preferences are `MenuItem`s too.
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

use mesimon_core::keymap::{self, MenuItem, Scope};

use crate::app::App;
use crate::text::{marquee_offset, marquee_window, truncate};

use super::dialog;

pub(super) fn draw(f: &mut Frame, app: &App, idx: usize) {
    let items = keymap::menu_items(&app.ctx());
    draw_list(f, app, idx, "MENU", Scope::Menu, &items);
}

/// The settings submenu: the same surface, its own name and keys. No row
/// here is ever a suggestion, so the lead never carries the mark.
pub(super) fn draw_settings(f: &mut Frame, app: &App, idx: usize) {
    let items = keymap::settings_items(&app.ctx());
    draw_list(f, app, idx, "SETTINGS", Scope::Settings, &items);
}

fn draw_list(
    f: &mut Frame,
    app: &App,
    idx: usize,
    name: &str,
    scope: Scope,
    items: &[&'static MenuItem],
) {
    let theme = &app.theme;
    let ctx = app.ctx();
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
            title: dialog::title(&theme.rest, name),
            tail: dialog::keys(app, scope, &theme.rest, inner_w.saturating_sub(4)),
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
        // A subtitle too long for the dialog reveals itself marquee-style on
        // the selected row — the board card title's clock, its reveal and its
        // one pass. A menu row's detail is where a preference says what it
        // will do, so `~` was cutting the half that matters.
        let budget = inner_w.saturating_sub(6);
        let detail = (item.detail)(&ctx);
        let overflow = detail.width().saturating_sub(budget);
        let scroll = if selected && overflow > 0 {
            let key = words_key(&detail);
            let ms = match app.menu_marquee.get() {
                Some((k, epoch)) if k == key => epoch.elapsed().as_millis() as u64,
                _ => {
                    app.menu_marquee.set(Some((key, std::time::Instant::now())));
                    0
                }
            };
            marquee_offset(ms, overflow)
        } else {
            0
        };
        let body = if scroll > 0 {
            marquee_window(&detail, budget, scroll)
        } else {
            truncate(&detail, budget)
        };
        let text = format!("     {body}");
        let pad = inner_w.saturating_sub(text.width());
        lines.push(
            Line::from(vec![Span::styled(text, theme.dim3()), Span::raw(" ".repeat(pad))])
                .style(row_style),
        );
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// The marquee clock's key: the subtitle's own words. Hashing the sentence
/// rather than the row means a toggle that rewrites its own detail restarts
/// the reveal, and a list that reorders under the cursor cannot carry a
/// half-scrolled clock onto somebody else's words.
fn words_key(detail: &str) -> u64 {
    crate::text::hash64(detail)
}
