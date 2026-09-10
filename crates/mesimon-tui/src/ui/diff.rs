//! The read-only diff viewer (M4b, docs/08 §1.3 hunk model + §10 layout minus
//! the dead rail). Review only — no accept, no checkout, no send-back; `!`
//! opens a shell in the worktree as the escape hatch. Same laws as every
//! screen: painted bands, never drawn rules (L1); no syntax highlighting
//! (L3 — one saturated colour, reserved). Adds/deletes ride the calm/err
//! registers (muted green/red; author 2026-08-30 amendment) plus glyph AND
//! weight, so review still reads correctly in mono.

use mesimon_core::diff::{Render, Sign};
use mesimon_core::keymap::{self, Scope, Verb};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, DiffState, PreviewView};
use crate::glyphs::Tier;
use crate::text::truncate;

use super::chrome;

/// docs/08 §10.2's breakpoint table minus the rail (D33k): below this the
/// screen is a single pane and `z p` swaps file list ⇄ diff.
const TWO_PANE_MIN_W: u16 = 100;
const FILES_W: u16 = 28;
/// ≥140 the file list gets the 08 §10.2 outline width — 28 truncates paths
/// early once the terminal has room to spare (dogfood 2026-08-30).
const FILES_W_WIDE: u16 = 36;
const TAB_W: usize = 8;

fn files_w(total: u16) -> u16 {
    if total >= 140 {
        FILES_W_WIDE
    } else {
        FILES_W
    }
}

/// The z z density cycle in words — `-U1/-U3/-U8` read as noise in dogfood.
pub(crate) fn density_word(context: u32) -> &'static str {
    match context {
        1 => "tight",
        8 => "wide",
        _ => "normal",
    }
}

pub(super) fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = f.area();
    let Some(d) = app.diff.as_ref() else { return };
    d.view.set(PreviewView::default());

    // ---- top block: the header (DIFF chip, breadcrumb, and the ticket as its
    // leaf where there is one — the checkout's diff belongs to the repo, which
    // the breadcrumb already names), identity, painted band -------------------
    let leaf = app.diff_ticket().and_then(|id| app.board.ticket(id)).map(|t| t.title.as_str());
    chrome::draw_header(f, Rect { x: area.x, y: area.y, width: area.width, height: 1 }, app, leaf);

    let (adds, dels) = d
        .files
        .iter()
        .fold((0u32, 0u32), |(a, del), f| (a + f.adds.unwrap_or(0), del + f.dels.unwrap_or(0)));
    let n = d.files.len();
    let noun = if n == 1 { "file" } else { "files" };
    // A branch is measured against the base it forked from; the checkout is
    // measured against itself, so the word is what it is rather than an oid.
    let against = if d.is_branch() {
        let base8: String = d.base_oid.chars().take(8).collect();
        format!("vs {base8}")
    } else {
        "uncommitted".to_string()
    };
    let summary = if d.commits {
        match &app.git.upstream {
            Some(upstream) => format!("push / pull ∙ {}", crate::text::one_line(upstream)),
            None => "push / pull".to_string(),
        }
    } else if n == 0 {
        "no changes".to_string()
    } else {
        format!("{against} ∙ {n} {noun} ∙ +{adds} -{dels}")
    };
    let branch = if d.commits { &app.git.branch } else { &d.branch };
    let mut ident = vec![
        Span::styled(format!(" ⎇ {}", crate::text::one_line(branch)), theme.dim1()),
        Span::styled(format!(" ∙ {summary}"), theme.dim2()),
    ];
    if d.is_branch() && !d.worktree_present {
        ident.push(Span::styled(" ∙ worktree evicted".to_string(), theme.dim2()));
    }

    // Real space cells — the empty-Line band idiom paints nothing (see the
    // identical note in ticket.rs).
    let band = match theme.selected_bg {
        Some(bg) => {
            Line::from(Span::raw(" ".repeat(area.width as usize))).style(Style::default().bg(bg))
        }
        None => Line::default(),
    };
    let top = vec![Line::default(), Line::from(ident), band];
    f.render_widget(
        Paragraph::new(top),
        Rect { x: area.x, y: area.y + 1, width: area.width, height: 3.min(area.height - 1) },
    );

    // ---- body: file list pane + hunk pane (or one of them, below the
    // breakpoint) — divider is a painted gap, never a rule (L1).
    let body_y = area.y + 5;
    let body_h = area.height.saturating_sub(6);
    if d.commits {
        draw_commits(
            f,
            Rect { x: area.x + 1, y: body_y, width: area.width.saturating_sub(2), height: body_h },
            app,
            d,
        );
    } else if area.width >= TWO_PANE_MIN_W {
        let fw = files_w(area.width);
        draw_files(f, Rect { x: area.x + 1, y: body_y, width: fw, height: body_h }, app, d);
        let hx = area.x + 1 + fw + 3;
        draw_hunks(
            f,
            Rect { x: hx, y: body_y, width: area.width.saturating_sub(hx + 1), height: body_h },
            app,
            d,
        );
    } else if d.swap {
        draw_hunks(
            f,
            Rect { x: area.x + 1, y: body_y, width: area.width.saturating_sub(2), height: body_h },
            app,
            d,
        );
    } else {
        draw_files(
            f,
            Rect { x: area.x + 1, y: body_y, width: area.width.saturating_sub(2), height: body_h },
            app,
            d,
        );
    }

    if d.view.get().key.is_none() {
        d.glide.set(None);
    }

    // ---- footer ----------------------------------------------------------
    // From the keymap, like every other screen. `z s` drops itself above the
    // two-pane breakpoint because there is nothing to swap up there — which
    // is why draw records the breakpoint for the keymap to read.
    app.diff_two_pane.set(area.width >= TWO_PANE_MIN_W);
    let footer = chrome::footer_line(app, area.width);
    f.render_widget(
        Paragraph::new(footer),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
}

/// Both directions share the reading keys and a single scroll position.
/// Counts stay exact even when the snapshot's bounded history is truncated.
fn draw_commits(f: &mut Frame, area: Rect, app: &App, d: &DiffState) {
    let theme = &app.theme;
    let g = &app.git;
    let w = area.width as usize;
    let mut rows = Vec::new();
    if !g.sampled {
        rows.push(Line::from(Span::styled(" Git status unavailable", theme.dim2())));
    } else if g.detached || g.upstream.is_none() {
        rows.push(Line::from(Span::styled(
            if g.detached {
                " Detached HEAD — no upstream comparison"
            } else {
                " No upstream configured"
            },
            theme.dim2(),
        )));
    } else {
        for (label, count, commits) in
            [("TO PUSH", g.ahead, &g.to_push), ("TO PULL", g.behind, &g.to_pull)]
        {
            rows.push(Line::from(Span::styled(
                format!(" {label} ({count})"),
                theme.dim1().add_modifier(Modifier::BOLD),
            )));
            rows.push(Line::default());
            if count == 0 {
                rows.push(Line::from(Span::styled(" Nothing pending", theme.dim3())));
            } else if let Some(commits) = commits {
                for commit in commits {
                    let oid: String = commit.oid.chars().take(7).collect();
                    let subject = crate::text::one_line(&commit.subject);
                    rows.push(Line::from(vec![
                        Span::styled(format!(" {oid}  "), theme.dim2()),
                        Span::styled(truncate(&subject, w.saturating_sub(10)), theme.base()),
                    ]));
                }
                let remaining = (count as usize).saturating_sub(commits.len());
                if remaining > 0 {
                    rows.push(Line::from(Span::styled(
                        format!(" … {remaining} more commits"),
                        theme.dim3(),
                    )));
                }
            } else {
                rows.push(Line::from(Span::styled(" Commit list unavailable", theme.dim2())));
            }
            rows.push(Line::default());
            rows.push(Line::default());
        }
        rows.push(Line::from(Span::styled(
            truncate(&format!(" {}", app.git_fetch_note()), w),
            theme.dim2(),
        )));
    }
    let visible = (area.height as usize).saturating_sub(2);
    let max = rows.len().saturating_sub(visible);
    let scroll = d.scroll.get().min(max);
    d.scroll.set(scroll);
    d.view.set(PreviewView {
        key: Some(u64::MAX),
        offset: scroll,
        max,
        page: visible.saturating_sub(1).max(1),
        follows_tail: false,
    });
    let at = d
        .glide
        .get()
        .filter(|g| g.key == u64::MAX && g.progress().is_some())
        .map_or(scroll, |g| g.offset(scroll).min(max));
    let mut lines =
        vec![Line::from(hints(app, &[Verb::PageDown, Verb::ScrollDown], w)), Line::default()];
    lines.extend(rows.into_iter().skip(at).take(visible));
    f.render_widget(Paragraph::new(lines), area);
}

/// The file-list pane: 2-cell gutter (stable letter + in-flight flag), path,
/// right-aligned +/- badge.
fn draw_files(f: &mut Frame, area: Rect, app: &App, d: &DiffState) {
    let theme = &app.theme;
    let w = area.width as usize;
    let mut head = vec![Span::styled(" FILES", theme.dim1().add_modifier(Modifier::BOLD))];
    let right = format!("({})", d.files.len());
    if d.files.len() > 1 {
        let keys = hints(app, &[Verb::NextFile], w.saturating_sub(6 + right.width() + 5));
        if !keys.is_empty() {
            head.push(Span::raw("  "));
            head.extend(keys);
        }
    }
    head.push(Span::raw(
        " ".repeat(w.saturating_sub(super::spans_width(&head) + right.width() + 1)),
    ));
    head.push(Span::styled(right, theme.dim2()));
    let mut lines: Vec<Line<'static>> = vec![Line::from(head), Line::default()];

    if d.files.is_empty() {
        let empty = if d.is_branch() {
            format!(" no commits on {} yet", d.branch)
        } else {
            format!(" nothing uncommitted on {}", d.branch)
        };
        lines.push(Line::from(Span::styled(empty, theme.dim3())));
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    // Window the list around the cursor when it overflows the pane.
    let visible = (area.height as usize).saturating_sub(2).max(1);
    let first = d
        .file_idx
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(d.files.len().saturating_sub(visible));
    for (i, entry) in d.files.iter().enumerate().skip(first).take(visible) {
        let selected = i == d.file_idx;
        let stable = if entry.status.is_empty() { "·" } else { entry.status.as_str() };
        // The in-flight column says what the committed state does not know
        // about. On the checkout diff EVERY row is uncommitted by
        // construction, so `D` there would be a letter on every line saying
        // nothing; `U` still earns its cell, because untracked is a different
        // thing from unstaged.
        let inflight = if entry.untracked {
            "U"
        } else if entry.dirty && d.is_branch() {
            "D"
        } else {
            " "
        };
        let badge = match (entry.adds, entry.dels) {
            (Some(a), Some(x)) => format!("+{a} -{x}"),
            _ if entry.status.is_empty() => String::new(),
            _ => "-".to_string(),
        };
        // Row: " SF" gutter (3) + " path" + ≥2 fill + badge + 1 right pad.
        let name_budget = w.saturating_sub(4 + badge.width() + 3);
        // An overflowing path on the selected row reveals itself marquee-style
        // (same clock behaviour as the board card title and the ticket rail:
        // reset on landing, one pass, rest truncated).
        let overflow = entry.path.width().saturating_sub(name_budget);
        let scroll = if selected && overflow > 0 {
            let ms = match d.marquee.get() {
                Some((idx, epoch)) if idx == i => epoch.elapsed().as_millis() as u64,
                _ => {
                    d.marquee.set(Some((i, std::time::Instant::now())));
                    0
                }
            };
            crate::text::marquee_offset(ms, overflow)
        } else {
            0
        };
        let path = if scroll > 0 {
            crate::text::marquee_window(&entry.path, name_budget, scroll)
        } else {
            truncate(&entry.path, name_budget)
        };
        let fill = w.saturating_sub(4 + path.width() + badge.width() + 1).max(2);
        let name_style = if selected {
            Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.base()
        };
        let row_style = if selected { theme.selected_row() } else { Style::default() };
        lines.push(
            Line::from(vec![
                Span::styled(format!(" {stable}{inflight}"), theme.dim2()),
                Span::styled(format!(" {path}"), name_style),
                Span::raw(" ".repeat(fill)),
                Span::styled(badge, theme.dim2()),
            ])
            .style(row_style),
        );
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// The hunk pane: per-file header, then hunk bands + soft-wrapped body lines.
fn draw_hunks(f: &mut Frame, area: Rect, app: &App, d: &DiffState) {
    let theme = &app.theme;
    let w = area.width as usize;
    let Some(entry) = d.files.get(d.file_idx) else {
        f.render_widget(Paragraph::new(Vec::<Line>::new()), area);
        return;
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    let counter = format!(" ∙ file {}/{}", d.file_idx + 1, d.files.len());
    let mut head = vec![
        Span::styled(
            truncate(&entry.path, w.saturating_sub(counter.width() + 12)),
            theme.base().add_modifier(Modifier::BOLD),
        ),
        Span::styled(counter, theme.dim2()),
    ];

    // Untracked-only rows are display-only — the honesty rule (docs/08 §2).
    if entry.status.is_empty() {
        lines.push(hunk_heading(app, d, head, w, f.area().width >= TWO_PANE_MIN_W));
        lines.push(Line::default());
        for row in [
            "untracked in the worktree — not reviewable",
            "mesimon reads committed state only, so nothing here can race the agent",
        ] {
            lines.push(Line::from(Span::styled(row.to_string(), theme.dim2())));
        }
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    let Some(fd) = d.cache.get(&entry.path) else {
        lines.push(hunk_heading(app, d, head, w, f.area().width >= TWO_PANE_MIN_W));
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("…".to_string(), theme.dim3())));
        f.render_widget(Paragraph::new(lines), area);
        return;
    };

    // Body content by render kind — the exhaustive enum (a skipped case is a
    // day-one panic, docs/08 §1.3).
    let mut body: Vec<Line<'static>> = Vec::new();
    match &fd.render {
        Render::Text | Render::Symlink => {
            let nh = fd.hunks.len();
            let noun = if nh == 1 { "hunk" } else { "hunks" };
            head.push(Span::styled(format!(" ∙ {nh} {noun}"), theme.dim2()));
            if let Some(old) = &fd.old_path {
                body.push(Line::from(Span::styled(format!("renamed from {old}"), theme.dim2())));
                body.push(Line::default());
            }
            if matches!(fd.render, Render::Symlink) {
                body.push(Line::from(Span::styled("symlink target".to_string(), theme.dim2())));
                body.push(Line::default());
            }
            if fd.hunks.is_empty() && fd.old_path.is_none() {
                body.push(Line::from(Span::styled("no content change".to_string(), theme.dim3())));
            }
            let tier = theme.glyph_tier();
            let cont = if tier == Tier::Ascii { '>' } else { '↳' };
            let code_w = w.saturating_sub(9).max(8);
            for h in &fd.hunks {
                // Hunk header: a painted band row, never a drawn rule (L1).
                // A Line's style covers only its text cells, so the band (and
                // every tinted row below) pads to the pane width by hand.
                let band_text = format!(
                    "@@ -{},{} +{},{} @@ {}",
                    h.old_start, h.old_len, h.new_start, h.new_len, h.header
                );
                let band_style = match theme.selected_bg {
                    Some(bg) => Style::default().bg(bg),
                    None => Style::default(),
                };
                body.push(
                    Line::from(Span::styled(pad_to(truncate(&band_text, w), w), theme.dim1()))
                        .style(band_style),
                );
                for l in &h.lines {
                    // Adds/deletes in colour (author 2026-08-30, amending the
                    // grey-ramp-only rule): the calm/err registers — muted
                    // green/red, theme- and profile-aware — never raw RGB.
                    // Full-line grounds ride the diff tints where the profile
                    // has them (a Line's style paints the whole row, the
                    // band-row precedent). Glyph + weight stay for mono.
                    let (sign, num, style, line_bg) = match l.sign {
                        Sign::Ctx => (' ', l.new_ln, theme.dim2(), None),
                        Sign::Add => (
                            '+',
                            l.new_ln,
                            theme.calm_text().add_modifier(Modifier::BOLD),
                            theme.diff_add_bg(),
                        ),
                        Sign::Del => ('-', l.old_ln, theme.err_text(), theme.diff_del_bg()),
                    };
                    let num = num.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".into());
                    for (j, seg) in wrap_code(&l.text, code_w).into_iter().enumerate() {
                        let row = if j == 0 {
                            format!("{sign} {num}  {seg}")
                        } else {
                            // Continuation: gutter marker, blank number
                            // column, and the seg's own 2-space indent — the
                            // prefix must stay 9 cols like "X 12345  " or the
                            // row overflows the pane (docs/08 §1.3).
                            format!("{cont}        {seg}")
                        };
                        let line = match line_bg {
                            Some(bg) => Line::from(Span::styled(pad_to(row, w), style))
                                .style(Style::default().bg(bg)),
                            None => Line::from(Span::styled(row, style)),
                        };
                        body.push(line);
                    }
                }
                body.push(Line::default());
            }
        }
        Render::Binary => {
            body.push(Line::from(Span::styled("binary ∙ no diff".to_string(), theme.dim1())));
        }
        Render::ModeOnly { old_mode, new_mode } => {
            body.push(Line::from(Span::styled(
                format!("mode {old_mode} → {new_mode}"),
                theme.dim1(),
            )));
        }
        Render::Submodule { old_oid, new_oid } => {
            let o: String = old_oid.chars().take(7).collect();
            let ne: String = new_oid.chars().take(7).collect();
            body.push(Line::from(Span::styled(
                format!("submodule {} {o} → {ne}", fd.path),
                theme.dim1(),
            )));
        }
        Render::TooLarge { bytes } => {
            // `!` is the reader for a worktree, which is a directory hard to
            // reach. On the checkout there is nothing to offer: the user is
            // already standing in it.
            let mb = *bytes as f64 / 1_048_576.0;
            let word = if d.is_branch() {
                format!("{mb:.1} MB — ! opens it in your shell")
            } else {
                format!("{mb:.1} MB — too large to render")
            };
            body.push(Line::from(Span::styled(word, theme.dim1())));
        }
        Render::Unresolvable { message } => {
            body.push(Line::from(Span::styled(message.clone(), theme.err_text())));
        }
    }

    // j/k scroll: clamp against the built content, write the clamp back.
    let visible = (area.height as usize).saturating_sub(2);
    let max_scroll = body.len().saturating_sub(visible);
    let scroll = d.scroll.get().min(max_scroll);
    d.scroll.set(scroll);
    d.view.set(PreviewView {
        key: Some(d.file_idx as u64),
        offset: scroll,
        max: max_scroll,
        page: visible.saturating_sub(1).max(1),
        follows_tail: false,
    });
    let at = match d.glide.get() {
        Some(g) if g.key == d.file_idx as u64 && g.progress().is_some() => {
            g.offset(scroll).min(max_scroll)
        }
        _ => {
            d.glide.set(None);
            scroll
        }
    };
    lines.push(hunk_heading(app, d, head, w, f.area().width >= TWO_PANE_MIN_W));
    lines.push(Line::default());
    lines.extend(body.into_iter().skip(at).take(visible));
    f.render_widget(Paragraph::new(lines), area);
}

fn hunk_heading(
    app: &App,
    d: &DiffState,
    mut head: Vec<Span<'static>>,
    w: usize,
    two_pane: bool,
) -> Line<'static> {
    let mut verbs = Vec::new();
    if d.view.get().max > 0 {
        verbs.push(Verb::PageDown);
    }
    if !two_pane && d.files.len() > 1 {
        verbs.push(Verb::NextFile);
    }
    if d.view.get().max > 0 {
        verbs.push(Verb::ScrollDown);
    }
    let keys = hints(app, &verbs, w / 2);
    if !keys.is_empty() {
        let keys_w = super::spans_width(&keys);
        let meta_w = super::spans_width(&head[1..]);
        head[0].content = truncate(&head[0].content, w.saturating_sub(meta_w + keys_w + 3)).into();
        head.push(Span::raw(" ".repeat(w.saturating_sub(super::spans_width(&head) + keys_w + 1))));
        head.extend(keys);
    }
    Line::from(head)
}

/// Contextual hints use the same binding text and styling as the footer.
fn hints(app: &App, verbs: &[Verb], width: usize) -> Vec<Span<'static>> {
    let ctx = app.ctx();
    let bindings: Vec<_> =
        verbs.iter().filter_map(|v| keymap::binding_for(Scope::Diff, *v, &ctx)).collect();
    chrome::hint_spans(&bindings, &ctx, &app.theme.rest, width)
}

/// Pad with trailing spaces to `width` cells so a row's ground colour spans
/// the pane (a Line's style paints only its text cells).
fn pad_to(mut s: String, width: usize) -> String {
    let have: usize = s.width();
    for _ in have..width {
        s.push(' ');
    }
    s
}

/// Soft-wrap one code line to `width` columns — long lines wrap, never
/// truncate (a truncated line is how you miss the thing you were reviewing).
/// Tabs expand at 8; a `\r` renders as a visible `^M` (a CRLF-vs-LF rewrite
/// must not be invisible). Continuations carry a 2-space indent.
pub(crate) fn wrap_code(text: &str, width: usize) -> Vec<String> {
    let width = width.max(4);
    // Expand tabs / make \r visible first, so wrapping sees real cells.
    let mut expanded = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        match ch {
            '\t' => {
                let next = (col / TAB_W + 1) * TAB_W;
                for _ in col..next {
                    expanded.push(' ');
                }
                col = next;
            }
            '\r' => {
                expanded.push_str("^M");
                col += 2;
            }
            c => {
                expanded.push(c);
                col += c.width().unwrap_or(0);
            }
        }
    }
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut row_w = 0usize;
    let cont_budget = width.saturating_sub(2);
    for c in expanded.chars() {
        let cw = c.width().unwrap_or(0);
        let budget = if rows.is_empty() { width } else { cont_budget };
        if row_w + cw > budget && row_w > 0 {
            rows.push(std::mem::take(&mut row));
            row_w = 0;
        }
        row.push(c);
        row_w += cw;
    }
    rows.push(row);
    let mut out = Vec::with_capacity(rows.len());
    for (i, r) in rows.into_iter().enumerate() {
        if i == 0 {
            out.push(r);
        } else {
            out.push(format!("  {r}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::wrap_code;

    #[test]
    fn preserves_indentation_and_wraps() {
        let rows = wrap_code("    let x = some_very_long_expression;", 20);
        assert!(rows[0].starts_with("    let"), "leading indent preserved: {:?}", rows[0]);
        assert!(rows.len() > 1);
        assert!(rows[1].starts_with("  "), "continuation indent");
    }

    #[test]
    fn tabs_expand_and_cr_is_visible() {
        let rows = wrap_code("\tx\r", 40);
        assert_eq!(rows[0], "        x^M");
    }

    #[test]
    fn wide_graphemes_do_not_overflow() {
        let rows = wrap_code("אאאא 統一碼統一碼統一碼", 8);
        for (i, r) in rows.iter().enumerate() {
            let w: usize =
                r.chars().map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)).sum();
            assert!(w <= 8, "row {i} width {w}: {r:?}");
        }
    }

    #[test]
    fn short_line_single_row() {
        assert_eq!(wrap_code("fn main() {}", 80), vec!["fn main() {}".to_string()]);
    }
}
