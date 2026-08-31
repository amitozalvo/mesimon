//! The `^t` picker: every tag on the board, laid out as a grid you steer.
//!
//! One row per group, the tags of that group across it, and a `+` cell at the
//! end of each row for making a new one. The cursor is a cell; `hjkl` walks
//! it, a digit jumps to that group's row and steps along it, and the actions
//! all apply to the cell under the cursor.
//!
//! It is a panel rather than a one-line hint because the vocabulary is the
//! user's own and mesimon seeds none: the only way a digit can mean anything
//! is for the panel to show what each one currently holds, in the colour it
//! will paint.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::board::MAX_TAGS_PER_GROUP;

use crate::app::App;

/// Groups the picker shows. Ten, addressed by `1`–`9` and `0`.
pub(crate) const GROUPS: u8 = 10;

/// A group's row is shown when it holds something, and the first empty group
/// is shown too so there is always somewhere to make the next tag. Beyond
/// that the rows stay hidden — a board that has never used group 7 should not
/// advertise it.
pub(crate) fn visible_groups(app: &App) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut spare = false;
    for g in 1..=GROUPS {
        if !app.board.group_entries(g).is_empty() {
            out.push(g);
        } else if !spare {
            out.push(g);
            spare = true;
        }
    }
    out
}

/// Cells in a group's row: its tags, plus the trailing `+` unless it is full.
pub(crate) fn row_len(app: &App, group: u8) -> usize {
    let n = app.board.group_entries(group).len();
    if n >= MAX_TAGS_PER_GROUP {
        n
    } else {
        n + 1
    }
}

/// How tall the panel wants to be, including its border row and hint row.
pub(crate) fn height(app: &App) -> u16 {
    visible_groups(app).len() as u16 + 2
}

fn digit_of(group: u8) -> char {
    if group == 10 {
        '0'
    } else {
        char::from(b'0' + group)
    }
}

pub(super) fn draw(f: &mut Frame, area: Rect, app: &App) {
    let Some(arm) = app.tag_armed.as_ref() else { return };
    let theme = &app.theme;
    let worn = app.tag_subject().unwrap_or(&[]);
    let groups = visible_groups(app);

    let h = height(app).min(area.height);
    let panel =
        Rect { x: area.x, y: area.y + area.height.saturating_sub(h), width: area.width, height: h };
    f.render_widget(Clear, panel);

    let mut lines: Vec<Line<'static>> = Vec::new();
    // Title row: what is being tagged, so the panel is never ambiguous about
    // its subject — especially in the composer, where the ticket has no name
    // on the board yet.
    let subject = match arm.ticket.and_then(|id| app.board.ticket(id)) {
        Some(t) => crate::text::truncate(&t.title, area.width.saturating_sub(10) as usize),
        None => "new ticket".to_string(),
    };
    lines.push(Line::from(vec![
        Span::styled(" TAGS ".to_string(), theme.dim1().add_modifier(Modifier::BOLD)),
        Span::styled(subject, theme.dim2()),
    ]));

    for (row, g) in groups.iter().enumerate() {
        let mut spans = vec![Span::styled(
            format!("  {}  ", digit_of(*g)),
            if arm.row == row { theme.base().add_modifier(Modifier::BOLD) } else { theme.dim3() },
        )];
        let entries = app.board.group_entries(*g);
        for (col, def) in entries.iter().enumerate() {
            let here = arm.row == row && arm.col == col;
            let is_worn = worn.iter().any(|w| w.group == *g && w.name == def.name);
            // The swatch is the tag's own colour — the same paint the card
            // band will use, so the picker is a preview and not a legend.
            let swatch = if theme.paints_bands() {
                Style::default().bg(theme.pip(def.tint() as usize))
            } else {
                Style::default().fg(theme.rest.dim2).add_modifier(Modifier::REVERSED)
            };
            // A worn tag is MARKED, not just coloured: whether this ticket
            // has it is the one thing here that must not be colour-only.
            let mark = if is_worn { '\u{2022}' } else { ' ' };
            let label = if here {
                theme.base().add_modifier(Modifier::BOLD)
            } else if is_worn {
                theme.dim1()
            } else {
                theme.dim2()
            };
            // The cursor is a bracket pair, never a colour: the cells are
            // already spending colour on the tag's own tint.
            let (open, close) = if here { ('[', ']') } else { (' ', ' ') };
            spans.push(Span::styled(open.to_string(), theme.base()));
            spans.push(Span::styled("  ".to_string(), swatch));
            spans.push(Span::styled(format!("{mark}{}", def.name), label));
            spans.push(Span::styled(close.to_string(), theme.base()));
            spans.push(Span::raw(" ".to_string()));
        }
        if entries.len() < MAX_TAGS_PER_GROUP {
            let here = arm.row == row && arm.col == entries.len();
            let style = if here { theme.base().add_modifier(Modifier::BOLD) } else { theme.dim3() };
            spans.push(Span::styled(
                if here { "[+ new]".to_string() } else { " + new ".to_string() },
                style,
            ));
        }
        // Naming happens in place, on the cell it belongs to.
        if arm.row == row {
            if let Some((purpose, buf)) = arm.naming.as_ref() {
                let verb = match purpose {
                    crate::app::Naming::New => "new",
                    crate::app::Naming::Rename => "rename",
                };
                spans = vec![
                    Span::styled(
                        format!("  {}  ", digit_of(*g)),
                        theme.base().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{verb}: "), theme.dim2()),
                    Span::styled(buf.as_str().to_string(), theme.base()),
                    Span::styled("\u{2588}".to_string(), theme.dim2()),
                ];
            }
        }
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let pad = (area.width as usize).saturating_sub(used);
        spans.push(Span::raw(" ".repeat(pad)));
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), panel);
}
