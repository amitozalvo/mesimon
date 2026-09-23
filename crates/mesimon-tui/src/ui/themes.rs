//! The theme picker — a menu row's second level, and the only one.
//!
//! The list IS the preview: `App::nav` repaints `app.theme` as the cursor
//! moves, so the board behind this popup is already wearing the row under
//! the cursor, and the popup has to say only what the board cannot — the
//! name, one line about it, and which slot each pick is saved in. Words,
//! never a mark: `◦` belongs to the suggestion chip and nothing else.

use ratatui::Frame;

use mesimon_core::keymap::Scope;

use crate::app::App;
use crate::theme::Flavor;

use super::dialog::{self, ListRow};

pub(super) fn draw(f: &mut Frame, app: &App, idx: usize) {
    let ctx = app.frame_ctx();
    let mut rows: Vec<ListRow> = Vec::with_capacity(app.theme_rows());
    // Board scope puts an inherit row first (T-361): its "flavor" is the
    // machine's pick for this ground, and choosing it drops the board's.
    if app.settings_board_scope {
        let machine = app.machine_prefs.for_ground(app.ground).name();
        rows.push(ListRow {
            lead: "   ".into(),
            head: "inherit".into(),
            right: String::new(),
            detail: Some(format!("the machine's pick: {machine}")),
        });
    }
    // The flavor's own ground sits where the menu puts a key: it is the one
    // fact a preview cannot show while the popup covers the board.
    rows.extend(Flavor::ALL.into_iter().map(|flavor| ListRow {
        lead: "   ".into(),
        head: flavor.name().into(),
        right: flavor.ground().word().into(),
        detail: Some(flavor.blurb().into()),
    }));
    let name = format!("THEME ∙ for a {} terminal", ctx.theme_slot_word);
    dialog::list(f, app, &name, false, Scope::Theme, idx, &rows);
}
