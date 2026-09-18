//! The agent-brief offer's confirm dialog (T-217, re-aimed at the system
//! prompt by T-224).
//!
//! mesimon's one modal confirmation, and the reason it exists: every other
//! confirm in the board is a chord tail (`d`, `a`, `z`) or the `m` key's arm,
//! which put their question in the status line and draw nothing. None of them
//! can show five lines of text — and README promise 3 says mesimon adds no
//! token to a conversation, so the one consented exception goes on the screen,
//! verbatim, before it goes on any argv.
//!
//! The text is drawn VERBATIM, never re-wrapped: `brief::TEXT` is authored at
//! `claudemd::WRAP` columns so it fits this frame as written, and
//! `the_text_fits_the_dialog` holds the two numbers together. A dialog that
//! reflowed the text would be showing something other than what it sends.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::brief;
use mesimon_core::keymap::Scope;

use crate::app::App;

use super::dialog::{self, Edges};

/// Two lines saying where the text goes and where it does not, a row of air,
/// the text, and a row of air under it.
fn rows() -> u16 {
    4 + brief::TEXT.lines().count() as u16
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
            title: dialog::title(&theme.rest, "AGENT BRIEF"),
            tail: dialog::keys(app, Scope::Brief, &theme.rest, inner_w.saturating_sub(4)),
        },
    );

    // What will happen, and to whom — the reach is the sentence the user is
    // consenting to, so it comes before the words: only the claude sessions
    // mesimon starts in this repo, in their system prompt, and nothing on
    // disk. Two short lines rather than one long one, so neither truncates
    // at the dialog's measure.
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(" in the system prompt of ", theme.dim2()),
            Span::styled("agent sessions mesimon starts here", theme.dim1()),
        ]),
        Line::from(Span::styled(
            " only those ∙ nothing written to disk ∙ Settings turns it off",
            theme.dim2(),
        )),
        Line::raw(""),
    ];

    // The text on the elevated surface: the treatment `rich.rs` gives a
    // fenced block, and this IS one — shrink-wrapped to the widest line, so
    // the paint says "this is the text" rather than filling the dialog.
    let widest = brief::TEXT.lines().map(|l| l.width()).max().unwrap_or(0);
    let block = (widest + 2).min(inner_w.saturating_sub(2));
    let code = match theme.code_bg() {
        Some(bg) => theme.dim1().bg(bg),
        None => theme.dim1(),
    };
    for line in brief::TEXT.lines() {
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
