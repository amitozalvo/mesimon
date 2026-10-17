//! Setup's page (T-816): nearly the whole screen, the steps across the top,
//! the question and its answers, and the practice board under them at the
//! full width, showing the answer under the cursor.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::glyphs::Tier;
use crate::setup::Setup;
use crate::text::truncate;

use super::dialog;

pub(super) fn draw(f: &mut Frame, app: &App) {
    let Some(setup) = app.setup.as_deref() else { return };
    let theme = &app.theme;
    // The whole screen, edge to edge: Setup is the board's first sight, and
    // the board behind it is not yet the person's. Its keys ride its own
    // bottom edge, since the footer is under it.
    let at = f.area();
    let folder =
        app.repo_root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let title = format!("mesimon › setup {folder}");
    let keys = dialog::keys(
        app,
        mesimon_core::keymap::Scope::Setup,
        &theme.rest,
        usize::from(at.width).saturating_sub(6),
    );
    let inner = dialog::frame(
        f,
        app,
        at,
        None,
        &theme.rest,
        dialog::Edges { title: dialog::title(&theme.rest, title), tail: keys },
    );
    if inner.width < 20 || inner.height < 8 {
        return;
    }
    let page = Rect { x: inner.x + 2, width: inner.width.saturating_sub(4), ..inner };
    let w = usize::from(page.width);
    let mut lines: Vec<Line<'static>> =
        vec![Line::default(), steps(app, setup, w), Line::default()];
    lines.push(Line::from(Span::styled(
        truncate(&setup.question(), w),
        theme.base().add_modifier(Modifier::BOLD),
    )));
    if let Some((words, _)) = setup.status() {
        lines.push(Line::from(Span::styled(truncate(&words, w), theme.dim2())));
    }
    lines.push(Line::default());
    for (i, o) in setup.options().iter().enumerate() {
        let on = i == setup.cursor;
        let ramp = if on { &theme.sel } else { &theme.rest };
        let mark = if on { "› " } else { "  " };
        let label_style = if on {
            Style::default().fg(ramp.base).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ramp.dim1)
        };
        let label = truncate(&o.label, w.saturating_sub(4));
        let mut spans = vec![
            Span::styled(mark, Style::default().fg(ramp.dim2)),
            Span::styled(label.clone(), label_style),
        ];
        let mut used = 2 + label.width();
        if o.recommended && used + RECOMMENDED.len() <= w {
            spans.push(Span::styled(RECOMMENDED, Style::default().fg(ramp.dim1)));
            used += RECOMMENDED.len();
        }
        let room = w.saturating_sub(used + 2);
        if !o.detail.is_empty() && room > 8 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(truncate(&o.detail, room), Style::default().fg(ramp.dim2)));
        }
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
        let ground = if on { theme.selected_row() } else { Style::default() };
        lines.push(Line::from(spans).style(ground));
    }
    let top = lines.len() as u16;
    f.render_widget(Paragraph::new(lines), Rect { height: top.min(page.height), ..page });

    // The practice board takes the rest.
    let y = page.y + top + 1;
    let pane = Rect { y, height: page.bottom().saturating_sub(y), ..page };
    if pane.height >= 6 {
        let board = dialog::frame(
            f,
            app,
            pane,
            None,
            &theme.rest,
            dialog::Edges { title: dialog::title(&theme.rest, "preview"), tail: Vec::new() },
        );
        // The preview is part of Setup's layer, not a dialog over it: a
        // click on Setup's own keys, on its bottom edge, still lands.
        app.hits.borrow_mut().join_top();
        // Without its footer: the practice board's keys are not Setup's,
        // and a person who has not seen a board yet has no use for them.
        super::tour::lay_board(f, app, &setup.canvas, &setup.sandbox, board, false, 1);
    }
}

/// Beside the answer a question's default is.
const RECOMMENDED: &str = " (recommended)";

/// The steps across the top: the one on screen lit, a tick on each
/// answered, a dash on each left on its default.
fn steps(app: &App, setup: &Setup, w: usize) -> Line<'static> {
    let theme = &app.theme;
    let tick = if theme.glyph_tier() == Tier::Ascii { "v" } else { "✓" };
    let mut spans: Vec<Span<'static>> = Vec::new();
    let asks = setup.asks();
    let names =
        asks.iter().map(|a| (Some(*a), setup.rail(*a))).chain([(None, "Ready".to_string())]);
    for (i, (ask, name)) in names.enumerate() {
        let here = i == setup.at;
        let mark = match ask {
            Some(a) if setup.skipped(a) => " –".to_string(),
            Some(a) if setup.answered(a) => format!(" {tick}"),
            _ => String::new(),
        };
        let style = if here {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else if ask.is_some_and(|a| setup.answered(a)) {
            theme.dim1()
        } else {
            theme.dim3()
        };
        spans.push(Span::styled(format!(" {name}{mark} "), style));
        spans.push(Span::raw(" "));
    }
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    if used > w {
        // Too narrow for every name: the page on screen alone, with where.
        let total = asks.len() + 1;
        let name = setup.ask().map_or_else(|| "Ready".to_string(), |a| setup.rail(a));
        return Line::from(Span::styled(
            truncate(&format!("{} of {total} ∙ {name}", setup.at + 1), w),
            theme.dim1(),
        ));
    }
    Line::from(spans)
}
