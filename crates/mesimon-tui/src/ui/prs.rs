//! The pull requests dialog (T-759, T-805): every PR or MR the forge bound
//! to a ticket, newest first. It is a table — one column per fact, under
//! dim headings, the dialog as wide as its widest row up to the screen, the
//! title wrapping inside its own column — so no fact is cut and none waits
//! on a marquee. Where even the table's fixed columns do not fit the
//! screen, each PR is a head line with its facts wrapped under it, at the
//! width every dialog keeps. `^h` opens it over the board or the page;
//! Enter opens the row's PR with the board's opener, and the row above the
//! footer is the cursor's link.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use mesimon_core::command::PrItem;
use mesimon_core::keymap::Scope;
use mesimon_core::text::scrub_cells;

use crate::app::App;

use super::dialog::{self, Edges};

/// What the dialog says when the forge has bound nothing to the ticket —
/// the PRs went while it was open, since `^h` is inert with none.
pub(super) const EMPTY: &str =
    "no pull requests ∙ the forge binds one the signed-in account opened from this ticket's worktree";

/// The mark's cells before the first column (` ✗ `), the room between two
/// columns, and the one cell of margin the right edge keeps.
const LEAD: usize = 3;
const GAP: usize = 2;
const MARGIN: usize = 1;

/// The title's column: as wide as its longest title up to `TITLE_PREF`
/// while another column can still wrap instead, and never narrower than
/// `TITLE_MIN` — below it a title is a word a line and the table gives way
/// to the stacked rows.
const TITLE_PREF: usize = 32;
const TITLE_MIN: usize = 20;

/// What a piece of a cell is drawn in, resolved against whether it sits on
/// the cursor's row: the mark keeps its register on every row.
#[derive(Clone, Copy, PartialEq)]
enum Ink {
    Base,
    Dim1,
    Dim2,
    Dim3,
    Mark(Style),
}

/// One line of a cell, or of a row: its pieces in order.
type Run = Vec<(String, Ink)>;

fn run_w(r: &Run) -> usize {
    r.iter().map(|(t, _)| t.width()).sum()
}

/// One PR's facts in the dialog's words. The forge's words cross
/// `scrub_cells` here; an empty fact is a cell left blank.
struct Facts {
    glyph: char,
    mark: Ink,
    number: String,
    repo: String,
    title: String,
    state: Run,
    pipeline: Run,
    review: String,
    reached: String,
    updated: String,
}

fn facts(app: &App, p: &PrItem, now_ms: u64) -> Facts {
    let one = |s: &str| crate::text::one_line(&scrub_cells(s, false));
    let (mark, reg) = app.pr_mark(p);
    let mut state = vec![(one(&p.state), Ink::Base)];
    if p.state == "merged" && !p.merged_oid.is_empty() {
        let short: String = one(&p.merged_oid).chars().take(7).collect();
        state.push((format!(" {short}"), Ink::Dim2));
    }
    if p.conflict {
        state.push((", conflicts".into(), Ink::Base));
    }
    let pipeline = match (p.pipeline.as_str(), p.job.as_deref()) {
        (w @ ("failed" | "waiting" | "running"), Some(job)) if !job.is_empty() => {
            vec![(format!("{w}:"), Ink::Base), (format!(" {}", one(job)), Ink::Dim2)]
        }
        (w @ ("failed" | "waiting" | "running" | "passed"), _) => vec![(w.to_string(), Ink::Base)],
        _ => Vec::new(),
    };
    let review = match p.review.as_str() {
        "approved" => "approved",
        "changes_requested" => "changes requested",
        "required" => "required",
        _ => "",
    };
    let updated =
        if p.updated_ms > 0 { crate::text::age_ago(now_ms, p.updated_ms) } else { String::new() };
    Facts {
        glyph: mark,
        mark: Ink::Mark(super::card::register_style(&app.theme, reg)),
        number: p.label(),
        repo: one(&p.repo),
        title: one(&p.title),
        state,
        pipeline,
        review: review.into(),
        reached: p.deployed.iter().map(|d| one(d)).collect::<Vec<_>>().join(", "),
        updated,
    }
}

/// A drawn PR: its lines, each a run as wide as the room or narrower.
struct Row {
    lines: Vec<Run>,
}

/// The table: the heading line, the rows and the inner width, or `None`
/// where it does not fit `room` with every column at its narrowest.
/// A column every row leaves blank is dropped, and REPO with it when every
/// PR is in one repository (the title edge names it then). Past the room,
/// the title gives way to `TITLE_PREF` first, then the columns of words
/// (state, pipeline, review, environments) wrap down to their longest word,
/// widest first, then the title to `TITLE_MIN`: a row grows a line rather
/// than lose a word.
fn table(all: &[Facts], many_repos: bool, room: usize) -> Option<(Run, Vec<Row>, usize)> {
    let text =
        |s: &str, ink: Ink| if s.is_empty() { Vec::new() } else { vec![(s.to_string(), ink)] };
    struct Col {
        head: &'static str,
        cells: Vec<Run>,
        title: bool,
        wraps: bool,
    }
    let col = |head, cells: Vec<Run>| Col { head, cells, title: false, wraps: false };
    let words = |head, cells: Vec<Run>| Col { head, cells, title: false, wraps: true };
    let mut cols = vec![col("PR", all.iter().map(|f| text(&f.number, Ink::Base)).collect())];
    if many_repos {
        cols.push(col("REPO", all.iter().map(|f| text(&f.repo, Ink::Dim1)).collect()));
    }
    cols.push(Col {
        head: "TITLE",
        cells: all.iter().map(|f| text(&f.title, Ink::Base)).collect(),
        title: true,
        wraps: true,
    });
    cols.push(words("STATE", all.iter().map(|f| f.state.clone()).collect()));
    cols.push(words("PIPELINE", all.iter().map(|f| f.pipeline.clone()).collect()));
    cols.push(words("REVIEW", all.iter().map(|f| text(&f.review, Ink::Base)).collect()));
    cols.push(words("REACHED", all.iter().map(|f| text(&f.reached, Ink::Base)).collect()));
    cols.push(col("UPDATED", all.iter().map(|f| text(&f.updated, Ink::Dim2)).collect()));
    cols.retain(|c| matches!(c.head, "PR" | "STATE") || c.cells.iter().any(|r| !r.is_empty()));

    let mut widths: Vec<usize> = cols
        .iter()
        .map(|c| c.cells.iter().map(run_w).max().unwrap_or(0).max(c.head.width()))
        .collect();
    let longest_word = |c: &Col| {
        let text: Vec<String> =
            c.cells.iter().map(|r| r.iter().map(|(t, _)| t.as_str()).collect()).collect();
        text.iter().flat_map(|t| t.split(' ')).map(UnicodeWidthStr::width).max().unwrap_or(0)
    };
    let floors: Vec<usize> = cols
        .iter()
        .zip(&widths)
        .map(|(c, w)| match (c.title, c.wraps) {
            (true, _) => TITLE_MIN.min(*w),
            (false, true) => longest_word(c).max(c.head.width()).min(*w),
            (false, false) => *w,
        })
        .collect();
    let total = |w: &[usize]| LEAD + MARGIN + GAP * (w.len() - 1) + w.iter().sum::<usize>();
    let title = cols.iter().position(|c| c.title);
    let others: Vec<usize> = (0..cols.len()).filter(|&i| cols[i].wraps && !cols[i].title).collect();
    if let Some(t) = title {
        let over = total(&widths).saturating_sub(room);
        squeeze(&mut widths, &[t], &[TITLE_PREF.max(floors[t])], over);
    }
    let over = total(&widths).saturating_sub(room);
    squeeze(&mut widths, &others, &floors, over);
    if let Some(t) = title {
        let over = total(&widths).saturating_sub(room);
        squeeze(&mut widths, &[t], &floors, over);
    }
    let inner_w = total(&widths);
    if inner_w > room {
        return None;
    }

    let mut head: Run = vec![(" ".repeat(LEAD), Ink::Dim3)];
    for (i, (c, w)) in cols.iter().zip(&widths).enumerate() {
        let gap = if i + 1 < cols.len() { GAP } else { 0 };
        head.push((format!("{}{}", c.head, " ".repeat(w - c.head.width() + gap)), Ink::Dim3));
    }
    let rows = all
        .iter()
        .enumerate()
        .map(|(r, f)| {
            // Each cell's lines: a cell of words wrapped to its column,
            // the number, the repository and the age one line each.
            let cells: Vec<Vec<Run>> = cols
                .iter()
                .zip(&widths)
                .map(
                    |(c, w)| if c.wraps { wrap(&c.cells[r], *w) } else { vec![c.cells[r].clone()] },
                )
                .collect();
            let height = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
            let lines = (0..height)
                .map(|y| {
                    let mut line: Run = if y == 0 {
                        vec![
                            (" ".into(), Ink::Base),
                            (f.glyph.to_string(), f.mark),
                            (" ".into(), Ink::Base),
                        ]
                    } else {
                        vec![(" ".repeat(LEAD), Ink::Base)]
                    };
                    for (i, (cell, w)) in cells.iter().zip(&widths).enumerate() {
                        let piece = cell.get(y).cloned().unwrap_or_default();
                        let gap = if i + 1 < cols.len() { GAP } else { 0 };
                        let pad = w.saturating_sub(run_w(&piece)) + gap;
                        line.extend(piece);
                        line.push((" ".repeat(pad), Ink::Base));
                    }
                    line
                })
                .collect();
            Row { lines }
        })
        .collect();
    Some((head, rows, inner_w))
}

/// Narrow the columns `which` toward their `floors` (indexed as `widths`,
/// or one floor for one column) by `excess` cells in all, a cell at a time
/// off the widest, so the columns that give way end up alike.
fn squeeze(widths: &mut [usize], which: &[usize], floors: &[usize], mut excess: usize) {
    let floor = |i: usize| if floors.len() == 1 { floors[0] } else { floors[i] };
    while excess > 0 {
        let Some(&i) = which.iter().filter(|&&i| widths[i] > floor(i)).max_by_key(|&&i| widths[i])
        else {
            return;
        };
        widths[i] -= 1;
        excess -= 1;
    }
}

/// The stacked rows, at `width`: the mark, the number and the title (or,
/// with none, the repository and the state) on the head line, and every
/// other fact labelled and wrapped under it, aligned past the number.
fn stacked(all: &[Facts], many_repos: bool, width: usize) -> Vec<Row> {
    let num_w = all.iter().map(|f| f.number.width()).max().unwrap_or(0);
    let indent = LEAD + num_w + GAP;
    let room = width.saturating_sub(indent + MARGIN).max(1);
    let dot = || (" ∙ ".to_string(), Ink::Dim3);
    all.iter()
        .map(|f| {
            let mut head: Run = Vec::new();
            let mut rest: Vec<Run> = Vec::new();
            if many_repos {
                let repo = vec![(f.repo.clone(), Ink::Dim1)];
                if f.title.is_empty() {
                    head.extend(repo)
                } else {
                    rest.push(repo)
                }
            }
            if f.title.is_empty() {
                if !head.is_empty() {
                    head.push(("  ".into(), Ink::Base));
                }
                head.extend(f.state.clone());
            } else {
                head.push((f.title.clone(), Ink::Base));
                rest.push(f.state.clone());
            }
            let label = |word: &str, value: Run| {
                let mut r = vec![(format!("{word} "), Ink::Dim2)];
                r.extend(value);
                r
            };
            if !f.pipeline.is_empty() {
                rest.push(label("pipeline", f.pipeline.clone()));
            }
            if !f.review.is_empty() {
                rest.push(label("review", vec![(f.review.clone(), Ink::Base)]));
            }
            if !f.reached.is_empty() {
                rest.push(label("in", vec![(f.reached.clone(), Ink::Base)]));
            }
            if !f.updated.is_empty() {
                rest.push(label("updated", vec![(f.updated.clone(), Ink::Dim2)]));
            }
            let mut facts: Run = Vec::new();
            for (i, r) in rest.into_iter().enumerate() {
                if i > 0 {
                    facts.push(dot());
                }
                facts.extend(r);
            }
            let mut lines: Vec<Run> = Vec::new();
            for (y, piece) in wrap(&head, room).into_iter().enumerate() {
                let mut line: Run = if y == 0 {
                    vec![
                        (" ".into(), Ink::Base),
                        (f.glyph.to_string(), f.mark),
                        (" ".into(), Ink::Base),
                        (format!("{:<num_w$}", f.number), Ink::Base),
                        (" ".repeat(GAP), Ink::Base),
                    ]
                } else {
                    vec![(" ".repeat(indent), Ink::Base)]
                };
                line.extend(piece);
                lines.push(line);
            }
            for piece in wrap(&facts, room) {
                let mut line: Run = vec![(" ".repeat(indent), Ink::Base)];
                line.extend(piece);
                lines.push(line);
            }
            Row { lines }
        })
        .collect()
}

/// A run word-wrapped to `width` cells, its inks kept: words break at
/// spaces, a word wider than the width splits where it fills a line, and a
/// lone `∙` holds on to the word before it, so no line opens on one.
fn wrap(run: &Run, width: usize) -> Vec<Run> {
    let width = width.max(1);
    let mut words: Vec<Vec<(char, Ink)>> = Vec::new();
    let mut cur: Vec<(char, Ink)> = Vec::new();
    for (text, ink) in run {
        for ch in text.chars() {
            if ch == ' ' {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push((ch, *ink));
            }
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let mut glued: Vec<Vec<(char, Ink)>> = Vec::new();
    for w in words {
        match glued.last_mut() {
            Some(last) if w.len() == 1 && w[0].0 == '∙' => {
                last.push((' ', w[0].1));
                last.extend(w);
            }
            _ => glued.push(w),
        }
    }
    let cw = |c: char| c.width().unwrap_or(0);
    let mut lines: Vec<Vec<(char, Ink)>> = Vec::new();
    let mut line: Vec<(char, Ink)> = Vec::new();
    let mut used = 0usize;
    for word in glued {
        let ww: usize = word.iter().map(|(c, _)| cw(*c)).sum();
        if used > 0 && used + 1 + ww <= width {
            line.push((' ', word[0].1));
            line.extend(word);
            used += 1 + ww;
            continue;
        }
        if used > 0 {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        for (c, ink) in word {
            if used > 0 && used + cw(c) > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push((c, ink));
            used += cw(c);
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
        .into_iter()
        .map(|chars| {
            let mut out: Run = Vec::new();
            for (c, ink) in chars {
                match out.last_mut() {
                    Some((t, i)) if *i == ink => t.push(c),
                    _ => out.push((c.to_string(), ink)),
                }
            }
            out
        })
        .collect()
}

/// A run as spans, on the cursor's row in the selection's register.
fn paint(app: &App, run: &Run, selected: bool) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let ramp = if selected { &theme.sel } else { &theme.rest };
    run.iter()
        .map(|(t, ink)| {
            let style = match ink {
                Ink::Base if selected => {
                    Style::default().fg(ramp.base).add_modifier(Modifier::BOLD)
                }
                Ink::Base => Style::default().fg(ramp.base),
                Ink::Dim1 => Style::default().fg(ramp.dim1),
                Ink::Dim2 => Style::default().fg(ramp.dim2),
                Ink::Dim3 => Style::default().fg(ramp.dim3),
                Ink::Mark(s) => *s,
            };
            Span::styled(t.clone(), style)
        })
        .collect()
}

/// The dialog over `ticket`, the cursor on `idx` (clamped to the rows the
/// snapshot left). The title says the repository when there is one, and
/// when the forge was last read, as the Usage dialog says it of a quota.
pub(super) fn draw(f: &mut Frame, app: &App, ticket: ulid::Ulid, idx: usize) {
    let theme = &app.theme;
    let key = app.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
    let prs = app.prs_of(ticket);
    let read = (app.forge.sampled_at_ms > 0).then(|| {
        let now = mesimon_core::clock::now_ms();
        format!(" ∙ read {}", crate::text::age_ago(now, app.forge.sampled_at_ms))
    });
    let screen = f.area();
    let room = usize::from(screen.width).saturating_sub(6);
    if prs.is_empty() {
        let name = format!("PULL REQUESTS ∙ {key}{}", read.unwrap_or_default());
        let room = usize::from(dialog::MAX_W).min(room);
        let lines = crate::peek::wrap(EMPTY, room.saturating_sub(2).max(1), usize::MAX);
        let area = dialog::centred(screen, lines.len() as u16, dialog::MAX_W);
        let inner_w = area.width.saturating_sub(2) as usize;
        let inner = dialog::frame(
            f,
            app,
            area,
            None,
            &theme.rest,
            Edges {
                title: dialog::title(&theme.rest, name),
                tail: dialog::keys(
                    app,
                    Scope::PullRequests,
                    &theme.rest,
                    inner_w.saturating_sub(4),
                ),
            },
        );
        let lines: Vec<Line<'static>> = lines
            .into_iter()
            .map(|l| Line::from(Span::styled(format!(" {l}"), theme.dim2())))
            .collect();
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let now = mesimon_core::clock::now_ms();
    let all: Vec<Facts> = prs.iter().map(|p| facts(app, p, now)).collect();
    let many_repos = all.iter().any(|f| f.repo != all[0].repo);
    let (head, rows, inner_w) = match table(&all, many_repos, room) {
        Some((head, rows, w)) => (Some(head), rows, w.max(usize::from(dialog::MAX_W)).min(room)),
        None => {
            let w = usize::from(dialog::MAX_W).min(room);
            (None, stacked(&all, many_repos, w), w)
        }
    };
    let idx = idx.min(rows.len() - 1);
    let heights: Vec<usize> = rows.iter().map(|r| r.lines.len()).collect();
    let head_h = usize::from(head.is_some());
    let wanted = head_h + heights.iter().sum::<usize>();
    let mut area =
        dialog::centred(screen, u16::try_from(wanted).unwrap_or(u16::MAX), inner_w as u16);
    // The row above the footer stays the cursor's link.
    let whole_y = screen.bottom().saturating_sub(2);
    area.height = area.height.min(whole_y.saturating_sub(area.y)).max(3);
    let room_h = usize::from(area.height.saturating_sub(2)).saturating_sub(head_h).max(1);
    let (first, last) = dialog::window(&heights, idx, room_h);

    let mut name = format!("PULL REQUESTS ∙ {key}");
    if last - first < rows.len() {
        name.push_str(&format!(" ∙ {}/{}", idx + 1, rows.len()));
    } else {
        name.push_str(&format!(" ∙ {}", rows.len()));
    }
    if !many_repos && !all[0].repo.is_empty() {
        name.push_str(&format!(" ∙ {}", all[0].repo));
    }
    name.push_str(&read.unwrap_or_default());
    let tail_w = usize::from(area.width.saturating_sub(2)).saturating_sub(4);
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        Edges {
            title: dialog::title(&theme.rest, name),
            tail: dialog::keys(app, Scope::PullRequests, &theme.rest, tail_w),
        },
    );
    let width = usize::from(inner.width);
    if let Some(url) = prs.get(idx).map(|p| p.url.as_str()).filter(|u| !u.is_empty()) {
        dialog::whole_row(f, app, whole_y, url, true);
    }

    let pad = |mut spans: Vec<Span<'static>>, run: &Run| {
        spans.push(Span::raw(" ".repeat(width.saturating_sub(run_w(run)))));
        spans
    };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(head) = &head {
        lines.push(Line::from(pad(paint(app, head, false), head)));
    }
    let mut hits = app.hits.borrow_mut();
    for (i, row) in rows.iter().enumerate().take(last).skip(first) {
        let selected = i == idx;
        let y = inner.y + lines.len() as u16;
        let h = (row.lines.len() as u16).min(inner.bottom().saturating_sub(y));
        hits.record(Rect { y, height: h, ..inner }, crate::mouse::Target::Row(i));
        let row_style = if selected { theme.selected_row() } else { Style::default() };
        for run in &row.lines {
            lines.push(Line::from(pad(paint(app, run, selected), run)).style(row_style));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}
