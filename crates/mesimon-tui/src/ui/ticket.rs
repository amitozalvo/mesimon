//! The ticket screen skeleton (07 §14, reshaped by dogfood 2026-08-30).
//! Header is a breadcrumb — `mesimon > repo > title` — with the needs-you
//! glyph + count beside the repo name (same accent as the board), and one
//! quiet identity line (short key ∙ column ∙ created) instead of the full
//! board strip. `r` edits the title in place. Nothing separates the header
//! from the body but a breathing row — no band, no rule (L1); the zone
//! divider is a 2-cell gap.

use mesimon_core::board::{NoteMeta, Provenance, SessionKind, SessionState};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap;

use crate::app::{App, InputPurpose, Mode, PreviewView, RailRow};
use crate::glyphs;
use crate::text::{
    age_created, age_in_column, age_slot, created_at_epoch_ms, edit_window, marquee_offset,
    marquee_window, truncate,
};

use super::chrome;

/// Below this width the rail IS the screen (06 §6.5 band model, simplified:
/// M3.5 has no PTY pane on this screen yet, so the left zone is what yields).
const TWO_ZONE_MIN_W: u16 = 107;
const RAIL_W: u16 = 30;
/// The state row's branch name keeps at least this many cells against the
/// tag chips: enough for ` ∙ ⎇ msmn/T-163~`, so a heavily tagged ticket
/// still names where its code lives.
const WT_BRANCH_FLOOR: usize = 16;
/// The description block's ceiling in rows; the zone below still has to
/// read. A third of the body, and never more than this.
const DESC_MAX_ROWS: usize = 8;

/// Who a note's author string names, in the page's own words: a person at
/// this board is `you`, an agent session is `claude`.
pub(super) fn author_word(by: &str) -> &'static str {
    if by.starts_with("agent:") {
        "claude"
    } else {
        "you"
    }
}

pub(super) fn draw(f: &mut Frame, app: &App, ticket_id: ulid::Ulid, rail_idx: usize) {
    let theme = &app.theme;
    let area = f.area();
    let Some(ticket) = app.board.ticket(ticket_id) else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // ---- row 0: the header (chip + breadcrumb, `chrome::draw_header`);
    // row 2: the TITLE, the page's own headline (T-158 — it lived only in
    // the breadcrumb before). `r` edits it right here (hardware cursor,
    // tail kept visible).
    chrome::draw_header(f, Rect { x: area.x, y: area.y, width: area.width, height: 1 }, app, None);
    // The header SECTION — title, state line, description — is a band on the
    // elevated surface (author 2026-09-03: "more distinctive"), read in the
    // `sel` ramp; the chip row above and the zones below stay on the page
    // ground. Where the profile paints no elevation the section keeps its
    // shape on the ground in the `rest` ramp.
    let (band, ink) = match theme.selected_bg {
        Some(bg) => (Style::default().bg(bg), &theme.sel),
        None => (Style::default(), &theme.rest),
    };
    let d1 = Style::default().fg(ink.dim1);
    let d2 = Style::default().fg(ink.dim2);
    let title_budget = (area.width as usize).saturating_sub(2);
    let editing = match &app.mode {
        Mode::Input { purpose: InputPurpose::Rename { id }, buffer } if *id == ticket_id => {
            Some(buffer)
        }
        _ => None,
    };
    let title_style = Style::default().fg(ink.base).add_modifier(Modifier::BOLD);
    let title_row = match editing {
        Some(buf) => {
            let budget = title_budget.saturating_sub(1);
            let (shown, cx) = edit_window(buf.as_str(), buf.width_before_cursor(), budget);
            f.set_cursor_position((area.x + (1 + cx).min(area.width - 1), area.y + 2));
            Line::from(vec![Span::raw(" "), Span::styled(shown, title_style)])
        }
        None => Line::from(vec![
            Span::raw(" "),
            Span::styled(truncate(&ticket.title, title_budget), title_style),
        ]),
    };

    // ---- row 3: the STATE line — column, time in that column (the card's
    // own age, `Ticket::column_since`), created age (short keys are hidden
    // from the UI for now, author 2026-08-30; "created by you" went with
    // T-158 — single-user v0.1 says nothing by it). M4: the workspace joins
    // the line — the strategy word until a binding exists, then the branch
    // and its state (short keys resurface through the branch name).
    let here = created_at_epoch_ms(ticket.column_since())
        .map(|ms| format!(" {}", age_in_column(now, ms)))
        .unwrap_or_default();
    let created = created_at_epoch_ms(&ticket.created_at)
        .map(|ms| format!(" ∙ {}", age_created(now, ms)))
        .unwrap_or_default();
    let mut ident_spans = vec![
        Span::styled(format!(" {}", ticket.column.to_uppercase()), d2),
        Span::styled(here, d2),
        Span::styled(created, d2),
    ];
    // The m flow's live reply (armed prompt, outcome, refusal) replaces the
    // resting branch-state hint for a beat — same spot, so the conversation
    // with the merge key happens in one place, never in the footer.
    let note = (!app.merge_note.is_empty()).then(|| crate::text::one_line(&app.merge_note));
    if ticket.is_archived() {
        // Grey ramp only — archived is the quiet condition, not an alarm.
        // The restore key comes from the table, which is also what makes the
        // word flip to "restore" while we are here.
        let how = keymap::hint_for(keymap::Scope::Ticket, keymap::Verb::Archive, &app.ctx())
            .map(|(show, hint)| format!(" ∙ {show} {hint}s"))
            .unwrap_or_default();
        ident_spans.push(Span::styled(format!(" ∙ archived{how}"), d1));
    }
    // The worktree clause is built aside so the tags can sit in front of it:
    // what a ticket IS reads before where its code lives (author 2026-09-01).
    let mut wt_spans = Vec::new();
    if let Some(w) = app.wt_item(ticket.id) {
        // Quiet-tickets rule: a mid-turn agent blocks the merge, so the hint
        // withholds the key (the count still shows what's waiting).
        let busy = app.ticket_busy(ticket.id);
        // What `m` would do next, in the keymap's own words — or nothing at
        // all when `m` is inert here, so the line never names a dead key.
        let ctx = app.ctx();
        let offer = keymap::hint_for(keymap::Scope::Ticket, keymap::Verb::Merge, &ctx)
            .map(|(show, word)| format!(" ∙ {show} {word}"))
            .unwrap_or_default();
        let state = if let Some(n) = &note {
            format!(" ∙ {n}")
        } else if w.conflict {
            " ∙ branch shared!".to_string()
        } else if w.merged {
            format!(" ∙ merged{offer}")
        } else if w.status != "attached" {
            format!(" ∙ {}", w.status)
        } else if w.needs_rebase {
            format!(" ∙ main moved{offer}")
        } else if w.ahead > 0 && busy {
            format!(" ∙ {} to merge", w.ahead)
        } else if w.ahead > 0 {
            format!(" ∙ {} to merge{offer}", w.ahead)
        } else {
            String::new()
        };
        wt_spans.push(Span::styled(format!(" ∙ ⎇ {}", w.branch), d1));
        if !state.is_empty() {
            let actionable =
                !w.merged && w.status == "attached" && (w.needs_rebase || (w.ahead > 0 && !busy));
            let style = if note.is_some() {
                theme.calm_text()
            } else if w.conflict {
                Style::default().fg(ink.base).add_modifier(ratatui::style::Modifier::BOLD)
            } else if actionable {
                theme.calm_text()
            } else {
                d2
            };
            wt_spans.push(Span::styled(state, style));
        }
        if note.is_none() {
            if let Some(d) = &w.detail {
                wt_spans.push(Span::styled(format!(" ∙ {d}"), d2));
            }
        }
    } else if let Some(n) = &note {
        // A merge reply with no binding ("no worktree on this ticket").
        wt_spans.push(Span::styled(format!(" ∙ {n}"), theme.calm_text()));
    } else if ticket.workspace_strategy() == mesimon_core::board::WorkspaceStrategy::Worktree {
        wt_spans.push(Span::styled(" ∙ ⎇ worktree", d2));
    }
    // Tags, spelled out: the ticket page is where you came to read, so there
    // is no reason to make you decode a pip here. Budgeted against the width
    // so a long vocabulary truncates the clause instead of wrapping the row.
    // The chips have first claim on the row and the branch clause takes what
    // is left (a 60-byte slug used to budget the tags out entirely — and the
    // ` ∙` separator was pushed before any chip was tried, so T-163's page
    // read `created 19m ago ∙ ∙ ⎇ msmn/…`, dogfood 2026-09-03). The clause
    // keeps a floor so a ticket wearing ten tags still says where it lives.
    let wt_width: usize = wt_spans.iter().map(|s| s.content.width()).sum();
    // What the clause holds besides its first span (the branch name): the
    // merge state and detail, which are never cut — only the name gives.
    let wt_rest = wt_width.saturating_sub(wt_spans.first().map_or(0, |s| s.content.width()));
    let wt_reserve = wt_width.min(wt_rest + WT_BRANCH_FLOOR);
    if !ticket.tags.is_empty() {
        let used: usize = ident_spans.iter().map(|s| s.content.width()).sum();
        let mut budget = (area.width as usize).saturating_sub(used + wt_reserve + 4);
        // Each tag as a short painted chip carrying its name — the same paint
        // the card band uses, so the two surfaces agree at a glance.
        let mut chips = Vec::new();
        for t in &ticket.tags {
            let text = format!(" {} ", t.name);
            if text.width() + 1 > budget {
                break;
            }
            budget -= text.width() + 1;
            chips.push(Span::raw(" "));
            let tint = theme.pip(app.board.tint_of(t) as usize);
            if theme.paints_tags() {
                chips.push(Span::styled(text, Style::default().bg(tint).fg(theme.tag_ink())));
            } else {
                chips.push(Span::styled(text, d1));
            }
        }
        // The separator belongs to the chips: none fitting means no bullet.
        if !chips.is_empty() {
            ident_spans.push(Span::styled(" ∙", d2));
            ident_spans.extend(chips);
        }
    }
    // The branch name fits the room the rest of the row leaves, cut with the
    // `~` marker (never below its floor) rather than the line running off the
    // right edge — `truncate` never marks what fits.
    let used: usize = ident_spans.iter().map(|s| s.content.width()).sum();
    let room = (area.width as usize).saturating_sub(used + 1);
    if wt_width > room {
        if let Some(first) = wt_spans.first_mut() {
            let cut = truncate(&first.content, room.saturating_sub(wt_rest).max(WT_BRANCH_FLOOR));
            *first = Span::styled(cut, first.style);
        }
    }
    ident_spans.extend(wt_spans);
    let ident = Line::from(ident_spans);

    // Breathing row between the header and the title (06 §5.5); the state
    // line sits directly under the title, the way a card's meta row does.
    // No band under it: the ticket header ends with its metadata (author
    // 2026-08-30).
    // The identity band — pad, title, state, pad — painted edge to edge with
    // real space cells, because an empty `Line` paints nothing.
    let head: Vec<Line<'static>> = vec![Line::default(), title_row, ident, Line::default()]
        .into_iter()
        .map(|mut l| {
            let used: usize = l.spans.iter().map(|s| s.content.width()).sum();
            l.spans.push(Span::raw(" ".repeat((area.width as usize).saturating_sub(used))));
            l.style(band)
        })
        .collect();
    f.render_widget(
        Paragraph::new(head),
        Rect { x: area.x, y: area.y + 1, width: area.width, height: 4.min(area.height - 1) },
    );
    // The description — the ticket's first note — is the CARD'S BODY (author
    // 2026-09-03, after a second band was refused): on the page ground under
    // the band, in the card's own frame `[pad 1][bar 1][pad 1][text]`, with
    // the accent bar down its left edge wearing the ticket's tags exactly as
    // the card's stripe does on the board (`tags::bar_cell` + `stack_full`,
    // the composer dialog's road). The page reads as the card, opened. Rich
    // text, capped: what the ticket IS reads before what its sessions are
    // doing. No heading over it; it is the ticket's own words. Nothing when
    // there is none, so the geometry below is untouched then.
    let body_rows = (area.height as usize).saturating_sub(7);
    let desc: Vec<Line<'static>> = ticket
        .description()
        .and_then(|m| app.note_text(m))
        .map(|text| {
            let cap = DESC_MAX_ROWS.min(body_rows / 3).max(1);
            let width = (area.width as usize).saturating_sub(4);
            crate::rich::render(text, width, cap, theme)
        })
        .unwrap_or_default();
    // The rows, then a breathing row.
    let extra = if desc.is_empty() { 0 } else { desc.len() as u16 + 1 };
    if !desc.is_empty() {
        let h = (desc.len() as u16).min(area.height.saturating_sub(7));
        let worn = crate::tags::painted(&app.board, &ticket.tags);
        let (plain_ch, plain_style) = theme.bar(crate::theme::BarWeight::Cursor);
        let (bar_ch, bar_style) = crate::tags::bar_cell(
            theme,
            plain_ch,
            plain_style,
            &worn,
            crate::theme::TagLevel::Selected,
        );
        let mut stripe: Vec<Line<'static>> =
            (0..h).map(|_| Line::from(Span::styled(bar_ch.clone(), bar_style))).collect();
        crate::tags::stack_full(
            theme,
            &mut stripe,
            plain_ch,
            plain_style,
            &worn,
            crate::theme::TagLevel::Selected,
        );
        f.render_widget(
            Paragraph::new(stripe),
            Rect { x: area.x + 1, y: area.y + 6, width: 1, height: h },
        );
        f.render_widget(
            Paragraph::new(desc),
            Rect { x: area.x + 3, y: area.y + 6, width: area.width.saturating_sub(4), height: h },
        );
    }

    // ---- body zones -------------------------------------------------------
    // One breathing row under the band or the body (06 §5.5) before the zones.
    let body_y = area.y + 6 + extra;
    // header 1 + band 4 + breathing 1 + footer 1, plus the description rows.
    let body_h = area.height.saturating_sub(7 + extra);
    let two_zone = area.width >= TWO_ZONE_MIN_W;
    if two_zone {
        // Transcript preview: the selected rail session's latest assistant
        // reply, read through the same draw cache as the board's `p` peek
        // (one slot is still enough — board and ticket never draw the same
        // frame). Bash sessions have no transcript and preview nothing.
        let row = app.rail_rows(ticket_id).get(rail_idx).copied();
        let sel = match row {
            Some(RailRow::Session(s)) => Some(s),
            _ => None,
        };
        // A note row: the note itself, whole, once it has been fetched.
        let note = match row {
            Some(RailRow::Note(n)) => Some((n, app.note_text(n))),
            _ => None,
        };
        let peek =
            sel.and_then(|s| s.transcript_path.as_deref()).and_then(|p| app.peek_cache.peek(p));
        // A mid-turn agent keeps composing past whatever the preview shows,
        // so the zone says so (bash panes work too, but have no transcript
        // for the line to qualify — Claude only).
        let working =
            sel.is_some_and(|s| s.kind == SessionKind::Claude && s.state == SessionState::Running);
        // A shell keeps no transcript — tmux is its only record — so the zone
        // shows the pane itself, under its own heading. Only ever what the
        // poll already fetched for THIS session: a stale capture under a
        // freshly selected row would be another session's screen.
        let shell = app
            .shell_tail
            .as_ref()
            .filter(|t| sel.is_some_and(|s| s.id == t.session))
            .map(|t| t.lines.as_slice());
        let left_w = area.width - RAIL_W - 3; // 1 pad + 2-cell divider gap
        draw_preview(
            f,
            Rect { x: area.x + 1, y: body_y, width: left_w, height: body_h },
            app,
            sel.map(|s| s.id),
            peek.as_ref(),
            working,
            shell,
            note,
        );
        draw_rail(
            f,
            Rect {
                x: area.x + left_w + 3,
                y: body_y,
                width: RAIL_W.saturating_sub(1),
                height: body_h,
            },
            app,
            ticket_id,
            rail_idx,
            now,
        );
    } else {
        // No zone, nothing to page: the footer must not offer `{ }`.
        app.preview_view.set(PreviewView::default());
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
    // Rendered from the keymap like every other screen (`m` stays out of it
    // by carrying prio 0 — the merge conversation happens on the identity
    // line, next to the branch state it acts on).
    let footer = chrome::footer_line(app, area.width);
    f.render_widget(
        Paragraph::new(footer),
        Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 },
    );
}

#[allow(clippy::too_many_arguments)] // one call site; a params struct would just rename the args
fn draw_preview(
    f: &mut Frame,
    area: Rect,
    app: &App,
    session: Option<uuid::Uuid>,
    peek: Option<&crate::peek::Peek>,
    working: bool,
    shell: Option<&[String]>,
    note: Option<(&NoteMeta, Option<&str>)>,
) {
    let theme = &app.theme;
    let mut lines: Vec<Line<'static>> = Vec::new();
    // The heading carries the paging keys on its right while the zone
    // overflows (T-158: the hint beside the thing it pages, off the footer)
    // — read from the LAST frame's measurement, before this one resets it.
    let ctx = app.ctx();
    let heading = || -> Line<'static> {
        let mut spans = vec![Span::styled(" PREVIEW", theme.dim1().add_modifier(Modifier::BOLD))];
        let keys = keymap::binding_for(keymap::Scope::Ticket, keymap::Verb::PageDown, &ctx)
            .map(|b| chrome::hint_spans(&[b], &ctx, &theme.rest, (area.width as usize) / 2))
            .unwrap_or_default();
        let keys_w: usize = keys.iter().map(|s| s.content.width()).sum();
        if keys_w > 0 {
            spans.push(Span::raw(" ".repeat((area.width as usize).saturating_sub(8 + keys_w + 1))));
            spans.extend(keys);
        }
        Line::from(spans)
    };
    // Until something below measures a document, there is nothing to page.
    app.preview_view.set(PreviewView::default());

    // The selected session's latest assistant reply, wrapped into whatever
    // height the zone has left. Absent transcript (bash, fresh spawn) means
    // no section at all — a heading over nothing is noise — UNLESS the agent
    // is mid-turn: then the section closes with the rail's own spinner and
    // state word, so a stale reply (or no reply yet) reads as in-progress.
    let reply = peek.and_then(|p| p.text.as_deref());
    // A shell keeps no transcript, so the zone shows its pane instead. One
    // heading covers both, and PREVIEW is the honest word for either: neither
    // side is the record, both are the last of it, and the rail row beside it
    // already says which session the cursor is on (author 2026-09-01).
    if let Some(tail) = shell {
        lines.push(heading());
        lines.push(Line::default());
        if tail.is_empty() {
            lines.push(Line::from(Span::styled("   nothing on screen yet", theme.dim3())));
        }
        let budget = (area.height as usize).saturating_sub(lines.len());
        let width = (area.width as usize).saturating_sub(4);
        let rows: Vec<Line<'static>> = tail
            .iter()
            .map(|row| {
                // Output is column-aligned — wrapping would mangle the
                // alignment it was printed with, so an over-wide line is cut
                // instead. It goes through the peek's sweep first: a pane
                // holds whatever a command decided to print, box-drawing and
                // all.
                let row = crate::text::truncate(&crate::peek::sanitize(row), width);
                Line::from(Span::styled(row, theme.dim1()))
            })
            .collect();
        // Newest at the bottom, exactly as the pane holds it: the latest
        // command and what it printed are what the rows are for, so a tail
        // too long for the zone loses its top, never its end — until `{`
        // asks for the top, and then the window is the reader's.
        let key = session.map(|s| doc_key(s, None));
        let shown = window(app, key, rows, budget, width, true);
        for row in shown {
            let mut spans = vec![Span::raw("   ")];
            spans.extend(row.spans);
            lines.push(Line::from(spans));
        }
    } else if let Some((meta, text)) = note {
        // A note, whole: the same rich text as a reply, paged the same way,
        // keyed to the note and its revision so an agent's rewrite starts
        // the page at the top.
        lines.push(heading());
        lines.push(Line::default());
        match text {
            None => lines.push(Line::from(Span::styled("   fetching", theme.dim3()))),
            Some(text) => {
                let budget = (area.height as usize).saturating_sub(lines.len());
                let width = (area.width as usize).saturating_sub(4);
                let rows = crate::rich::render_all(text, width, theme);
                let shown = window(app, Some(note_key(meta)), rows, budget, width, false);
                for row in shown {
                    let mut spans = vec![Span::raw("   ")];
                    spans.extend(row.spans);
                    lines.push(Line::from(spans));
                }
            }
        }
    } else if reply.is_some() || working {
        lines.push(heading());
        lines.push(Line::default());
        if let Some(text) = reply {
            // Reserve the indicator's rows so a long reply never pushes it off.
            let reserve = if working { 2 } else { 0 };
            let budget = (area.height as usize).saturating_sub(lines.len() + reserve);
            // Rich text, not the source: an agent reply is markdown, and the
            // ticket page is the surface with room to read it as such
            // (rich.rs — value, weight, paint and space only).
            let width = (area.width as usize).saturating_sub(4);
            // Keyed to the reply as well as the session: a page into this
            // reply must not open the next one halfway down.
            let key = session.map(|s| doc_key(s, Some(text)));
            let rows = crate::rich::render_all(text, width, theme);
            let shown = window(app, key, rows, budget, width, false);
            for row in shown {
                let mut spans = vec![Span::raw("   ")];
                spans.extend(row.spans);
                lines.push(Line::from(spans));
            }
            if working {
                lines.push(Line::default());
            }
        }
        if working {
            // The indicator says what the agent is doing, not just that it
            // is: the newest tool call's own title, after the state word so
            // the vocabulary stays the rail's (07 §11.3). `thinking` REPLACES
            // the state word — "working ∙ thinking" says one thing twice.
            let tier = theme.glyph_tier();
            let working_word = glyphs::state_word(&SessionState::Running);
            let row = match peek.and_then(|p| p.activity.as_ref()) {
                Some(crate::peek::Doing::Tool(t)) => format!("{working_word} ∙ {t}"),
                Some(crate::peek::Doing::Thinking) => "thinking".to_string(),
                None => working_word.to_string(),
            };
            let row = crate::text::truncate(&row, (area.width as usize).saturating_sub(6));
            let mark =
                if glyphs::pulse_lit(app.spin_frame()) { theme.dim2() } else { theme.dim3() };
            lines.push(Line::from(vec![
                Span::styled(format!("   {} ", glyphs::pulse(tier)), mark),
                Span::styled(row, theme.dim2()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// Which document the preview zone is showing, for the scroll to belong to:
/// the session, and for an agent the reply itself (a shell's pane is one
/// continuous stream, so new output does not make it a new document).
fn doc_key(session: uuid::Uuid, reply: Option<&str>) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    session.hash(&mut h);
    reply.hash(&mut h);
    h.finish()
}

/// A note's document key: the note and its revision, with a discriminant
/// so it can never collide with a session's.
fn note_key(meta: &NoteMeta) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    1u8.hash(&mut h);
    meta.id.hash(&mut h);
    meta.rev.hash(&mut h);
    h.finish()
}

/// The zone's window onto `rows`: honours the offset `{ }` asked for
/// (`App::preview_scroll`, only if it was asked of THIS document), clamps
/// it the way the diff pane does and writes the clamp back, and records what
/// was shown (`App::preview_view`) so the next press and the footer know
/// the page size and whether there is a further one. A window that stops
/// short of the last row ends in the `~` cut mark.
fn window(
    app: &App,
    key: Option<u64>,
    rows: Vec<Line<'static>>,
    budget: usize,
    width: usize,
    follows_tail: bool,
) -> Vec<Line<'static>> {
    let total = rows.len();
    let max = total.saturating_sub(budget);
    let asked = match (app.preview_scroll.get(), key) {
        (Some((k, n)), Some(key)) if k == key => Some(n),
        _ => None,
    };
    let offset = match asked {
        Some(n) => n.min(max),
        None if follows_tail => max,
        None => 0,
    };
    if asked.is_some() {
        // A tail scrolled to its bottom is released, not pinned to it.
        let back = if follows_tail && offset >= max { None } else { key.map(|k| (k, offset)) };
        app.preview_scroll.set(back);
    }
    app.preview_view.set(PreviewView {
        key,
        offset,
        max,
        page: budget.saturating_sub(1).max(1),
        follows_tail,
    });
    let mut shown: Vec<Line<'static>> = rows.into_iter().skip(offset).take(budget).collect();
    if offset + shown.len() < total {
        crate::rich::mark_cut(&mut shown, width, &app.theme);
    }
    shown
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
    let notes: &[NoteMeta] = app.board.ticket(ticket_id).map(|t| t.notes.as_slice()).unwrap_or(&[]);
    let w = area.width as usize;

    let mut head = vec![Span::styled(" SESSIONS", theme.dim1().add_modifier(Modifier::BOLD))];
    let right = rail.len().to_string();
    let used: usize = 9 + right.width() + 1;
    head.push(Span::raw(" ".repeat(w.saturating_sub(used))));
    head.push(Span::styled(right, theme.dim2()));
    let mut lines: Vec<Line<'static>> = vec![Line::from(head), Line::default()];

    // The sessions' keys sit under the sessions (T-158): the empty state
    // names the two spawn verbs (07 §16.2) and a populated rail adds the
    // one that acts on the selected row — in the keymap's words, through
    // the footer's own span builder, so nothing here can drift from the
    // keys. The footer no longer carries them.
    let ctx = app.ctx();
    // Packed into as many rows as the rail's width needs — a key that does
    // not fit is wrapped, never dropped, because these ARE the hints now.
    let trailer = |verbs: &[keymap::Verb]| -> Vec<Line<'static>> {
        let bound: Vec<&keymap::Binding> = verbs
            .iter()
            .filter_map(|v| keymap::binding_for(keymap::Scope::Ticket, *v, &ctx))
            .collect();
        let budget = w.saturating_sub(1);
        let mut rows: Vec<Vec<&keymap::Binding>> = Vec::new();
        let mut used = 0usize;
        for b in bound {
            let need = b.show.width() + 1 + (b.hint)(&ctx).width();
            let add = if used == 0 { need } else { need + 3 };
            match rows.last_mut() {
                Some(row) if used + add <= budget => {
                    row.push(b);
                    used += add;
                }
                _ => {
                    rows.push(vec![b]);
                    used = need;
                }
            }
        }
        rows.into_iter()
            .map(|row| {
                let mut spans = vec![Span::raw(" ")];
                spans.extend(chrome::hint_spans(&row, &ctx, &theme.rest, budget));
                Line::from(spans)
            })
            .collect()
    };
    if rail.is_empty() {
        lines.extend(trailer(&[keymap::Verb::Claude, keymap::Verb::Shell]));
    }

    for (i, s) in rail.iter().enumerate() {
        let selected = i == rail_idx;
        let (g, reg) = glyphs::session_glyph(s, tier, app.spin_frame());
        let glyph_style = match reg {
            glyphs::Register::Attn => theme.attn_text(),
            glyphs::Register::Err => theme.err_text(),
            glyphs::Register::Calm => theme.calm_text(),
            glyphs::Register::Grey => theme.dim2(),
            glyphs::Register::Dormant => theme.dim3(),
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
        let kind =
            if scroll > 0 { marquee_window(kind, budget, scroll) } else { truncate(kind, budget) };
        let mark = glyphs::kind_mark(s.kind, tier);
        let age = s
            .state_changed_at
            .map(|ms| age_slot(now, ms, glyphs::is_working(s)))
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
        let mut spans =
            vec![Span::styled(g.to_string(), glyph_style), Span::styled(name.clone(), name_style)];
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
            let q = s.detail.clone().unwrap_or_else(|| glyphs::state_word(&s.state).to_lowercase());
            let text = format!("    {}", truncate(&q, w.saturating_sub(4)));
            let pad = w.saturating_sub(text.width());
            lines.push(
                Line::from(vec![Span::styled(text, theme.attn_text()), Span::raw(" ".repeat(pad))])
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
                    Line::from(vec![Span::styled(text, theme.dim2()), Span::raw(" ".repeat(pad))])
                        .style(row_style),
                );
            }
        }
    }

    if !rail.is_empty() {
        lines.extend(trailer(&[keymap::Verb::Claude, keymap::Verb::Shell, keymap::Verb::Sleep]));
    }

    // ---- the notes, under the sessions ------------------------------------
    // Same shape as the sessions: a heading with the count, one row each —
    // mark, name (the note's first line), then who last wrote it and when.
    // `n` opens the selected one; the preview zone reads it.
    if !notes.is_empty() {
        lines.push(Line::default());
        let mut head = vec![Span::styled(" NOTES", theme.dim1().add_modifier(Modifier::BOLD))];
        let right = notes.len().to_string();
        let used: usize = 6 + right.width() + 1;
        head.push(Span::raw(" ".repeat(w.saturating_sub(used))));
        head.push(Span::styled(right, theme.dim2()));
        lines.push(Line::from(head));
        lines.push(Line::default());
        for (j, n) in notes.iter().enumerate() {
            let selected = rail.len() + j == rail_idx;
            let who = author_word(&n.edited_by);
            let age = created_at_epoch_ms(&n.edited_at)
                .map(|ms| age_slot(now, ms, false))
                .unwrap_or_default();
            let tail = format!("{who} {age}");
            let budget = w.saturating_sub(4 + tail.width() + 1);
            let name = truncate(&n.name, budget);
            let name_style = if selected {
                Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)
            } else {
                theme.base()
            };
            let mark = glyphs::note_mark(tier);
            let head = format!(" {mark} {name}");
            let used: usize = head.width() + tail.width() + 1;
            let spans = vec![
                Span::styled(head, name_style),
                Span::raw(" ".repeat(w.saturating_sub(used))),
                Span::styled(tail, theme.dim2()),
            ];
            let row_style = if selected { theme.selected_row() } else { Style::default() };
            lines.push(Line::from(spans).style(row_style));
        }
        lines.extend(trailer(&[keymap::Verb::NoteNew]));
    }

    f.render_widget(Paragraph::new(lines), area);
}
