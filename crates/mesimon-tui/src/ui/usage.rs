//! The subscription quota on the board (T-327): the line on the right of the
//! row above the keys, the Usage dialog the menu opens, and the menu row's
//! own summary.
//!
//! Every percentage is the provider's, and the line never grades one: a
//! window is grey while its provider calls it normal, and the provider's
//! warning or critical steps it to full ink with its reset beside it — the
//! value step, never the one saturated colour, which stays needs-you's (L3).
//! A throttled card already wears its grey `~`. A notice or an undo keeps
//! its place on the row; the line takes what is left and gives up its parts
//! in a fixed order (`line`), or stands aside whole.

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::keymap::Scope;
use mesimon_core::usage::{pace, Provider, Reading, Severity, Window, WindowKind};

use crate::app::App;
use crate::prefs::{Prefs, UsageLine, UsageResets};
use crate::text::truncate;

use super::dialog;

/// A provider's words (a model's name, a plan, an error) are another
/// process's text: scrubbed before they are drawn.
fn clean(s: &str) -> String {
    mesimon_core::text::scrub_cells(s, false)
}

fn hot(w: &Window) -> bool {
    w.severity != Severity::Normal
}

/// The most severe, then the provider's own pick, then the fullest.
fn hottest<'a>(rows: &[&'a Window]) -> Option<&'a Window> {
    rows.iter().copied().max_by(|a, b| {
        (a.severity, a.headline)
            .cmp(&(b.severity, b.headline))
            .then(a.percent.partial_cmp(&b.percent).unwrap_or(std::cmp::Ordering::Equal))
    })
}

fn enabled(p: &Prefs, provider: Provider) -> bool {
    match provider {
        Provider::Claude => p.usage_claude,
        Provider::Codex => p.usage_codex,
    }
}

/// A reading's windows that still exist: one whose reset passed since it
/// was read is about a window that is gone, until the next read.
fn live(r: &Reading, now: u64) -> Vec<&Window> {
    r.windows.iter().filter(|w| w.resets_at_ms.is_none_or(|t| t > now)).collect()
}

/// The windows the settings let onto the line: the kinds they name, and a
/// meter this build does not know only while it warns.
fn allowed<'a>(p: &Prefs, r: &'a Reading, now: u64) -> Vec<&'a Window> {
    live(r, now)
        .into_iter()
        .filter(|w| match w.kind {
            WindowKind::Session => p.usage_5h,
            WindowKind::Weekly => p.usage_week,
            WindowKind::Model => p.usage_model,
            WindowKind::Other => hot(w),
        })
        .collect()
}

/// What the line shows of one provider before it is fitted.
fn chosen<'a>(p: &Prefs, r: &'a Reading, now: u64) -> Vec<&'a Window> {
    let rows = allowed(p, r, now);
    match p.usage_line {
        UsageLine::Off => Vec::new(),
        UsageLine::Near => rows.into_iter().filter(|w| hot(w)).collect(),
        UsageLine::Every => rows,
        UsageLine::Headline => hottest(&rows).into_iter().collect(),
    }
}

const WDAY: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MON: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// When a window starts over, in the board's clock: `20:50` today, `Sat
/// 11:00` within the week, `Oct 3` past it.
pub(crate) fn reset_word(app: &App, at_ms: u64) -> String {
    let now = (app.now)();
    let (Some(t), Some(n)) = ((app.clock)(at_ms / 1000), (app.clock)(now / 1000)) else {
        return String::new();
    };
    if (t.year, t.mon, t.mday) == (n.year, n.mon, n.mday) {
        format!("{:02}:{:02}", t.hour, t.min)
    } else if at_ms.saturating_sub(now) < 6 * 86_400_000 {
        let day = WDAY.get(t.wday as usize).copied().unwrap_or("");
        format!("{day} {:02}:{:02}", t.hour, t.min)
    } else {
        format!("{} {}", MON.get(t.mon as usize).copied().unwrap_or(""), t.mday)
    }
}

/// Which windows name their reset, for one attempt at fitting the line.
type ResetRule<'r> = &'r dyn Fn(&Window) -> bool;

struct Group<'a> {
    provider: Provider,
    rows: Vec<&'a Window>,
}

fn spans_of(app: &App, groups: &[Group<'_>], reset: ResetRule<'_>) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let mut out: Vec<Span<'static>> = Vec::new();
    for (i, g) in groups.iter().enumerate() {
        if i > 0 {
            out.push(Span::raw("   "));
        }
        out.push(Span::styled(g.provider.word().to_string(), theme.dim3()));
        for (j, w) in g.rows.iter().enumerate() {
            out.push(Span::styled(if j == 0 { " " } else { " ∙ " }.to_string(), theme.dim3()));
            let warm = hot(w);
            let (label, value) =
                if warm { (theme.base(), theme.base()) } else { (theme.dim3(), theme.dim2()) };
            out.push(Span::styled(format!("{} ", clean(&w.label)), label));
            out.push(Span::styled(w.percent_word(), value));
            if let Some(at) = w.resets_at_ms.filter(|_| reset(w)) {
                let word = reset_word(app, at);
                if !word.is_empty() {
                    out.push(Span::styled(
                        format!(" resets {word}"),
                        if warm { theme.base() } else { theme.dim3() },
                    ));
                }
            }
        }
    }
    out
}

fn keep_all<'a>(groups: &[Group<'a>]) -> Vec<Group<'a>> {
    groups.iter().map(|g| Group { provider: g.provider, rows: g.rows.clone() }).collect()
}

/// Each provider's warnings and its headline; its calm windows go.
fn warm_and_head<'a>(groups: &[Group<'a>]) -> Vec<Group<'a>> {
    groups
        .iter()
        .map(|g| {
            let top = hottest(&g.rows);
            let rows = g
                .rows
                .iter()
                .copied()
                .filter(|w| hot(w) || top.is_some_and(|t| std::ptr::eq(t, *w)))
                .collect();
            Group { provider: g.provider, rows }
        })
        .collect()
}

/// Each provider's headline alone.
fn heads<'a>(groups: &[Group<'a>]) -> Vec<Group<'a>> {
    groups
        .iter()
        .map(|g| Group { provider: g.provider, rows: hottest(&g.rows).into_iter().collect() })
        .collect()
}

/// The quota line for `room` cells, or nothing. What the settings choose,
/// then — while it does not fit — reset times only beside a warning, each
/// provider's calm windows but its headline gone, its headline alone, the
/// headline without its reset, and last a whole provider, calmest first.
/// Below the last of those the line stands aside rather than lie by cutting.
pub(super) fn line(app: &App, room: usize) -> Vec<Span<'static>> {
    let p = &app.prefs;
    if p.usage_line == UsageLine::Off || room == 0 {
        return Vec::new();
    }
    let now = (app.now)();
    let mut groups: Vec<Group<'_>> = Vec::new();
    for provider in Provider::ALL {
        if !enabled(p, provider) {
            continue;
        }
        let Some(r) = app.usage.get(provider).reading.as_ref() else { continue };
        let rows = chosen(p, r, now);
        if !rows.is_empty() {
            groups.push(Group { provider, rows });
        }
    }
    if groups.is_empty() {
        return Vec::new();
    }
    let as_set = |w: &Window| match p.usage_resets {
        UsageResets::Always => true,
        UsageResets::Near => hot(w),
        UsageResets::Never => false,
    };
    let warm_only = |w: &Window| p.usage_resets != UsageResets::Never && hot(w);
    let none = |_: &Window| false;
    let fits = |spans: &[Span<'static>]| super::spans_width(spans) <= room;

    let tries: [(Vec<Group<'_>>, ResetRule<'_>); 5] = [
        (keep_all(&groups), &as_set),
        (keep_all(&groups), &warm_only),
        (warm_and_head(&groups), &warm_only),
        (heads(&groups), &warm_only),
        (heads(&groups), &none),
    ];
    for (gs, reset) in &tries {
        let spans = spans_of(app, gs, *reset);
        if fits(&spans) {
            return spans;
        }
    }
    // A provider at a time, the calmest first.
    let mut left = heads(&groups);
    while left.len() > 1 {
        let calmest = left
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let (x, y) = (a.1.rows[0], b.1.rows[0]);
                x.severity
                    .cmp(&y.severity)
                    .then(x.percent.partial_cmp(&y.percent).unwrap_or(std::cmp::Ordering::Equal))
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        left.remove(calmest);
        for reset in [&warm_only as ResetRule<'_>, &none] {
            let spans = spans_of(app, &left, reset);
            if fits(&spans) {
                return spans;
            }
        }
    }
    Vec::new()
}

/// The menu row's words (`claude Fable 64% ∙ codex signed out`): each
/// provider the settings name, by its headline or by why it has none.
pub(crate) fn summary(app: &App) -> String {
    let now = (app.now)();
    let mut parts = Vec::new();
    for provider in Provider::ALL {
        if !enabled(&app.prefs, provider) {
            continue;
        }
        let u = app.usage.get(provider);
        if let Some(problem) = &u.problem {
            parts.push(format!("{} {}", provider.word(), problem.short()));
        } else if let Some(w) = u.reading.as_ref().and_then(|r| hottest(&live(r, now))) {
            parts.push(format!("{} {} {}", provider.word(), clean(&w.label), w.percent_word()));
        }
    }
    parts.join(" ∙ ")
}

/// The width a window's label column takes in the dialog.
const LABEL_W: usize = 18;

/// The Usage dialog: per provider, a heading with where its numbers stand
/// (official, and how old), then every window with its reset, the pace of
/// its headline week (experimental, and said so), or why there is nothing.
pub(super) fn draw(f: &mut Frame, app: &App) {
    let theme = &app.theme;
    let now = (app.now)();
    let rows_guess = 12u16;
    let probe = dialog::centred(f.area(), rows_guess, dialog::MAX_W);
    let w = probe.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line<'static>> = vec![Line::default()];
    // Left words, right words, right-aligned with two cells of margin.
    let split = |left: Vec<Span<'static>>, right: Vec<Span<'static>>| -> Line<'static> {
        let lw = super::spans_width(&left);
        let rw = super::spans_width(&right);
        let mut spans = left;
        if rw > 0 && lw + rw + 3 <= w {
            spans.push(Span::raw(" ".repeat(w - lw - rw - 2)));
            spans.extend(right);
        }
        Line::from(spans)
    };
    for (i, provider) in Provider::ALL.into_iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        let u = app.usage.get(provider);
        let reading_now = app.usage.reading.contains(&provider);
        let mut head = vec![Span::styled(format!("   {}", provider.word()), theme.base())];
        if let Some(plan) = u.reading.as_ref().and_then(|r| r.plan.as_deref()) {
            head.push(Span::styled(format!(" ∙ {}", clean(plan)), theme.dim2()));
        }
        let right: Vec<Span<'static>> = if !enabled(&app.prefs, provider) {
            vec![Span::styled("off ∙ s turns it on".to_string(), theme.dim3())]
        } else if reading_now {
            vec![Span::styled("reading".to_string(), theme.dim2())]
        } else if let Some(r) = &u.reading {
            vec![
                Span::styled("official".to_string(), theme.dim2()),
                Span::styled(
                    format!(" ∙ read {} ago", crate::text::age_slot(now, r.read_at_ms, false)),
                    theme.dim3(),
                ),
            ]
        } else {
            Vec::new()
        };
        lines.push(split(head, right));
        if !enabled(&app.prefs, provider) {
            continue;
        }
        if let Some(problem) = &u.problem {
            let words = truncate(&clean(&problem.words(provider)), w.saturating_sub(6));
            lines.push(Line::from(Span::styled(format!("     {words}"), theme.dim2())));
        }
        let Some(r) = &u.reading else {
            if u.problem.is_none() {
                let words =
                    if reading_now { "asking it now" } else { "not read yet ∙ r reads it" };
                lines.push(Line::from(Span::styled(format!("     {words}"), theme.dim2())));
            }
            continue;
        };
        let windows = live(r, now);
        if windows.is_empty() {
            lines.push(Line::from(Span::styled(
                "     no windows reported".to_string(),
                theme.dim2(),
            )));
        }
        for win in &windows {
            let warm = hot(win);
            let (label_ink, value_ink, reset_ink) = if warm {
                (theme.base(), theme.base(), theme.base())
            } else {
                (theme.dim2(), theme.dim2(), theme.dim3())
            };
            let label = truncate(&clean(&win.long), LABEL_W);
            let pad = LABEL_W.saturating_sub(label.width());
            let mut spans = vec![
                Span::styled(format!("     {label}{}", " ".repeat(pad)), label_ink),
                Span::styled(format!("{:>4}", win.percent_word()), value_ink),
            ];
            if let Some(at) = win.resets_at_ms {
                let word = reset_word(app, at);
                if !word.is_empty() {
                    spans.push(Span::styled(format!("   resets {word}"), reset_ink));
                }
            }
            lines.push(Line::from(spans));
        }
        // The experimental row: where the headline week is heading.
        let week = hottest(&windows).filter(|h| h.length_mins.is_some_and(|m| m >= 1440));
        if let Some(p) = week.and_then(|h| pace(h, now)) {
            let (value, tail) = match p.full_at_ms {
                Some(at) => ("full".to_string(), format!("   by {}", reset_word(app, at))),
                None => (format!("~{}%", p.projected), "   at this rate".to_string()),
            };
            let pad = LABEL_W.saturating_sub("pace".width());
            let left = vec![
                Span::styled(format!("     pace{}", " ".repeat(pad)), theme.dim2()),
                Span::styled(format!("{value:>4}"), theme.dim2()),
                Span::styled(tail, theme.dim3()),
            ];
            lines.push(split(left, vec![Span::styled("experimental".to_string(), theme.dim3())]));
        }
    }
    lines.push(Line::default());
    let area = dialog::centred(f.area(), lines.len() as u16, dialog::MAX_W);
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner = dialog::frame(
        f,
        app,
        area,
        None,
        &theme.rest,
        dialog::Edges {
            title: dialog::title(&theme.rest, "USAGE"),
            tail: dialog::keys(app, Scope::Usage, &theme.rest, inner_w.saturating_sub(4)),
        },
    );
    f.render_widget(Paragraph::new(lines), inner);
}
