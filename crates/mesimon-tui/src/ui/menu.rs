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

use crate::app::{App, ColumnSubject, Mode, SharingRow};
use crate::text::truncate;

use super::dialog::{self, ListRow};

pub(super) fn draw(f: &mut Frame, app: &App, idx: usize) {
    let items = keymap::menu_items(&app.frame_ctx());
    draw_list(f, app, idx, "MENU", Scope::Menu, &items);
}

/// The settings submenu: the same surface, its own name and keys. No row
/// here is ever a suggestion, so the lead never carries the mark.
pub(super) fn draw_settings(f: &mut Frame, app: &App, idx: usize) {
    let items = keymap::settings_items(&app.frame_ctx());
    draw_list(f, app, idx, &scoped(app, app.settings_section.title()), Scope::Settings, &items);
}

/// The notifications list, one level under it (T-282): the same surface
/// again, its own name and its own rows.
pub(super) fn draw_notify(f: &mut Frame, app: &App, idx: usize) {
    let items = keymap::notify_items(&app.frame_ctx());
    draw_list(f, app, idx, &scoped(app, "NOTIFICATIONS"), Scope::Notifications, &items);
}

/// The dialog's name with the scope after it when it is this board's
/// (T-361): the title and the `b` hint are the two places that say which
/// scope is live, so a stale scope is never silent.
fn scoped(app: &App, name: &str) -> String {
    if app.settings_board_scope {
        format!("{name} ∙ THIS BOARD")
    } else {
        name.to_string()
    }
}

/// The column settings dialog (T-117): thirteen rows at one line each, the
/// selected row's detail on the last inner line — `draw_list`'s two lines a
/// row would not fit `layout::MIN_H`. The Name row is a text field while
/// the mode says so, edited in place with the hardware cursor.
pub(super) fn draw_column(f: &mut Frame, app: &App) {
    let Mode::ColumnSettings { subject, idx, naming, .. } = &app.mode else { return };
    let ctx = app.frame_ctx();
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

/// The agent-prompt list (T-353): the four sentences mesimon types into an
/// agent's box, one row each. Dense like the column dialog and for the same
/// reason — a row here becomes a text field in place, and `draw_list` has no
/// room for the cursor. The lead keeps the row's name in front of the field,
/// so the sentence being rewritten never loses its label.
pub(super) fn draw_prompts(f: &mut Frame, app: &App) {
    let Mode::Prompts { idx, editing } = &app.mode else { return };
    let ctx = app.frame_ctx();
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
/// The sharing dialog (T-334; one dialog since T-335): the identity, this
/// board and the boards this device belongs to, each under a heading the
/// cursor skips. The rows come from `App::sharing_rows`; a row that is a
/// field (the relay, the name, a code) is edited in place; the frame's
/// title carries the sync word and the drafts waiting to go.
pub(super) fn draw_sharing(f: &mut Frame, app: &App) {
    let Mode::Sharing { idx, editing, armed } = &app.mode else { return };
    let rows = app.sharing_rows();
    let words: Vec<(String, String)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (label, detail, _) = app.sharing_words(r, *armed && i == *idx);
            (label, detail)
        })
        .collect();
    let headings: Vec<bool> = rows.iter().map(|r| matches!(r, SharingRow::Heading(_))).collect();
    let title = if app.mesophon_dialog {
        "REMOTE CONTROL".to_string()
    } else {
        match &app.team.board {
            Some(b) => {
                let mut t = format!("SHARING ∙ {}", b.sync.state.to_uppercase());
                if b.sync.drafts > 0 {
                    t.push_str(&format!(" ∙ {} DRAFTS", b.sync.drafts));
                }
                t
            }
            None => "SHARING".to_string(),
        }
    };
    let lead = match rows.get(*idx) {
        Some(SharingRow::Relay) => "Relay: ",
        Some(SharingRow::Name) => "Display name: ",
        Some(SharingRow::Join) => "Code: ",
        _ => "",
    };
    let field = editing.as_ref().map(|b| (lead, b));
    draw_rows(f, app, *idx, &title, &words, &headings, field);
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
    draw_rows(f, app, idx, name, &words, &[], field);
}

/// One line a row, the selected row's detail on the last inner line, and
/// an optional text field in place of the selected row's label. A row
/// `headings` marks is a section's name: drawn quiet, never selected.
fn draw_rows(
    f: &mut Frame,
    app: &App,
    idx: usize,
    name: &str,
    items: &[(String, String)],
    headings: &[bool],
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
        let heading = headings.get(i).copied().unwrap_or(false);
        let style = if selected {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else if heading {
            theme.dim3()
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
        // A heading sits one cell in, its rows three: the indent is the
        // section, the way the settings groups read.
        let lead = if heading { " " } else { "   " };
        let pad = inner_w.saturating_sub(lead.len() + text.width());
        lines.push(
            Line::from(vec![
                Span::raw(lead),
                Span::styled(text, style),
                Span::raw(" ".repeat(pad)),
            ])
            .style(row_style),
        );
    }
    lines.push(Line::default());
    // The selected row's detail, marquee-revealed when it overflows — the
    // same clock `dialog::list` runs.
    let body = dialog::reveal(app, &items[idx].1, inner_w.saturating_sub(6));
    lines.push(Line::from(Span::styled(format!("     {body}"), theme.dim3())));
    f.render_widget(Paragraph::new(lines), inner);
    if let Some((x, y)) = cursor_at {
        f.set_cursor_position((x.min(inner.x + inner.width.saturating_sub(1)), y));
    }
}

/// The menu-shaped list — a `MenuItem` a row, its key at the right edge
/// (the menu is a discovery surface, not a second way to hide the keymap)
/// and its detail under it. A suggested row wears the mark the header's
/// chips wear: same glyph, same words, same order, and they sort first, so
/// the offer the header names is the row already under the cursor.
fn draw_list(
    f: &mut Frame,
    app: &App,
    idx: usize,
    name: &str,
    scope: Scope,
    items: &[&'static MenuItem],
) {
    let ctx = app.frame_ctx();
    let mark = crate::glyphs::suggest_mark(app.theme.glyph_tier());
    let rows: Vec<ListRow> = items
        .iter()
        .map(|item| ListRow {
            lead: if keymap::is_suggested(item.verb, &ctx) {
                format!(" {mark} ")
            } else {
                "   ".to_string()
            },
            head: (item.label)(&ctx),
            right: item.key.to_string(),
            detail: Some(keymap::item_detail(item, &ctx)),
        })
        .collect();
    dialog::list(f, app, name, false, scope, idx, &rows);
}
