//! The pull requests dialog (T-759): every PR or MR the forge bound to a
//! ticket, newest first, one row each — the ticket page's rows unfolded,
//! where the page stops at three. `^h` opens it over the board or the page;
//! Enter opens the row's PR with the board's opener.

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::Scope;
use mesimon_core::text::scrub_cells;

use crate::app::App;
use crate::text::truncate;

use super::dialog::{self, Edges, ListRow};

/// What the dialog says when the forge has bound nothing to the ticket —
/// the PRs went while it was open, since `^h` is inert with none.
pub(super) const EMPTY: &str =
    "no pull requests ∙ the forge binds one the signed-in account opened from this ticket's worktree";

/// The rows: the mark in its register (`glyphs::pr_mark`, on the frame
/// clock the card's brief turns on), the forge's number, the repository,
/// and the page's words for the rest — state first, then `pr_clauses`.
/// The number and repository columns are as wide as their widest row, so
/// the states line up.
pub(super) fn rows(app: &App, ticket: ulid::Ulid) -> Vec<ListRow> {
    let theme = &app.theme;
    let prs = app.prs_of(ticket);
    let numbers: Vec<String> = prs.iter().map(|p| p.label()).collect();
    let repos: Vec<String> =
        prs.iter().map(|p| crate::text::one_line(&scrub_cells(&p.repo, false))).collect();
    let num_w = numbers.iter().map(|n| n.width()).max().unwrap_or(0);
    let repo_w = repos.iter().map(|r| r.width()).max().unwrap_or(0).min(REPO_MAX);
    prs.iter()
        .zip(numbers.iter().zip(&repos))
        .map(|(p, (number, repo))| {
            let (mark, reg) = app.pr_mark(p);
            let mut words = vec![p.state.clone()];
            words.extend(super::ticket::pr_clauses(p));
            let repo = truncate(repo, repo_w);
            let head = format!(
                "{number}{}  {repo}{}  {}",
                " ".repeat(num_w.saturating_sub(number.width())),
                " ".repeat(repo_w.saturating_sub(repo.width())),
                words.join(" ∙ "),
            );
            ListRow {
                lead: format!(" {mark} "),
                lead_ink: Some(super::card::register_style(theme, reg)),
                head,
                right: String::new(),
                detail: None,
                heading: false,
            }
        })
        .collect()
}

/// The widest a repository's path stands in its column before it is cut;
/// the cursor's row says it whole above the footer.
const REPO_MAX: usize = 24;

/// The dialog over `ticket`, the cursor on `idx` (clamped to the rows the
/// snapshot left). The title says when the forge was last read, as the
/// Usage dialog says it of a quota.
pub(super) fn draw(f: &mut Frame, app: &App, ticket: ulid::Ulid, idx: usize) {
    let theme = &app.theme;
    let key = app.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
    let rows = rows(app, ticket);
    let mut name = format!("PULL REQUESTS ∙ {key}");
    if !rows.is_empty() {
        name.push_str(&format!(" ∙ {}", rows.len()));
    }
    if app.forge.sampled_at_ms > 0 {
        let now = mesimon_core::clock::now_ms();
        name.push_str(&format!(" ∙ read {}", crate::text::age_ago(now, app.forge.sampled_at_ms)));
    }
    if rows.is_empty() {
        let room = usize::from(dialog::MAX_W).min(usize::from(f.area().width).saturating_sub(6));
        let lines = crate::peek::wrap(EMPTY, room.saturating_sub(2).max(1), usize::MAX);
        let area = dialog::centred(f.area(), lines.len() as u16, dialog::MAX_W);
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
    let idx = idx.min(rows.len() - 1);
    dialog::list(f, app, &name, false, Scope::PullRequests, idx, &rows);
}
