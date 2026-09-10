//! The ticket screen skeleton (07 §14, reshaped by dogfood 2026-08-30).
//! Header is a breadcrumb — `mesimon > repo > title` — with the needs-you
//! glyph + count beside the repo name (same accent as the board), and one
//! quiet identity line (short key ∙ column ∙ created) instead of the full
//! board strip. `r` edits the title in place. Nothing separates the header
//! from the body but a breathing row — no band, no rule (L1); the zone
//! divider is a 2-cell gap.

use mesimon_core::board::{NoteMeta, Provenance, SessionKind, SessionState, StopReason};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::rc::Rc;

use super::RichCache;
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
/// this board is `you`, and a retained agent record names its provider.
pub(super) fn author_word(by: &str, app: &App) -> &'static str {
    if let Some(id) = by.strip_prefix("agent:") {
        id.parse::<uuid::Uuid>()
            .ok()
            .and_then(|id| app.board.sessions.iter().find(|s| s.id == id))
            .and_then(|s| s.kind.provider())
            .map(keymap::agent_word)
            .unwrap_or("agent")
    } else {
        "you"
    }
}

/// Where the branch's work landed, in words (T-267). A plain fast-forward
/// into the checkout's own default branch stays the bare `merged` it has
/// always been — that is the ref the whole page is already about. Anything
/// else names the ref, because the user did not do it here: a PR squashed
/// into `origin/main` shows up after a fetch, and the commit carrying it is
/// named so it can be looked at (`git show 1a2b3c4`).
fn merged_word(w: &mesimon_core::command::WorktreeItem) -> String {
    if w.merged_in.is_empty() {
        return "merged".to_string();
    }
    let short = &w.merged_oid[..w.merged_oid.len().min(7)];
    if short.is_empty() {
        format!("merged into {}", w.merged_in)
    } else {
        format!("merged into {} as {short}", w.merged_in)
    }
}

pub(super) fn draw(f: &mut Frame, app: &App, ticket_id: ulid::Ulid, rail_idx: usize) {
    let theme = &app.theme;
    let area = f.area();
    let Some(ticket) = app.board.ticket(ticket_id) else {
        return;
    };
    let now = mesimon_core::clock::now_ms();
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
    // `d` arms here too, and the title row is the page's card: it flashes
    // as a deletion on the same clock the board card does.
    let doomed = app.doomed(ticket_id) && theme.delete_lit(app.spin_frame());
    // `z` arms here too: the title blinks on the move ghost's clock, the
    // board card's treatment for an armed snooze, in this band's own ramp.
    let snoozing = app.snooze_row(ticket_id).is_some();
    let title_style = if doomed {
        theme.err_text().add_modifier(Modifier::BOLD)
    } else if snoozing {
        let blink = theme.move_blink(app.spin_frame());
        let fg = if blink.fg == Some(theme.sel.dim3) { ink.dim3 } else { ink.base };
        Style::default().fg(fg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ink.base).add_modifier(Modifier::BOLD)
    };
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
    // T-158 — single-user v0.1 says nothing by it — and since T-253 the
    // clause names the author only when it was NOT the person reading:
    // `created 2d ago by claude on T-241` on a ticket an agent filed through
    // `create_ticket` — `Ticket::created_by`, and `created_from` resolved to
    // the parent's key while that ticket is still on the board). M4: the workspace joins
    // the line — the strategy word until a binding exists, then the branch
    // and its state (short keys resurface through the branch name).
    let here = created_at_epoch_ms(ticket.column_since())
        .map(|ms| format!(" {}", age_in_column(now, ms)))
        .unwrap_or_default();
    let by = if ticket.agent_created() {
        let on = ticket
            .created_from
            .and_then(|from| app.board.ticket(from))
            .map(|parent| format!(" on {}", parent.short_key))
            .unwrap_or_default();
        format!(" by {}{on}", author_word(&ticket.created_by, app))
    } else {
        String::new()
    };
    let created = created_at_epoch_ms(&ticket.created_at)
        .map(|ms| format!(" ∙ {}{by}", age_created(now, ms)))
        .unwrap_or_default();
    let previous = ticket
        .previous_column
        .as_ref()
        .filter(|stay| stay.seconds > 60)
        .map(|stay| {
            format!(
                " ∙ previously {} for {}",
                crate::text::one_line(&stay.column).to_uppercase(),
                crate::text::column_duration(stay.seconds)
            )
        })
        .unwrap_or_default();
    let mut ident_spans = vec![
        Span::styled(format!(" {}", ticket.column.to_uppercase()), d2),
        Span::styled(here, d2),
        Span::styled(previous, d2),
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
        // A snooze says when it ends; restoring it by hand ends it early.
        let wakes = ticket
            .snooze_until_secs()
            .map(|until| format!(" ∙ wakes {}", crate::text::until_word(now, until * 1000)))
            .unwrap_or_default();
        ident_spans.push(Span::styled(format!(" ∙ archived{wakes}{how}"), d1));
    } else if let Some(r) = ticket.raised.as_ref() {
        // An agent asked for a person (T-107). The page is where the ask is
        // answered, so the row says when it went up and repeats the words
        // the card had room for only one line of — and unlike the snooze
        // mark this one survives the arrival: it is lowered on the way OUT,
        // which is what gives the sentence time to be read.
        let when = created_at_epoch_ms(&r.at)
            .map(|ms| match age_slot(now, ms, false).as_str() {
                "now" => " just now".to_string(),
                age => format!(" {age} ago"),
            })
            .unwrap_or_default();
        ident_spans.push(Span::styled(format!(" ∙ {} asked{when}", author_word(&r.by, app)), d1));
        ident_spans.push(Span::styled(format!(" ∙ {}", crate::text::one_line(&r.reason)), d1));
    } else if ticket.is_woke() {
        // Back from a snooze and not yet looked at: the page IS the look, so
        // the keypress that opened it is clearing the mark as this draws.
        ident_spans.push(Span::styled(" ∙ back from snooze".to_string(), d1));
    } else if ticket.description().is_some()
        && app.board.live_agent(ticket.id).is_some_and(|s| s.state.has_prompted() && !s.ticket_read)
    {
        // The ticket has a brief and its claude has taken a turn without
        // reading it — neither `get_ticket` nor the composed spawn's paste
        // (`SessionRecord::ticket_read`, T-224). The skip was invisible until
        // the work came back wrong; here it sits next to the description it
        // is about, in the value step: a nudge to Shift+Enter "read the
        // ticket", not an alarm.
        ident_spans.push(Span::styled(" ∙ description unread".to_string(), d1));
    }
    // `^k` is not named on this row (T-312, user request), the way `!` came
    // off the header and the footers before it (T-277): the binding stays,
    // and `?` is where it is taught, like every other overlay-only key of
    // this screen.
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
        // The ask the agent already has takes the offer's place: "main moved
        // ∙ rebase requested" until it lands or the cooldown passes
        // (`App::merge_outstanding`), never the same ask offered twice.
        let offer = keymap::hint_for(keymap::Scope::Ticket, keymap::Verb::Merge, &ctx)
            .map(|(show, word)| format!(" ∙ {show} {word}"))
            .or_else(|| app.merge_outstanding(ticket.id).map(|w| format!(" ∙ {w}")))
            .unwrap_or_default();
        let state = if let Some(n) = &note {
            format!(" ∙ {n}")
        } else if w.conflict {
            " ∙ branch shared!".to_string()
        } else if w.merged {
            format!(" ∙ {}{offer}", merged_word(w))
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
            let actionable = !w.merged
                && w.status == "attached"
                && (w.needs_rebase || (w.ahead > 0 && !busy))
                && app.merge_outstanding(ticket.id).is_none();
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
    // What mesimon owes this ticket, in the card's own words (2026-09-04):
    // `queued ∙ after T-12`, `train ∙ merges when quiet`. The value step,
    // never calm — calm is the `m` offer's register on this row.
    if let Some(row) = app.pending_row(ticket.id) {
        wt_spans.push(Span::styled(format!(" ∙ {row}"), d2));
    }
    // Tags, spelled out: the ticket page is where you came to read, so there
    // is no reason to make you decode a pip here. Budgeted against the width
    // so a long vocabulary truncates the clause instead of wrapping the row.
    // The chips have first claim on the row and the branch clause takes what
    // is left (a 60-byte slug used to budget the tags out entirely — and the
    // ` ∙` separator was pushed before any chip was tried, so T-163's page
    // read `created 19m ago ∙ ∙ ⎇ msmn/…`, dogfood 2026-09-03). The clause
    // keeps a floor so a ticket wearing ten tags still says where it lives.
    let wt_width: usize = super::spans_width(&wt_spans);
    // What the clause holds besides its first span (the branch name): the
    // merge state and detail, which are never cut — only the name gives.
    let wt_rest = wt_width.saturating_sub(wt_spans.first().map_or(0, |s| s.content.width()));
    let wt_reserve = wt_width.min(wt_rest + WT_BRANCH_FLOOR);
    if !ticket.tags.is_empty() {
        let used: usize = super::spans_width(&ident_spans);
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
    let used: usize = super::spans_width(&ident_spans);
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
    // ONE band for the whole header section (author 2026-09-03, third pass):
    // pad, title, state line, one blank row, the description, pad — every
    // row painted edge to edge with real space cells, because an empty
    // `Line` paints nothing. The description is the card's body inside it:
    // `[pad 1][bar 1][pad 1][text]`, the bar the card's NEUTRAL cursor-weight
    // bar thinned to a quarter cell (`Theme::desc_bar`; no tag tints — the
    // state line already names the tags), the text in
    // the `sel` ramp with code sunk to the page ground (`Surface::Elevated`).
    // Rich text, capped: what the ticket IS reads before what its sessions
    // are doing. No heading over it; it is the ticket's own words.
    let body_rows = (area.height as usize).saturating_sub(7);
    let desc: Vec<Line<'static>> = ticket
        .description()
        .and_then(|m| app.note_text(m))
        .map(|text| {
            let cap = DESC_MAX_ROWS.min(body_rows / 3).max(1);
            let surface = if theme.selected_bg.is_some() {
                crate::rich::Surface::Elevated
            } else {
                crate::rich::Surface::Ground
            };
            let width = (area.width as usize).saturating_sub(4);
            crate::rich::render_on(text, width, cap, theme, surface)
        })
        .unwrap_or_default();
    // The description's rows plus its bottom pad; the blank over it is the
    // band's own fourth row.
    let extra = if desc.is_empty() { 0 } else { desc.len() as u16 + 1 };
    let mut rows = vec![Line::default(), title_row, ident, Line::default()];
    if !desc.is_empty() {
        let (bar_ch, bar_style) = theme.desc_bar();
        for row in desc {
            let mut spans =
                vec![Span::raw(" "), Span::styled(bar_ch.to_string(), bar_style), Span::raw(" ")];
            spans.extend(row.spans);
            rows.push(Line::from(spans));
        }
        rows.push(Line::default());
    }
    let head: Vec<Line<'static>> = rows
        .into_iter()
        .enumerate()
        .map(|(i, mut l)| {
            let used: usize = super::spans_width(&l.spans);
            l.spans.push(Span::raw(" ".repeat((area.width as usize).saturating_sub(used))));
            // Row 1 is the title row; lit, it takes the deletion ground.
            if doomed && i == 1 {
                l.style(theme.delete_row())
            } else {
                l.style(band)
            }
        })
        .collect();
    f.render_widget(
        Paragraph::new(head),
        Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: (4 + extra).min(area.height.saturating_sub(1)),
        },
    );

    // ---- body zones -------------------------------------------------------
    // One breathing row under the band (06 §5.5) before the zones.
    let body_y = area.y + 6 + extra;
    // header 1 + band 4 + breathing 1 + footer 1, plus the description rows.
    let body_h = area.height.saturating_sub(7 + extra);
    let two_zone = area.width >= TWO_ZONE_MIN_W;
    if two_zone {
        // Transcript preview: the selected rail session's latest assistant
        // reply, read through the same draw cache as the board's `p` peek
        // and the spoke scan (one entry per path since T-173). Bash sessions
        // have no transcript and preview nothing.
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
        let peek = sel.and_then(|s| app.peek_cache.peek_for(s.kind, crate::peek::preview_path(s)?));
        // A mid-turn agent keeps composing past whatever the preview shows,
        // so the zone says so (bash panes work too, but have no transcript
        // for the line to qualify — Claude only).
        let working = sel.is_some_and(|s| s.kind.is_agent() && s.state == SessionState::Running);
        // A shell keeps no transcript — tmux is its only record — so the zone
        // shows the pane itself, under its own heading. Only ever what the
        // poll already fetched for THIS session: a stale capture under a
        // freshly selected row would be another session's screen.
        let shell = app
            .shell_tail
            .as_ref()
            .filter(|t| sel.is_some_and(|s| s.id == t.session))
            .map(|t| t.lines.as_slice());
        // The empty seat (T-308): the cursor is on the `+ claude session`
        // row and there is no document to show, so the zone previews the
        // SESSION the press would start instead of standing empty.
        let seat = matches!(row, Some(RailRow::NewAgent)).then_some(ticket_id);
        let left_w = area.width - RAIL_W - 3; // 1 pad + 2-cell divider gap
        draw_preview(
            f,
            Rect { x: area.x + 1, y: body_y, width: left_w, height: body_h },
            app,
            sel,
            peek.as_deref(),
            working,
            shell,
            note,
            seat,
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
    record: Option<&mesimon_core::board::SessionRecord>,
    peek: Option<&crate::peek::Peek>,
    working: bool,
    shell: Option<&[String]>,
    note: Option<(&NoteMeta, Option<&str>)>,
    seat: Option<ulid::Ulid>,
) {
    let theme = &app.theme;
    let session = record.map(|s| s.id);
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
        let keys_w: usize = super::spans_width(&keys);
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
        let shown = window(app, key, &rows, budget, width, true);
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
                let key = note_key(meta);
                let rows = rendered(app, key, width, text);
                let shown = window(app, Some(key), &rows, budget, width, false);
                for row in shown {
                    let mut spans = vec![Span::raw("   ")];
                    spans.extend(row.spans);
                    lines.push(Line::from(spans));
                }
            }
        }
    } else if reply.is_some() {
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
            let rows = match key {
                Some(key) => rendered(app, key, width, text),
                None => std::rc::Rc::new(crate::rich::render_all(text, width, theme)),
            };
            let shown = window(app, key, &rows, budget, width, false);
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
            lines.push(working_row(app, peek, area.width as usize));
        }
    } else if let Some(rec) = record {
        // A session with nothing to read (T-308): the young one whose first
        // words have not landed, the corpse that never spoke, the shell whose
        // pane has not been captured yet. The zone stood blank for all of
        // them — the same blank the empty seat had, one press later.
        lines.push(heading());
        lines.extend(quiet_session(app, rec, peek, area));
    } else if let Some(ticket) = seat.and_then(|id| app.board.ticket(id)) {
        lines.push(heading());
        lines.extend(empty_seat(app, ticket, area));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// The working indicator: what the agent is DOING, not just that it is —
/// the newest tool call's own title after the state word, so the vocabulary
/// stays the rail's (07 §11.3). `thinking` REPLACES the state word: "working
/// ∙ thinking" says one thing twice. It closes a reply that is still being
/// written, and it IS the headline for a turn that has not said anything
/// yet (T-308).
fn working_row(app: &App, peek: Option<&crate::peek::Peek>, width: usize) -> Line<'static> {
    let theme = &app.theme;
    let working_word = glyphs::state_word(&SessionState::Running);
    let row = match peek.and_then(|p| p.activity.as_ref()) {
        Some(crate::peek::Doing::Tool(t)) => format!("{working_word} ∙ {t}"),
        Some(crate::peek::Doing::Thinking) => "thinking".to_string(),
        None => working_word.to_string(),
    };
    let row = truncate(&row, width.saturating_sub(6));
    let mark = if glyphs::pulse_lit(app.spin_frame()) { theme.dim2() } else { theme.dim3() };
    Line::from(vec![
        Span::styled(format!("   {} ", glyphs::pulse(app.theme.glyph_tier())), mark),
        Span::styled(row, theme.dim2()),
    ])
}

/// A selected session with nothing to read (T-308). The zone drew NOTHING
/// for every one of these — a claude still coming up, one waiting for the
/// first prompt, a turn in flight before its first words land, a corpse that
/// never spoke, a shell whose pane has not been captured yet — which is the
/// same blank the empty seat had, one press later.
///
/// It is a REPORT where `empty_seat` is an invitation, and that is why it
/// carries no press row: the offer's row has no hint of its own, so the mark
/// has to say what Enter does, while a session row is already spelled twice
/// (its own `enter resumes` badge, and the footer). What this owes the reader
/// instead is WHY there is nothing, which nothing else on the page says.
fn quiet_session(
    app: &App,
    rec: &mesimon_core::board::SessionRecord,
    peek: Option<&crate::peek::Peek>,
    area: Rect,
) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let w = area.width as usize;
    let (headline, clauses) = quiet_words(rec);
    let mut text: Vec<Line<'static>> = Vec::new();
    if rec.state == SessionState::Running {
        // The pulse is the headline here: a turn in flight has a live thing
        // to say about itself, and `quiet_words` leaves the row to it.
        text.push(working_row(app, peek, w));
    } else if !headline.is_empty() {
        // A needs-you session's headline is its own question, in the
        // attention register the rail row beside it is already wearing: the
        // one place in this zone a saturated colour is earned, and the law
        // that reserves it is about what the colour MEANS, not how many
        // cells spend it. The rail cuts that question to its 26; here it has
        // the zone's width and wraps — the card-versus-page split again, the
        // index truncates and the reading surface reads.
        let attn = matches!(rec.state, SessionState::RequiresAction { .. });
        let style = if attn { theme.attn_text() } else { theme.dim1() };
        for row in crate::peek::wrap(&headline, w.saturating_sub(4), 3) {
            text.push(Line::from(vec![Span::raw("   "), Span::styled(row, style)]));
        }
    }
    for c in clauses {
        for row in crate::peek::wrap(&c, w.saturating_sub(4), 2) {
            text.push(Line::from(vec![Span::raw("   "), Span::styled(row, theme.dim2())]));
        }
    }
    let block_w = text.iter().map(|l| super::spans_width(&l.spans)).max().unwrap_or(0).min(w);
    // The mark stands while the conversation has not STARTED — the empty
    // seat's own face, one beat later, so the zone does not blink between the
    // press and the first prompt. A turn in flight is a conversation, and
    // there the live pulse is the focal point: a static mascot over it was
    // built, seen, and cut the same hour (it read as "nothing here" beside a
    // row saying something was happening). Never over a corpse, a sleeper, a
    // failure or a raised permission prompt either — the mascot has nothing
    // to say about any of those.
    let unspoken = rec.kind.is_agent()
        && rec.transcript_path.is_none()
        && matches!(
            rec.state,
            SessionState::Spawning | SessionState::Idle { stop_reason: StopReason::Unknown }
        );
    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    if unspoken {
        lines.extend(mascot_rows(app, area, block_w, text.len()));
    }
    lines.extend(text);
    lines
}

/// Why there is nothing to read, and what the session is doing instead. Pure
/// over the record, so the words can be read in a test without a frame. The
/// state word is the rail's own (`glyphs::state_word`) wherever nothing
/// better is true — this zone may not invent a second vocabulary for states
/// the card and the rail already name.
pub(super) fn quiet_words(rec: &mesimon_core::board::SessionRecord) -> (String, Vec<String>) {
    let word = glyphs::state_word(&rec.state).to_string();
    // A shell keeps no transcript at all, so "nothing to read" is never news
    // about a shell — what it means is that the pane has not been captured
    // yet, which lasts one poll (`App::poll_shell_tail`, a 1 s clock).
    if rec.kind == SessionKind::Bash {
        return if rec.state.has_pane() {
            (
                "reading its pane".into(),
                vec!["a shell keeps no transcript ∙ the pane is the whole record".into()],
            )
        } else {
            (word, vec!["its pane is gone, and the pane was the record".into()])
        };
    }
    match &rec.state {
        // The turn is in flight; `quiet_session` gives the row to the pulse.
        SessionState::Running => (
            String::new(),
            vec!["nothing said yet ∙ the first words land when the turn does".into()],
        ),
        // The question, whole. It is the reason the session is stopped and
        // the only thing worth the zone.
        SessionState::RequiresAction { .. } => (
            // The rail's own fallback for a question nothing recorded, not a
            // Debug-printed enum: one vocabulary, and this is not its home.
            rec.detail.clone().unwrap_or_else(|| word.to_lowercase()),
            vec!["it cannot go on until you answer ∙ its pane is where you do".into()],
        ),
        // Before the first turn: mesimon typed the ticket title into the box
        // on the way up and whether the Enter is ours or yours is the one
        // thing worth saying about it (`spawn_session`, `pending_submit`).
        SessionState::Spawning => ("starting up".into(), vec![box_clause(rec)]),
        // `Idle{Unknown}` is a session that has never been prompted — the
        // one Idle `SessionState::has_prompted` refuses, which is the same
        // predicate the `description unread` clause reads. An `EndTurn` with
        // no readable words is a finished turn, not a fresh box, and falls
        // through to the state word below.
        SessionState::Idle { stop_reason: StopReason::Unknown } => (
            if rec.pending_submit { "starting up".into() } else { "waiting for you".into() },
            vec![box_clause(rec)],
        ),
        // A conversation that was never written down: waking one resumes
        // nothing, so `resume_session` mints a fresh one under a new uuid —
        // which the reader should know BEFORE pressing, not after.
        SessionState::Sleeping if rec.kind == SessionKind::Codex => {
            (word, vec!["wake resumes this Codex conversation".into()])
        }
        SessionState::Sleeping if rec.transcript_path.is_none() => {
            (word, vec!["no conversation to resume ∙ waking it starts a fresh one".into()])
        }
        SessionState::Unknown { .. } => {
            (word, vec!["mesimon lost track of its state ∙ the next thing it does will say".into()])
        }
        _ => {
            let why = if rec.kind == SessionKind::Codex {
                "no reply preview available"
            } else if rec.transcript_path.is_none() {
                "it left no transcript"
            } else {
                "nothing to read in its transcript"
            };
            let mut clauses = vec![why.to_string()];
            if let Some(d) = rec.detail.clone() {
                clauses.push(d);
            }
            (word, clauses)
        }
    }
}

/// What is in a not-yet-prompted claude's input box, and whose Enter it is
/// waiting on. `pending_submit` is mesimon's own owed keypress (T-224's
/// retry clock); without it the first turn is the user's to start, which is
/// README promise 3 seen from the inside.
fn box_clause(rec: &mesimon_core::board::SessionRecord) -> String {
    if rec.pending_submit {
        "the ticket title is in its box ∙ mesimon presses enter when it is ready".into()
    } else {
        "the ticket title is in its box, unsent".into()
    }
}

/// What the `+ claude session` row would do, previewed (T-308). The zone
/// stood empty on that row — the one row on the page whose whole purpose is
/// a press nobody has made yet — so it now shows the mark, the press in the
/// keymap's own words, and the clauses that say what the session about to
/// exist will BE. Every fact is one the page already holds; nothing is asked
/// of the daemon to draw it.
fn empty_seat(app: &App, ticket: &mesimon_core::board::Ticket, area: Rect) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let w = area.width as usize;
    // The words first: they are what sizes the block, and the mark is hung
    // over their middle rather than over the zone's. The zone is far wider
    // than these sentences, so centring the art in IT would leave the
    // picture floating off to the right of everything it is about.
    let ctx = app.ctx();
    let press = keymap::binding_for(keymap::Scope::Ticket, keymap::Verb::Act, &ctx);
    let rows = seat_rows(app, ticket);
    let mut text: Vec<Line<'static>> = Vec::new();
    if let Some(b) = press {
        let mut spans = vec![Span::raw("   ")];
        spans.extend(chrome::hint_spans(&[b], &ctx, &theme.rest, w.saturating_sub(3)));
        text.push(Line::from(spans));
        text.push(Line::default());
    }
    for row in &rows {
        text.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(truncate(row, w.saturating_sub(4)), theme.dim2()),
        ]));
    }
    let block_w = text.iter().map(|l| super::spans_width(&l.spans)).max().unwrap_or(0).min(w);

    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    lines.extend(mascot_rows(app, area, block_w, text.len()));
    lines.extend(text);
    lines
}

/// The shin is centred over the words. Reserve the heading, leading blank,
/// one breathing row and ALL text before admitting the art. Mono gets the
/// wordmark; a short or narrow preview gives the space back to its sentences.
fn mascot_rows(app: &App, area: Rect, block_w: usize, text_h: usize) -> Vec<Line<'static>> {
    let unicode = app.theme.glyph_tier() == glyphs::Tier::Unicode;
    let art = if unicode { crate::mascot::COMPACT } else { "mesimon" };
    let art_h = art.lines().count();
    let art_w = art.lines().map(|r| r.width()).max().unwrap_or(0);
    if (area.height as usize) < 2 + art_h + 1 + text_h || (area.width as usize) < art_w + 6 {
        return Vec::new();
    }
    let pad = block_w.max(art_w).saturating_sub(art_w) / 2;
    if unicode {
        *app.mascot.borrow_mut() =
            Some(Rect::new(area.x + pad as u16, area.y + 2, art_w as u16, art_h as u16));
    }
    let mut lines: Vec<Line<'static>> = art
        .lines()
        .map(|row| {
            Line::from(vec![
                Span::raw(" ".repeat(pad)),
                Span::styled(row.to_string(), app.theme.dim1()),
            ])
        })
        .collect();
    lines.push(Line::default());
    lines
}

/// The sentences under the mark. The first says what the session will be, in
/// the state row's bullet-joined grammar; the second is mesimon's own
/// contract at the moment it is about to be kept — this road spawns with
/// `submit_prompt: false`, so the title is typed and the description stays
/// here, which is the whole difference from the composer's Shift+Enter; the
/// third stands only where the press would put a SECOND writer into a
/// checkout somebody is already working in, the hazard T-294 exists for and
/// the one line here that might change the answer.
fn seat_rows(app: &App, ticket: &mesimon_core::board::Ticket) -> Vec<String> {
    use mesimon_core::board::{AgentTools, ClaudeMode, WorkspaceStrategy};
    let mut clauses = vec![if ticket.workspace_strategy() == WorkspaceStrategy::Worktree
        || app.wt_item(ticket.id).is_some()
    {
        // The branch is on the state row already; naming it here would
        // be the only thing this clause could add, said twice.
        "in a worktree of its own".to_string()
    } else {
        "in the checkout".to_string()
    }];
    // What the ticket's column hands the session (T-117), and only where it
    // differs from what a spawn by hand would get: a column that changes
    // nothing has nothing to preview.
    let settings = app.board.column(&ticket.column).map(|c| &c.settings);
    if app.board.agent_provider == mesimon_core::board::AgentProvider::ClaudeCode {
        if let Some(m) = settings.map(|s| s.claude_mode).filter(|m| *m != ClaudeMode::Inherit) {
            clauses.push(format!("{} mode", m.word()));
        }
    } else if let Some(settings) = settings {
        if !settings.codex_sandbox.is_inherit() {
            clauses.push(format!("{} sandbox", settings.codex_sandbox.word()));
        }
        if !settings.codex_approval.is_inherit() {
            clauses.push(format!("approvals {}", settings.codex_approval.word()));
        }
    }
    let tools = settings.map(|s| s.agent_tools).unwrap_or_default();
    if !app.board.mcp_tools || tools == AgentTools::Off {
        clauses.push("no mesimon tools".to_string());
    } else if tools != AgentTools::Full {
        clauses.push(format!("{} tools", tools.word()));
    }
    let mut rows = vec![format!("starts {}", clauses.join(" ∙ "))];
    rows.push("types the ticket title into its box, and sends nothing".to_string());
    if app.checkout_busy(ticket.id) {
        rows.push("another agent is already writing in this checkout".to_string());
    }
    rows
}

/// Which document the preview zone is showing, for the scroll to belong to:
/// the session, and for an agent the reply itself (a shell's pane is one
/// continuous stream, so new output does not make it a new document).
fn doc_key(session: uuid::Uuid, reply: Option<&str>) -> u64 {
    crate::text::hash64((session, reply))
}

/// A note's document key: the note and its revision, with a discriminant
/// so it can never collide with a session's.
fn note_key(meta: &NoteMeta) -> u64 {
    crate::text::hash64((1u8, meta.id, meta.rev))
}

/// The zone's window onto `rows`: honours the offset `{ }` asked for
/// (`App::preview_scroll`, only if it was asked of THIS document), clamps
/// it the way the diff pane does and writes the clamp back, and records what
/// was shown (`App::preview_view`) so the next press and the footer know
/// the page size and whether there is a further one. A window that stops
/// short of the last row ends in the `~` cut mark. While a page turn is in
/// motion (`App::preview_glide`) the rows drawn are the glide's frame, on
/// the way to the offset recorded — the record is where the reader is
/// going, the glide is where the eye is.
/// The zone's markdown, rendered once per document, width and theme and
/// kept on `App::rich_cache`: `rich::render_all` parses and wraps the whole
/// reply, and the draw runs at 60 fps through a glide only to keep a
/// window of it. Keyed the way the page scroll is (`doc_key` / `note_key`).
fn rendered(app: &App, key: u64, width: usize, text: &str) -> Rc<Vec<Line<'static>>> {
    let flavor = app.theme.flavor;
    let mut slot = app.rich_cache.borrow_mut();
    if let Some(c) = slot.as_ref() {
        if c.key == key && c.width == width && c.flavor == flavor {
            return Rc::clone(&c.rows);
        }
    }
    let rows = Rc::new(crate::rich::render_all(text, width, &app.theme));
    *slot = Some(RichCache { key, width, flavor, rows: Rc::clone(&rows) });
    rows
}

fn window(
    app: &App,
    key: Option<u64>,
    rows: &[Line<'static>],
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
    // A glide on another document, or one that has landed, is retired here
    // so `App::animating` stops asking for fast frames the moment it can.
    let at = match app.preview_glide.get() {
        Some(g) if Some(g.key) == key && g.progress().is_some() => g.offset(offset).min(max),
        Some(_) => {
            app.preview_glide.set(None);
            offset
        }
        None => offset,
    };
    let mut shown: Vec<Line<'static>> = rows.iter().skip(at).take(budget).cloned().collect();
    if at + shown.len() < total {
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
    // `rail_rows` is sessions, then the offer, then the notes, and THAT is
    // what `rail_idx` counts — so every row this function paints has to
    // measure its own index the same way. `notes_start` is the one place the
    // offer's row is added to the offset; getting it from `rail.len()` alone
    // painted the first note and the offer together, and then nothing at all
    // one press down (dogfood, minutes after T-300 shipped).
    let offer = app.new_agent_row(ticket_id);
    let notes_start = rail.len() + usize::from(offer);

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
            SessionKind::Codex => "codex",
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

    // The offer to start the ticket's claude (T-300), under the sessions and
    // before the notes — a row, not a hint, so the gesture is the one every
    // other row already teaches: put the cursor on it and press Enter. It
    // replaced the pair of spawn hints an empty rail used to carry (`c start
    // claude ∙ s shell`), which asked a first-time reader to choose between
    // two words before either meant anything.
    if offer {
        let selected = rail.len() == rail_idx;
        let name = truncate(
            &format!("+ {} session", keymap::agent_word(app.board.agent_provider)),
            w.saturating_sub(2),
        );
        let style = if selected {
            Style::default().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.dim2()
        };
        let text = format!(" {name}");
        let pad = w.saturating_sub(text.width());
        let row_style = if selected { theme.selected_row() } else { Style::default() };
        lines.push(
            Line::from(vec![Span::styled(text, style), Span::raw(" ".repeat(pad))])
                .style(row_style),
        );
    }

    // What is left of the sessions' own keys: `c` only while a parked claude
    // is there to wake, `s` only where a ticket may still grow a shell
    // (T-300), `x` only on a selected row. All three can stand down, and
    // then the rail carries no trailer at all.
    lines.extend(trailer(&[keymap::Verb::Agent, keymap::Verb::Shell, keymap::Verb::Sleep]));

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
            let selected = notes_start + j == rail_idx;
            let who = author_word(&n.edited_by, app);
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
