//! The Theme section's page (T-717): which state a pick is saved for, the
//! flavors, and beside them a small board drawn in the one under the
//! cursor. The board itself keeps its theme until Enter keeps the pick, so
//! browsing never repaints anything but the preview.
//!
//! The preview is its own frame, on its own flavor's ground and in its own
//! inks — a recorded frame like any dialog's, joined to the Settings
//! dialog's layer so a click on the list beside it is not a click off it.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap;

use crate::app::{App, Mode};
use crate::glyphs::Tier;
use crate::text::truncate;
use crate::theme::{Flavor, Ground, Slot, Theme};

use super::dialog;

/// The list's width, and the narrowest preview worth drawing beside it.
const LIST_W: u16 = 24;
const PREVIEW_MIN_W: u16 = 34;

pub(super) fn draw_page(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    if area.height < 4 {
        return;
    }
    let (cursor, slot) = match app.mode {
        Mode::Theme { idx, slot } => (Some(idx), slot),
        _ => (None, Slot::Both),
    };
    let tier = theme.glyph_tier();
    let (set, unset) = if tier == Tier::Ascii { ("*", "-") } else { ("●", "○") };
    // Which state the pick is for: Tab's three stops, the live one marked.
    let mut head: Vec<Span<'static>> = vec![Span::raw(" ")];
    for (i, s) in [Slot::Both, Slot::One(Ground::Dark), Slot::One(Ground::Light)].iter().enumerate()
    {
        if i > 0 {
            head.push(Span::raw("   "));
        }
        let word = match s {
            Slot::Both => "dark and light".to_string(),
            Slot::One(g) => g.word().to_string(),
        };
        if *s == slot {
            head.push(Span::styled(
                format!("{set} {word}"),
                theme.base().add_modifier(Modifier::BOLD),
            ));
        } else {
            head.push(Span::styled(format!("{unset} {word}"), theme.dim2()));
        }
    }
    f.render_widget(Paragraph::new(Line::from(head)), Rect { height: 1, ..area });

    // The rows: an inherit row first in board scope (T-361), then every
    // flavor with its ground at the right.
    let rows = app.theme_rows();
    let shown_at = cursor.unwrap_or_else(|| kept_row(app, slot)).min(rows - 1);
    let kept = kept_row(app, slot);
    let list = Rect {
        y: area.y + 2,
        width: LIST_W.min(area.width),
        height: area.height.saturating_sub(4),
        ..area
    };
    let room = usize::from(list.height);
    let first = if rows > room { shown_at.saturating_sub(room / 2).min(rows - room) } else { 0 };
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut hits = app.hits.borrow_mut();
    for i in first..rows.min(first + room) {
        let y = list.y + lines.len() as u16;
        hits.record(Rect { y, height: 1, ..list }, crate::mouse::Target::Row(i));
        let (name, right) = match app.theme_at(i) {
            Some(fl) => (fl.name().to_string(), fl.ground().word().to_string()),
            None => ("inherit".to_string(), String::new()),
        };
        let selected = cursor == Some(i);
        let ramp = if selected { &theme.sel } else { &theme.rest };
        let ground = if selected { theme.selected_row() } else { Style::default() };
        let mark = if i == kept { format!(" {set} ") } else { "   ".to_string() };
        let name_style = if selected || i == kept {
            Style::default().fg(ramp.base).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ramp.dim1)
        };
        let width = usize::from(list.width);
        let name = truncate(&name, width.saturating_sub(4 + right.width() + 1));
        let pad = width.saturating_sub(3 + name.width() + right.width() + 1);
        lines.push(
            Line::from(vec![
                Span::styled(mark, Style::default().fg(ramp.base)),
                Span::styled(name, name_style),
                Span::raw(" ".repeat(pad)),
                Span::styled(right, Style::default().fg(ramp.dim3)),
                Span::raw(" "),
            ])
            .style(ground),
        );
    }
    drop(hits);
    f.render_widget(Paragraph::new(lines), list);

    // The preview, beside the list where there is the width for one.
    let px = list.x + list.width + 3;
    let pw = area.right().saturating_sub(px);
    if pw >= PREVIEW_MIN_W {
        let at = Rect { x: px, y: list.y, width: pw.min(56), height: list.height };
        preview(f, app, at, app.theme_preview(shown_at, slot));
    }

    // The foot: the flavor under the cursor, or what the slots hold.
    let ctx = app.frame_ctx();
    let foot = match cursor {
        Some(i) => match app.theme_at(i) {
            Some(fl) => fl.blurb().to_string(),
            None => format!("the machine's pick: {}", app.theme_preview(i, slot).name()),
        },
        None => keymap::theme_detail(&ctx),
    };
    let body = dialog::reveal(app, &foot, usize::from(area.width));
    let at = Rect { y: area.bottom() - 1, height: 1, ..area };
    f.render_widget(Paragraph::new(Span::styled(body, theme.dim2())), at);
}

/// The row of the pick the slot holds: the board's own in board scope (the
/// inherit row while it has none), else the machine's — the live theme for
/// the board's own ground, which under a pin is not the slot's.
fn kept_row(app: &App, slot: Slot) -> usize {
    let ground = match slot {
        Slot::One(g) => g,
        Slot::Both => app.ground,
    };
    let pick = if app.settings_board_scope {
        app.board_prefs.flavor(ground)
    } else {
        Some(app.prefs.for_ground(ground))
    };
    match pick {
        Some(f) => {
            Flavor::ALL.iter().position(|x| *x == f).unwrap_or(0)
                + usize::from(app.settings_board_scope)
        }
        None => 0,
    }
}

/// A small board in `flavor`, at the top of `room`: the header, three
/// columns, the cursor's card, a card that needs you and one that is done,
/// and the tag ring. Its frame is as tall as what it holds.
fn preview(f: &mut Frame, app: &App, room: Rect, flavor: Flavor) {
    let t = Theme::new(flavor, app.theme.profile);
    let tall = if t.paints_tags() { 9 } else { 7 };
    let at = Rect { height: room.height.min(tall + 2), ..room };
    let inner = dialog::frame(
        f,
        app,
        at,
        t.bg,
        &t.rest,
        dialog::Edges {
            title: dialog::title(&t.rest, flavor.name()),
            tail: vec![Span::styled(flavor.ground().word().to_string(), t.dim3())],
        },
    );
    // Part of the Settings dialog, not a dialog over it.
    app.hits.borrow_mut().join_top();
    let w = usize::from(inner.width);
    let cw = w.saturating_sub(2) / 3;
    let bar = if t.glyph_tier() == Tier::Ascii { "|" } else { "▎" };
    let cell = |text: &str, right: &str, style: Style, tag: Option<usize>, ground: Style| {
        let mut spans = match tag {
            Some(n) => vec![Span::styled(bar, ground.fg(t.pip(n))), Span::styled(" ", ground)],
            None => vec![Span::styled("  ", ground)],
        };
        let room = cw.saturating_sub(3 + right.width() + 1);
        let text = truncate(text, room);
        let pad = cw.saturating_sub(2 + text.width() + right.width() + 1);
        spans.push(Span::styled(text, style));
        spans.push(Span::styled(" ".repeat(pad), ground));
        spans.push(Span::styled(right.to_string(), ground.fg(t.rest.dim2)));
        spans.push(Span::styled(" ", ground));
        spans
    };
    let row = |cells: Vec<Vec<Span<'static>>>| {
        let mut spans = vec![Span::raw(" ")];
        for c in cells {
            spans.extend(c);
        }
        Line::from(spans)
    };
    let plain = Style::default();
    let lit = t.selected_row();
    let tick = if t.glyph_tier() == Tier::Ascii { "v " } else { "✓ " };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" BOARD", t.base().add_modifier(Modifier::BOLD)),
            Span::styled("  kanban ∙ 5 tickets", t.dim2()),
        ]),
        Line::default(),
        row(vec![
            cell("TODO", "2", t.dim1(), None, plain),
            cell("IN PROGRESS", "2", t.dim1(), None, plain),
            cell("REVIEW", "1", t.dim1(), None, plain),
        ]),
        Line::default(),
        row(vec![
            cell("Keymap", "2d", t.base(), Some(0), plain),
            cell("Fix parser", "3h", lit.fg(t.sel.base).add_modifier(Modifier::BOLD), Some(6), lit),
            cell(&format!("{tick}Graphemes"), "1d", t.base(), Some(2), plain),
        ]),
        Line::default(),
        row(vec![
            cell("Decay", "5h", t.base(), Some(3), plain),
            cell("Drawer", "1h", t.attn_row(), Some(8), plain),
            cell("", "", plain, None, plain),
        ]),
    ];
    if t.paints_tags() {
        let mut ring = vec![Span::raw(" ")];
        for n in 0..crate::theme::PIPS {
            ring.push(Span::styled("▉▉", Style::default().fg(t.pip(n))));
            ring.push(Span::raw(" "));
        }
        lines.push(Line::default());
        lines.push(Line::from(ring));
    }
    // The rest of the frame on the flavor's ground, under every line.
    let ground = t.bg.map_or(Style::default(), |bg| Style::default().bg(bg));
    f.render_widget(Paragraph::new(lines).style(ground), inner);
}
