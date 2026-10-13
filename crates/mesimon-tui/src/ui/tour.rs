//! The guided tour's dialog (T-763): nearly the whole screen, the lessons
//! down the left under their headings and, beside them, the lesson under
//! the cursor — its title, what it is about, and the practice board
//! playing it, with the line under the board saying what the key just
//! pressed did.
//!
//! The practice board is drawn whole, header and footer and all, by the
//! very `ui::draw` that draws this one, onto a canvas of its own size, and
//! its cells are laid into its frame here: so what it shows is the board.
//! Its own dialog frames join this frame's record, so the laws read its
//! composer's border as a frame like any other.

use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::glyphs::Tier;
use crate::text::truncate;
use crate::tour::{Tour, LESSONS};

use super::dialog;

/// The lesson list's width, beside the lesson.
const LIST_W: u16 = 30;
/// The narrowest practice board worth drawing: the board's own floor.
const BOARD_MIN_W: u16 = crate::layout::MIN_W;

pub(super) fn draw(f: &mut Frame, app: &App) {
    let Some(tour) = app.tour.as_deref() else { return };
    let theme = &app.theme;
    let area = f.area();
    // Every row but the footer's, a cell of board around it.
    let at = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let tail = if tour.trying {
        vec![Span::styled("q on the practice board returns to the lessons", theme.dim2())]
    } else {
        vec![Span::styled(format!("{} of {}", tour.idx + 1, LESSONS.len()), theme.dim3())]
    };
    let inner = dialog::frame(
        f,
        app,
        at,
        None,
        &theme.rest,
        dialog::Edges { title: dialog::title(&theme.rest, "GUIDED TOUR"), tail },
    );
    if inner.width < 10 || inner.height < 6 {
        return;
    }
    // The list beside the lesson where both fit; the lesson alone where not.
    let listed = inner.width >= LIST_W + 3 + BOARD_MIN_W + 2;
    let lesson_at = if listed {
        let list = Rect { x: inner.x + 1, width: LIST_W, ..inner };
        draw_list(f, app, tour, list);
        let x = list.x + LIST_W + 2;
        Rect { x, width: inner.right().saturating_sub(x + 1), ..inner }
    } else {
        Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner }
    };
    draw_lesson(f, app, tour, lesson_at);
}

/// The lessons under their headings, the one under the cursor lit, and a
/// tick on each one tried this opening.
fn draw_list(f: &mut Frame, app: &App, tour: &Tour, area: Rect) {
    let theme = &app.theme;
    let tick = if theme.glyph_tier() == Tier::Ascii { "v" } else { "✓" };
    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    let mut cursor_line = 0;
    let mut group = "";
    for (i, lesson) in LESSONS.iter().enumerate() {
        if lesson.group != group {
            if !group.is_empty() {
                lines.push(Line::default());
            }
            group = lesson.group;
            lines.push(Line::from(Span::styled(
                format!(" {}", group.to_uppercase()),
                theme.dim2().add_modifier(Modifier::BOLD),
            )));
        }
        let selected = i == tour.idx;
        if selected {
            cursor_line = lines.len();
        }
        let ramp = if selected { &theme.sel } else { &theme.rest };
        let ground = if selected { theme.selected_row() } else { Style::default() };
        let name_style = if selected {
            Style::default().fg(ramp.base).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ramp.dim1)
        };
        let mark = if tour.is_done(i) { tick } else { " " };
        let width = usize::from(area.width);
        let name = truncate(lesson.title, width.saturating_sub(5));
        let pad = width.saturating_sub(3 + name.width() + 2);
        lines.push(
            Line::from(vec![
                Span::raw("   "),
                Span::styled(name, name_style),
                Span::raw(" ".repeat(pad)),
                Span::styled(mark, Style::default().fg(ramp.dim2)),
                Span::raw(" "),
            ])
            .style(ground),
        );
    }
    // The cursor's row stays on screen: the list scrolls by half a page.
    let room = usize::from(area.height);
    let first = if lines.len() > room && cursor_line >= room.saturating_sub(2) {
        (cursor_line + 2).saturating_sub(room).min(lines.len() - room)
    } else {
        0
    };
    let shown: Vec<Line<'static>> = lines.into_iter().skip(first).take(room).collect();
    f.render_widget(Paragraph::new(shown), area);
}

/// The lesson: its title, what it is about, the practice board, and the
/// line under the board.
fn draw_lesson(f: &mut Frame, app: &App, tour: &Tour, area: Rect) {
    let theme = &app.theme;
    let lesson = tour.lesson();
    let w = usize::from(area.width);
    let mut y = area.y + 1;
    let head = Line::from(Span::styled(
        truncate(lesson.title, w),
        theme.base().add_modifier(Modifier::BOLD),
    ));
    f.render_widget(Paragraph::new(head), Rect { y, height: 1, ..area });
    y += 2;
    let about = Paragraph::new(Span::styled(lesson.about, theme.dim1())).wrap(Wrap { trim: true });
    let about_h = (wrapped_rows(lesson.about, w) as u16).min(5);
    f.render_widget(about, Rect { y, height: about_h, ..area });
    y += about_h + 1;

    // The board takes the rest but the line under it.
    let bottom = area.bottom().saturating_sub(2);
    let pane = Rect { y, height: bottom.saturating_sub(y), ..area };
    if pane.height >= 5 {
        let inner = dialog::frame(
            f,
            app,
            pane,
            None,
            &theme.rest,
            dialog::Edges { title: dialog::title(&theme.rest, "practice board"), tail: Vec::new() },
        );
        draw_board(f, app, tour, inner);
    }

    // The line under the board: what was just pressed, or the ask.
    let line = if tour.trying && tour.is_done(tour.idx) {
        let tick = if theme.glyph_tier() == Tier::Ascii { "v" } else { "✓" };
        Line::from(vec![
            Span::styled(format!("{tick} done "), theme.base().add_modifier(Modifier::BOLD)),
            Span::styled(format!("∙ {}", tour.caption), theme.dim2()),
        ])
    } else if tour.trying {
        Line::from(Span::styled(tour.caption.to_string(), theme.base()))
    } else {
        Line::from(Span::styled(tour.caption.to_string(), theme.dim1()))
    };
    f.render_widget(Paragraph::new(line), Rect { y: bottom, height: 1, ..area });
}

/// The practice board, drawn whole on its own canvas and laid into `at`.
fn draw_board(f: &mut Frame, app: &App, tour: &Tour, at: Rect) {
    if at.width == 0 || at.height == 0 {
        return;
    }
    let mut canvas = tour.canvas.borrow_mut();
    let fits = canvas.as_ref().is_some_and(|t| {
        let b = t.backend().buffer().area;
        b.width == at.width && b.height == at.height
    });
    if !fits {
        let backend = ratatui::backend::TestBackend::new(at.width, at.height);
        *canvas = ratatui::Terminal::new(backend).ok();
    }
    let Some(term) = canvas.as_mut() else { return };
    // The practice board's hints are its own: the board's under it keep
    // theirs for the mouse.
    let hints = crate::mouse::stash_hints();
    let drawn = term.draw(|g| super::draw(g, &tour.sandbox)).is_ok();
    crate::mouse::restore_hints(hints);
    if !drawn {
        return;
    }
    let src = term.backend().buffer();
    let dst = f.buffer_mut();
    for y in 0..at.height {
        for x in 0..at.width {
            if let (Some(from), Some(to)) =
                (src.cell(Position { x, y }), dst.cell_mut(Position { x: at.x + x, y: at.y + y }))
            {
                *to = from.clone();
            }
        }
    }
    // Its frames are frames on this one, where they now are.
    let mut frames = app.frames.borrow_mut();
    for r in tour.sandbox.frames.borrow().iter() {
        frames.push(Rect { x: r.x + at.x, y: r.y + at.y, ..*r });
    }
    drop(frames);
    // A field being typed in on the practice board shows the terminal's
    // cursor where its own would be.
    if tour.trying && tour.sandbox.typing_now() {
        let Ok(p) = term.get_cursor_position();
        if p.x < at.width && p.y < at.height {
            f.set_cursor_position((at.x + p.x, at.y + p.y));
        }
    }
}

/// How many rows `text` takes wrapped at `width` by words, as the
/// paragraph wraps it.
fn wrapped_rows(text: &str, width: usize) -> usize {
    let width = width.max(1);
    let mut rows = 1;
    let mut used = 0;
    for word in text.split_whitespace() {
        let w = word.width();
        if used == 0 {
            used = w;
        } else if used + 1 + w <= width {
            used += 1 + w;
        } else {
            rows += 1;
            used = w;
        }
    }
    rows
}
