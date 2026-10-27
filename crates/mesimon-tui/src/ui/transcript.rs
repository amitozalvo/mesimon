//! The transcript dialog (T-921): one message of the cursor card's agent
//! conversation, hung under the card over a receded board. The title says
//! where in the conversation the reader is, `TRANSCRIPT (i of n)`, and
//! what came since; the message's first row says who wrote it, when, and
//! the steps that followed it; the rest is the message as the ticket
//! page's preview renders a reply, scrolled by `jk`.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use mesimon_core::board::SessionKind;
use mesimon_core::keymap::{self, Scope, Verb};
use mesimon_core::mesophon::RowKind;

use super::dialog::{self, Edges};
use crate::app::App;
use crate::glyphs::Tier;
use crate::rich::Newline;
use crate::text::truncate;

/// The dialog's inner rows and width, terminal permitting: a medium
/// window, the reply's reading measure.
const ROWS: u16 = 14;
const WIDTH: u16 = 68;

/// When a message was written: the clock today, how long ago before that.
fn when(ms: Option<u64>, now_ms: u64) -> String {
    match ms {
        Some(ms) if now_ms.saturating_sub(ms) < 20 * 3_600_000 => {
            crate::localtime::clock_word(ms / 1000).unwrap_or_default()
        }
        Some(ms) => crate::text::age_slot(now_ms, ms, false),
        None => String::new(),
    }
}

/// `9 steps ∙ Read, Edit, Bash`: what followed the message.
fn steps_word(steps: usize, tools: &[String]) -> String {
    match steps {
        0 => String::new(),
        1 => format!("1 step ∙ {}", tools.join(", ")),
        n => format!("{n} steps ∙ {}", tools.join(", ")),
    }
}

pub(super) fn draw(f: &mut Frame, app: &App) {
    let Some(t) = app.open_transcript_view() else { return };
    let theme = &app.theme;
    let screen = f.area();
    let mut area = dialog::placed(app, screen, ROWS, WIDTH);
    // The row above the footer stays the screen's, as a list's does.
    let whole_y = screen.bottom().saturating_sub(2);
    area.height = area.height.min(whole_y.saturating_sub(area.y)).max(3);
    // Inside the frame, and two cells in from its edge: the title's column.
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let body = Rect { x: inner.x + 2, width: inner.width.saturating_sub(4), ..inner };
    let fits = body.width >= 8 && body.height >= 3;

    // The words are measured before the edge is drawn, so the edge's `jk`
    // offers a scroll on the very frame that has one.
    let words = t.message().filter(|_| fits).map(|m| words(app, t, m, body));
    app.preview_measured();

    let mut title = dialog::title(&theme.rest, "TRANSCRIPT");
    if !t.messages().is_empty() {
        title.push(Span::styled(
            format!(" ({} of {})", t.idx + 1, t.messages().len()),
            theme.dim2(),
        ));
    }
    let fresh = t.unseen();
    if fresh > 0 {
        title.push(Span::styled(" ∙ ", theme.dim2()));
        title.push(Span::styled(
            format!("{fresh} new"),
            Style::default().fg(theme.rest.base).add_modifier(Modifier::BOLD),
        ));
    }
    // The keys that read go in the edge; Esc is the footer's.
    let ctx = app.frame_ctx();
    let (own, _) = keymap::footer_split(Scope::Transcript, &ctx);
    let own: Vec<_> = own.into_iter().filter(|b| b.verb != Verb::Back).collect();
    let tail =
        super::chrome::hint_spans(&own, &ctx, &theme.rest, inner.width.saturating_sub(4).into());
    dialog::frame(f, app, area, None, &theme.rest, Edges { title, tail });
    dialog::notch(f, app, area);
    if !fits {
        return;
    }
    let Some((head, shown)) = words else {
        // A conversation cut short, or started over with nothing said yet.
        let quiet = Line::from(Span::styled("nothing said yet", theme.dim2()));
        f.render_widget(Paragraph::new(quiet), Rect { height: 1, ..body });
        return;
    };
    f.render_widget(Paragraph::new(head), Rect { height: 1, ..body });
    f.render_widget(Paragraph::new(shown), Rect { y: body.y + 2, height: body.height - 2, ..body });
}

/// The message's first row (who, when, the steps that followed it) and its
/// words in the window `jk` scrolls, measured into the dialog's `Pager`.
fn words(
    app: &App,
    t: &crate::transcript::Transcript,
    m: &crate::transcript::Message,
    body: Rect,
) -> (Line<'static>, Vec<Line<'static>>) {
    let theme = &app.theme;
    let width = body.width as usize;

    // Who, when, and the steps that followed, at the right.
    let who = match (m.kind, t.kind) {
        (RowKind::Prompt, _) => "you",
        (RowKind::Notice, _) => "",
        (_, SessionKind::Codex) => "codex",
        _ => "claude",
    };
    let who_style = match m.kind {
        RowKind::Reply => Style::default().fg(theme.rest.base).add_modifier(Modifier::BOLD),
        _ => theme.dim1(),
    };
    let time = when(m.ms, mesimon_core::clock::now_ms());
    let left = if who.is_empty() { time.clone() } else { format!("{who}  {time}") };
    let room = width.saturating_sub(left.width() + 2);
    let steps = truncate(&steps_word(m.steps, &m.tools), room);
    let gap = width.saturating_sub(left.width() + steps.width());
    let mut head = Vec::new();
    if !who.is_empty() {
        head.push(Span::styled(who.to_string(), who_style));
        head.push(Span::styled(format!("  {time}"), theme.dim2()));
    } else {
        head.push(Span::styled(time, theme.dim2()));
    }
    head.push(Span::raw(" ".repeat(gap)));
    head.push(Span::styled(steps, theme.dim3()));

    // The words: a person's behind the quote bar, read as they typed
    // them; the agent's as the ticket page reads a reply; a notice in
    // the quiet ink.
    let prompt = m.kind == RowKind::Prompt;
    let bar = if prompt { 2 } else { 0 };
    let newline = if prompt { Newline::Break } else { Newline::Space };
    let key = crate::transcript::doc_key(m);
    let rows = super::ticket::rendered(
        app,
        key,
        width - bar,
        || crate::peek::sanitize(t.text(m)),
        newline,
    );
    let budget = usize::from(body.height.saturating_sub(2));
    let at = t.pager.window(Some(key), rows.len(), budget, false);
    let mut shown: Vec<Line<'static>> = rows.iter().skip(at).take(budget).cloned().collect();
    if at + shown.len() < rows.len() {
        crate::rich::mark_cut(&mut shown, width - bar, theme);
    }
    for line in &mut shown {
        if m.kind == RowKind::Notice {
            for s in &mut line.spans {
                s.style = s.style.fg(theme.rest.dim2);
            }
        }
        if prompt {
            let mark = if theme.glyph_tier() == Tier::Ascii { "> " } else { "▎ " };
            line.spans.insert(0, Span::styled(mark, theme.dim3()));
        }
    }
    (Line::from(head), shown)
}
