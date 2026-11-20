//! The board frame: column geometry from `layout::board_geometry` (07 §2,
//! post-D33k arithmetic), expanded columns, 1-cell spines, the minted
//! cursor-column treatment, and vertical scroll with the D33k chevron badges.

use mesimon_core::board::{SessionRecord, SessionState, Ticket};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap;

use crate::app::{App, AskTarget, InputPurpose, Mode};
use crate::layout::{self, Slot};
use crate::text::{edit_window, EditBuffer};

use super::card::{self, CardCtx};

use mesimon_core::clock::now_ms;

pub(super) fn draw_columns(f: &mut Frame, area: Rect, app: &App) {
    let cols = app.columns();
    if cols.is_empty() {
        return;
    }
    let mut window = app.col_window.get();
    // A column pinned collapsed (T-117) is a spine unless the cursor or a
    // pending move's ghost (T-1133) is in it.
    let pinned = app.pinned_columns();
    let geom = layout::board_geometry(
        area.width,
        cols.len(),
        app.cursor_col,
        app.move_target(),
        pinned,
        &app.column_sizes(),
        &mut window,
    );
    app.col_window.set(window);

    for (ci, name) in cols.iter().enumerate() {
        match geom.slots[ci] {
            Slot::Expanded { x, width } => {
                let rect = Rect { x: area.x + x, y: area.y, width, height: area.height };
                // The wheel turns the column under it; its header row is
                // the column's own spot (T-716). The cards land on top.
                let mut hits = app.hits.borrow_mut();
                hits.record(rect, crate::mouse::Target::Lane(ci));
                hits.record(Rect { height: 1, ..rect }, crate::mouse::Target::Column(ci));
                drop(hits);
                app.spots.borrow_mut().columns.push((ci, rect.x, width));
                app.spots.borrow_mut().head_rows.push((name.clone(), Rect { height: 1, ..rect }));
                draw_column(f, rect, app, ci, name);
            }
            Slot::Spine { x } => {
                let rect = Rect { x: area.x + x, y: area.y, width: 1, height: area.height };
                // A folded column's title (its marks, count and letters)
                // is its header: the click that opens it, and all a hover
                // lights (T-1142). The strip under it is the wheel's lane.
                let title = draw_spine(f, rect, app, name);
                let mut hits = app.hits.borrow_mut();
                hits.record(rect, crate::mouse::Target::Lane(ci));
                hits.record(Rect { height: title, ..rect }, crate::mouse::Target::Column(ci));
                drop(hits);
                app.spots.borrow_mut().columns.push((ci, rect.x, 1));
                app.spots.borrow_mut().head_rows.push((name.clone(), Rect { height: 1, ..rect }));
            }
        }
    }
}

/// The card's reply, through the draw cache: the highest-precedence
/// session that has a transcript (bash never does), and a stale activity
/// row dropped on anything that is not running. Shared by the board's
/// cards and the search preview.
pub(super) fn card_peek(
    app: &App,
    sessions: &[&SessionRecord],
) -> Option<std::rc::Rc<crate::peek::Peek>> {
    // `min_by_key` keeps the first of equal keys, as the stable sort did.
    let s = sessions
        .iter()
        .filter(|s| crate::peek::preview_path(s).is_some())
        .min_by_key(|s| (mesimon_core::attention::rank(&s.state), s.id))?;
    let pk = app.peek_cache.displayed(s)?;
    // The activity row is a claim about NOW: a parked, finished or waiting
    // session's last tool call is history.
    Some(if s.state == SessionState::Running || pk.activity.is_none() {
        pk
    } else {
        std::rc::Rc::new(crate::peek::Peek { activity: None, ..(*pk).clone() })
    })
}

fn ticket_sessions(app: &App, ticket: ulid::Ulid) -> Vec<&SessionRecord> {
    app.board.sessions.iter().filter(|s| s.ticket == ticket && s.state.is_live()).collect()
}

/// One card's rows, in display order, and what the column needs to know
/// about them.
struct Group {
    next_frame: Option<std::time::Duration>,
    lines: Vec<Line<'static>>,
    /// The cursor card (any height): scroll keeps it visible.
    cursor: bool,
    waiting: bool,
    /// Line offset (within the group) and x-offset of a live edit cursor.
    /// The offset is not always 0: a rename edits the card's first line,
    /// a prompt edits a row appended under the whole card.
    edit_cursor: Option<(usize, u16)>,
    /// The refusal shake's offset this frame (T-423): the whole card is
    /// drawn this many cells off its place, bar and all. 0 at rest.
    shake: i16,
    /// The ticket drawn, for the bolts' spots (T-544); `None` for the
    /// composer's phantom card.
    id: Option<ulid::Ulid>,
    /// A moved card's trail where it stood (T-544), not the card.
    trail: bool,
    /// Where the pull-request brief sits (T-759): its row in the card
    /// (the second, or an open card's last, T-789), x offset and
    /// width, for its click.
    prs_at: Option<(usize, (u16, u16))>,
    /// A second tag, hung in the blank row under the card (T-1159).
    hang: Option<crate::tags::StripeEnd>,
}

/// Every card of column `ci`, drawn `width` cells wide, with the move's
/// ghost, the crown's trails and burns and the composer's phantom card among
/// them, and the column's count. `row` draws each card as a table row
/// (T-1240).
fn column_groups(app: &App, ci: usize, name: &str, width: u16, row: bool) -> (Vec<Group>, usize) {
    let theme = &app.theme;
    let tickets = app.board.column_tickets(name);
    let is_cursor_col = app.cursor_col == ci;
    let ghost = match &app.mode {
        Mode::Move { ticket, col, idx, .. } if *col == ci => Some((*ticket, *idx)),
        _ => None,
    };
    // While a move is pending, the card at its ORIGINAL spot stays visible
    // but semi-transparent — the trail under the blinking ghost.
    let moving: Option<ulid::Ulid> = match &app.mode {
        Mode::Move { ticket, .. } => Some(*ticket),
        _ => None,
    };

    // Rows: tickets minus the moving ghost, with the ghost re-inserted at its index.
    let rows: Vec<&Ticket> = match ghost {
        Some((gid, _)) => tickets.iter().filter(|t| t.id != gid).copied().collect(),
        None => tickets,
    };
    let count = rows.len() + ghost.map(|_| 1).unwrap_or(0);

    let ctx =
        CardCtx { theme, width, now_ms: now_ms(), spin: app.spin_frame(), names_key: true, row };

    // Marquee clock: reset when the cursor lands on a different ticket.
    let marquee_ms = |t: &Ticket| -> u64 {
        match app.marquee.get() {
            Some((id, epoch)) if id == t.id => epoch.elapsed().as_millis() as u64,
            _ => {
                app.marquee.set(Some((t.id, std::time::Instant::now())));
                0
            }
        }
    };

    // In-place title edit (rename on the card itself; create as a phantom
    // card at the column tail — where the daemon will append it).
    let editing: Option<(&InputPurpose, &EditBuffer)> = match &app.mode {
        Mode::Input { purpose, buffer } => Some((purpose, buffer)),
        _ => None,
    };
    let rename_of = |t: &Ticket| -> Option<&EditBuffer> {
        match editing {
            Some((InputPurpose::Rename { id }, buf)) if *id == t.id => Some(buf),
            _ => None,
        }
    };
    // The prompt field (Shift+Enter) hangs UNDER its card instead of taking
    // the title line the way a rename does: the ticket is not what is being
    // edited here, it is who the text is going to — so it has to stay whole
    // and stay on screen while the sentence is typed.
    type Prompted<'a> = (&'a EditBuffer, bool, bool, bool, bool, Option<&'a str>);
    let prompt_of = |t: &Ticket| -> Option<Prompted> {
        match editing {
            Some((
                InputPurpose::Prompt {
                    target: AskTarget::Ticket(ticket),
                    queued,
                    immediately,
                    accept_plan,
                    plan,
                    tier,
                    ..
                },
                buf,
            )) if *ticket == t.id => {
                Some((buf, *queued, *immediately, *accept_plan, *plan, tier.as_deref()))
            }
            _ => None,
        }
    };

    let mut groups: Vec<Group> = Vec::new();
    // `left`: this is the trail a card the crown moved out left behind
    // (T-544), drawn in the move trail's look and with no crown mark.
    // `glide`: a card some other mover took out (T-817) — the same ghost,
    // drawn as itself, since `glide::draw` fades it along the move.
    // The card the pointer opened (T-716).
    let hovered = app.hover_peek();
    let build_card = |t: &Ticket, selected: bool, held: bool, left: bool, glide: bool| {
        let sessions = ticket_sessions(app, t.id);
        let waiting = card::needs_you(t, &sessions);
        // The registry lives on the board, so colours resolve here rather
        // than inside the card, which never sees it.
        let painted = crate::tags::painted(&app.board, &t.tags);
        if let Some(buf) = rename_of(t).filter(|_| !left) {
            let (mut line, x_off) = card::render_edit(&ctx, buf, &painted);
            // A table row has no row under it for the stripe's end (T-1240).
            let hang = crate::tags::StripeEnd::new(ctx.theme, &painted, ctx.theme.selected_bg)
                .filter(|_| !row);
            if let Some(end) = &hang {
                end.foot(&mut line);
            }
            return Group {
                next_frame: None,
                lines: vec![line],
                cursor: true,
                waiting,
                edit_cursor: Some((0, x_off)),
                shake: 0,
                id: Some(t.id),
                trail: false,
                prs_at: None,
                hang,
            };
        }
        let trail = (left && !glide) || (!held && moving == Some(t.id));
        let mq = if selected { Some(marquee_ms(t)) } else { None };
        // Is this card open? The `p` preference, or a quick-tag digit still
        // inside its reveal — on the cursor card; `P` opens every card
        // (T-237). The card is open on this alone — the transcript below may
        // or may not exist, and the tag row does not depend on it.
        // A moving card's ghost (T-817) is drawn as the card looked, its
        // open rows, reply and pull requests included, and shrinks from
        // there; a crown trail is the card's title alone.
        let bare = left && !glide;
        // The card the cursor rode away leaves a ghost that is open as it
        // was under the cursor, its rows listed, without the cursor's look.
        let shown = selected || (glide && app.glide_was_cursor(t.id));
        // Under its transcript dialog the card stays as it was, open or
        // not, as under the note reader (T-1145).
        let open = !bare && (app.peek_all || (shown && app.peek_showing(t.id)));
        let peek_of = || card_peek(app, &sessions);
        // The pointer's card opens in place too, as `p` opens the cursor's
        // — and only with a reply to read: open, a card with none says
        // nothing it did not say closed.
        let pointed = (!open && !left && hovered == Some(t.id)).then(peek_of).flatten();
        let open = open || pointed.is_some();
        let peek = if pointed.is_some() {
            pointed
        } else if open {
            peek_of()
        } else {
            None
        };
        let wt = app.wt_item(t.id);
        // Has the agent said something the cursor has not been here for?
        // The done mark decays on it. Never for the cursor card or the move
        // ghost (both `cursorish`, and their ticket is the subject being
        // acked), so the frame between a cursor move and the ack agrees.
        let unseen = !(selected || held) && app.spoke_unseen(t.id);
        // What that agent said, for a resting card's new-reply row (T-720),
        // or what went between it and the crown since (T-783).
        let fresh = if !(selected || held || bare) { app.card_news(t.id) } else { None };
        // Its pull requests, for the line under the title (T-739).
        let brief = if bare { Vec::new() } else { app.pr_brief(t.id) };
        // And what the newest of them is at, beside its mark (T-794).
        let pr_words = if brief.is_empty() {
            None
        } else {
            card::pr_words(&app.card_prs(t.id), |p| app.pr_passed_news(p))
        };
        // A held ask's keys ride the cursor card (T-551) — the subject the
        // frame's `Ctx` was built for, so the keys it names act on THIS card.
        // The trust dialog's accept rides the same row (T-844).
        let keys_ctx = app.frame_ctx();
        let owed_keys = if selected
            && (app.ticket_held(t.id)
                || app.unsent_of(t.id).is_some()
                || app.ticket_trust_ready(t.id))
        {
            super::ask_keys(app, &keys_ctx)
        } else {
            Vec::new()
        };
        // The ticket's summary (T-696) and, on the cursor card, the key that
        // opens it, for the fold row.
        let summary = (app.prefs.summary != crate::prefs::SummaryShow::None)
            .then(|| app.ticket_summary(t.id))
            .flatten();
        let summary_keys: Vec<&mesimon_core::keymap::Binding> = if selected {
            mesimon_core::keymap::binding_for(
                app.scope(),
                mesimon_core::keymap::Verb::Summary,
                &keys_ctx,
            )
            .into_iter()
            .collect()
        } else {
            Vec::new()
        };
        let (owed_row, ship_row) = app.card_owed(t.id);
        let render = |open: bool, peek: Option<&crate::peek::Peek>| {
            card::render(
                &ctx,
                t,
                &sessions,
                app.terminal_busy(t.id),
                wt,
                shown,
                held,
                trail,
                mq,
                open,
                peek,
                &painted,
                app.doomed(t.id),
                app.archiving(t.id),
                unseen,
                app.snooze_row(t.id).as_deref(),
                app.owed(t.id),
                owed_row.as_deref(),
                ship_row.as_deref(),
                (!owed_keys.is_empty()).then_some((owed_keys.as_slice(), &*keys_ctx)),
                app.remote_initials(t.id).as_deref(),
                if left { crate::ui::CrownMark::None } else { app.crown_mark(t.id) },
                // A table row names the tier its ticket runs on, whichever
                // it is (T-1240); a card names a tier that is not the default.
                if row { Some(app.ticket_tier_name(t.id)) } else { app.card_tier_word(t.id) }
                    .as_deref(),
                app.card_corner(t.id),
                summary.as_ref(),
                (!summary_keys.is_empty()).then_some(summary_keys.as_slice()),
                app.summary_pulse(t.id),
                app.prefs.summary == crate::prefs::SummaryShow::Full,
                fresh.as_ref().map(|(words, ago, crown)| (words.as_str(), *ago, *crown)),
                &brief,
                pr_words.as_ref().map(|(w, running)| (w.as_str(), *running)),
                app.ticket_note_links(t.id),
                !left && app.moved_unseen(t.id),
                glide,
            )
        };
        let (mut lines, end) = render(open, peek.as_deref());
        // A card on the move gives up its rows where it was and takes them
        // back where it went (T-817): its ghost shrinks a row at a time from
        // the move's start, and the card where it goes is one row until it
        // has landed, then opens a row at a time. An open card's rows held
        // blank while the trail ran were a block in its column.
        let rows = if glide {
            Some(lines.len().saturating_sub(app.ghost_dropped(t.id)))
        } else if left {
            None
        } else {
            app.landing_rows(t.id)
        };
        if let Some(n) = rows {
            lines.truncate(n.max(1));
        }
        // The stripe ends on the card's last row drawn (T-1159).
        if let (Some(end), Some(last)) = (&end, lines.last_mut()) {
            end.foot(last);
        }
        let t_cells = usize::from(ctx.width).saturating_sub(crate::tags::BAR + 2);
        // Line 2, or an open card's pull-request row (T-789, T-829).
        let spans = card::brief_spans(ctx.theme, &brief, t_cells);
        // The words beside the marks (T-794) are the same click, where they
        // hold the line: a resting card's new reply does otherwise.
        let words = pr_words.as_ref().filter(|_| open || fresh.is_none()).map(|(w, _)| w.as_str());
        // Line 2 on a closed card; on an open one, the row over the tags
        // (T-829).
        let prs_at = (1..lines.len())
            .find_map(|row| brief_at(lines.get(row), &spans, words).map(|at| (row, at)));
        // The card is drawn WHOLE first — glyph, title, sessions, peek — and
        // the field is added under it. That order is the point: what you are
        // about to talk to stays legible while you type at it.
        let field = prompt_of(t).filter(|_| !left).map(
            |(buf, queued, immediately, accept_plan, plan, tier)| {
                // The prompt rows and the delivery row, shared with the
                // ticket page (T-476): the cursor is on its line's row.
                let (rows, x_off, y_off) = card::render_ask_field(
                    &ctx,
                    app,
                    t.id,
                    buf,
                    (queued, immediately),
                    accept_plan,
                    plan,
                    tier,
                );
                let at = lines.len();
                lines.extend(rows);
                (at, y_off, x_off)
            },
        );
        let edit_cursor = field.map(|(at, y_off, x_off)| (at + usize::from(y_off), x_off));
        // The second tag hangs in the row under the card: the field's first
        // row when one hangs there, else the blank row between cards.
        let hang = match (end, field) {
            (Some(end), Some((at, ..))) => {
                end.hang(&mut lines[at]);
                None
            }
            (end, _) => end,
        };
        Group {
            next_frame: None,
            lines,
            cursor: selected || held,
            waiting,
            edit_cursor,
            shake: if left { 0 } else { app.shake_dx(t.id) },
            id: Some(t.id),
            trail: left,
            prs_at: if left { None } else { prs_at },
            hang,
        }
    };
    let mut push_card = |t: &Ticket, selected, held, left, glide| {
        let (mut group, delay) =
            crate::frame_clock::collect(|| build_card(t, selected, held, left, glide));
        group.next_frame = delay;
        groups.push(group);
    };
    match ghost {
        Some((gid, gidx)) => {
            let ghost_ticket = app.board.ticket(gid);
            for (i, t) in rows.iter().enumerate() {
                if i == gidx {
                    if let Some(gt) = ghost_ticket {
                        push_card(gt, false, true, false, false);
                    }
                }
                push_card(t, false, false, false, false);
            }
            if gidx >= rows.len() {
                if let Some(gt) = ghost_ticket {
                    push_card(gt, false, true, false, false);
                }
            }
        }
        None => {
            // A card the crown just archived burns in its old place, and one
            // it moved out leaves a trail in its old place (T-544): drawn
            // among the others by the order they had here, never counted,
            // never the cursor's — `i` counts the column's own cards only.
            #[derive(PartialEq)]
            enum Place {
                Own,
                Burning,
                Trail,
                Glide,
            }
            let mut merged: Vec<(&Ticket, &str, Place)> =
                rows.iter().map(|t| (*t, t.order.as_str(), Place::Own)).collect();
            let ghosts = app
                .burning(name)
                .into_iter()
                .map(|b| (b, b.order.as_str(), Place::Burning))
                .chain(app.leaving(name).into_iter().map(|(t, o)| (t, o, Place::Trail)))
                // A card another mover took plays its move on the columns
                // alone (T-817): the table has no gutters to run it through.
                .chain(
                    app.gliding_out(name)
                        .into_iter()
                        .filter(|_| !row)
                        .map(|(t, o)| (t, o, Place::Glide)),
                );
            for (g, order, place) in ghosts {
                let at = merged.partition_point(|(t, o, _)| (*o, t.id) < (order, g.id));
                merged.insert(at, (g, order, place));
            }
            let mut i = 0;
            for (t, _, place) in merged {
                if place != Place::Own {
                    let glide = place == Place::Glide;
                    push_card(t, false, false, place == Place::Trail || glide, glide);
                    continue;
                }
                // A prompted card stays the cursor card. Every other text
                // field drops the selection (the composer's phantom card
                // becomes the cursor card instead), but this one is anchored
                // to a real ticket, and collapsing it mid-prompt would take
                // the agent's own state off screen while you type at it.
                // So does the card a dialog is about (T-817): it stays lit and
                // whole under its links, summary, pull requests or merge, so
                // the board still says which card the dialog came from.
                let selected = (is_cursor_col
                    && app.cursor_row == Some(i)
                    && (matches!(app.mode, Mode::Normal) || prompt_of(t).is_some()))
                    || app.dialog_card() == Some(t.id);
                push_card(t, selected, false, false, false);
                i += 1;
            }
        }
    }
    // New-ticket entry: a phantom card at the column tail, edited in place.
    // The second line is the M4 workspace selector (Shift+Tab cycles it).
    if is_cursor_col {
        if let Some((InputPurpose::Create { workspace, tags, plan, tier, .. }, buf)) = editing {
            // The tags picked with `^t` stripe the phantom card exactly as
            // they will stripe the real one — otherwise you are picking
            // blind until the ticket exists.
            let painted = crate::tags::painted(&app.board, tags);
            let (mut line, x_off) = card::render_edit(&ctx, buf, &painted);
            let tier_word = app.field_tier_word(None, tier.as_deref());
            let mut selector =
                card::render_workspace_selector(&ctx, *workspace, *plan, tier_word.as_deref());
            // Its second tag hangs under the stripe, in the selector's row.
            if let Some(end) =
                crate::tags::StripeEnd::new(theme, &painted, theme.selected_bg).filter(|_| !row)
            {
                end.foot(&mut line);
                end.hang(&mut selector);
            }
            groups.push(Group {
                next_frame: None,
                lines: vec![line, selector],
                cursor: true,
                waiting: false,
                edit_cursor: Some((0, x_off)),
                shake: 0,
                id: None,
                trail: false,
                prs_at: None,
                hang: None,
            });
        }
    }

    (groups, count)
}

/// A ticket's card as laid out in a view's flat lines, before the scroll:
/// what its spots and the mouse's targets are made from once the window is
/// known.
struct Placed {
    next_frame: Option<std::time::Duration>,
    start: usize,
    end: usize,
    id: ulid::Ulid,
    /// A moved card's trail where it stood (T-544), not the card.
    trail: bool,
    /// The pull-request brief's flat line, x offset and width (T-759).
    prs_at: Option<(usize, (u16, u16))>,
    /// On the holder, the crown mark's column.
    mark: Option<u16>,
}

/// Where each placed card landed on screen in the window `lo..hi` of flat
/// lines, `row_of` turning a shown line into its screen row: the dialog's
/// card (T-817), the mouse's card and brief targets (T-716, T-759) and the
/// crown's bolt spots (T-544). A card scrolled out of the window has none.
fn record_cards(
    app: &App,
    area: Rect,
    placed: &[Placed],
    (lo, hi): (usize, usize),
    row_of: impl Fn(usize) -> u16,
) {
    let shown = |line: usize| line >= lo && line < hi;
    let mut spots = app.spots.borrow_mut();
    let mut hits = app.hits.borrow_mut();
    for p in placed {
        let (top, bottom) = (p.start.max(lo), p.end.min(hi));
        // The card a dialog is about, whole on screen, is what the dialog
        // hangs from (T-817).
        if !p.trail && p.start >= lo && p.end <= hi && app.dialog_card() == Some(p.id) {
            spots.about = Some(Rect {
                x: area.x,
                y: row_of(p.start),
                width: area.width,
                height: (p.end - p.start) as u16,
            });
        }
        if let Some(delay) = p.next_frame.filter(|_| top < bottom) {
            crate::frame_clock::after(delay);
        }
        // The card's rows on screen are the mouse's to pick (T-716); a
        // trail is where the card was, not where it is.
        if !p.trail && top < bottom {
            let rect = Rect {
                x: area.x,
                y: row_of(top),
                width: area.width,
                height: (bottom - top) as u16,
            };
            hits.record(rect, crate::mouse::Target::Card(p.id));
            // The brief is the card's `^h` (T-759), over the card's own.
            if let Some((line, (x, w))) = p.prs_at.filter(|(line, _)| shown(*line)) {
                let rect = Rect { x: area.x + x, y: row_of(line), width: w, height: 1 };
                hits.record(rect.intersection(area), crate::mouse::Target::Prs(p.id));
            }
        }
        if shown(p.start) {
            let spot = crate::strike::CardSpot {
                id: p.id,
                x: area.x,
                y: row_of(p.start),
                width: area.width,
                rows: (p.end.min(hi) - p.start) as u16,
                mark: p.mark.map(|m| area.x + m),
            };
            if p.trail {
                spots.trails.push(spot);
            } else {
                spots.cards.push(spot);
            }
        }
    }
}

fn draw_column(f: &mut Frame, area: Rect, app: &App, ci: usize, name: &str) {
    let theme = &app.theme;
    let is_cursor_col = app.cursor_col == ci;
    let editing: Option<(&InputPurpose, &EditBuffer)> = match &app.mode {
        Mode::Input { purpose, buffer } => Some((purpose, buffer)),
        _ => None,
    };
    let ctx = CardCtx {
        theme,
        width: area.width,
        now_ms: now_ms(),
        spin: app.spin_frame(),
        names_key: true,
        row: false,
    };
    let (groups, count) = column_groups(app, ci, name, area.width, false);

    // Flatten with rhythm: one blank row between cards. Track the cursor
    // card's line range and each card's, for scroll + badges.
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut card_ranges: Vec<(usize, usize, bool)> = Vec::new(); // (start, end, waiting)
    let mut placed: Vec<Placed> = Vec::new();
    let crown_glyph = crate::glyphs::crown(theme.glyph_tier());
    let mut cursor_range: Option<(usize, usize)> = None;
    let mut edit_at: Option<(usize, u16)> = None; // (flat line idx, x offset)
                                                  // The card mid-shake, if any: its flat line range and its offset.
    let mut shaken: Option<(usize, usize, i16)> = None;
    // The blank rows a second tag hangs in (T-1159), by flat line.
    let mut hangs: Vec<usize> = Vec::new();
    for g in groups {
        let start = lines.len();
        lines.extend(g.lines);
        let end = lines.len();
        if g.shake != 0 {
            shaken = Some((start, end, g.shake));
        }
        // One blank row between cards (06 §5.5 — reinstated after dogfood:
        // contiguous one-line cards were hard to separate by eye). A second
        // tag hangs in its first cell (T-1159).
        let mut gap = Line::default();
        if let Some(end) = &g.hang {
            end.hang(&mut gap);
            hangs.push(lines.len());
        }
        lines.push(gap);
        if g.cursor {
            cursor_range = Some((start, end));
        }
        if let Some((off, x)) = g.edit_cursor {
            edit_at = Some((start + off, x));
        }
        card_ranges.push((start, end, g.waiting));
        if let Some(id) = g.id {
            let mark = if app.board.is_crowned(id) {
                lines.get(start).and_then(|l| cell_of(l, crown_glyph))
            } else {
                None
            };
            placed.push(Placed {
                next_frame: g.next_frame,
                start,
                end,
                id,
                trail: g.trail,
                prs_at: g.prs_at.map(|(row, at)| (start + row, at)),
                mark,
            });
        }
    }
    // No trailing blank after the last card, and nothing for scroll to
    // count; a second tag it wears still hangs under it (T-1159).
    let tail = lines.pop();

    // The column's ask field (T-378) hangs UNDER THE HEADER the way a
    // ticket's hangs under its card: the header is what names where the
    // words go, so it stays whole and the field takes rows from the body.
    let column_ask = column_ask_rows(app, &ctx, name, editing);
    // Header, blank — plus the field and its delivery row while it is open.
    let head_rows = 2 + column_ask.as_ref().map_or(0, |(rows, _)| rows.len());
    // Keep two blank rows inside the column above the board's footer gap.
    // Overflow cues also reserve a row plus a blank next to visible cards.
    let body_h = area.height.saturating_sub(2 + head_rows as u16) as usize;
    let total = lines.len();
    let mut scroll =
        if is_cursor_col { app.scroll_row.get().min(total.saturating_sub(1)) } else { 0 };
    // On the header the column shows its top, so `j` lands on a visible card
    // — including while the cursor has stepped one row further up, onto the
    // board's own top row (T-305), which is where `j` comes back from.
    let at_header = is_cursor_col
        && app.at_column_header()
        && !matches!(app.mode, Mode::Input { purpose: InputPurpose::Create { .. }, .. });
    // The cursor ITSELF is here — the bar cell — only while the top row does
    // not hold it. A focused header leaves the column its painted band, which
    // is what says where `j` returns to, and takes the bar with it.
    let col_header = at_header && !app.header_focus;
    if at_header {
        scroll = 0;
    }
    if is_cursor_col {
        if let Some(range) = cursor_range {
            let focus = edit_at.map(|(line, _)| line);
            scroll = scroll_into_view(scroll, total, body_h, range, focus, 2);
        }
        if total <= body_h {
            scroll = 0;
        }
        app.scroll_row.set(scroll);
    }

    // Show complete cards between explicit overflow cues. The cursor card
    // takes priority over a cue when it is taller than the available slice.
    let end_vis = (scroll + body_h).min(total);
    let not_cursor = |r: &&(usize, usize, bool)| Some((r.0, r.1)) != cursor_range;
    // Each cue costs two rows: its text and a blank separating it from cards.
    // Bottom: the first card that no longer fits after reserving the cue.
    let bottom: Option<(usize, usize, bool)> = if end_vis < total {
        card_ranges
            .iter()
            .find(|(_, end, _)| *end > end_vis.saturating_sub(2))
            .filter(not_cursor)
            .copied()
    } else {
        None
    };
    // Top: the last card still cut once the top rows are reserved.
    let top: Option<(usize, usize, bool)> = if scroll > 0 {
        card_ranges
            .iter()
            .rev()
            .find(|(start, _, _)| *start < scroll + 2)
            .filter(not_cursor)
            .copied()
    } else {
        None
    };
    // Content excludes hidden cards and their separators. Cues are their
    // own rows; never restyle a card or change its tag bar at the edge.
    let content_start = top.map(|(_, end, _)| (end + 1).max(scroll + 2)).unwrap_or(scroll);
    let content_end = bottom
        .map(|(start, _, _)| start.saturating_sub(1).min(end_vis.saturating_sub(2)))
        .unwrap_or(end_vis)
        .max(content_start);
    // Count everything actually omitted, including the boundary cards
    // removed to make room for a cue. Hidden attention stays in the header.
    let above = card_ranges.iter().filter(|(_, end, _)| *end <= content_start).count();
    let below = card_ranges.iter().filter(|(start, _, _)| *start >= content_end).count();
    let attn_out = card_ranges
        .iter()
        .filter(|(start, end, waiting)| *waiting && (*start < content_start || *end > content_end))
        .count();
    let overflow_line = |count: usize, up: bool| {
        Line::from(vec![
            Span::raw(" ".repeat(crate::tags::BAR + 1)),
            Span::styled(crate::glyphs::overflow_cue(theme.glyph_tier(), count, up), theme.dim2()),
        ])
    };

    // ---- header row (cursor-column treatment, reminted 2026-08-30) --------
    // The cursor column's header is a full-width painted band on the
    // `selected` surface — a 1-cell bar there read as another card. The name
    // still steps dim1 -> base. A value and a shape, never a hue. With the
    // cursor ON the header (T-117) the band's first cell is the cursor BAR,
    // the way a selected card's is: the same shape one row up, still no hue.
    let quiet = if is_cursor_col { Style::default().fg(theme.sel.dim2) } else { theme.dim2() };
    let header_edit: Option<&EditBuffer> = match editing {
        Some((InputPurpose::RenameColumn { name: n }, buf)) if n == name => Some(buf),
        _ => None,
    };
    // The first `d` on the header (T-775): the row flashes as a doomed card
    // does, the diff's deleted-line ground under an `err` name, on the same
    // 400 ms phase, until the second `d` or the stray key that cancels.
    let doomed =
        header_edit.is_none() && app.column_doomed(name) && theme.delete_lit(app.spin_frame());
    let mut head: Vec<Span<'static>> = Vec::new();
    if col_header || header_edit.is_some() {
        let (bar_ch, bar_style) = theme.bar(crate::theme::BarWeight::Cursor);
        head.push(Span::styled(bar_ch.to_string().repeat(crate::tags::BAR), bar_style));
        head.push(Span::raw(" "));
    } else {
        head.push(Span::raw(" ".repeat(crate::tags::BAR + 1)));
    }
    // The name, or its rename edited in place with the hardware cursor
    // (06 §5.7), the badges standing down for the field.
    let (name_span, name_cursor) =
        head_name(theme, name, is_cursor_col, doomed, header_edit, area.width as usize);
    head.push(name_span);
    let mut header_cursor_x: Option<u16> = name_cursor;
    let mut right: Vec<Span<'static>> = Vec::new();
    if header_edit.is_none() {
        if attn_out > 0 {
            right.push(Span::styled(format!("!{attn_out} "), theme.attn_text()));
        }
        // A tall cursor card may need the cue rows. Keep its hidden counts
        // in the header in that case, without sacrificing the input field.
        if above > 0 && top.is_none() {
            right.push(Span::styled(
                format!("{}{above} ", crate::glyphs::ahead_mark(theme.glyph_tier())),
                quiet,
            ));
        }
        if below > 0 && bottom.is_none() {
            right.push(Span::styled(
                format!("{}{below} ", crate::glyphs::behind_mark(theme.glyph_tier())),
                quiet,
            ));
        }
        right.push(Span::styled(format!("{count}"), quiet));
    }
    let used: usize = head.iter().chain(right.iter()).map(|s| s.content.width()).sum();
    let mut fill = (area.width as usize).saturating_sub(used + 1);
    // The column's one optional mark (T-117): it does something to a ticket
    // — a move on an edge, the train. After the count,
    // in the quiet register, and the first thing to go when the row is tight.
    let automated =
        header_edit.is_none() && app.board.column(name).is_some_and(|c| c.settings.automated());
    if automated && fill >= 3 {
        right.push(Span::styled(
            format!(" {}", crate::glyphs::auto_mark(theme.glyph_tier())),
            quiet,
        ));
        fill -= 2;
    }
    head.push(Span::raw(" ".repeat(fill)));
    head.extend(right);
    head.push(Span::raw(" "));
    let head_line = if doomed {
        Line::from(head).style(theme.delete_row())
    } else if is_cursor_col {
        Line::from(head).style(theme.selected_row())
    } else {
        Line::from(head)
    };

    let mut out: Vec<Line<'static>> = vec![head_line];
    let mut header_cursor_y: u16 = 0;
    if let Some((rows, x_off)) = column_ask {
        out.extend(rows);
        header_cursor_x = Some(x_off);
        header_cursor_y = 1;
    }
    out.push(Line::default());
    debug_assert_eq!(out.len(), head_rows);

    // ---- body -------------------------------------------------------------
    let mut shaken_rows: Option<(u16, Vec<Line<'static>>, i16)> = None;
    if lines.is_empty() {
        // Empty column: a legal cursor position (07 §16.2) — the hint shows
        // only under the cursor (author 2026-08-30).
        if is_cursor_col {
            let nudge =
                keymap::hint_for(keymap::Scope::Board, keymap::Verb::OpenTicket, &app.frame_ctx())
                    .map(|(show, hint)| format!("  {show}  {hint}"))
                    .unwrap_or_default();
            out.push(Line::from(Span::styled(nudge, theme.dim3())));
        }
    } else {
        if top.is_some() {
            out.push(overflow_line(above, true));
            out.push(Line::default());
        }
        out.extend(lines[content_start..content_end].iter().cloned());
        // The last card drawn keeps its second tag in the row under it, the
        // blank before a cue or the column's end included.
        if hangs.contains(&content_end) {
            out.extend(lines.get(content_end).or(tail.as_ref()).cloned());
        }
        // The shaken card's rows are left blank here and drawn after the
        // column, one cell off (T-423): a card that moves has to leave its
        // place, and the gutter either side of a column is the one cell it
        // moves into. Only the rows on screen; a cut card shakes its
        // visible part.
        if let Some((cs, ce, dx)) = shaken {
            let lo = cs.max(content_start);
            let hi = ce.min(content_end);
            if lo < hi {
                let at = head_rows + usize::from(top.is_some()) * 2 + (lo - content_start);
                let rows: Vec<Line<'static>> = out[at..at + (hi - lo)].to_vec();
                for l in &mut out[at..at + (hi - lo)] {
                    *l = Line::default();
                }
                shaken_rows = Some((at as u16, rows, dx));
            }
        }
        if bottom.is_some() {
            // Keep the cue at the viewport edge even when a tall hidden card
            // leaves unused rows after the final complete visible card.
            out.resize(head_rows + body_h.saturating_sub(1), Line::default());
            out.push(overflow_line(below, false));
        }
    }

    f.render_widget(Paragraph::new(out), area);
    if let Some((row, rows, dx)) = shaken_rows {
        // `LPAD`, `GUT` and `RPAD` are each a cell, so one cell either way is
        // always inside the frame; the clamp is for a geometry that is not.
        let x = (i32::from(area.x) + i32::from(dx)).clamp(0, i32::from(f.area().right()) - 1);
        let width = area.width.min(f.area().right().saturating_sub(x as u16));
        let rect = Rect { x: x as u16, y: area.y + row, width, height: rows.len() as u16 };
        f.render_widget(Paragraph::new(rows), rect);
    }

    // The hardware cursor sits in the edited title (06 §5.7: visible bar in
    // any text input — never a drawn glyph).
    let top_cue_rows = top.map(|_| 2usize).unwrap_or(0);
    let row_of =
        |line: usize| area.y + head_rows as u16 + (top_cue_rows + line - content_start) as u16;
    record_cards(app, area, &placed, (content_start, content_end), row_of);
    if let Some((line, x)) = edit_at {
        if line >= content_start && line < content_end {
            let at = (area.x + x.min(area.width.saturating_sub(1)), row_of(line));
            f.set_cursor_position(at);
            super::field_spot(app, area, at);
        }
    }
    // The cursor card, where it landed on screen — the composer's phantom
    // card or the ticket under the cursor: Tab's dialog grows out of this
    // rectangle. Only the whole card counts — a card cut by the window's
    // edge would put the dialog's origin off screen. The card's own
    // rectangle, bar cell included: the dialog's frame is the card's, so at
    // frame zero its bar and title stand in the card's cells.
    if let Some((cs, ce)) = cursor_range {
        let rect = (cs >= content_start && ce <= content_end).then(|| Rect {
            x: area.x,
            y: row_of(cs),
            width: area.width,
            height: (ce - cs) as u16,
        });
        app.cursor_card.set(rect);
    } else if at_header {
        // No card is the cursor card on a header: nothing may grow out of
        // last frame's rectangle.
        app.cursor_card.set(None);
    }
    if let Some(x) = header_cursor_x {
        let at = (area.x + x.min(area.width.saturating_sub(1)), area.y + header_cursor_y);
        f.set_cursor_position(at);
        super::field_spot(app, area, at);
    }
}

/// The board as a table (T-1240): every column a group, one under another,
/// its header a row and each of its cards a row as wide as the screen —
/// the title whole where the screen allows, its tags, agent and boxes named
/// beside it — with at most one line under it. One scroll for the whole
/// table keeps the cursor's row in view. A column folded on the board is its
/// header alone here until the cursor enters it, as a spine opens. Every key
/// is the board's: the cursor is the same column and row.
pub(super) fn draw_table(f: &mut Frame, area: Rect, app: &App) {
    let cols = app.columns();
    if cols.is_empty() {
        return;
    }
    let theme = &app.theme;
    let pinned = app.pinned_columns();
    let ctx = CardCtx {
        theme,
        width: area.width,
        now_ms: now_ms(),
        spin: app.spin_frame(),
        names_key: true,
        row: true,
    };
    let crown_glyph = crate::glyphs::crown(theme.glyph_tier());
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut heads: Vec<(usize, usize)> = Vec::new();
    let mut lanes: Vec<(usize, usize, usize)> = Vec::new();
    let mut placed: Vec<Placed> = Vec::new();
    let mut cursor_range: Option<(usize, usize)> = None;
    let mut card_range: Option<(usize, usize)> = None;
    let mut edit_at: Option<(usize, u16)> = None;
    for (ci, name) in cols.iter().enumerate() {
        if ci > 0 {
            lines.push(Line::default());
        }
        let lane = lines.len();
        let is_cursor_col = app.cursor_col == ci;
        let folded = pinned.get(ci).copied().unwrap_or(false)
            && !is_cursor_col
            && app.move_target() != Some(ci);
        let (groups, count) = if folded {
            (Vec::new(), app.board.column_tickets(name).len())
        } else {
            column_groups(app, ci, name, area.width, true)
        };
        // A folded group hides its cards, so it says when one needs you.
        let waiting = if folded {
            app.board
                .column_tickets(name)
                .iter()
                .filter(|t| card::needs_you(t, &ticket_sessions(app, t.id)))
                .count()
        } else {
            0
        };
        let (head, head_cursor) = table_head(app, &ctx, ci, name, count, waiting);
        let at = lines.len();
        heads.push((ci, at));
        if is_cursor_col && app.on_column_header() {
            cursor_range = Some((at, at + head.len()));
        }
        if let Some((row, x)) = head_cursor {
            edit_at = Some((at + row, x));
        }
        lines.extend(head);
        if groups.is_empty() && is_cursor_col && !folded {
            // An empty column's one position is its header; the hint shows
            // under the cursor, as on the board.
            let nudge =
                keymap::hint_for(keymap::Scope::Board, keymap::Verb::OpenTicket, &app.frame_ctx())
                    .map(|(show, hint)| format!("  {show}  {hint}"))
                    .unwrap_or_default();
            lines.push(Line::from(Span::styled(nudge, theme.dim3())));
        }
        for g in groups {
            let start = lines.len();
            lines.extend(g.lines);
            let end = lines.len();
            if g.cursor {
                cursor_range = Some((start, end));
                card_range = Some((start, end));
            }
            if let Some((off, x)) = g.edit_cursor {
                edit_at = Some((start + off, x));
            }
            if let Some(id) = g.id {
                let mark = if app.board.is_crowned(id) {
                    lines.get(start).and_then(|l| cell_of(l, crown_glyph))
                } else {
                    None
                };
                placed.push(Placed {
                    next_frame: g.next_frame,
                    start,
                    end,
                    id,
                    trail: g.trail,
                    prs_at: g.prs_at.map(|(row, at)| (start + row, at)),
                    mark,
                });
            }
        }
        lanes.push((ci, lane, lines.len()));
    }

    // One scroll: the cursor's rows in view with a row of margin, the field
    // being typed in where they are taller than the screen.
    let body_h = usize::from(area.height.saturating_sub(1)).max(1);
    let total = lines.len();
    let mut scroll = app.scroll_row.get().min(total.saturating_sub(1));
    if app.header_focus && app.cursor_col == 0 {
        scroll = 0;
    }
    if let Some(range) = cursor_range {
        let focus = edit_at.map(|(line, _)| line);
        scroll = scroll_into_view(scroll, total, body_h, range, focus, 1);
    }
    if total <= body_h {
        scroll = 0;
    }
    app.scroll_row.set(scroll);
    let end_vis = (scroll + body_h).min(total);
    f.render_widget(Paragraph::new(lines.drain(scroll..end_vis).collect::<Vec<_>>()), area);

    let row_of = |line: usize| area.y + (line - scroll) as u16;
    let shown = |line: usize| line >= scroll && line < end_vis;
    let mut spots = app.spots.borrow_mut();
    let mut hits = app.hits.borrow_mut();
    // The wheel turns the group under it; the header is its column's spot,
    // and the cards land on top (T-716).
    for &(ci, start, end) in &lanes {
        let (lo, hi) = (start.max(scroll), end.min(end_vis));
        if lo < hi {
            let rect =
                Rect { x: area.x, y: row_of(lo), width: area.width, height: (hi - lo) as u16 };
            hits.record(rect, crate::mouse::Target::Lane(ci));
        }
    }
    for &(ci, line) in &heads {
        if !shown(line) {
            continue;
        }
        let rect = Rect { x: area.x, y: row_of(line), width: area.width, height: 1 };
        hits.record(rect, crate::mouse::Target::Column(ci));
        spots.head_rows.push((cols[ci].clone(), rect));
    }
    drop((spots, hits));
    record_cards(app, area, &placed, (scroll, end_vis), row_of);
    if let Some((line, x)) = edit_at.filter(|(line, _)| shown(*line)) {
        let at = (area.x + x.min(area.width.saturating_sub(1)), row_of(line));
        f.set_cursor_position(at);
        super::field_spot(app, area, at);
    }
    // The cursor card, whole on screen, is what Tab's dialog grows out of.
    match card_range {
        Some((cs, ce)) => app.cursor_card.set((cs >= scroll && ce <= end_vis).then(|| Rect {
            x: area.x,
            y: row_of(cs),
            width: area.width,
            height: (ce - cs) as u16,
        })),
        None if app.at_column_header() => app.cursor_card.set(None),
        None => {}
    }
}

/// The scroll that keeps the cursor's rows `cs..ce` in a `body_h`-row view of
/// `total` rows, with `margin` rows either side of them where they fit. The
/// margins yield before they cut the cursor's rows; rows taller than the view
/// keep `focus`, the field being typed in, on its last row instead.
fn scroll_into_view(
    mut scroll: usize,
    total: usize,
    body_h: usize,
    (cs, ce): (usize, usize),
    focus: Option<usize>,
    margin: usize,
) -> usize {
    let lo = cs.saturating_sub(margin);
    let hi = (ce + margin).min(total);
    if hi > scroll + body_h {
        scroll = hi - body_h;
    }
    if lo < scroll {
        scroll = lo;
    }
    if ce - cs <= body_h {
        scroll.clamp(ce.saturating_sub(body_h), cs)
    } else {
        focus.unwrap_or(cs).saturating_sub(body_h.saturating_sub(1)).max(cs)
    }
}

/// A group's header in the table (T-1240): the column's name and count, the
/// cursor's bar on it when the cursor is, the column's automation mark, and
/// for a folded group how many of its hidden cards need you. The column's
/// rename edits it in place, its first `d` flashes it, and its ask field
/// hangs under it, as on the board. Returns its rows and, while a field is
/// open on it, the row and x of the field's cursor.
fn table_head(
    app: &App,
    ctx: &CardCtx,
    ci: usize,
    name: &str,
    count: usize,
    waiting: usize,
) -> (Vec<Line<'static>>, Option<(usize, u16)>) {
    let theme = &app.theme;
    let width = usize::from(ctx.width);
    let is_cursor_col = app.cursor_col == ci;
    let on_head = is_cursor_col
        && app.on_column_header()
        && !matches!(app.mode, Mode::Input { purpose: InputPurpose::Create { .. }, .. });
    let editing: Option<(&InputPurpose, &EditBuffer)> = match &app.mode {
        Mode::Input { purpose, buffer } => Some((purpose, buffer)),
        _ => None,
    };
    let header_edit: Option<&EditBuffer> = match editing {
        Some((InputPurpose::RenameColumn { name: n }, buf)) if n == name => Some(buf),
        _ => None,
    };
    let doomed =
        header_edit.is_none() && app.column_doomed(name) && theme.delete_lit(app.spin_frame());
    let quiet = if is_cursor_col { Style::default().fg(theme.sel.dim2) } else { theme.dim2() };
    let mut head: Vec<Span<'static>> = Vec::new();
    if on_head || header_edit.is_some() {
        // The cursor's bar in the main tag's shape, as the rows wear it.
        let (bar_ch, bar_style) = theme.bar(crate::theme::BarWeight::Cursor);
        head.push(crate::tags::thin_bar(Span::styled(
            bar_ch.to_string().repeat(crate::tags::BAR),
            bar_style,
        )));
        head.push(Span::raw(" "));
    } else {
        head.push(Span::raw(" ".repeat(crate::tags::BAR + 1)));
    }
    let (name_span, name_cursor) =
        head_name(theme, name, is_cursor_col, doomed, header_edit, width);
    head.push(name_span);
    let mut cursor: Option<(usize, u16)> = name_cursor.map(|x| (0, x));
    if header_edit.is_none() {
        head.push(Span::styled(format!("  {count}"), quiet));
        if waiting > 0 {
            head.push(Span::styled(format!("  !{waiting}"), theme.attn_text()));
        }
        if app.board.column(name).is_some_and(|c| c.settings.automated()) {
            head.push(Span::styled(
                format!("  {}", crate::glyphs::auto_mark(theme.glyph_tier())),
                quiet,
            ));
        }
    }
    let used = super::spans_width(&head);
    head.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    let head_line = if doomed {
        Line::from(head).style(theme.delete_row())
    } else if is_cursor_col {
        Line::from(head).style(theme.selected_row())
    } else {
        Line::from(head)
    };
    let mut out = vec![head_line];
    // The column's ask field (T-378) hangs under its header, as on the board.
    if let Some((rows, x_off)) = column_ask_rows(app, ctx, name, editing) {
        out.extend(rows);
        cursor = Some((1, x_off));
    }
    (out, cursor)
}

/// A column's ask field (T-378) and, where its asks can queue, the delivery
/// row under it: the rows that hang under the column's header, and the x of
/// the field's cursor. `None` while no ask is open on `name`.
fn column_ask_rows(
    app: &App,
    ctx: &CardCtx,
    name: &str,
    editing: Option<(&InputPurpose, &EditBuffer)>,
) -> Option<(Vec<Line<'static>>, u16)> {
    let (InputPurpose::Prompt { target: AskTarget::Column(n), queued, accept_plan, .. }, buf) =
        editing?
    else {
        return None;
    };
    if n != name {
        return None;
    }
    // The empty field says what it is for in the key's own words — or,
    // where the column holds a seat a blank Enter would fill, what that
    // Enter does (T-405), the plural of the card's `start on the title`
    // and by the same rule `commit_input` judges it. At `accept plan`
    // (T-429) a blank Enter accepts every plan in the column.
    let placeholder = if *accept_plan {
        "enter accepts the plans"
    } else if app.column_starts(name) {
        "start on the titles"
    } else {
        "ask every agent"
    };
    let (line, x_off) = card::render_prompt(ctx, buf, placeholder);
    let mut rows = vec![line];
    if app.column_ask_queueable(name) {
        rows.push(card::render_ask_mode(
            ctx,
            crate::app::App::ask_mode_word(*accept_plan, *queued, false, false),
            true,
            None,
            // A column's ask has no `immediately` stop (T-601).
            false,
            crate::app::App::shell_line(buf.as_str()),
        ));
    }
    Some((rows, x_off))
}

/// A column header's name in its ink (doomed, the cursor column's, quiet), or
/// its rename edited in place of it: the span, and while the name is edited
/// the x of the field's cursor in a row `width` cells wide.
fn head_name(
    theme: &crate::theme::Theme,
    name: &str,
    is_cursor_col: bool,
    doomed: bool,
    header_edit: Option<&EditBuffer>,
    width: usize,
) -> (Span<'static>, Option<u16>) {
    let bold = |style: Style| style.add_modifier(Modifier::BOLD);
    let on_cursor = Style::default().fg(theme.sel.base);
    if let Some(buf) = header_edit {
        let budget = width.saturating_sub(crate::tags::BAR + 2);
        let (shown, cx) = edit_window(buf.as_str(), buf.width_before_cursor(), budget);
        return (Span::styled(shown, bold(on_cursor)), Some(crate::tags::BAR as u16 + 1 + cx));
    }
    let style = if doomed {
        theme.err_text()
    } else if is_cursor_col {
        on_cursor
    } else {
        theme.dim1()
    };
    (Span::styled(name.to_uppercase(), bold(style)), None)
}

/// One phase of a linked card's flash, in spinner frames: the MOVE
/// ghost's 400 ms (`Theme::move_blink`).
const LINK_FLASH_FRAMES: usize = 4;

/// The board behind a card's dialog (T-817): every cell of the columns but
/// their header row and the card the dialog is about recedes toward the
/// ground, so that card is the one the board still shows; behind the
/// composer's dialog (T-1144) it is the card its ticket's note or prompt
/// came from, or none. Drawn after the columns and the bolts, before the
/// dialogs.
pub(super) fn recede(f: &mut Frame, app: &App, area: Rect) {
    // The composer's dialog recedes the board too (T-1144), with no card
    // of its own to keep: a new ticket's, or a column's prompt.
    let composing = matches!(app.mode, Mode::Editor(_));
    if app.dialog_card().is_none() && !composing {
        return;
    }
    let keep = app.dialog_anchor();
    // The card a links row names (T-820) blinks out of the recess on the
    // MOVE ghost's cadence: lit, then receded with the rest, while hovered.
    let flash = app
        .link_hovered()
        .and_then(|id| {
            app.spots.borrow().card(id).map(|c| Rect {
                x: c.x,
                y: c.y,
                width: c.width,
                height: c.rows,
            })
        })
        .filter(|_| crate::glyphs::lit(app.spin_frame(), LINK_FLASH_FRAMES));
    let theme = &app.theme;
    let buf = f.buffer_mut();
    for y in area.y.saturating_add(1)..area.bottom() {
        for x in area.x..area.right() {
            let at = ratatui::layout::Position { x, y };
            if keep.is_some_and(|r| r.contains(at)) || flash.is_some_and(|r| r.contains(at)) {
                continue;
            }
            let cell = &mut buf[(x, y)];
            cell.fg = theme.recede(cell.fg);
            if cell.bg != ratatui::style::Color::Reset && Some(cell.bg) != theme.bg {
                cell.bg = theme.recede_ground(cell.bg);
            }
        }
    }
}

/// Where `brief` (`card::brief_spans`) stands on `line`, the card's second
/// line, as its x offset and width — `None` where the line does not end in
/// it and its one-cell margin (a trail, the delete flash, no PRs). Where
/// `words` (`card::pr_words`) stand before it, cut or whole, the span
/// starts at them (T-794).
fn brief_at(
    line: Option<&Line<'_>>,
    brief: &[Span<'_>],
    words: Option<&str>,
) -> Option<(u16, u16)> {
    let spans = &line?.spans;
    let n = brief.len();
    if n == 0 || spans.len() < n + 1 {
        return None;
    }
    let at = spans.len() - 1 - n;
    if !spans[at..at + n].iter().zip(brief).all(|(a, b)| a.content == b.content) {
        return None;
    }
    let x = u16::try_from(super::spans_width(&spans[..at])).ok()?;
    let w = u16::try_from(super::spans_width(brief)).ok()?;
    let Some(words) = words else { return Some((x, w)) };
    // The words end where the padding before the marks begins.
    let before: String = spans[..at.saturating_sub(1)].iter().map(|s| s.content.as_ref()).collect();
    let start = before.char_indices().map(|(i, _)| i).find(|&i| {
        let tail = &before[i..];
        let cut = tail.strip_suffix('~').unwrap_or(tail);
        before[..i].ends_with(' ')
            && !cut.is_empty()
            && !cut.starts_with(' ')
            && words.starts_with(cut)
    });
    match start.and_then(|i| u16::try_from(before[..i].width()).ok()) {
        Some(wx) if wx < x => Some((wx, x + w - wx)),
        _ => Some((x, w)),
    }
}

/// The column of the first `glyph` on `line`, in cells from its start.
fn cell_of(line: &Line<'_>, glyph: &str) -> Option<u16> {
    let mut at = 0usize;
    for span in &line.spans {
        if let Some(i) = span.content.find(glyph) {
            return u16::try_from(at + span.content[..i].width()).ok();
        }
        at += span.content.width();
    }
    None
}

/// A collapsed column (07 §3.1): 1 cell, reads vertically, codepoints
/// ⊆ [!A-Z0-9⁺ space]. The TOP carries what an expanded column's header
/// row carries — the `!` iff the column holds a waiting session, then the
/// count, ONE digit — and the name runs down from under them, one letter
/// per row, truncating when the column is short.
///
/// The count is one cell, never two (T-359): ten or more reads `9` with a
/// `⁺` in the row under it, the row a smaller count leaves blank, so the
/// name starts on the same row whatever the column holds. Stacking the
/// digits (`1` over `0`) read as two counts and pushed the name down.
///
/// The count sat at the FOOT until T-302 (2026-09-07, user: "bottom too
/// far"), which put a 1-cell column's only number twenty rows away from
/// every other count on the board. With nothing waiting its first digit
/// lands on row 0, the very row the expanded columns write their own count
/// on; a `!` claims that cell and pushes it one row down, because the mark
/// is what the folded column is standing in for.
///
/// The `!` is INVERTED — `theme.attn_row()`, the needs-you title row's own
/// treatment, bold like the header's `!N` chip (T-271, 2026-09-06, user:
/// "more aggressiveness"). One cell of attn-coloured glyph is the smallest
/// mark the board can make and it was reading as quiet next to the folded
/// column it stands for; painting the cell spends the same one colour on
/// the whole cell instead of on a stroke.
/// Draws a folded column's spine; returns the rows its title takes.
fn draw_spine(f: &mut Frame, area: Rect, app: &App, name: &str) -> u16 {
    let theme = &app.theme;
    let tickets = app.board.column_tickets(name);
    let waiting = tickets.iter().any(|t| {
        let sessions = ticket_sessions(app, t.id);
        card::needs_you(t, &sessions)
    });

    let h = area.height as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    // Row 0 aligns with the column headers.
    if waiting {
        lines.push(Line::from(Span::styled("!", theme.attn_row().add_modifier(Modifier::BOLD))));
    }
    let (digit, more) = spine_count(tickets.len());
    lines.push(Line::from(Span::styled(digit.to_string(), theme.dim2())));
    // The row under the count is blank under headers; `⁺` when it overflows.
    lines.push(Line::from(Span::styled(more.to_string(), theme.dim2())));

    let name_rows = h.saturating_sub(lines.len());
    let upper = name.to_uppercase();
    let letters = upper.chars().filter(|c| c.is_ascii_alphanumeric()).take(name_rows);
    for c in letters {
        lines.push(Line::from(Span::styled(c.to_string(), theme.dim1())));
    }

    let title = (lines.len() as u16).min(area.height);
    f.render_widget(Paragraph::new(lines), area);
    title
}

/// The spine's count block: the digit, and the cell under it — a space, or
/// `⁺` when the column holds more than the one digit can say.
fn spine_count(n: usize) -> (char, char) {
    match n {
        0..=9 => (char::from(b'0' + n as u8), ' '),
        _ => ('9', '⁺'),
    }
}
