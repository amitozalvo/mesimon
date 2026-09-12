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

use mesimon_core::keymap::{self, MenuItem, Scope, Verb};

use crate::app::{App, ColumnSubject, Mode};
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
    draw_list(f, app, idx, app.settings_section.title(), Scope::Settings, &items);
}

/// The notifications list, one level under it (T-282): the same surface
/// again, its own name and its own rows.
pub(super) fn draw_notify(f: &mut Frame, app: &App, idx: usize) {
    let items = keymap::notify_items(&app.ctx());
    draw_list(f, app, idx, "NOTIFICATIONS", Scope::Notifications, &items);
}

/// The column settings dialog (T-117): thirteen rows at one line each, the
/// selected row's detail on the last inner line — `draw_list`'s two lines a
/// row would not fit `layout::MIN_H`. The Name row is a text field while
/// the mode says so, edited in place with the hardware cursor.
pub(super) fn draw_column(f: &mut Frame, app: &App) {
    let Mode::ColumnSettings { subject, idx, naming, .. } = &app.mode else { return };
    let ctx = app.ctx();
    let items = keymap::column_items(&ctx);
    let title = match subject {
        ColumnSubject::Existing(name) if app.column_agents => {
            format!("AGENT BEHAVIOUR ∙ {}", name.to_uppercase())
        }
        ColumnSubject::Existing(name) => format!("COLUMN ∙ {}", name.to_uppercase()),
        ColumnSubject::New { .. } => "NEW COLUMN".to_string(),
    };
    draw_dense(f, app, &ctx, *idx, &title, &items, naming.as_ref().map(|b| ("Name: ", b)));
}

/// The agent-prompt list (T-353): the three sentences mesimon types into an
/// agent's box, one row each. Dense like the column dialog and for the same
/// reason — a row here becomes a text field in place, and `draw_list` has no
/// room for the cursor. The lead keeps the row's name in front of the field,
/// so the sentence being rewritten never loses its label.
pub(super) fn draw_prompts(f: &mut Frame, app: &App) {
    let Mode::Prompts { idx, editing } = &app.mode else { return };
    let ctx = app.ctx();
    let items = keymap::prompt_items(&ctx);
    let lead = items
        .get(*idx)
        .and_then(|m| keymap::prompt_of(m.verb))
        .map(|w| format!("{}: ", w.label()))
        .unwrap_or_default();
    let field = editing.as_ref().map(|b| (lead.as_str(), b));
    draw_dense(f, app, &ctx, *idx, "AGENT PROMPTS", &items, field);
}

/// The team list (T-334), one level under Settings: the relay and the
/// display name are fields in place like the prompt list's rows, and the
/// lead keeps the row's name in front of the field.
pub(super) fn draw_team(f: &mut Frame, app: &App) {
    let Mode::Team { idx, editing, .. } = &app.mode else { return };
    let ctx = app.ctx();
    let items = keymap::team_items(&ctx);
    let lead = match items.get(*idx).map(|m| m.verb) {
        Some(Verb::TeamRelay) => "Relay: ",
        Some(Verb::TeamName) => "Display name: ",
        _ => "",
    };
    let field = editing.as_ref().map(|b| (lead, b));
    draw_dense(f, app, &ctx, *idx, "TEAM", &items, field);
}

/// The sharing dialog (T-334). Its rows are the board's members, so they
/// come from `App::share_rows` rather than a static list, and the frame's
/// title carries the sync word and the drafts waiting to go.
pub(super) fn draw_share(f: &mut Frame, app: &App) {
    let Mode::Share { idx, armed } = &app.mode else { return };
    let rows = app.share_rows();
    let words: Vec<(String, String)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (label, detail, _) = app.share_words(r, *armed && i == *idx);
            (label, detail)
        })
        .collect();
    let title = match &app.team.board {
        Some(b) => {
            let mut t = format!("SHARING ∙ {}", b.sync.state.to_uppercase());
            if b.sync.drafts > 0 {
                t.push_str(&format!(" ∙ {} DRAFTS", b.sync.drafts));
            }
            t
        }
        None => "SHARING".to_string(),
    };
    draw_rows(f, app, *idx, &title, &words, None);
}

fn draw_dense(
    f: &mut Frame,
    app: &App,
    ctx: &mesimon_core::keymap::Ctx,
    idx: usize,
    name: &str,
    items: &[&'static MenuItem],
    field: Option<(&str, &crate::text::EditBuffer)>,
) {
    let words: Vec<(String, String)> =
        items.iter().map(|m| ((m.label)(ctx), (m.detail)(ctx))).collect();
    draw_rows(f, app, idx, name, &words, field);
}

/// One line a row, the selected row's detail on the last inner line, and
/// an optional text field in place of the selected row's label.
fn draw_rows(
    f: &mut Frame,
    app: &App,
    idx: usize,
    name: &str,
    items: &[(String, String)],
    field: Option<(&str, &crate::text::EditBuffer)>,
) {
    let theme = &app.theme;
    if items.is_empty() {
        return;
    }
    let idx = idx.min(items.len() - 1);
    let area = dialog::centred(f.area(), items.len() as u16 + 2, dialog::MAX_W);
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        dialog::Edges {
            title: dialog::title(&theme.rest, name),
            // The scope as it stands: a naming dialog's edge reads the text
            // field's `enter save ∙ esc cancel`.
            tail: dialog::keys(app, app.scope(), &theme.rest, inner_w.saturating_sub(4)),
        },
    );
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cursor_at: Option<(u16, u16)> = None;
    for (i, item) in items.iter().enumerate() {
        let selected = i == idx;
        let style = if selected {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.base()
        };
        let row_style = if selected { theme.selected_row() } else { Style::default() };
        let text = match field {
            Some((lead, buf)) if selected => {
                let budget = inner_w.saturating_sub(3 + lead.width() + 1);
                let (shown, cx) =
                    crate::text::edit_window(buf.as_str(), buf.width_before_cursor(), budget);
                cursor_at = Some((inner.x + 3 + lead.width() as u16 + cx, inner.y + i as u16));
                format!("{lead}{shown}")
            }
            _ => truncate(&item.0, inner_w.saturating_sub(4)),
        };
        let pad = inner_w.saturating_sub(3 + text.width());
        lines.push(
            Line::from(vec![
                Span::raw("   "),
                Span::styled(text, style),
                Span::raw(" ".repeat(pad)),
            ])
            .style(row_style),
        );
    }
    lines.push(Line::default());
    // The selected row's detail, marquee-revealed when it overflows — the
    // same clock `draw_list` runs.
    let budget = inner_w.saturating_sub(6);
    let detail = items[idx].1.clone();
    let overflow = detail.width().saturating_sub(budget);
    let scroll = if overflow > 0 {
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
    lines.push(Line::from(Span::styled(format!("     {body}"), theme.dim3())));
    f.render_widget(Paragraph::new(lines), inner);
    if let Some((x, y)) = cursor_at {
        f.set_cursor_position((x.min(inner.x + inner.width.saturating_sub(1)), y));
    }
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
