//! The release notes screen — the Esc menu's `Release notes` row.
//!
//! The changelog this binary was built with (`relnotes::SOURCE`, parsed once
//! on open into `App::releases`), laid out as one document read top to
//! bottom: a painted band per release — its tag, its date in words, and
//! `this build` on the entry the running binary answers to — with the notes
//! under it as rendered markdown (`rich.rs`: value, weight, paint and space,
//! no rules, no colour). The column is a reading measure centred on the
//! screen rather than the terminal's whole width: prose past a hundred cells
//! is a long line to carry the eye back from, and a 200-column terminal
//! should make the margins wider, not the sentences.
//!
//! Same laws as every screen: bands are painted, never drawn (L1); the one
//! saturated colour stays needs-you's (L3); bold only on the tag, which is
//! the entry's title. The band of the release the window starts inside stays
//! pinned to the first row while its notes scroll under it, so a reader
//! halfway down a long entry can always see whose it is.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::relnotes::Release;

use crate::app::{App, ReleasesState, ReleasesView};
use crate::theme::Theme;

use super::chrome;

/// The reading measure's ceiling, in cells.
const MEASURE_MAX: usize = 100;
/// Page margin on either side of the column, at least.
const MARGIN_MIN: usize = 2;
/// The notes sit this far inside the band's left edge.
const INDENT: usize = 2;

pub(super) fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = f.area();
    let Some(r) = app.releases.as_ref() else { return };

    // ---- header, then the document's own identity row --------------------
    chrome::draw_header(f, Rect { x: area.x, y: area.y, width: area.width, height: 1 }, app, None);
    let n = r.releases.len();
    let noun = if n == 1 { "release" } else { "releases" };
    let ident = Line::from(vec![
        Span::styled(format!(" {n} {noun}"), theme.dim1()),
        Span::styled(format!(" ∙ newest first ∙ this is {}", r.build), theme.dim2()),
    ]);
    f.render_widget(
        Paragraph::new(vec![Line::default(), ident]),
        Rect { x: area.x, y: area.y + 1, width: area.width, height: 2.min(area.height - 1) },
    );

    // ---- the document, windowed ---------------------------------------------
    let body_y = area.y + 4;
    let body_h = area.height.saturating_sub(5) as usize;
    let width = area.width as usize;
    let measure = width.saturating_sub(MARGIN_MIN * 2).min(MEASURE_MAX);
    let left = (width - measure) / 2;

    let doc = document(r, measure, left, theme);
    let max = doc.lines.len().saturating_sub(body_h);
    let top = r.scroll.get().min(max);
    r.view.set(ReleasesView { max, page: body_h.saturating_sub(1).max(1) });
    *r.starts.borrow_mut() = doc.starts.clone();

    let mut shown: Vec<Line<'static>> = doc.lines.into_iter().skip(top).take(body_h).collect();
    // The release the window starts inside keeps its band on the first row
    // — unless that band IS the first row, in which case it is already there.
    if let Some(i) = doc.starts.iter().rposition(|s| *s <= top) {
        if doc.starts[i] < top {
            if let Some(first) = shown.first_mut() {
                *first = doc.bands[i].clone();
            }
        }
    }
    f.render_widget(
        Paragraph::new(shown),
        Rect { x: area.x, y: body_y, width: area.width, height: body_h as u16 },
    );

    // ---- footer, from the keymap like every screen ----------------------------
    let footer = chrome::footer_line(app, area.width);
    f.render_widget(
        Paragraph::new(footer),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
}

/// The whole document at one width: every row, each release's band on its
/// own (for pinning), and the row each band sits on (for `n`/`N`).
struct Document {
    lines: Vec<Line<'static>>,
    bands: Vec<Line<'static>>,
    starts: Vec<usize>,
}

fn document(r: &ReleasesState, measure: usize, left: usize, theme: &Theme) -> Document {
    let margin = " ".repeat(left + INDENT);
    let mut doc = Document { lines: Vec::new(), bands: Vec::new(), starts: Vec::new() };
    for rel in &r.releases {
        let band = band(rel, rel.tag == r.build, measure, left, theme);
        doc.starts.push(doc.lines.len());
        doc.bands.push(band.clone());
        doc.lines.push(band);
        doc.lines.push(Line::default());
        for row in crate::rich::render_all(&rel.body, measure.saturating_sub(INDENT), theme) {
            let mut spans = vec![Span::raw(margin.clone())];
            spans.extend(row.spans);
            doc.lines.push(Line::from(spans));
        }
        // Two rows of air between releases: one is a paragraph break inside
        // the notes, and the band needs to read as a new page, not a new line.
        doc.lines.push(Line::default());
        doc.lines.push(Line::default());
    }
    while doc.lines.last().is_some_and(|l| l.spans.is_empty()) {
        doc.lines.pop();
    }
    doc
}

/// One release's band: the elevated surface across the measure, the tag in
/// bold, the date a step down, `this build` at the right edge on the entry
/// the binary answers to. Painted per span, not per line, so the margins on
/// either side stay the page ground.
fn band(rel: &Release, current: bool, measure: usize, left: usize, theme: &Theme) -> Line<'static> {
    let ink = &theme.sel;
    let bg = theme.selected_row();
    let tag = format!(" {}", rel.tag);
    let date = rel.date_words();
    let date = if date.is_empty() { String::new() } else { format!("  ∙  {date}") };
    let right = if current { "this build ".to_string() } else { String::new() };
    let pad = measure.saturating_sub(tag.width() + date.width() + right.width());
    Line::from(vec![
        Span::raw(" ".repeat(left)),
        Span::styled(tag, bg.fg(ink.base).add_modifier(Modifier::BOLD)),
        Span::styled(date, bg.fg(ink.dim1)),
        Span::styled(" ".repeat(pad), bg),
        Span::styled(right, bg.fg(ink.dim2)),
    ])
    .style(Style::default())
}
