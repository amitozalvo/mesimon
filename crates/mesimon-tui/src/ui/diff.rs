//! The read-only diff viewer (M4b, docs/08 §1.3 hunk model + §10 layout minus
//! the dead rail). Review only — no accept, no checkout, no send-back; `!`
//! opens a shell in the worktree as the escape hatch. Same laws as every
//! screen: painted bands, never drawn rules (L1); no syntax highlighting
//! (L3 — one saturated colour, reserved). Adds/deletes ride the calm/err
//! registers (muted green/red; author 2026-08-30 amendment) plus glyph AND
//! weight, so review still reads correctly in mono.

use mesimon_core::diff::{Render, Sign};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, DiffState};
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

pub(super) fn draw(f: &mut Frame, app: &App, ticket_id: ulid::Ulid) {
    let theme = &app.theme;
    let area = f.area();
    let Some(ticket) = app.board.ticket(ticket_id) else { return };
    let Some(d) = app.diff.as_ref() else { return };

    // ---- top block: breadcrumb > title > diff, identity, painted band -----
    let mut head = chrome::breadcrumb(app);
    let sep = Style::default().fg(theme.rest.dim3);
    head.push(Span::styled(" > ".to_string(), sep));
    let prefix_w: usize = head.iter().map(|s| s.content.width()).sum();
    let budget = (area.width as usize).saturating_sub(prefix_w + " > diff ".len());
    head.push(Span::styled(truncate(&ticket.title, budget), Style::default().fg(theme.rest.base)));
    head.push(Span::styled(" > ".to_string(), sep));
    head.push(Span::styled("diff".to_string(), theme.dim1()));

    let (adds, dels) = d.files.iter().fold((0u32, 0u32), |(a, del), f| {
        (a + f.adds.unwrap_or(0), del + f.dels.unwrap_or(0))
    });
    let base8: String = d.base_oid.chars().take(8).collect();
    let n = d.files.len();
    let noun = if n == 1 { "file" } else { "files" };
    let mut ident = vec![
        Span::styled(format!(" ⎇ {}", d.branch), theme.dim1()),
        Span::styled(
            format!(
                " ∙ vs {base8} ∙ {n} {noun} ∙ +{adds} -{dels} ∙ {}",
                density_word(d.density)
            ),
            theme.dim2(),
        ),
    ];
    if !d.worktree_present {
        ident.push(Span::styled(" ∙ worktree evicted".to_string(), theme.dim2()));
    }

    let band = match theme.selected_bg {
        Some(bg) => Line::default().style(Style::default().bg(bg)),
        None => Line::default(),
    };
    let top = vec![Line::from(head), Line::default(), Line::from(ident), band];
    f.render_widget(
        Paragraph::new(top),
        Rect { x: area.x, y: area.y, width: area.width, height: 4.min(area.height) },
    );

    // ---- body: file list pane + hunk pane (or one of them, below the
    // breakpoint) — divider is a painted gap, never a rule (L1).
    let body_y = area.y + 5;
    let body_h = area.height.saturating_sub(6);
    if area.width >= TWO_PANE_MIN_W {
        let fw = files_w(area.width);
        draw_files(
            f,
            Rect { x: area.x + 1, y: body_y, width: fw, height: body_h },
            app,
            d,
        );
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

    // ---- footer ----------------------------------------------------------
    let footer = if app.status.is_empty() {
        let hint = if area.width >= TWO_PANE_MIN_W {
            "jk scroll ∙ {} page ∙ hl file ∙ R refresh ∙ zz density ∙ ! shell ∙ q back".to_string()
        } else {
            let other = if d.swap { "files" } else { "diff" };
            format!("jk scroll ∙ {{}} page ∙ hl file ∙ zp {other} ∙ R refresh ∙ zz density ∙ ! shell ∙ q back")
        };
        chrome::mode_line(app, "DIFF", &hint)
    } else {
        Line::from(Span::styled(format!(" {}", app.status), theme.base()))
    };
    f.render_widget(
        Paragraph::new(footer),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
}

/// The file-list pane: 2-cell gutter (stable letter + in-flight flag), path,
/// right-aligned +/- badge.
fn draw_files(f: &mut Frame, area: Rect, app: &App, d: &DiffState) {
    let theme = &app.theme;
    let w = area.width as usize;
    let mut head = vec![Span::styled(" FILES", theme.dim1().add_modifier(Modifier::BOLD))];
    let right = format!("({})", d.files.len());
    head.push(Span::raw(" ".repeat(w.saturating_sub(6 + right.width() + 1))));
    head.push(Span::styled(right, theme.dim2()));
    let mut lines: Vec<Line<'static>> = vec![Line::from(head), Line::default()];

    if d.files.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(" no commits on {} yet", d.branch),
            theme.dim3(),
        )));
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    // Window the list around the cursor when it overflows the pane.
    let visible = (area.height as usize).saturating_sub(2).max(1);
    let first = d.file_idx.saturating_sub(visible.saturating_sub(1) / 2).min(
        d.files.len().saturating_sub(visible),
    );
    for (i, entry) in d.files.iter().enumerate().skip(first).take(visible) {
        let selected = i == d.file_idx;
        let stable = if entry.status.is_empty() { "·" } else { entry.status.as_str() };
        let inflight = if entry.untracked {
            "U"
        } else if entry.dirty {
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
        let fill = w
            .saturating_sub(4 + path.width() + badge.width() + 1)
            .max(2);
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
        lines.push(Line::from(head));
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
        lines.push(Line::from(head));
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
            body.push(Line::from(Span::styled(
                format!("{:.1} MB — ! opens it in your shell", *bytes as f64 / 1_048_576.0),
                theme.dim1(),
            )));
        }
        Render::Unresolvable { message } => {
            body.push(Line::from(Span::styled(message.clone(), theme.err_text())));
        }
    }

    lines.push(Line::from(head));
    lines.push(Line::default());

    // j/k scroll: clamp against the built content, write the clamp back.
    let visible = (area.height as usize).saturating_sub(2);
    let max_scroll = body.len().saturating_sub(visible);
    let scroll = d.scroll.get().min(max_scroll);
    d.scroll.set(scroll);
    lines.extend(body.into_iter().skip(scroll).take(visible));
    f.render_widget(Paragraph::new(lines), area);
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
            let w: usize = r.chars().map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)).sum();
            assert!(w <= 8, "row {i} width {w}: {r:?}");
        }
    }

    #[test]
    fn short_line_single_row() {
        assert_eq!(wrap_code("fn main() {}", 80), vec!["fn main() {}".to_string()]);
    }
}
