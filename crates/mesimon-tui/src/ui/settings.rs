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
    // Every row and heading as a line; `at` is each row's line.
    let name_w = items.iter().map(|m| (m.label)(&ctx).width()).max().unwrap_or(0).min(NAME_W);
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
        lines.push(row_line(app, &ctx, item, name_w, width, cursor == Some(i)));
    }
    // The preview goes first, and only where every row still fits under it.
    let preview = preview(app, width)
        .map(|mut p| {
            while p.last().is_some_and(|l| l.width() == 0) {
                p.pop();
            }
            p
        })
        .filter(|p| p.len() + 1 + lines.len() <= body_h);
    let top = preview.as_ref().map_or(0, |p| p.len() + 1);
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
    let mut shown: Vec<Line<'static>> = preview
        .map(|mut p| {
            p.push(Line::default());
            p
        })
        .unwrap_or_default();
    shown.extend(lines.into_iter().skip(first).take(room));
    f.render_widget(Paragraph::new(shown), Rect { height: body_h as u16, ..area });
    // The selected row's one line, at the foot, revealed when it is long.
    if let Some(c) = cursor {
        let detail = keymap::item_detail(items[c], &ctx);
        let body = dialog::reveal(app, &detail, width);
        let foot = Rect { y: area.bottom() - 1, height: 1, ..area };
        f.render_widget(Paragraph::new(Span::styled(body, theme.dim2())), foot);
    }
}

/// One row: its name, its value at the value column, and in board scope
/// where the value comes from, at the right edge (T-361).
fn row_line(
    app: &App,
    ctx: &keymap::Ctx,
    item: &MenuItem,
    name_w: usize,
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
        spans.extend(value_spans(app, &value, room, on, off));
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

/// A value in `room` cells: a switch's word with its mark, every option of
/// a choice with the chosen one marked (or the chosen one alone where they
/// do not all fit), a word, or a door's word and its `›`.
fn value_spans(app: &App, value: &Value, room: usize, on: Style, off: Style) -> Vec<Span<'static>> {
    let tier = app.theme.glyph_tier();
    let (set, unset) = if tier == Tier::Ascii { ("*", "-") } else { ("●", "○") };
    let fit = |s: &str| truncate(s, room);
    match value {
        Value::Switch(true) => vec![Span::styled(fit(&format!("{set} on")), on)],
        Value::Switch(false) => vec![Span::styled(fit(&format!("{unset} off")), off)],
        Value::Choice { options, at } => {
            let all: usize = options.iter().map(|o| o.width() + 2).sum::<usize>()
                + 3 * options.len().saturating_sub(1);
            if all > room {
                let chosen = options.get(*at).copied().unwrap_or_default();
                return vec![Span::styled(fit(chosen), on)];
            }
            let mut spans = Vec::new();
            for (i, o) in options.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw("   "));
                }
                if i == *at {
                    spans.push(Span::styled(format!("{set} {o}"), on));
                } else {
                    spans.push(Span::styled(format!("{unset} {o}"), off));
                }
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

/// What the section's rows do, drawn small at the top of its page: the
/// card and the pane's bar (Appearance), the banner (Notifications), the
/// tab (Terminal), the quota line (Usage). `None` for a section with
/// nothing to picture.
fn preview(app: &App, width: usize) -> Option<Vec<Line<'static>>> {
    match app.settings_section {
        SettingsSection::Appearance => Some(appearance(app, width)),
        SettingsSection::Notifications => Some(banner(app, width)),
        SettingsSection::Terminal => Some(tab(app, width)),
        SettingsSection::Usage => Some(quota(app, width)),
        _ => None,
    }
}

/// A line of `width` cells: `spans`, padded on `ground`.
fn padded(mut spans: Vec<Span<'static>>, width: usize, ground: Style) -> Vec<Span<'static>> {
    let used = super::spans_width(&spans);
    spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), ground));
    spans
}

/// A card as the board draws the cursor's, with its corner and its summary
/// as the rows set them, beside an agent's pane with tmux's bar at the top
/// or the bottom.
fn appearance(app: &App, width: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let p = &app.prefs;
    let card_w = 40.min(width);
    let lit = theme.selected_row();
    let title_style = {
        let s = lit.fg(theme.sel.base).add_modifier(Modifier::BOLD);
        match (p.summary, theme.summary_under(true, false)) {
            (crate::prefs::SummaryShow::Full, Some(u)) => s.patch(u),
            _ => s,
        }
    };
    let corner = if p.card_corner == crate::prefs::CardCorner::Cost { "$1.84" } else { "3h" };
    let title = truncate("Fix the OSC reply parser", card_w.saturating_sub(4 + corner.width()));
    let bar = glyph(app, "▎ ", "| ");
    let mut card = vec![Span::styled(bar, lit.fg(theme.pip(6))), Span::styled(title, title_style)];
    card = padded(card, card_w.saturating_sub(corner.width() + 1), lit);
    card.push(Span::styled(corner.to_string(), lit.fg(theme.sel.dim2)));
    card.push(Span::styled(" ", lit));
    let summary = if p.summary == crate::prefs::SummaryShow::None {
        Vec::new()
    } else {
        let tick = glyph(app, "✓", "v");
        vec![Span::styled(format!("  {tick} read the reply ∙ parse it in one pass"), theme.dim2())]
    };
    let mut lines = vec![card, padded(summary, card_w, Style::default()), Vec::new()];
    // The pane, beside the card where the page is wide enough for both.
    let pane_x = card_w + 4;
    if width >= pane_x + 24 {
        let pane_w = (width - pane_x).min(36);
        let bar = padded(
            vec![Span::styled(" T-12 Fix the OSC parser", lit.fg(theme.sel.dim1))],
            pane_w,
            lit,
        );
        let dot = glyph(app, "⏺", "*");
        let text = vec![Span::styled(format!(" {dot} Reading src/osc.rs"), theme.dim2())];
        let (top, mid, low) =
            if p.status_top { (bar, text, Vec::new()) } else { (Vec::new(), text, bar) };
        for (line, part) in lines.iter_mut().zip([top, mid, low]) {
            let used = super::spans_width(line);
            line.push(Span::raw(" ".repeat(pane_x.saturating_sub(used))));
            line.extend(part);
        }
    }
    lines.into_iter().map(Line::from).collect()
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

/// The terminal's tab as the rows set it, from this board as it stands:
/// its title, the ring, the needs-you colour, and the subtitle under it.
fn tab(app: &App, width: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let p = &app.prefs;
    let needs_you = app.board.needs_you_count();
    let working = app.board.sessions.iter().filter(|s| crate::glyphs::is_working(s)).count();
    let w = 48.min(width);
    let colour = p.tab_color != crate::prefs::TabColor::Off && needs_you > 0;
    // The whole tab in the needs-you colour while it stands, else the
    // theme's band, else the terminal's own (the page's ground here).
    let ground = if colour && p.tab_color == crate::prefs::TabColor::Tab {
        theme.attn_row()
    } else if p.tab_theme {
        theme.selected_row().fg(theme.sel.base)
    } else {
        theme.base()
    };
    let mut head = vec![Span::styled(" ", ground)];
    if p.tab_progress {
        let ring = if needs_you > 0 {
            glyph(app, "◉ ", "! ")
        } else if working > 0 {
            glyph(app, "◔ ", "* ")
        } else {
            glyph(app, "○ ", "- ")
        };
        head.push(Span::styled(ring, ground));
    }
    if colour && p.tab_color == crate::prefs::TabColor::Dot {
        head.push(Span::styled(glyph(app, "● ", "* "), ground.patch(theme.attn_text())));
    }
    let title = app.tab_title().unwrap_or_else(|| "the terminal's own title".into());
    head.push(Span::styled(
        truncate(&title, w.saturating_sub(6)),
        ground.add_modifier(Modifier::BOLD),
    ));
    let mut lines = vec![Line::from(padded(head, w, ground))];
    if p.tab_subtitle {
        let sub = crate::title::subtitle(needs_you, working);
        let sub = if sub.is_empty() { "nothing needs you".to_string() } else { sub };
        lines.push(Line::from(padded(vec![Span::styled(format!(" {sub}"), ground)], w, ground)));
    }
    lines
}

/// The quota line as it would stand above the keys now.
fn quota(app: &App, width: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let line = super::usage::line(app, width);
    if !line.is_empty() {
        return vec![Line::from(line)];
    }
    let why = match app.prefs.usage_line {
        crate::prefs::UsageLine::Off => "nothing above the keys",
        crate::prefs::UsageLine::Near => "quiet until a provider nears a limit",
        _ => "no reading yet",
    };
    vec![Line::from(Span::styled(why, theme.dim3()))]
}
