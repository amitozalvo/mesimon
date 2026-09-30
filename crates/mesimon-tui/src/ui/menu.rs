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

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::{self, MenuItem, Scope};

use crate::app::{App, ColumnSubject, Mode, SharingRow, TierField};
use crate::qr::Qr;
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
    let Mode::ColumnSettings { subject, idx, naming, describing, .. } = &app.mode else { return };
    let ctx = app.frame_ctx();
    let items = keymap::column_items(&ctx);
    let title = match subject {
        ColumnSubject::Existing(name) if app.column_agents => {
            format!("AGENT BEHAVIOUR ∙ {}", name.to_uppercase())
        }
        ColumnSubject::Existing(name) => format!("COLUMN ∙ {}", name.to_uppercase()),
        ColumnSubject::New { .. } => "NEW COLUMN".to_string(),
    };
    let field = naming
        .as_ref()
        .map(|b| ("Name: ", b))
        .or_else(|| describing.as_ref().map(|b| ("Description: ", b)));
    draw_dense(f, app, &ctx, *idx, &title, &items, field);
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
/// title carries the sync word and the drafts waiting to go, or Remote
/// Control's state.
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
        let state = if !app.control.enabled {
            "OFF"
        } else if app.control.connected {
            "CONNECTED"
        } else {
            "DISCONNECTED"
        };
        format!("REMOTE CONTROL ∙ {state}")
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
        Some(SharingRow::AccessCode) => "Access code: ",
        Some(SharingRow::Join) | Some(SharingRow::Redeem) => "Code: ",
        _ => "",
    };
    let field = editing.as_ref().map(|b| (lead, b));
    // The pairing code as a QR a phone's camera opens (T-497), while a code
    // is live and the theme draws pictures. The code row still says it all.
    let qr = app
        .control
        .code
        .as_deref()
        .filter(|_| app.mesophon_dialog && !app.control.origin.is_empty())
        .and_then(|code| Qr::encode(&mesimon_core::mesophon::pair_link(&app.control.origin, code)))
        .zip(app.theme.qr_inks());
    let paint = |f: &mut Frame, at: Rect| {
        let Some((qr, (dark, light))) = &qr else { return };
        let (w, h) = qr.cells();
        qr.paint(f.buffer_mut(), (at.x, at.y), *dark, *light);
        *app.qr.borrow_mut() = Some(Rect { width: w, height: h, ..at });
        let caption = Line::from(Span::styled(QR_CAPTION, app.theme.dim3()));
        let below = Rect { y: at.y + h, height: 1, ..at };
        f.render_widget(Paragraph::new(caption).alignment(Alignment::Center), below);
    };
    let picture = qr.as_ref().map(|(qr, _)| {
        let (w, h) = qr.cells();
        Picture { w, h: h + 1, paint: &paint }
    });
    draw_rows_with(f, app, *idx, &title, &words, &headings, field, picture);
}

/// Under the pairing QR: what to do with it.
const QR_CAPTION: &str = "scan with your phone's camera";

/// The agent tiers list (T-443): a row per tier in the dialog's scope and
/// one that makes a new one, dense like the sharing list because the last
/// row becomes a name field in place. The title says the scope, the
/// Settings list's rule.
pub(super) fn draw_tiers(f: &mut Frame, app: &App) {
    let Mode::Tiers { idx, naming } = &app.mode else { return };
    let rows = app.tier_rows();
    let words: Vec<(String, String)> = rows.iter().map(|r| app.tier_row_words(r)).collect();
    let field = naming.as_ref().map(|b| ("Name: ", b));
    draw_rows(f, app, *idx, &scoped(app, "TIERS"), &words, &[], field);
}

/// One tier's page (T-443): the column dialog's shape — a row a setting,
/// the Name and Model rows fields in place.
pub(super) fn draw_tier_edit(f: &mut Frame, app: &App) {
    let Mode::TierEdit { idx, field, armed, .. } = &app.mode else { return };
    let fields = app.tier_fields();
    let words: Vec<(String, String)> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| app.tier_field_words(*f, *armed && i == *idx))
        .collect();
    let name = app.edited_tier().map(|(t, _)| t.name.to_uppercase()).unwrap_or_default();
    let lead = match fields.get(*idx) {
        Some(TierField::Name) => "Name: ",
        Some(TierField::Model) => "Model: ",
        _ => "",
    };
    let field = field.as_ref().map(|b| (lead, b));
    draw_rows(f, app, *idx, &scoped(app, &format!("TIER ∙ {name}")), &words, &[], field);
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
    draw_rows_with(f, app, idx, name, items, headings, field, None);
}

/// A picture a row dialog draws with its rows (the pairing QR, T-497): `w`
/// by `h` cells, painted by `paint` into the rect it is given.
struct Picture<'a> {
    w: u16,
    h: u16,
    paint: &'a dyn Fn(&mut Frame, Rect),
}

/// Where a picture goes: beside the rows where the screen is wide enough,
/// else under them where it is tall enough.
#[derive(Clone, Copy)]
enum Place {
    Beside,
    Below,
}

/// Cells between the rows and a picture beside them.
const PICTURE_GAP: u16 = 2;

/// Where `picture` fits beside or under `rows` inner rows on `screen`, if
/// anywhere: the room is what `dialog::centred` can give, the screen less
/// its margins and the frame.
fn place(screen: Rect, rows: u16, picture: &Picture) -> Option<Place> {
    let (room_w, room_h) = (screen.width.saturating_sub(6), screen.height.saturating_sub(4));
    if dialog::MAX_W + PICTURE_GAP + picture.w <= room_w && rows.max(picture.h) <= room_h {
        Some(Place::Beside)
    } else if picture.w <= dialog::MAX_W.min(room_w) && rows + 1 + picture.h <= room_h {
        Some(Place::Below)
    } else {
        None
    }
}

/// `draw_rows`, with a picture beside or under the rows when one fits. One
/// that does not is left out: the rows still say everything it does.
#[allow(clippy::too_many_arguments)]
fn draw_rows_with(
    f: &mut Frame,
    app: &App,
    idx: usize,
    name: &str,
    items: &[(String, String)],
    headings: &[bool],
    field: Option<(&str, &crate::text::EditBuffer)>,
    picture: Option<Picture>,
) {
    let theme = &app.theme;
    if items.is_empty() {
        return;
    }
    let idx = idx.min(items.len() - 1);
    let rows_h = items.len() as u16 + 2;
    let placed = picture.and_then(|p| Some((place(f.area(), rows_h, &p)?, p)));
    let (inner_h, inner_max) = match &placed {
        Some((Place::Beside, p)) => (rows_h.max(p.h), dialog::MAX_W + PICTURE_GAP + p.w),
        Some((Place::Below, p)) => (rows_h + 1 + p.h, dialog::MAX_W),
        None => (rows_h, dialog::MAX_W),
    };
    let area = dialog::centred(f.area(), inner_h, inner_max);
    let frame_w = area.width.saturating_sub(2) as usize;
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
            tail: dialog::keys(app, app.scope(), &theme.rest, frame_w.saturating_sub(4)),
        },
    );
    let rows_area = match &placed {
        Some((Place::Beside, p)) => {
            Rect { width: inner.width.saturating_sub(PICTURE_GAP + p.w), ..inner }
        }
        Some((Place::Below, _)) => Rect { height: rows_h.min(inner.height), ..inner },
        None => inner,
    };
    let inner_w = rows_area.width as usize;
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
    f.render_widget(Paragraph::new(lines), rows_area);
    if let Some((x, y)) = cursor_at {
        f.set_cursor_position((x.min(rows_area.x + rows_area.width.saturating_sub(1)), y));
    }
    if let Some((place, p)) = placed {
        let at = match place {
            Place::Beside => Rect {
                x: inner.x + inner.width.saturating_sub(p.w),
                y: inner.y + inner.height.saturating_sub(p.h) / 2,
                width: p.w,
                height: p.h,
            },
            Place::Below => Rect {
                x: inner.x + inner.width.saturating_sub(p.w) / 2,
                y: inner.y + rows_h + 1,
                width: p.w,
                height: p.h,
            },
        };
        (p.paint)(f, at.intersection(inner));
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
