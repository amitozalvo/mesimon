//! The CLAUDE.md offer's confirm dialog (T-217).
//!
//! mesimon's one modal confirmation, and the reason it exists: every other
//! confirm in the board is a chord tail (`d`, `a`, `z`) or the `m` key's arm,
//! which put their question in the status line and draw nothing. None of them
//! can show four lines of text — and `<repo>/CLAUDE.md` is the only file
//! mesimon writes that the user tracks in git, so the bytes go on the screen
//! before any of them go on the disk.
//!
//! The snippet is drawn VERBATIM, never re-wrapped: `claudemd::SNIPPET` is
//! authored at `claudemd::WRAP` columns so it fits this frame as written, and
//! `the_snippet_fits_the_dialog` holds the two numbers together. A dialog that
//! reflowed the text would be showing something other than what it writes.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::claudemd;
use mesimon_core::keymap::Scope;

use crate::app::App;
use crate::text::truncate;

use super::dialog::{self, Edges};

/// One row of air, the destination line, another row of air, then the snippet.
fn rows() -> u16 {
    3 + claudemd::SNIPPET.lines().count() as u16
}

pub(super) fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = dialog::centred(f.area(), rows(), dialog::MAX_W);
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        Edges {
            title: dialog::title(&theme.rest, "CLAUDE.MD"),
            tail: dialog::keys(app, Scope::ClaudeMd, &theme.rest, inner_w.saturating_sub(4)),
        },
    );

    // What will happen, and to what. The verb differs on one fact only, and
    // "creates" is the half a user is most likely to want to stop.
    let verb = if app.claude_md.exists { "appends to" } else { "creates" };
    // The path is the long half, so it is what gives: the leading words are
    // fixed and short, and a truncated tail still names the file.
    let lead = format!(" {verb} ");
    let budget = inner_w.saturating_sub(lead.width() + 1);
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(lead, theme.dim2()),
            Span::styled(truncate(&app.claude_md.path, budget), theme.dim1()),
        ]),
        Line::raw(""),
    ];

    // The snippet on the elevated surface: the treatment `rich.rs` gives a
    // fenced block, and this IS one — shrink-wrapped to the widest line, so
    // the paint says "this is the text" rather than filling the dialog.
    let widest = claudemd::SNIPPET.lines().map(|l| l.width()).max().unwrap_or(0);
    let block = (widest + 2).min(inner_w.saturating_sub(2));
    let code = match theme.code_bg() {
        Some(bg) => theme.dim1().bg(bg),
        None => theme.dim1(),
    };
    for line in claudemd::SNIPPET.lines() {
        let body = format!(" {line}");
        let pad = block.saturating_sub(body.width());
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{body}{}", " ".repeat(pad)), code),
        ]));
    }
    // A trailing blank keeps the last painted row off the frame's bottom edge.
    lines.push(Line::from(Span::styled(String::new(), Style::default())));

    f.render_widget(Paragraph::new(lines), inner);
}
