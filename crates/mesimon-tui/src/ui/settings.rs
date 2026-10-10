//! The Settings dialog (T-717): one large frame, the sections down its left
//! and the page of the one under the cursor beside them, so nothing is a
//! submenu of a submenu. A page is a line a setting — its name, then its
//! value — and the selected row's one line of detail at the foot, which is
//! the whole of what a row says until it is chosen. Theme's page is the
//! picker (`themes`); four others open on a preview of what their rows do.
//!
//! On a screen too narrow for both, the dialog shows one at a time: the
//! section list, or the page Enter stepped into.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::{self, MenuItem, SettingsSection, Value};

use crate::app::{App, Mode};
use crate::glyphs::Tier;
use crate::text::truncate;

use super::dialog;

/// The page's inner size the dialog asks for, terminal permitting.
const WIDTH: u16 = 106;
const ROWS: u16 = 24;
/// The section list's column, its gap to the page, and the narrowest
/// inner width that still holds both: a page needs room for a name and a
/// value side by side.
const NAV_W: u16 = 18;
const NAV_GAP: u16 = 3;
const WIDE: u16 = 84;
/// The longest name the value column waits for; a longer one is cut.
const NAME_W: usize = 28;

/// `uni` where the theme draws unicode, else `ascii` (the mono profile).
fn glyph(app: &App, uni: &'static str, ascii: &'static str) -> &'static str {
    if app.theme.glyph_tier() == Tier::Ascii {
        ascii
    } else {
        uni
    }
}

pub(super) fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = dialog::centred(f.area(), ROWS, WIDTH);
    let inner_w = area.width.saturating_sub(2);
    let wide = inner_w >= WIDE;
    let on_page = !matches!(app.mode, Mode::Sections);
    // The title names the scope (T-361) and, where the page stands alone,
    // the section it is.
    let mut name = "SETTINGS".to_string();
    if !wide && on_page {
        name.push_str(&format!(" ∙ {}", app.settings_section.name().to_uppercase()));
    }
    if app.settings_board_scope {
        name.push_str(" ∙ THIS BOARD");
    }
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        dialog::Edges {
            title: dialog::title(&theme.rest, name),
            tail: dialog::keys(
                app,
                app.scope(),
                &theme.rest,
                usize::from(inner_w).saturating_sub(4),
            ),
        },
    );
    // The section list stands on a ground of its own, the whole height of
    // the frame: painted, never drawn (L1). Halfway from the page to the
    // cursor's surface where the profile has a halfway; else one painted
    // column of the cursor's surface between the list and the page.
    if wide {
        let ground = |f: &mut Frame, r: Rect, c: ratatui::style::Color| {
            f.buffer_mut().set_style(r, Style::default().bg(c));
        };
        match (theme.hover_bg(), theme.selected_bg) {
            (Some(panel), _) => ground(f, Rect { width: NAV_W + 1, ..inner }, panel),
            (None, Some(rule)) => {
                ground(f, Rect { x: inner.x + NAV_W + 1, width: 1, ..inner }, rule)
            }
            (None, None) => {}
        }
    }
    // A row of air under the title where the height allows, for both
    // halves alike, so the list and the page start on one line.
    let inner = if inner.height >= 16 {
        Rect { y: inner.y + 1, height: inner.height - 1, ..inner }
    } else {
        inner
    };
    if wide {
        let nav = Rect { width: NAV_W, ..inner };
        let page = Rect {
            x: inner.x + NAV_W + NAV_GAP,
            width: inner.width.saturating_sub(NAV_W + NAV_GAP + 1),
            ..inner
        };
        draw_nav(f, app, nav);
        draw_page(f, app, page);
    } else if on_page {
        draw_page(f, app, Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner });
    } else {
        draw_nav(f, app, inner);
    }
}

/// The section list. The section under the cursor is lit while the list
/// has the keys, and keeps its mark while its page has them, so the page
/// always says whose it is.
fn draw_nav(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let focus = matches!(app.mode, Mode::Sections);
    let all = SettingsSection::ALL;
    // A line between sections where the dialog has the height for it.
    let spaced = usize::from(area.height) >= all.len() * 2;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut hits = app.hits.borrow_mut();
    for (i, section) in all.iter().enumerate() {
        if spaced && i > 0 {
            lines.push(Line::default());
        }
        let y = area.y + lines.len() as u16;
        if y >= area.bottom() {
            break;
        }
        hits.record(Rect { y, height: 1, ..area }, crate::mouse::Target::Section(i));
        let name = truncate(section.name(), usize::from(area.width).saturating_sub(3));
        let pad = usize::from(area.width).saturating_sub(2 + name.width());
        let line = if *section != app.settings_section {
            Line::from(vec![Span::raw("  "), Span::styled(name, theme.dim2())])
        } else if focus {
            let lit = theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD);
            Line::from(vec![
                Span::styled(glyph(app, "▎ ", "> "), lit),
                Span::styled(name, lit),
                Span::styled(" ".repeat(pad), lit),
            ])
        } else {
            Line::from(vec![
                Span::styled(glyph(app, "▎ ", "> "), theme.dim2()),
                Span::styled(name, theme.base().add_modifier(Modifier::BOLD)),
            ])
        };
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// The page of the section under the list's cursor: its preview where it
/// has one and the height allows, its rows under their headings, and the
/// selected row's detail on the last line.
fn draw_page(f: &mut Frame, app: &App, area: Rect) {
    if app.settings_section == SettingsSection::Theme {
        super::themes::draw_page(f, app, area);
        return;
    }
    let theme = &app.theme;
    let ctx = app.frame_ctx();
    let items = keymap::settings_items(&ctx);
    if items.is_empty() || area.height < 3 {
        return;
    }
    let cursor = match app.mode {
        Mode::Settings { idx } => Some(idx.min(items.len() - 1)),
        _ => None,
    };
    let width = usize::from(area.width);
    let body_h = usize::from(area.height.saturating_sub(2));
    // The name column, and a column per option across every row, so the
    // page's values line up as a table does.
    let name_w = items.iter().map(|m| (m.label)(&ctx).width()).max().unwrap_or(0).min(NAME_W);
    let cols = option_columns(items.iter().filter_map(|m| keymap::value(m.verb, &ctx)));
    // Every row and heading as a line; `at` is each row's line.
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut at: Vec<usize> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(heading) = keymap::settings_heading(item.verb) {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(Span::styled(heading.to_string(), theme.dim3())));
        }
        at.push(lines.len());
        lines.push(row_line(app, &ctx, item, name_w, &cols, width, cursor == Some(i)));
    }
    // The preview goes first, and only where every row still fits under it.
    let preview = preview(app, width).filter(|p| usize::from(p.height) + 1 + lines.len() <= body_h);
    let top = preview.as_ref().map_or(0, |p| usize::from(p.height) + 1);
    let room = body_h - top;
    // A window over the rows that keeps the cursor's row in it.
    let first = match cursor {
        Some(c) if lines.len() > room => at[c].saturating_sub(room / 2).min(lines.len() - room),
        _ => 0,
    };
    let mut hits = app.hits.borrow_mut();
    for (i, line) in at.iter().enumerate() {
        if *line >= first && *line < first + room {
            let y = area.y + (top + line - first) as u16;
            hits.record(Rect { y, height: 1, ..area }, crate::mouse::Target::Row(i));
        }
    }
    drop(hits);
    if let Some(p) = preview {
        (p.draw)(f, Rect { height: p.height, ..area });
    }
    let rows = Rect { y: area.y + top as u16, height: room as u16, ..area };
    f.render_widget(
        Paragraph::new(lines.into_iter().skip(first).take(room).collect::<Vec<_>>()),
        rows,
    );
    // The selected row's one line, at the foot, revealed when it is long.
    if let Some(c) = cursor {
        let detail = keymap::item_detail(items[c], &ctx);
        let body = dialog::reveal(app, &detail, width);
        let foot = Rect { y: area.bottom() - 1, height: 1, ..area };
        f.render_widget(Paragraph::new(Span::styled(body, theme.dim2())), foot);
    }
}

/// The cells each option column takes on a page: a switch is its first
/// column, a choice's options are its columns in order, each wide enough
/// for its widest mark and word.
fn option_columns(values: impl Iterator<Item = Value>) -> Vec<usize> {
    let mut cols: Vec<usize> = Vec::new();
    let mut take = |i: usize, w: usize| {
        if cols.len() <= i {
            cols.resize(i + 1, 0);
        }
        cols[i] = cols[i].max(w);
    };
    for v in values {
        match v {
            Value::Switch(_) => take(0, "○ off".width()),
            Value::Choice { options, .. } => {
                for (i, o) in options.iter().enumerate() {
                    take(i, o.width() + 2);
                }
            }
            Value::Word(_) | Value::Door(_) => {}
        }
    }
    cols
}

/// One row: its name, its value at the value column, and in board scope
/// where the value comes from, at the right edge (T-361).
fn row_line(
    app: &App,
    ctx: &keymap::Ctx,
    item: &MenuItem,
    name_w: usize,
    cols: &[usize],
    width: usize,
    selected: bool,
) -> Line<'static> {
    let theme = &app.theme;
    let ramp = if selected { &theme.sel } else { &theme.rest };
    let ground = if selected { theme.selected_row() } else { Style::default() };
    let name = truncate(&(item.label)(ctx), name_w);
    let name_style = if selected {
        Style::default().fg(ramp.base).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ramp.dim1)
    };
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(name.clone(), name_style),
        Span::raw(" ".repeat(name_w.saturating_sub(name.width()) + 3)),
    ];
    let tag = keymap::scope_word(item, ctx);
    let tag_w = tag.map_or(0, |t| t.width() + 2);
    let room = width.saturating_sub(1 + name_w + 3 + tag_w);
    let on = Style::default().fg(ramp.base).add_modifier(Modifier::BOLD);
    let off = Style::default().fg(ramp.dim2);
    if let Some(value) = keymap::value(item.verb, ctx) {
        spans.extend(value_spans(app, &value, cols, room, on, off));
    }
    let used: usize = super::spans_width(&spans);
    if let Some(t) = tag {
        let ink = if t == "set here" { ramp.dim1 } else { ramp.dim3 };
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used + t.width() + 1))));
        spans.push(Span::styled(t.to_string(), Style::default().fg(ink)));
        spans.push(Span::raw(" "));
    } else {
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    }
    Line::from(spans).style(ground)
}

/// Columns between two options.
const OPTION_GAP: usize = 3;

/// A value in `room` cells: a switch's word with its mark, every option of
/// a choice with the chosen one marked, each in its page column (or the
/// chosen one alone where they do not all fit), a word, or a door's word
/// and its `›`.
fn value_spans(
    app: &App,
    value: &Value,
    cols: &[usize],
    room: usize,
    on: Style,
    off: Style,
) -> Vec<Span<'static>> {
    let tier = app.theme.glyph_tier();
    let (set, unset) = if tier == Tier::Ascii { ("*", "-") } else { ("●", "○") };
    let fit = |s: &str| truncate(s, room);
    match value {
        Value::Switch(true) => vec![Span::styled(fit(&format!("{set} on")), on)],
        Value::Switch(false) => vec![Span::styled(fit(&format!("{unset} off")), off)],
        Value::Choice { options, at } => {
            let col = |i: usize| cols.get(i).copied().unwrap_or(0);
            let all: usize = (0..options.len()).map(col).sum::<usize>()
                + OPTION_GAP * options.len().saturating_sub(1);
            if all > room {
                let chosen = options.get(*at).copied().unwrap_or_default();
                return vec![Span::styled(fit(chosen), on)];
            }
            let mut spans = Vec::new();
            for (i, o) in options.iter().enumerate() {
                let word = if i == *at { format!("{set} {o}") } else { format!("{unset} {o}") };
                let pad = if i + 1 < options.len() {
                    col(i).saturating_sub(word.width()) + OPTION_GAP
                } else {
                    0
                };
                spans.push(Span::styled(word, if i == *at { on } else { off }));
                spans.push(Span::raw(" ".repeat(pad)));
            }
            spans
        }
        Value::Word(w) => vec![Span::styled(fit(w), on)],
        Value::Door(w) => {
            let crumb = crate::glyphs::crumb(tier);
            let w = truncate(w, room.saturating_sub(2));
            vec![
                Span::styled(w, on.remove_modifier(Modifier::BOLD)),
                Span::styled(format!(" {crumb}"), off),
            ]
        }
    }
}

/// What draws a preview into the rows it was given.
type PaintFn<'a> = Box<dyn Fn(&mut Frame, Rect) + 'a>;

/// A section's preview: how many rows it takes, and what draws it there.
struct Preview<'a> {
    height: u16,
    draw: PaintFn<'a>,
}

/// What the section's rows do, drawn at the top of its page: two cards and
/// an agent's pane (Appearance), the banner (Notifications), the tab
/// (Terminal), the quota itself (Usage). `None` for a section with nothing
/// to picture.
fn preview(app: &App, width: usize) -> Option<Preview<'_>> {
    match app.settings_section {
        SettingsSection::Appearance => Some(appearance(app, width)),
        SettingsSection::Notifications => Some(lines_preview(banner(app, width))),
        SettingsSection::Terminal => tab(app, width),
        SettingsSection::Usage => Some(lines_preview(super::usage::readout(app, width))),
        _ => None,
    }
}

/// A preview that is lines of text.
fn lines_preview(lines: Vec<Line<'static>>) -> Preview<'static> {
    Preview {
        height: lines.len() as u16,
        draw: Box::new(move |f, at| f.render_widget(Paragraph::new(lines.clone()), at)),
    }
}

/// A line of `width` cells: `spans`, padded on `ground`.
fn padded(mut spans: Vec<Span<'static>>, width: usize, ground: Style) -> Vec<Span<'static>> {
    let used = super::spans_width(&spans);
    spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), ground));
    spans
}

/// A ticket the previews draw: the board's own cards are never borrowed,
/// so a preview says the same on every board.
fn sample_ticket(n: u128, key: &str, title: &str) -> mesimon_core::board::Ticket {
    mesimon_core::board::Ticket {
        id: ulid::Ulid::from(n),
        short_key: key.into(),
        title: title.into(),
        column: String::new(),
        order: String::new(),
        created_at: String::new(),
        created_by: String::new(),
        created_from: None,
        entered_at: None,
        previous_column: None,
        picked: None,
        woke_at: None,
        manual_merge: false,
        execution_policy: Default::default(),
        tier: None,
        import_origin: None,
        envelope: None,
        raised: None,
        workspace: None,
        tags: Vec::new(),
        notes: Vec::new(),
        archived: None,
    }
}

/// Appearance, as the board draws it: the cursor's card with its summary
/// and its corner as the rows set them, a card beside it, and an agent's
/// pane with tmux's bar at its top or its bottom. While `Crown's actions`
/// is the row under the cursor or the pointer, the two are the crowned card
/// and a card the crown moved, in the crown's tint; turning it to lightning
/// strikes the bolt between them once.
fn appearance(app: &App, width: usize) -> Preview<'_> {
    use crate::ui::{CrownMark, Land};
    let p = &app.prefs;
    let card_w: u16 = if width >= 56 { 26 } else { (width.saturating_sub(2) / 2) as u16 };
    let pane_x = 2 * card_w + 2 + 3;
    let pane = usize::from(pane_x) + 24 <= width;
    // The cursor card opens to its summary rows, which set the height.
    let shows = p.summary != crate::prefs::SummaryShow::None;
    let height = if shows { 6 } else { 4 };
    let crown = on_crown_row(app);
    Preview {
        height,
        draw: Box::new(move |f, at| {
            let theme = &app.theme;
            let crowned = sample_ticket(1, "T-1", "Plan the beta");
            let moved = sample_ticket(2, "T-2", "Fix the parser");
            let caption = |f: &mut Frame, x: u16, words: &str| {
                let r = Rect { x: at.x + x, y: at.y, width: card_w, height: 1 };
                f.render_widget(Paragraph::new(Span::styled(words.to_string(), theme.dim3())), r);
            };
            let (first, second) = if crown {
                ("the crowned card", "a card it moved")
            } else {
                ("the cursor's card", "another card")
            };
            caption(f, 0, first);
            caption(f, card_w + 2, second);
            let row = at.y + 2;
            let left = Rect { x: at.x, y: row, width: card_w, height: at.bottom() - row };
            let right = Rect { x: at.x + card_w + 2, ..left };
            // The one strike the row's change started, if it is running:
            // the word lands as the bolt arrives; otherwise it stands.
            let strike = app.preview_strike_ms().filter(|_| crown && app.motion());
            let land = strike.map(|t| Land {
                kind: crate::theme::LandKind::of("moved"),
                ms: t as i64 - crate::strike::LEADER_MS as i64,
            });
            let (mark, touched) = if crown {
                (CrownMark::Holder { sweep: None }, CrownMark::Touched { action: "moved", land })
            } else {
                (CrownMark::None, CrownMark::None)
            };
            let holder = sample_card(app, card_w, &crowned, true, mark);
            let struck = sample_card(app, card_w, &moved, false, touched);
            f.render_widget(Paragraph::new(holder), left);
            f.render_widget(Paragraph::new(struck), right);
            // The bolt, from the crown's mark to the moved card's title,
            // arcing through the row above them.
            if let Some(t) = strike {
                let bar = crate::tags::BAR_WIDTH as u16 + 1;
                let stops = [(left.x + bar, row), (right.x + bar, row)];
                let dots = crate::strike::bolt(0x5EED, &stops, at.y + 1);
                let skip = |_x: u16, y: u16| y == row;
                let sky = Rect { x: at.x, y: at.y + 1, width: 2 * card_w + 2, height: 2 };
                crate::strike::paint(f.buffer_mut(), sky, theme, &[(dots, t)], &skip);
            }
            // The pane, its own small frame, the bar where the row puts it.
            if pane {
                let pr = Rect {
                    x: at.x + pane_x,
                    y: at.y + 1,
                    width: (at.width - pane_x).min(36),
                    height: 4,
                };
                let inner = dialog::frame(
                    f,
                    app,
                    pr,
                    None,
                    &theme.rest,
                    dialog::Edges {
                        title: vec![Span::styled("an agent's pane", theme.dim3())],
                        tail: Vec::new(),
                    },
                );
                app.hits.borrow_mut().join_top();
                let w = usize::from(inner.width);
                let lit = theme.selected_row();
                let bar = Line::from(padded(
                    vec![Span::styled(" T-2 Fix the parser", lit.fg(theme.sel.dim1))],
                    w,
                    lit,
                ));
                let dot = glyph(app, "⏺", "*");
                let text =
                    Line::from(Span::styled(format!(" {dot} Reading src/osc.rs"), theme.dim2()));
                let lines = if app.prefs.status_top { vec![bar, text] } else { vec![text, bar] };
                f.render_widget(Paragraph::new(lines), inner);
            }
        }),
    }
}

/// Is `Crown's actions` the Appearance row under the cursor, or under the
/// pointer?
fn on_crown_row(app: &App) -> bool {
    let items = keymap::settings_items(&app.frame_ctx());
    let is_crown = |i: usize| items.get(i).is_some_and(|m| m.verb == keymap::Verb::CrownLightning);
    let cursor = matches!(app.mode, Mode::Settings { idx } if is_crown(idx));
    let pointer = matches!(app.hover.get(), Some((crate::mouse::Target::Row(i), _)) if is_crown(i));
    cursor || pointer
}

/// A sample card as the board draws it, `width` cells wide, with the
/// corner, the summary and its underline as the Appearance rows set them;
/// the cursor's card opens to its summary rows, and the other wears its
/// agent's new reply while `New replies` is on (T-720), at rest.
fn sample_card(
    app: &App,
    width: u16,
    ticket: &mesimon_core::board::Ticket,
    selected: bool,
    crown: crate::ui::CrownMark<'_>,
) -> Vec<Line<'static>> {
    use crate::app::{SummaryRow, TicketSummary};
    let p = &app.prefs;
    let shows = p.summary != crate::prefs::SummaryShow::None;
    let row = |line: usize, text: &str, done: Option<bool>| SummaryRow {
        note: ticket.id,
        line,
        text: text.into(),
        done,
    };
    let summary = TicketSummary {
        rows: vec![
            row(1, "one pass over the reply", None),
            row(2, "read the reply", Some(true)),
            row(3, "parse it", Some(false)),
        ],
        count: mesimon_core::summary::Count { done: 1, total: 2 },
    };
    let corner = if p.card_corner == crate::prefs::CardCorner::Cost { "$1.84" } else { "3h" };
    let ctx = super::card::CardCtx {
        theme: &app.theme,
        width,
        now_ms: (app.now)(),
        spin: app.spin_frame(),
        names_key: false,
    };
    super::card::render(
        &ctx,
        ticket,
        &[],
        false,
        None,
        selected,
        false,
        false,
        None,
        selected && shows,
        None,
        &[],
        false,
        false,
        false,
        None,
        false,
        None,
        None,
        None,
        crown,
        None,
        Some(corner.to_string()),
        shows.then_some(&summary),
        None,
        None,
        p.summary == crate::prefs::SummaryShow::Full,
        (p.new_replies && !selected)
            .then_some(("Parser fixed, tests green.", crate::theme::REPLY_REVEAL_MS)),
    )
}

/// A banner as the rows would post it: who posts it, the words, and the
/// sound and the bounce beside it. Off, a line that says so.
fn banner(app: &App, width: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let p = &app.prefs;
    if !p.notify {
        return vec![Line::from(Span::styled("nothing leaves the board's window", theme.dim3()))];
    }
    let w = 44.min(width);
    let lit = theme.selected_row();
    let who =
        if p.notify_via == crate::prefs::NotifyVia::Terminal { "your terminal" } else { "mesimon" };
    let words =
        if p.notify_words { "“Keep the old parser as a fallback?”" } else { "a question" };
    let rows = [
        (format!(" {who}"), lit.fg(theme.sel.base).add_modifier(Modifier::BOLD)),
        (format!(" {} ∙ Fix the OSC parser", app.board_name()), lit.fg(theme.sel.dim1)),
        (format!(" {words}"), lit.fg(theme.sel.dim2)),
    ];
    let note = glyph(app, "♪", "sound:");
    let mut beside = vec![format!("{note} {}", p.notify_sound_needs_you.name())];
    if p.notify_dock_bounce && app.ctx().iterm2 {
        beside.push("the dock bounces".into());
    }
    rows.into_iter()
        .enumerate()
        .map(|(i, (text, style))| {
            let mut spans = padded(vec![Span::styled(truncate(&text, w), style)], w, lit);
            if let Some(b) = beside.get(i).filter(|b| width >= w + 3 + b.width()) {
                spans.push(Span::raw("   "));
                spans.push(Span::styled(b.clone(), theme.dim2()));
            }
            Line::from(spans)
        })
        .collect()
}

/// The terminal's tab bar as the rows set it, from this board: the tab as
/// it stands now and the same tab while a ticket needs you, side by side,
/// each with what this terminal can show of it — the icon, the ring, the
/// needs-you dot or colour, the title and the subtitle. `None` where the
/// page is too narrow for one tab.
fn tab(app: &App, width: usize) -> Option<Preview<'_>> {
    const TAB_W: usize = 40;
    if width < TAB_W {
        return None;
    }
    let two = width >= 2 * TAB_W + 2;
    Some(Preview {
        height: 4,
        draw: Box::new(move |f, at| {
            let now = app.board.needs_you_count();
            let working =
                app.board.sessions.iter().filter(|s| crate::glyphs::is_working(s)).count();
            draw_tab(f, app, Rect { width: TAB_W as u16, ..at }, now, working, "now");
            if two {
                let x = at.x + TAB_W as u16 + 2;
                let r = Rect { x, width: TAB_W as u16, ..at };
                draw_tab(f, app, r, now.max(1), working, "when a ticket needs you");
            }
        }),
    })
}

/// One tab, framed: on the tab's own colour, the icon, the progress ring,
/// the dot, the title; the subtitle under it; `caption` in its edge.
fn draw_tab(f: &mut Frame, app: &App, at: Rect, needs_you: usize, working: usize, caption: &str) {
    let theme = &app.theme;
    let p = &app.prefs;
    let ctx = app.ctx();
    // What iTerm2 3.7's session status draws, and iTerm2's own colours.
    let status = ctx.iterm2 && ctx.iterm2_status;
    let whole = ctx.iterm2 && p.tab_color == crate::prefs::TabColor::Tab && needs_you > 0;
    let dot = status && p.tab_color == crate::prefs::TabColor::Dot && needs_you > 0;
    let surface = if whole {
        Some(theme.attn)
    } else if ctx.iterm2 && p.tab_theme {
        theme.selected_bg
    } else {
        None
    };
    let ink = if whole { Style::default().fg(theme.attn_ink) } else { theme.base() };
    let inner = dialog::frame(
        f,
        app,
        at,
        surface,
        &theme.rest,
        dialog::Edges {
            title: Vec::new(),
            tail: vec![Span::styled(caption.to_string(), theme.dim3())],
        },
    );
    app.hits.borrow_mut().join_top();
    let mut head: Vec<Span<'static>> = vec![Span::raw(" ")];
    if status && p.tab_icon {
        head.push(Span::styled(format!("{} ", glyph(app, "ש", "S")), ink));
    }
    if p.tab_progress {
        let ring = if needs_you > 0 {
            Span::styled(format!("{} ", glyph(app, "◉", "!")), Style::default().fg(theme.err))
        } else if working > 0 {
            let spin = ["◐", "◓", "◑", "◒"][app.spin_frame() % 4];
            Span::styled(format!("{} ", glyph(app, spin, "*")), ink)
        } else {
            Span::styled(format!("{} ", glyph(app, "○", "-")), ink)
        };
        head.push(ring);
    }
    if dot {
        head.push(Span::styled(format!("{} ", glyph(app, "●", "*")), theme.attn_text()));
    }
    let title = if p.tab_title.is_on() {
        crate::title::board(
            &app.board_name(),
            p.tab_title == crate::prefs::TabTitle::Mesimon,
            p.tab_title_needs_you.then_some(needs_you),
        )
    } else {
        "the terminal's own title".to_string()
    };
    let used = super::spans_width(&head);
    head.push(Span::styled(
        truncate(&title, usize::from(inner.width).saturating_sub(used + 1)),
        ink.add_modifier(Modifier::BOLD),
    ));
    let mut lines = vec![Line::from(head)];
    if status && p.tab_subtitle {
        let sub = crate::title::subtitle(needs_you, working);
        lines.push(Line::from(Span::styled(format!(" {sub}"), ink)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}
