//! The read-only diff viewer (M4b, docs/08 §1.3 hunk model + §10 layout minus
//! the dead rail). Review only — no accept, no checkout, no send-back; `!`
//! opens a shell in the worktree as the escape hatch. Same laws as every
//! screen: painted bands, never drawn rules (L1); no syntax highlighting
//! (L3 — one saturated colour, reserved). Adds/deletes ride the calm/err
//! registers (muted green/red; author 2026-08-30 amendment) plus glyph AND
//! weight, so review still reads correctly in mono.

use std::ops::Range;
use std::rc::Rc;

use mesimon_core::command::GitCommit;
use mesimon_core::diff::{self, FileDiff, Marks, Render, Sign};
use mesimon_core::keymap::{self, Scope, Verb};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, DiffState};
use crate::glyphs::Tier;
use crate::text::truncate;
use crate::theme::{Flavor, Theme};

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

/// The commit list's document key on the diff pager — a file's is its
/// index, so a page turn on one never carries into the other.
pub(crate) const COMMITS_KEY: u64 = u64::MAX;

pub(super) fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = f.area();
    let Some(d) = app.diff.as_ref() else { return };
    // Until a pane below measures itself there is nothing to page.
    d.pager.view.set(crate::app::View::default());

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
    // A workspace's lists name each repo's upstream on its own heading.
    let summary = if d.commits {
        match &app.git.upstream {
            Some(upstream) if app.git.nested.is_empty() => {
                format!("push / pull ∙ {}", crate::text::one_line(upstream))
            }
            _ => "push / pull".to_string(),
        }
    } else if let Some(from) = &d.from_list {
        let list = if from.incoming { "to pull" } else { "to push" };
        match n {
            0 => format!("{list} ∙ no changes"),
            _ => format!("{list} ∙ {n} {noun} ∙ +{adds} -{dels}"),
        }
    } else if n == 0 {
        "no changes".to_string()
    } else {
        format!("{against} ∙ {n} {noun} ∙ +{adds} -{dels}")
    };
    // A folder of repos has no branch; the checkout's list names it by its
    // count (`checkout_diff_list`), and the commits view says the same.
    let branch = match (d.commits, app.git.branch.is_empty()) {
        (true, true) => mesimon_core::workspace::repos_word(app.git.repos.len()),
        (true, false) => app.git.branch.clone(),
        (false, _) => d.branch.clone(),
    };
    // A commit is named the way its list row named it, oid and subject — and
    // on a workspace the repo it was listed under; the subject yields to the
    // counts on a narrow row.
    let mut ident = match &d.from_list {
        Some(from) => {
            let oid: String = d.branch_oid.chars().take(7).collect();
            let repo = from.repo.as_deref().map(|r| format!(" {} ∙", crate::text::one_line(r)));
            let repo = repo.unwrap_or_default();
            let room = (area.width as usize)
                .saturating_sub(summary.width() + oid.width() + repo.width() + 12);
            vec![
                Span::styled(repo, theme.dim1()),
                Span::styled(format!(" {oid}"), theme.dim2()),
                Span::styled(
                    format!(" {}", truncate(&crate::text::one_line(&from.subject), room)),
                    theme.dim1(),
                ),
            ]
        }
        None => {
            vec![Span::styled(format!(" ⎇ {}", crate::text::one_line(&branch)), theme.dim1())]
        }
    };
    // The checkout's push/pull state belongs on the title, not behind a key
    // press (T-347): before this, the only thing the screen said about it was
    // the footer's standing `tab` — an offer that read the same whether or
    // not anything was pending, so the only way to learn there were commits
    // was to go and look. The arrows hang off the branch in the board
    // header's exact spelling and register, so the two surfaces read alike.
    ident.extend(sync_marks(app, d));
    ident.push(Span::styled(format!(" ∙ {summary}"), theme.dim2()));
    if d.is_branch() && !d.worktree_present {
        ident.push(Span::styled(" ∙ worktree evicted".to_string(), theme.dim2()));
    }
    // …and the key that crosses to the other view sits beside the state that
    // is the reason to press it, the way `n N file` sits beside FILES. It is
    // this row's only hint, so `tab` is off the footer (`prio: 0`); a row too
    // tight to hold it drops it and `?` still lists it.
    let used = super::spans_width(&ident);
    let keys = hints(app, &[Verb::GitCommits], (area.width as usize).saturating_sub(used + 4));
    if !keys.is_empty() {
        ident.push(Span::raw("   ".to_string()));
        ident.extend(keys);
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

    if d.pager.view.get().key.is_none() {
        d.pager.glide.set(None);
    }

    // ---- footer ----------------------------------------------------------
    // From the keymap, like every other screen. `z s` drops itself above the
    // two-pane breakpoint because there is nothing to swap up there — which
    // is why draw records the breakpoint for the keymap to read.
    app.set_diff_two_pane(area.width >= TWO_PANE_MIN_W);
    let footer = chrome::footer_line(app, area.width);
    f.render_widget(
        Paragraph::new(footer),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
}

/// The checkout's `↑N ↓N` for the identity row (T-347) — `glyphs`' own
/// arrows, so "ahead, push due" / "behind, pull due" is spelled here exactly
/// as the board header spells it. Empty on a ticket's branch diff (the
/// worktree's branch is not what `app.git` measures), before a sample lands,
/// and when the branch is level with its upstream — an absent clause reads as
/// nothing pending, the same silence the header keeps. A workspace's repos
/// add their sum (T-455): after the count where the count is the name, else
/// as a clause of its own after the root's.
fn sync_marks(app: &App, d: &DiffState) -> Vec<Span<'static>> {
    let g = &app.git;
    if !d.is_checkout() || !g.sampled {
        return Vec::new();
    }
    let tier = app.theme.glyph_tier();
    let (na, nb) = g.nested_ahead_behind();
    let mut own = chrome::arrows(tier, g.ahead, g.behind);
    let nested = chrome::arrows(tier, na, nb);
    let mut out = Vec::new();
    if g.branch.is_empty() {
        own.push_str(&nested);
    }
    if !own.is_empty() {
        out.push(Span::styled(own, app.theme.calm_text()));
    }
    if !g.branch.is_empty() && !nested.is_empty() {
        let word = mesimon_core::workspace::repos_word(g.repos.len());
        out.push(Span::styled(format!(" ∙ {word}"), app.theme.dim2()));
        out.push(Span::styled(nested, app.theme.calm_text()));
    }
    out
}

/// Both directions share one cursor (`App::commit_rows`' order: outgoing
/// first) and a window that follows it. Counts stay exact even when the
/// snapshot's bounded history is truncated; only a listed commit opens.
fn draw_commits(f: &mut Frame, area: Rect, app: &App, d: &DiffState) {
    let theme = &app.theme;
    let g = &app.git;
    let w = area.width as usize;
    let listed = app.commit_rows().len();
    let mut rows = CommitLines {
        app,
        w,
        cursor: (listed > 0).then(|| d.commit_idx.min(listed - 1)),
        idx: 0,
        cursor_row: None,
        lines: Vec::new(),
    };
    if !g.sampled {
        rows.dim(" Git status unavailable");
    } else if !g.nested.is_empty() {
        workspace_commits(&mut rows);
    } else if g.detached || g.upstream.is_none() {
        rows.dim(if g.detached {
            " Detached HEAD — no upstream comparison"
        } else {
            " No upstream configured"
        });
    } else {
        for (label, count, commits) in
            [("TO PUSH", g.ahead, &g.to_push), ("TO PULL", g.behind, &g.to_pull)]
        {
            rows.lines.push(Line::from(Span::styled(
                format!(" {label} ({count})"),
                theme.dim1().add_modifier(Modifier::BOLD),
            )));
            rows.lines.push(Line::default());
            if count == 0 {
                rows.lines.push(Line::from(Span::styled(" Nothing pending", theme.dim3())));
            } else {
                rows.commits(&[], count, commits.as_deref());
            }
            rows.lines.push(Line::default());
            rows.lines.push(Line::default());
        }
        rows.lines.push(Line::from(Span::styled(
            truncate(&format!(" {}", app.git_fetch_note()), w),
            theme.dim2(),
        )));
    }
    let CommitLines { cursor_row, lines: rows, .. } = rows;
    let visible = (area.height as usize).saturating_sub(2);
    // The window follows the cursor, and a list's first commit keeps its
    // heading in view above it.
    if let Some(row) = cursor_row {
        let top = match d.pager.request.get() {
            Some((COMMITS_KEY, n)) => n,
            _ => 0,
        };
        let want = if row < top.saturating_add(2) {
            row.saturating_sub(2)
        } else if row >= top.saturating_add(visible) {
            row + 1 - visible
        } else {
            top
        };
        d.pager.request.set(Some((COMMITS_KEY, want)));
    }
    let at = d.pager.window(Some(COMMITS_KEY), rows.len(), visible, false);
    let keys = [Verb::PageDown, Verb::ScrollDown, Verb::OpenCommit];
    let mut lines = vec![Line::from(hints(app, &keys, w)), Line::default()];
    lines.extend(rows.into_iter().skip(at).take(visible));
    f.render_widget(Paragraph::new(lines), area);
}

/// The push / pull lists' rows as they are built, with the flat index of
/// the next commit and the row the cursor's commit landed on.
struct CommitLines<'a> {
    app: &'a App,
    w: usize,
    cursor: Option<usize>,
    idx: usize,
    cursor_row: Option<usize>,
    lines: Vec<Line<'static>>,
}

impl CommitLines<'_> {
    fn dim(&mut self, text: &str) {
        self.lines.push(Line::from(Span::styled(text.to_string(), self.app.theme.dim2())));
    }

    /// One direction's commits after `lead` (the workspace's `↑`/`↓`, or
    /// nothing): the rows, the count the bounded history left out, or the
    /// words for a list that is missing.
    fn commits(&mut self, lead: &[Span<'static>], count: u32, commits: Option<&[GitCommit]>) {
        let theme = &self.app.theme;
        let pad = super::spans_width(lead);
        let with_lead = |mut rest: Vec<Span<'static>>| {
            let mut spans = lead.to_vec();
            spans.append(&mut rest);
            spans
        };
        let Some(commits) = commits else {
            let text = Span::styled(" Commit list unavailable", theme.dim2());
            self.lines.push(Line::from(with_lead(vec![text])));
            return;
        };
        for commit in commits {
            let oid: String = commit.oid.chars().take(7).collect();
            let subject =
                truncate(&crate::text::one_line(&commit.subject), self.w.saturating_sub(10 + pad));
            let selected = self.cursor == Some(self.idx);
            self.idx += 1;
            let head = Span::styled(format!(" {oid}  "), theme.dim2());
            if !selected {
                let rest = vec![head, Span::styled(subject, theme.base())];
                self.lines.push(Line::from(with_lead(rest)));
                continue;
            }
            // The file list's selected row, spelled the same way.
            self.cursor_row = Some(self.lines.len());
            let fill = self.w.saturating_sub(10 + pad + subject.width());
            let rest = vec![
                head,
                Span::styled(
                    subject,
                    Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" ".repeat(fill)),
            ];
            self.lines.push(Line::from(with_lead(rest)).style(theme.selected_row()));
        }
        let remaining = (count as usize).saturating_sub(commits.len());
        if remaining > 0 {
            let text = format!("{} … {remaining} more commits", " ".repeat(pad));
            self.lines.push(Line::from(Span::styled(text, theme.dim3())));
        }
    }
}

/// A workspace's lists (T-455): one row per repo — name, branch, state and
/// when it was last fetched — so the whole workspace reads down one column
/// at a glance. Repos with something to push or pull come first, each with
/// its commits under it marked `↑` (to push) or `↓` (to pull), the way the
/// header's arrows mark them; then the repos in sync, whose `✓` is the calm
/// register the card's done mark wears; then the ones with nothing to
/// compare against. The board's own branch is a row like any other, named
/// by the board, where the root is a repository.
fn workspace_commits(rows: &mut CommitLines<'_>) {
    let app = rows.app;
    let theme = &app.theme;
    let tier = theme.glyph_tier();
    let board = app.board_name();
    let mut groups = app.sync_groups();
    // Stable, so each state keeps census order — and the pending ones stay
    // in `commit_rows`' order, which is the cursor's.
    groups.sort_by_key(|s| match (s.pending(), s.compared(), s.detached) {
        (true, _, _) => 0,
        (false, true, _) => 1,
        (false, false, false) => 2,
        (false, false, true) => 3,
    });
    let name_of = |s: &crate::app::SyncGroup<'_>| crate::text::one_line(s.repo.unwrap_or(&board));
    let name_w = groups.iter().map(|s| name_of(s).width()).max().unwrap_or(0).min(28);
    let branch_w =
        groups.iter().map(|s| crate::text::one_line(s.branch).width()).max().unwrap_or(0).min(24);
    let state_of = |s: &crate::app::SyncGroup<'_>| -> (String, Style) {
        if s.pending() {
            (chrome::arrows(tier, s.ahead, s.behind).trim_start().to_string(), theme.calm_text())
        } else if s.compared() {
            (format!("{} in sync", crate::glyphs::merged_mark(tier)), theme.calm_text())
        } else if s.detached {
            ("detached".to_string(), theme.dim2())
        } else {
            ("no upstream".to_string(), theme.dim2())
        }
    };
    let state_w = groups.iter().map(|s| state_of(s).0.width()).max().unwrap_or(0);
    let now = mesimon_core::clock::now_ms();
    let cell = |text: &str, w: usize| {
        let text = truncate(text, w);
        let fill = w.saturating_sub(text.width());
        format!("{text}{}", " ".repeat(fill))
    };
    for s in &groups {
        let pending = s.pending();
        let (state, state_style) = state_of(s);
        let name_style =
            if pending { theme.base().add_modifier(Modifier::BOLD) } else { theme.dim1() };
        let mut line = vec![
            Span::styled(format!(" {}   ", cell(&name_of(s), name_w)), name_style),
            Span::styled(
                format!(
                    "{} {}   ",
                    crate::glyphs::branch_mark(tier),
                    cell(&crate::text::one_line(s.branch), branch_w)
                ),
                theme.dim2(),
            ),
            Span::styled(cell(&state, state_w), state_style),
        ];
        if s.compared() && s.fetched_ms > 0 {
            let since = std::time::Duration::from_millis(now.saturating_sub(s.fetched_ms));
            let when = format!("   fetched {}", crate::release::ago(since));
            line.push(Span::styled(when, theme.dim3()));
        }
        rows.lines.push(Line::from(line));
        if !pending {
            continue;
        }
        for (mark, count, commits) in [
            (crate::glyphs::ahead_mark(tier), s.ahead, s.to_push),
            (crate::glyphs::behind_mark(tier), s.behind, s.to_pull),
        ] {
            if count > 0 {
                let lead = [Span::raw("   "), Span::styled(mark.to_string(), theme.calm_text())];
                rows.commits(&lead, count, commits);
            }
        }
        rows.lines.push(Line::default());
    }
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
        let empty = if d.from_list.is_some() {
            " an empty commit".to_string()
        } else if d.is_branch() {
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

    if matches!(fd.render, Render::Text | Render::Symlink) {
        let nh = fd.hunks.len();
        let noun = if nh == 1 { "hunk" } else { "hunks" };
        head.push(Span::styled(format!(" ∙ {nh} {noun}"), theme.dim2()));
    }
    let body = cached_body(d, fd, w, theme);

    // j/k scroll: clamp against the built content, write the clamp back
    // (`Pager::window`), and draw the glide's row while a turn is in motion.
    let visible = (area.height as usize).saturating_sub(2);
    let at = d.pager.window(Some(d.file_idx as u64), body.rows.len(), visible, false);
    lines.push(hunk_heading(app, d, head, w, f.area().width >= TWO_PANE_MIN_W));
    lines.push(Line::default());
    lines.extend(body.rows.iter().skip(at).take(visible).cloned());
    f.render_widget(Paragraph::new(lines), area);
}

/// The hunk pane's rows for one fetched file at one width and flavor: the
/// hunk bands and every soft-wrapped line, of which a frame shows a window.
pub(crate) struct Body {
    file: Rc<FileDiff>,
    width: usize,
    flavor: Flavor,
    /// `diff::intraline` of each hunk. It is the file's and not the
    /// width's, so a resize lays the rows out again without diffing again.
    marks: Rc<Vec<Vec<Option<Marks>>>>,
    rows: Vec<Line<'static>>,
}

/// The pane's rows, kept on `DiffState::body` (T-444): a file's diff can
/// run to thousands of lines, and the draw runs at 60 fps through a glide
/// and once per key while `j` is held, only to keep a window of it. Keyed
/// on the fetched file itself, so `R` and the density cycle — which fetch
/// it again — lay it out again, and on the width and flavor like the
/// PREVIEW zone's `ticket::rendered`.
fn cached_body(d: &DiffState, fd: &Rc<FileDiff>, width: usize, theme: &Theme) -> Rc<Body> {
    let mut slot = d.body.borrow_mut();
    let marks = match slot.as_ref() {
        Some(b) if Rc::ptr_eq(&b.file, fd) => {
            if b.width == width && b.flavor == theme.flavor {
                return Rc::clone(b);
            }
            Rc::clone(&b.marks)
        }
        _ => Rc::new(fd.hunks.iter().map(|h| diff::intraline(&h.lines)).collect()),
    };
    let rows = body_rows(fd, &marks, d.is_branch(), width, theme);
    let b = Rc::new(Body { file: Rc::clone(fd), width, flavor: theme.flavor, marks, rows });
    *slot = Some(Rc::clone(&b));
    b
}

fn body_rows(
    fd: &FileDiff,
    marks: &[Vec<Option<Marks>>],
    branch: bool,
    w: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    // Body content by render kind — the exhaustive enum (a skipped case is a
    // day-one panic, docs/08 §1.3).
    let mut body: Vec<Line<'static>> = Vec::new();
    match &fd.render {
        Render::Text | Render::Symlink => {
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
            for (h, marks) in fd.hunks.iter().zip(marks) {
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
                for (l, marks) in h.lines.iter().zip(marks) {
                    // Adds/deletes in colour (author 2026-08-30, amending the
                    // grey-ramp-only rule): the calm/err registers — muted
                    // green/red, theme- and profile-aware — never raw RGB.
                    // Full-line grounds ride the diff tints where the profile
                    // has them (a Line's style paints the whole row, the
                    // band-row precedent). Glyph + weight stay for mono.
                    let (sign, num, style, line_bg, hi_bg) = match l.sign {
                        Sign::Ctx => (' ', l.new_ln, theme.dim2(), None, None),
                        Sign::Add => (
                            '+',
                            l.new_ln,
                            theme.calm_text().add_modifier(Modifier::BOLD),
                            theme.diff_add_bg(),
                            theme.diff_add_hi(),
                        ),
                        Sign::Del => (
                            '-',
                            l.old_ln,
                            theme.err_text(),
                            theme.diff_del_bg(),
                            theme.diff_del_hi(),
                        ),
                    };
                    // A paired line (T-454) says where it changed: its words
                    // drop to the register's regular weight and the changed
                    // ones are bold on the line's ground a step stronger —
                    // bold alone where the profile has no quiet tint. An
                    // unpaired line changed whole and keeps the line's style.
                    let (text_style, mark_style) = match marks {
                        Some(_) => {
                            let plain = style.remove_modifier(Modifier::BOLD);
                            let strong = plain.add_modifier(Modifier::BOLD);
                            (plain, hi_bg.map_or(strong, |bg| strong.bg(bg)))
                        }
                        None => (style, style),
                    };
                    let num = num.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".into());
                    let rows = wrap_code(&l.text, code_w, marks.as_deref().unwrap_or_default());
                    for (j, runs) in rows.into_iter().enumerate() {
                        let gutter = if j == 0 {
                            format!("{sign} {num}  ")
                        } else {
                            // Continuation: gutter marker, blank number
                            // column, and the row's own 2-space indent — the
                            // prefix must stay 9 cols like "X 12345  " or the
                            // row overflows the pane (docs/08 §1.3).
                            format!("{cont}        ")
                        };
                        let mut used = gutter.width();
                        let mut spans = vec![Span::styled(gutter, style)];
                        for (run, marked) in runs {
                            used += run.width();
                            spans.push(Span::styled(
                                run,
                                if marked { mark_style } else { text_style },
                            ));
                        }
                        let line = match line_bg {
                            Some(bg) => {
                                spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
                                Line::from(spans).style(Style::default().bg(bg))
                            }
                            None => Line::from(spans),
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
            let word = if branch {
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

    body
}

fn hunk_heading(
    app: &App,
    d: &DiffState,
    mut head: Vec<Span<'static>>,
    w: usize,
    two_pane: bool,
) -> Line<'static> {
    let mut verbs = Vec::new();
    if d.pager.view.get().max > 0 {
        verbs.push(Verb::PageDown);
    }
    if !two_pane && d.files.len() > 1 {
        verbs.push(Verb::NextFile);
    }
    if d.pager.view.get().max > 0 {
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
    let ctx = app.frame_ctx();
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
/// must not be invisible). Continuations carry a 2-space indent. Each row
/// comes back as runs, each saying whether its cells fall inside `marks` —
/// byte ranges of `text`, `diff::intraline`'s — so a changed word keeps its
/// mark across a tab, a `^M` and a wrap.
pub(crate) fn wrap_code(
    text: &str,
    width: usize,
    marks: &[Range<usize>],
) -> Vec<Vec<(String, bool)>> {
    let width = width.max(4);
    // Expand tabs / make \r visible first, so wrapping sees real cells.
    let mut expanded: Vec<(char, bool)> = Vec::new();
    let mut col = 0usize;
    for (at, ch) in text.char_indices() {
        let marked = marks.iter().any(|m| m.contains(&at));
        match ch {
            '\t' => {
                let next = (col / TAB_W + 1) * TAB_W;
                for _ in col..next {
                    expanded.push((' ', marked));
                }
                col = next;
            }
            '\r' => {
                expanded.extend([('^', marked), ('M', marked)]);
                col += 2;
            }
            c => {
                expanded.push((c, marked));
                col += c.width().unwrap_or(0);
            }
        }
    }
    let mut rows: Vec<Vec<(char, bool)>> = Vec::new();
    let mut row = Vec::new();
    let mut row_w = 0usize;
    let cont_budget = width.saturating_sub(2);
    for (c, marked) in expanded {
        let cw = c.width().unwrap_or(0);
        let budget = if rows.is_empty() { width } else { cont_budget };
        if row_w + cw > budget && row_w > 0 {
            rows.push(std::mem::take(&mut row));
            row_w = 0;
        }
        row.push((c, marked));
        row_w += cw;
    }
    rows.push(row);
    let mut out = Vec::with_capacity(rows.len());
    for (i, r) in rows.into_iter().enumerate() {
        let mut runs: Vec<(String, bool)> = Vec::new();
        if i > 0 {
            runs.push(("  ".to_string(), false));
        }
        for (c, marked) in r {
            match runs.last_mut() {
                Some((run, m)) if *m == marked => run.push(c),
                _ => runs.push((c.to_string(), marked)),
            }
        }
        out.push(runs);
    }
    out
}

#[cfg(test)]
mod tests {
    /// The rows as text, marks dropped.
    fn wrap_code(text: &str, width: usize) -> Vec<String> {
        super::wrap_code(text, width, &[])
            .into_iter()
            .map(|runs| runs.into_iter().map(|(run, _)| run).collect())
            .collect()
    }

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

    /// A mark is on bytes of the raw line and has to land on the cells they
    /// became: a tab's eight spaces, a `\r`'s `^M`, the far side of a wrap.
    #[test]
    fn marks_follow_their_bytes_into_cells() {
        let rows = super::wrap_code("\tab\r", 40, &[0..1, 3..4]);
        assert_eq!(
            rows,
            vec![vec![
                ("        ".to_string(), true),
                ("ab".to_string(), false),
                ("^M".to_string(), true)
            ]]
        );
        let rows = super::wrap_code("aaaa bbbbbb", 6, &[0..1, 5..11]);
        assert_eq!(
            rows[0],
            vec![("a".to_string(), true), ("aaa ".to_string(), false), ("b".to_string(), true)]
        );
        assert_eq!(rows[1], vec![("  ".to_string(), false), ("bbbb".to_string(), true)]);
        assert_eq!(rows[2], vec![("  ".to_string(), false), ("b".to_string(), true)]);
    }
}
