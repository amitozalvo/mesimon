//! Framed dialogs — the one place structure is DRAWN (author 2026-09-03,
//! T-158). A floating surface over the board or a page gets a frame with its
//! title set into the top edge and its keys set into the bottom edge, so a
//! dialog names itself and teaches itself in the same cells, and the footer
//! under it has nothing to repeat. Every frame is recorded on `App::frames`
//! for the frame it was drawn in: `test_no_drawn_structure` admits a box
//! glyph on a recorded perimeter and nowhere else, which is what keeps L1
//! ("no drawn structure") a law with one allowlisted role rather than a
//! preference. The board, its cards and the ticket page stay painted.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::{self, Scope};

use crate::app::App;
use crate::text::{marquee_offset, marquee_window, truncate};
use crate::theme::Ramp;

use super::chrome;

/// The one width every centred dialog settles at, terminal permitting —
/// the menu, the theme picker, the archived list and the drawer were 54, 62
/// and 64 wide before, for no reason anyone could name.
pub(super) const MAX_W: u16 = 64;

/// A centred rectangle holding `rows` inner rows inside a frame, at most
/// `max_w` inner cells wide (plus the frame's two), never wider than the
/// screen less a two-cell margin a side.
pub(super) fn centred(screen: Rect, rows: u16, max_w: u16) -> Rect {
    let w = (max_w + 2).min(screen.width.saturating_sub(4)).max(4);
    let h = (rows + 2).min(screen.height.saturating_sub(2)).max(3);
    Rect {
        x: screen.x + screen.width.saturating_sub(w) / 2,
        y: screen.y + screen.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// What the frame carries: the title in its top edge, the keys in its
/// bottom edge. Either may be empty; the edge is then a plain rule.
pub(super) struct Edges {
    pub title: Vec<Span<'static>>,
    pub tail: Vec<Span<'static>>,
}

/// The dialog's name, in the frame's own register: `dim1` + bold, the
/// weight every section heading carries.
pub(super) fn title(ink: &Ramp, text: impl Into<String>) -> Vec<Span<'static>> {
    vec![Span::styled(text.into(), Style::default().fg(ink.dim1).add_modifier(Modifier::BOLD))]
}

/// The dialog's own keys for its bottom edge — the left cluster of its
/// scope's footer (the app-level keys stay in the footer under it, which is
/// where `? keys` lives).
pub(super) fn keys(app: &App, scope: Scope, ink: &Ramp, budget: usize) -> Vec<Span<'static>> {
    let ctx = app.frame_ctx();
    let (own, _) = keymap::footer_split(scope, &ctx);
    chrome::hint_spans(&own, &ctx, ink, budget)
}

/// Clears `area`, paints it `surface` (the page ground when `None`), draws
/// the frame in `ink.dim3` with `edges` set into it, records the frame, and
/// returns the inner rectangle. Below three rows or four columns there is
/// no frame to draw and the whole area is returned as it is.
pub(super) fn frame(
    f: &mut Frame,
    app: &App,
    area: Rect,
    surface: Option<Color>,
    ink: &Ramp,
    edges: Edges,
) -> Rect {
    let theme = &app.theme;
    let ground = surface.or(theme.bg);
    f.render_widget(Clear, area);
    if let Some(bg) = ground {
        f.render_widget(Block::default().style(Style::default().bg(bg)), area);
    }
    if area.width < 4 || area.height < 3 {
        return area;
    }
    let g = crate::glyphs::frame_set(theme.glyph_tier());
    let rule = Style::default().fg(ink.dim3);
    let w = area.width as usize;
    // Corners, one rule cell, a space each side of the words, one rule cell.
    let budget = w.saturating_sub(6);

    let mut top: Vec<Span<'static>> = vec![Span::styled(g.tl.to_string(), rule)];
    let words = fit(edges.title, budget);
    if words.is_empty() {
        top.push(Span::styled(g.h.to_string().repeat(w - 2), rule));
    } else {
        let used: usize = super::spans_width(&words);
        top.push(Span::styled(format!("{} ", g.h), rule));
        top.extend(words);
        top.push(Span::styled(format!(" {}", g.h.to_string().repeat(w - 5 - used)), rule));
    }
    top.push(Span::styled(g.tr.to_string(), rule));

    let mut bottom: Vec<Span<'static>> = vec![Span::styled(g.bl.to_string(), rule)];
    let words = fit(edges.tail, budget);
    if words.is_empty() {
        bottom.push(Span::styled(g.h.to_string().repeat(w - 2), rule));
    } else {
        let used: usize = super::spans_width(&words);
        bottom.push(Span::styled(format!("{} ", g.h.to_string().repeat(w - 5 - used)), rule));
        bottom.extend(words);
        bottom.push(Span::styled(format!(" {}", g.h), rule));
    }
    bottom.push(Span::styled(g.br.to_string(), rule));

    f.render_widget(
        Paragraph::new(Line::from(top)),
        Rect { x: area.x, y: area.y, width: area.width, height: 1 },
    );
    f.render_widget(
        Paragraph::new(Line::from(bottom)),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
    let side: Vec<Line<'static>> =
        (0..area.height - 2).map(|_| Line::from(Span::styled(g.v.to_string(), rule))).collect();
    f.render_widget(
        Paragraph::new(side.clone()),
        Rect { x: area.x, y: area.y + 1, width: 1, height: area.height - 2 },
    );
    f.render_widget(
        Paragraph::new(side),
        Rect { x: area.x + area.width - 1, y: area.y + 1, width: 1, height: area.height - 2 },
    );
    app.frames.borrow_mut().push(area);
    Rect { x: area.x + 1, y: area.y + 1, width: area.width - 2, height: area.height - 2 }
}

/// The spans that fit in `budget` cells, in order; the first that does not
/// is cut to the room left and ends the run.
fn fit(spans: Vec<Span<'static>>, budget: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for s in spans {
        let w = s.content.width();
        if used + w <= budget {
            used += w;
            out.push(s);
            continue;
        }
        let room = budget.saturating_sub(used);
        if room > 0 {
            out.push(Span::styled(truncate(&s.content, room), s.style));
        }
        break;
    }
    out
}

/// One row of a framed selectable list — what the menu, the settings and
/// notifications lists, the theme picker, the archived list, the links list
/// and the External drawer differ in, and nothing else.
pub(super) struct ListRow {
    /// The cells before the head: the menu's suggestion mark, or spaces.
    /// Drawn a step quieter than the head unless the row is selected, so a
    /// marked row reads as a label with a mark, never as a bulleted list.
    pub lead: String,
    pub head: String,
    /// What sits at the right edge, dim — a key, a ground word — or `""`.
    pub right: String,
    /// A dim line under the head. A list is two lines a row when any row
    /// carries one, and the selected row's detail marquee-reveals when it
    /// overflows.
    pub detail: Option<String>,
    /// A subtitle over the rows under it (T-610): one dim line, never the
    /// cursor's — a caller keeps its cursor over its own rows and passes
    /// `idx` already past every heading above it.
    pub heading: bool,
}

/// A centred framed list: `name` in the top edge, `scope`'s keys in the
/// bottom one, one selected row. Nothing to draw when `rows` is empty.
/// A list dialog, sized to its rows up to the screen and scrolled past
/// that (T-437): the window keeps the cursor at its middle once the list
/// outgrows it, so both directions move smoothly with no offset to keep.
/// `count` puts the row count in the title (`ARCHIVED ∙ 3`); a list that
/// overflows shows the position instead (`EXTERNAL ∙ 12/459`), counted
/// or not — a menu on a short terminal scrolls too.
pub(super) fn list(
    f: &mut Frame,
    app: &App,
    name: &str,
    count: bool,
    scope: Scope,
    idx: usize,
    rows: &[ListRow],
) {
    let theme = &app.theme;
    if rows.is_empty() {
        return;
    }
    let idx = idx.min(rows.len() - 1);
    let tall = rows.iter().any(|r| r.detail.is_some());
    let per = if tall { 2 } else { 1 };
    let height = |r: &ListRow| if r.heading { 1 } else { per };
    let wanted = u16::try_from(rows.iter().map(height).sum::<usize>()).unwrap_or(u16::MAX);
    let area = centred(f.area(), wanted, MAX_W);
    // The window, in lines: the cursor's row, about half the room above it,
    // the rest below, and above again where the list ends first. With every
    // row one height this is the row arithmetic it replaced; a heading
    // (T-610) is one line in a list of two.
    let room = usize::from(area.height.saturating_sub(2));
    let (mut first, mut last, mut used) = (idx, idx + 1, height(&rows[idx]));
    while first > 0 && used + height(&rows[first - 1]) <= room / 2 + height(&rows[idx]) {
        first -= 1;
        used += height(&rows[first]);
    }
    while last < rows.len() && used + height(&rows[last]) <= room {
        used += height(&rows[last]);
        last += 1;
    }
    while first > 0 && used + height(&rows[first - 1]) <= room {
        first -= 1;
        used += height(&rows[first]);
    }
    // The count is of the rows a cursor can stand on: a heading is a label.
    let selectable = rows.iter().filter(|r| !r.heading).count();
    let name = if last - first < rows.len() {
        format!("{name} ∙ {}/{selectable}", rows[..=idx].iter().filter(|r| !r.heading).count())
    } else if count {
        format!("{name} ∙ {selectable}")
    } else {
        name.to_string()
    };
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner = frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        Edges {
            title: title(&theme.rest, name),
            tail: keys(app, scope, &theme.rest, inner_w.saturating_sub(4)),
        },
    );
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, row) in rows.iter().enumerate().take(last).skip(first) {
        if row.heading {
            // One cell in, its rows three: the indent is the group, the
            // way the sharing dialog's headings read.
            let head = truncate(&row.head, inner_w.saturating_sub(2));
            let pad = inner_w.saturating_sub(1 + head.width());
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(head, theme.dim3()),
                Span::raw(" ".repeat(pad)),
            ]));
            continue;
        }
        let selected = i == idx;
        let style = if selected {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.base()
        };
        let row_style = if selected { theme.selected_row() } else { Style::default() };
        let lead_style = if selected { style } else { theme.dim3() };
        // The right edge keeps one cell of margin after it.
        let edge = if row.right.is_empty() { 0 } else { row.right.width() + 1 };
        // The cursor's head reveals what a cut hides, the board card
        // title's way (T-702); a two-line list reveals its detail instead,
        // since the two would share one clock.
        let budget = inner_w.saturating_sub(row.lead.width() + edge);
        let head = if selected && !tall {
            reveal(app, &row.head, budget)
        } else {
            truncate(&row.head, budget)
        };
        let pad = inner_w.saturating_sub(row.lead.width() + head.width() + edge);
        let mut spans = vec![
            Span::styled(row.lead.clone(), lead_style),
            Span::styled(head, style),
            Span::raw(" ".repeat(pad)),
        ];
        if edge > 0 {
            spans.push(Span::styled(row.right.clone(), theme.dim2()));
            spans.push(Span::raw(" "));
        }
        lines.push(Line::from(spans).style(row_style));
        if tall {
            let detail = row.detail.as_deref().unwrap_or("");
            let budget = inner_w.saturating_sub(6);
            let body =
                if selected { reveal(app, detail, budget) } else { truncate(detail, budget) };
            let text = format!("     {body}");
            let pad = inner_w.saturating_sub(text.width());
            lines.push(
                Line::from(vec![Span::styled(text, theme.dim3()), Span::raw(" ".repeat(pad))])
                    .style(row_style),
            );
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// The selected row's detail (or, in a one-line list, its head),
/// marquee-revealed when it overflows `budget` — the board card title's clock, its reveal and its one pass. A menu
/// row's detail is where a preference says what it will do, so `~` was
/// cutting the half that matters. The clock is keyed on the words
/// themselves: a toggle that rewrites its own detail restarts the reveal,
/// and a list that reorders under the cursor cannot carry a half-scrolled
/// clock onto somebody else's words.
pub(super) fn reveal(app: &App, detail: &str, budget: usize) -> String {
    let overflow = detail.width().saturating_sub(budget);
    if overflow == 0 {
        return truncate(detail, budget);
    }
    let key = crate::text::hash64(detail);
    let ms = match app.menu_marquee.get() {
        Some((k, epoch)) if k == key => epoch.elapsed().as_millis() as u64,
        _ => {
            app.menu_marquee.set(Some((key, std::time::Instant::now())));
            0
        }
    };
    let scroll = marquee_offset(ms, overflow);
    if scroll > 0 {
        marquee_window(detail, budget, scroll)
    } else {
        truncate(detail, budget)
    }
}

/// The archived-tickets dialog: restore or open.
pub(super) fn draw_archived(f: &mut Frame, app: &App, idx: usize) {
    let archived = app.board.archived_tickets();
    let now = mesimon_core::clock::now_ms();
    let rows: Vec<ListRow> = archived
        .iter()
        .map(|t| {
            // A snoozed ticket says when it comes back; a plain archive says
            // how long it has been gone. Unparsable stamps show nothing.
            let age = match t.snooze_until_secs() {
                Some(until) => format!("wakes {}", crate::text::until_word(now, until * 1000)),
                None => t
                    .archived
                    .as_ref()
                    .and_then(|a| mesimon_core::board::stamp_secs(&a.at))
                    .map(|secs| crate::text::age_slot(now, secs * 1000, false))
                    .unwrap_or_default(),
            };
            ListRow {
                lead: " ".into(),
                head: format!(
                    "{}  {} ∙ {} ∙ {}",
                    t.short_key,
                    truncate(&t.title, 28),
                    t.column,
                    age
                ),
                right: String::new(),
                detail: None,
                heading: false,
            }
        })
        .collect();
    list(f, app, "ARCHIVED", true, Scope::Archived, idx, &rows);
}

/// The links dialog (T-256): what the ticket's notes point at, one row per
/// target — the kind word, then the markdown label if the note gave one,
/// then the target as written (a ticket row names the ticket). Enter opens,
/// `c` copies, and a row that will not fit is cut with `~`, never wrapped;
/// the cursor's row scrolls the rest into view, as a card title does.
pub(super) fn draw_links(
    f: &mut Frame,
    app: &App,
    ticket: ulid::Ulid,
    links: &[crate::app::TicketLink],
    idx: usize,
) {
    let key = app.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
    let rows: Vec<ListRow> = links
        .iter()
        .map(|l| {
            let body = match (&l.target, &l.label) {
                (crate::app::LinkTarget::Ticket(id), _) => {
                    let title = app.board.ticket(*id).map(|t| t.title.as_str()).unwrap_or("");
                    format!("{} ∙ {title}", l.text)
                }
                (crate::app::LinkTarget::Attachment { ticket, attachment }, label) => {
                    let label = label.as_deref().unwrap_or("Picture");
                    let available = app.board.ticket(*ticket).is_some_and(|t| {
                        app.repo_root
                            .join(".mesimon/board/tickets")
                            .join(&t.short_key)
                            .join("attachments")
                            .join(format!("{attachment}.png"))
                            .is_file()
                    });
                    if available {
                        format!("[{label}]")
                    } else {
                        format!("[{label}] ∙ image unavailable on this machine")
                    }
                }
                (_, Some(label)) => format!("{label} ∙ {}", l.text),
                (_, None) => l.text.clone(),
            };
            ListRow {
                lead: " ".into(),
                head: format!("{:<6} {}", l.kind(), crate::text::one_line(&body)),
                right: String::new(),
                detail: None,
                heading: false,
            }
        })
        .collect();
    list(f, app, &format!("LINKS ∙ {key}"), true, Scope::Links, idx, &rows);
}

/// The summary dialog (T-696): one row per line of the ticket's `Summary`
/// sections, a box before each task — the card's own marks — and nothing
/// before a plain row. Read off the cache each frame, so a tick shows on
/// the frame it lands; `idx` is clamped to the rows there are.
pub(super) fn draw_summary(f: &mut Frame, app: &App, ticket: ulid::Ulid, idx: usize) {
    let key = app.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
    let Some(s) = app.ticket_summary(ticket) else {
        return;
    };
    let tier = app.theme.glyph_tier();
    let rows: Vec<ListRow> = s
        .rows
        .iter()
        .map(|r| {
            let words = crate::text::one_line(&mesimon_core::text::scrub_cells(&r.text, false));
            let head = match r.done {
                Some(done) => format!("{} {words}", super::card::box_mark(tier, Some(done))),
                None => format!("- {words}"),
            };
            ListRow { lead: " ".into(), head, right: String::new(), detail: None, heading: false }
        })
        .collect();
    let idx = idx.min(rows.len().saturating_sub(1));
    list(f, app, &format!("SUMMARY ∙ {key}"), true, Scope::Summary, idx, &rows);
}

/// The merge dialog (T-431). Two rows under a `MERGE ∙ T-n` title: what the
/// next `m` does and the reason to stay on this page, with the keys in the
/// bottom edge from the `MergeChord` scope — which go quiet while the merge
/// runs, when the working spinner in front of the first row is the one thing
/// moving. The same frame every stage, so a merge landing turns the words
/// over in place instead of closing the dialog under the reader.
pub(super) fn draw_merge(f: &mut Frame, app: &App, d: &crate::app::MergeDialog) {
    let theme = &app.theme;
    let key = app.board.ticket(d.ticket).map(|t| t.short_key.clone()).unwrap_or_default();
    let rows = app.merge_dialog_rows();
    if rows.is_empty() {
        return;
    }
    let area = centred(f.area(), rows.len() as u16, MAX_W);
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner = frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        Edges {
            title: title(&theme.rest, format!("MERGE ∙ {key}")),
            tail: keys(app, Scope::MergeChord, &theme.rest, inner_w.saturating_sub(4)),
        },
    );
    let lead = if d.running.is_some() {
        format!(" {} ", crate::glyphs::spinner(theme.glyph_tier(), app.spin_frame()))
    } else {
        " ".to_string()
    };
    let lines: Vec<Line<'static>> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let (head, style) = if i == 0 {
                (lead.clone(), theme.calm_text())
            } else {
                (" ".to_string(), theme.dim2())
            };
            let body = truncate(row, inner_w.saturating_sub(head.width() + 1));
            Line::from(vec![Span::styled(head, theme.calm_text()), Span::styled(body, style)])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

/// The External drawer (19 §4): discovered foreign sessions, observe/resume.
/// While the daemon walks `~/.claude` (T-437) the title spins over the last
/// answer, and a first walk with nothing to show yet says so in the frame.
pub(super) fn draw_drawer(f: &mut Frame, app: &App, idx: usize) {
    let theme = &app.theme;
    let spinner = crate::glyphs::spinner(theme.glyph_tier(), app.spin_frame());
    if app.external.is_empty() {
        if !app.external_scanning {
            return;
        }
        let area = centred(f.area(), 1, MAX_W);
        let inner_w = area.width.saturating_sub(2) as usize;
        let inner = frame(
            f,
            app,
            area,
            None,
            &theme.rest,
            Edges {
                title: title(&theme.rest, "EXTERNAL"),
                tail: keys(app, Scope::Drawer, &theme.rest, inner_w.saturating_sub(4)),
            },
        );
        let line = Line::from(vec![
            Span::styled(format!(" {spinner} "), theme.calm_text()),
            Span::styled("scanning for sessions started outside mesimon", theme.dim2()),
        ]);
        f.render_widget(Paragraph::new(vec![line]), inner);
        return;
    }
    let name =
        if app.external_scanning { format!("EXTERNAL {spinner}") } else { "EXTERNAL".into() };
    let now = mesimon_core::clock::now_ms();
    let rows: Vec<ListRow> = app
        .external
        .iter()
        .map(|item| {
            let name = item.name.clone().unwrap_or_else(|| item.id.to_string()[..8].to_string());
            let mut badges = format!("  ∙ {}", item.provider.label());
            if item.running_elsewhere {
                badges.push_str("  ∙ running elsewhere");
            }
            ListRow {
                lead: " ".into(),
                head: format!(
                    "{}  {}{badges}",
                    truncate(&name, 24),
                    crate::text::age_slot(now, item.mtime_ms, false)
                ),
                right: String::new(),
                detail: Some(item.preview.clone().unwrap_or_default()),
                heading: false,
            }
        })
        .collect();
    list(f, app, &name, true, Scope::Drawer, idx, &rows);
}
