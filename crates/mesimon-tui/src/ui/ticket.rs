//! The ticket screen skeleton (07 §14, reshaped by dogfood 2026-08-30).
//! Header is a breadcrumb — `mesimon > repo > title` — with the needs-you
//! glyph + count beside the repo name (same accent as the board), and one
//! quiet identity line (short key ∙ column ∙ created) instead of the full
//! board strip. `r` edits the title in place. Zone bands are painted rows,
//! never drawn rules (L1); the zone divider is a 2-cell gap.

use mesimon_core::board::{Provenance, SessionKind, SessionState};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, InputPurpose, Mode};
use crate::glyphs;
use crate::text::{
    age_slot, created_at_epoch_ms, edit_window, marquee_offset, marquee_window, truncate,
};

use super::chrome;

/// Below this width the rail IS the screen (06 §6.5 band model, simplified:
/// M3.5 has no PTY pane on this screen yet, so the left zone is what yields).
const TWO_ZONE_MIN_W: u16 = 107;
const RAIL_W: u16 = 30;

pub(super) fn draw(f: &mut Frame, app: &App, ticket_id: ulid::Ulid, rail_idx: usize) {
    let theme = &app.theme;
    let area = f.area();
    let Some(ticket) = app.board.ticket(ticket_id) else { return };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // ---- line 0: breadcrumb (the shared component + the ticket leaf) ------
    let mut head = chrome::breadcrumb(app);
    head.push(Span::styled(" > ".to_string(), Style::default().fg(theme.rest.dim3)));
    let prefix_w: usize = head.iter().map(|s| s.content.width()).sum();
    let title_budget = (area.width as usize).saturating_sub(prefix_w + 1);

    // `r` edits the title right here (hardware cursor, tail kept visible).
    let editing = match &app.mode {
        Mode::Input { purpose: InputPurpose::Rename { id }, buffer } if *id == ticket_id => {
            Some(buffer)
        }
        _ => None,
    };
    match editing {
        Some(buf) => {
            let budget = title_budget.saturating_sub(1);
            let (shown, cx) = edit_window(buf.as_str(), buf.width_before_cursor(), budget);
            let x = prefix_w as u16 + cx;
            // Plain base: the breadcrumb's bold belongs to the project.
            head.push(Span::styled(shown, Style::default().fg(theme.rest.base)));
            f.set_cursor_position((area.x + x.min(area.width - 1), area.y));
        }
        None => {
            head.push(Span::styled(
                truncate(&ticket.title, title_budget),
                Style::default().fg(theme.rest.base),
            ));
        }
    }

    // ---- line 1: identity — column ∙ created (short keys are hidden from
    // the UI for now, author 2026-08-30). Single-user v0.1: creator is you.
    let created = created_at_epoch_ms(&ticket.created_at)
        .map(|ms| format!(" ∙ created by you {} ago", age_slot(now, ms, false)))
        .unwrap_or_else(|| " ∙ created by you".to_string());
    // M4: the workspace joins the identity line — the strategy word until a
    // binding exists, then the branch and its state (short keys resurface
    // through the branch name, which embeds them).
    let mut ident_spans = vec![
        Span::styled(format!(" {}", ticket.column.to_uppercase()), theme.dim2()),
        Span::styled(created, theme.dim2()),
    ];
    // The m flow's live reply (armed prompt, outcome, refusal) replaces the
    // resting branch-state hint for a beat — same spot, so the conversation
    // with the merge key happens in one place, never in the footer.
    let note = (!app.merge_note.is_empty()).then(|| crate::text::one_line(&app.merge_note));
    if let Some(w) = app.wt_item(ticket.id) {
        // Quiet-tickets rule: a mid-turn agent blocks the merge, so the hint
        // withholds the key (the count still shows what's waiting).
        let busy = app.ticket_busy(ticket.id);
        let state = if let Some(n) = &note {
            format!(" ∙ {n}")
        } else if w.conflict {
            " ∙ branch shared!".to_string()
        } else if w.merged {
            " ∙ merged".to_string()
        } else if w.status != "attached" {
            format!(" ∙ {}", w.status)
        } else if w.needs_rebase {
            // The m flow's rebase stage — main moved past this branch.
            " ∙ main moved ∙ m rebases".to_string()
        } else if w.ahead > 0 && busy {
            format!(" ∙ {} to merge", w.ahead)
        } else if w.ahead > 0 {
            // Merge available — the count and the key, calm register.
            format!(" ∙ {} to merge ∙ m", w.ahead)
        } else {
            String::new()
        };
        ident_spans.push(Span::styled(format!(" ∙ ⎇ {}", w.branch), theme.dim1()));
        if !state.is_empty() {
            let actionable = !w.merged
                && w.status == "attached"
                && (w.needs_rebase || (w.ahead > 0 && !busy));
            let style = if note.is_some() {
                theme.calm_text()
            } else if w.conflict {
                theme.base().add_modifier(ratatui::style::Modifier::BOLD)
            } else if actionable {
                theme.calm_text()
            } else {
                theme.dim2()
            };
            ident_spans.push(Span::styled(state, style));
        }
        if note.is_none() {
            if let Some(d) = &w.detail {
                ident_spans.push(Span::styled(format!(" ∙ {d}"), theme.dim2()));
            }
        }
    } else if let Some(n) = &note {
        // A merge reply with no binding ("no worktree on this ticket").
        ident_spans.push(Span::styled(format!(" ∙ {n}"), theme.calm_text()));
    } else if ticket.workspace_strategy() == mesimon_core::board::WorkspaceStrategy::Worktree {
        ident_spans.push(Span::styled(" ∙ ⎇ worktree", theme.dim2()));
    }
    let ident = Line::from(ident_spans);

    let band = match theme.selected_bg {
        Some(bg) => Line::default().style(Style::default().bg(bg)),
        None => Line::default(),
    };

    // Breathing row between the breadcrumb and the identity line too — the
    // title never touches its metadata (06 §5.5).
    let top = vec![Line::from(head), Line::default(), ident, band];
    f.render_widget(
        Paragraph::new(top),
        Rect { x: area.x, y: area.y, width: area.width, height: 4.min(area.height) },
    );

    // ---- body zones -------------------------------------------------------
    // One breathing row under the band (06 §5.5) before the zones begin.
    let body_y = area.y + 5;
    let body_h = area.height.saturating_sub(6); // top 4 + breathing 1 + footer 1
    let two_zone = area.width >= TWO_ZONE_MIN_W;
    if two_zone {
        // Transcript preview: the selected rail session's latest assistant
        // reply, read through the same draw cache as the board's `p` peek
        // (one slot is still enough — board and ticket never draw the same
        // frame). Bash sessions have no transcript and preview nothing.
        let sel = app.rail_sessions(ticket_id).into_iter().nth(rail_idx);
        let peek = sel
            .and_then(|s| s.transcript_path.as_deref())
            .and_then(|p| app.peek_cache.text(p));
        // A mid-turn agent keeps composing past whatever the preview shows,
        // so the zone says so (bash panes work too, but have no transcript
        // for the line to qualify — Claude only).
        let working = sel
            .is_some_and(|s| s.kind == SessionKind::Claude && s.state == SessionState::Running);
        let left_w = area.width - RAIL_W - 3; // 1 pad + 2-cell divider gap
        draw_documents(
            f,
            Rect { x: area.x + 1, y: body_y, width: left_w, height: body_h },
            app,
            peek.as_deref(),
            working,
        );
        draw_rail(
            f,
            Rect { x: area.x + left_w + 3, y: body_y, width: RAIL_W.saturating_sub(1), height: body_h },
            app,
            ticket_id,
            rail_idx,
            now,
        );
    } else {
        draw_rail(
            f,
            Rect { x: area.x + 1, y: body_y, width: area.width.saturating_sub(2), height: body_h },
            app,
            ticket_id,
            rail_idx,
            now,
        );
    }

    // ---- footer -----------------------------------------------------------
    // A pending status (a daemon refusal, mostly) outranks the key hints —
    // the board footer does the same in chrome::draw_footer.
    let footer = if app.status.is_empty() {
        // `w` only while the daemon would still accept it: locked once the
        // ticket has sessions or a worktree binding (server::set_workspace).
        let has_wt = app.wt_item(ticket_id).is_some();
        let locked = has_wt || app.board.sessions.iter().any(|s| s.ticket == ticket_id);
        // `m` is deliberately absent: the merge lives on the identity line
        // (its hint and every reply of the flow render up there, once).
        let w_hint = if locked { "" } else { "w worktree/shared ∙ " };
        // `v` only once a binding exists — that is when the diff can answer.
        let v_hint = if has_wt { "v diff ∙ " } else { "" };
        let hint = format!(
            "jk select ∙ enter focus ∙ c claude ∙ s shell ∙ {w_hint}{v_hint}r rename ∙ esc board"
        );
        chrome::mode_line(app, "TICKET", &hint)
    } else {
        Line::from(Span::styled(
            format!(" {}", crate::text::one_line(&app.status)),
            theme.base(),
        ))
    };
    f.render_widget(
        Paragraph::new(footer),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
}

fn draw_documents(f: &mut Frame, area: Rect, app: &App, peek: Option<&str>, working: bool) {
    let theme = &app.theme;
    let mut head = vec![Span::styled(
        " DOCUMENTS",
        theme.dim1().add_modifier(Modifier::BOLD),
    )];
    let right = "(0)";
    let used: usize = 10 + right.width() + 1;
    head.push(Span::raw(" ".repeat((area.width as usize).saturating_sub(used))));
    head.push(Span::styled(right.to_string(), theme.dim2()));
    let mut lines = vec![
        Line::from(head),
        Line::default(),
        Line::from(Span::styled("   drop files into this ticket's directory", theme.dim3())),
        Line::from(Span::styled("   ticket directories land with the adoption IA (M4)", theme.dim3())),
    ];

    // The selected session's latest assistant reply, wrapped into whatever
    // height the zone has left. Absent transcript (bash, fresh spawn) means
    // no section at all — a heading over nothing is noise — UNLESS the agent
    // is mid-turn: then the section closes with the rail's own spinner and
    // state word, so a stale reply (or no reply yet) reads as in-progress.
    if peek.is_some() || working {
        lines.push(Line::default());
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            " TRANSCRIPT",
            theme.dim1().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::default());
        if let Some(text) = peek {
            // Reserve the indicator's rows so a long reply never pushes it off.
            let reserve = if working { 2 } else { 0 };
            let budget = (area.height as usize).saturating_sub(lines.len() + reserve);
            for row in crate::peek::wrap(text, (area.width as usize).saturating_sub(4), budget) {
                lines.push(Line::from(Span::styled(format!("   {row}"), theme.dim1())));
            }
            if working {
                lines.push(Line::default());
            }
        }
        if working {
            let g = glyphs::spinner(theme.glyph_tier(), app.spin_frame());
            let word = glyphs::state_word(&SessionState::Running);
            lines.push(Line::from(Span::styled(format!("   {g} {word}"), theme.dim2())));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_rail(
    f: &mut Frame,
    area: Rect,
    app: &App,
    ticket_id: ulid::Ulid,
    rail_idx: usize,
    now: u64,
) {
    let theme = &app.theme;
    let tier = theme.glyph_tier();
    let rail = app.rail_sessions(ticket_id);
    let w = area.width as usize;

    let mut head = vec![Span::styled(
        " SESSIONS",
        theme.dim1().add_modifier(Modifier::BOLD),
    )];
    let right = rail.len().to_string();
    let used: usize = 9 + right.width() + 1;
    head.push(Span::raw(" ".repeat(w.saturating_sub(used))));
    head.push(Span::styled(right, theme.dim2()));
    let mut lines: Vec<Line<'static>> = vec![Line::from(head), Line::default()];

    if rail.is_empty() {
        // 07 §16.2: the empty state names the two spawn verbs and nothing else.
        lines.push(Line::from(Span::styled(" c claude ∙ s bash", theme.dim3())));
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    for (i, s) in rail.iter().enumerate() {
        let selected = i == rail_idx;
        let (g, reg) = glyphs::session_glyph(&s.state, tier, app.spin_frame());
        let glyph_style = match reg {
            glyphs::Register::Attn => theme.attn_text(),
            glyphs::Register::Err => theme.err_text(),
            glyphs::Register::Calm => theme.calm_text(),
            glyphs::Register::Grey => theme.dim2(),
        };
        // The session's own name (OSC-0 title, same as the tmux status bar's
        // breadcrumb leaf) when it set one, else the kind word.
        let kind = s.title.as_deref().unwrap_or(match s.kind {
            SessionKind::Claude => "claude",
            SessionKind::Bash => "bash",
        });
        // Row budget: glyph + " {mark} {name}" + ≥1 fill + age. An
        // overflowing name on the selected row reveals itself marquee-style
        // (same clock behaviour as the board's card title: reset on landing,
        // one pass, rest truncated).
        let budget = w.saturating_sub(9);
        let overflow = kind.width().saturating_sub(budget);
        let scroll = if selected && overflow > 0 {
            let ms = match app.rail_marquee.get() {
                Some((id, epoch)) if id == s.id => epoch.elapsed().as_millis() as u64,
                _ => {
                    app.rail_marquee.set(Some((s.id, std::time::Instant::now())));
                    0
                }
            };
            marquee_offset(ms, overflow)
        } else {
            0
        };
        let kind = if scroll > 0 {
            marquee_window(kind, budget, scroll)
        } else {
            truncate(kind, budget)
        };
        let mark = glyphs::kind_mark(s.kind, tier);
        let age = s
            .state_changed_at
            .map(|ms| age_slot(now, ms, s.state == SessionState::Running))
            .unwrap_or_default();
        // A dead row (the rail's one resumable corpse) wears the dim register
        // so the living read first; selection still lifts it to legibility.
        let name_style = if selected {
            Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else if !s.state.is_live() {
            theme.dim2()
        } else {
            theme.base()
        };
        let name = format!(" {mark} {kind}");
        let mut spans = vec![
            Span::styled(g.to_string(), glyph_style),
            Span::styled(name.clone(), name_style),
        ];
        let used: usize = 1 + name.width() + age.width() + 1;
        spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
        spans.push(Span::styled(age, theme.dim2()));
        let row_style = if selected { theme.selected_row() } else { Style::default() };
        lines.push(Line::from(spans).style(row_style));

        // One line per session — the glyph already carries the state. A
        // second line exists only when there is something extra to say (the
        // waiting session's actual question, or badges), and it wears the
        // same row surface so it reads as part of its session, never as an
        // item of its own.
        if matches!(s.state, SessionState::RequiresAction { .. }) {
            let q = s
                .detail
                .clone()
                .unwrap_or_else(|| glyphs::state_word(&s.state).to_lowercase());
            let text = format!("    {}", truncate(&q, w.saturating_sub(4)));
            let pad = w.saturating_sub(text.width());
            lines.push(
                Line::from(vec![
                    Span::styled(text, theme.attn_text()),
                    Span::raw(" ".repeat(pad)),
                ])
                .style(row_style),
            );
        } else {
            let mut badges: Vec<&str> = Vec::new();
            if s.provenance == Provenance::Adopted && s.argv.is_empty() {
                badges.push("external");
            }
            if s.pinned_awake {
                badges.push("pinned");
            }
            if matches!(s.state, SessionState::Exited { .. }) {
                badges.push("enter resumes");
            }
            if !badges.is_empty() {
                let text = format!("    {}", badges.join(" ∙ "));
                let pad = w.saturating_sub(text.width());
                lines.push(
                    Line::from(vec![
                        Span::styled(text, theme.dim2()),
                        Span::raw(" ".repeat(pad)),
                    ])
                    .style(row_style),
                );
            }
        }
    }

    f.render_widget(Paragraph::new(lines), area);
}
