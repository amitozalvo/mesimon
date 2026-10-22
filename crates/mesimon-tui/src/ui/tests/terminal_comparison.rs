//! Replayable output of the real renderer, without a daemon, tmux or user data.
//! Optional export: MESIMON_TERMINAL_COMPARISON=/tmp/comparison.json cargo test
//! -p mesimon-tui terminal_comparison_export. See ci/terminal-compare.py.

use super::*;
use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::{TerminalOptions, Viewport};
use serde_json::{json, Value};

use crate::quiet::{self, Quiet};

#[derive(Clone, Default)]
struct Output(Rc<RefCell<Vec<u8>>>);

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Output {
    fn take(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.0.borrow_mut())).unwrap()
    }
}

fn terminal(out: Output, area: Rect, program: Option<&str>) -> Terminal<Quiet<Output>> {
    Terminal::with_options(
        Quiet::for_terminal(out, program),
        TerminalOptions { viewport: Viewport::Fixed(area) },
    )
    .unwrap()
}

fn paint(terminal: &mut Terminal<Quiet<Output>>, buffer: &Buffer) {
    quiet::draw(terminal, |f| *f.buffer_mut() = buffer.clone()).unwrap();
}

fn specimen(flavor: Flavor, colored: bool) -> App {
    let mut app = app_summary();
    let mut board = (*app.board).clone();
    board.crown = Some(ulid_n(3));
    board.tickets[0].title = "Unicode: café e\u{301} 日本語 ☕️ ✓ ✔".into();
    board.tickets[0].tags = vec![
        mesimon_core::board::TagRef { name: "BUG".into(), group: 1 },
        mesimon_core::board::TagRef { name: "QUESTION".into(), group: 2 },
    ];
    // Fixed spinner phase: sleep sessions here; the glyph sheet below
    // separately samples every moving frame without a wall-clock dependency.
    for s in &mut board.sessions {
        if s.state == SessionState::Running {
            s.state = SessionState::Sleeping;
        }
    }
    let waiting = board.sessions.iter_mut().find(|s| s.ticket == ulid_n(4)).unwrap();
    waiting.state = SessionState::RequiresAction { reason: Reason::Permission };
    waiting.waiting_since = Some(1);
    waiting.detail = Some("Review the proposed change".into());
    app.board = std::sync::Arc::new(board);
    app.theme = Theme::new(flavor, Profile::TrueColor).for_terminal(Some(if colored {
        "iTerm.app"
    } else {
        "Apple_Terminal"
    }));
    app.now = || 10_000;
    app.rich_keys = false; // no negotiated extended keys in this specimen
    app.legacy_shift_enter = !colored; // Terminal.app has a distinct Escape+Return
    app
}

fn frames(flavor: Flavor, colored: bool, w: u16, h: u16) -> Vec<(String, u64, Buffer)> {
    let mut app = specimen(flavor, colored);
    let mut out = Vec::new();
    let mut add = |name: &str, ms, app: &App| out.push((name.into(), ms, cells(app, w, h)));
    add("selected card / progress 2 of 5", 900, &app);
    app.cursor_col = 0;
    add("leave card / progress 2 of 5", 900, &app);
    app.cursor_col = 1;
    app.peek = true;
    add("open summary rows", 900, &app);
    app.peek = false;
    add("close summary rows", 900, &app);
    app.mode = Mode::Summary { ticket: ulid_n(3), idx: 2 };
    add("summary dialog", 900, &app);
    app.mode = Mode::Normal;
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    add("ticket page / notes", 900, &app);
    app.screen = Screen::Board;
    add("return to board", 900, &app);
    app.help = true;
    add("legacy keyboard hints", 900, &app);
    app.help = false;
    for elapsed in (0..=2_000).step_by(100) {
        app.crowned_at = Some((ulid_n(3), 10_000 - elapsed));
        add(&format!("crown sweep {elapsed} ms"), 100, &app);
    }
    app.crowned_at = None;
    app.cursor_col = 0;
    add("leave card after animation", 900, &app);
    app.cursor_col = 1;
    // A live agent keeps the completed run visible; no spawned process.
    let mut board = (*app.board).clone();
    board.sessions[0].state = SessionState::Idle { stop_reason: StopReason::EndTurn };
    app.board = std::sync::Arc::new(board);
    for done in [0, 1, 3, 5] {
        let note = format!(
            "## Summary\n{}",
            (0..5)
                .map(|n| format!("- [{}] step {}\n", if n < done { 'x' } else { ' ' }, n + 1))
                .collect::<String>()
        );
        app.remember_note(ulid_n(90), 1, Some(crate::peek::sanitize(&note)));
        app.summary_pulses.clear();
        add(&format!("progress {done} of 5"), 900, &app);
    }
    app.remember_note(ulid_n(90), 1, Some(crate::peek::sanitize(RICH_REPLY)));
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    add("rich note", 1400, &app);
    install_diff(&mut app);
    add("diff colors and wrapping", 1400, &app);
    let mut detail = app_noted();
    detail.theme = Theme::new(flavor, Profile::TrueColor).for_terminal(Some(if colored {
        "iTerm.app"
    } else {
        "Apple_Terminal"
    }));
    detail.legacy_shift_enter = !colored;
    detail.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    add("description bar including blank rows", 1400, &detail);
    detail.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
    add("mascot solid pixels", 1400, &detail);
    // Actual status glyphs, isolated so a width disagreement is visible
    // against the ASCII end marker without depending on a running agent.
    let glyphs = [
        "text accents: café e\u{301} | END",
        "wide text: 日本語 | END",
        "emoji: ☕️ | END",
        "status: ✓ ✔ ♛ ● ▎ | END",
        "working: ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ | END",
        "waiting: ⠉⠘⠰⠤⠆⠃ | END",
    ];
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| {
        use ratatui::{
            style::Style,
            widgets::{Block, Paragraph},
        };
        f.render_widget(
            Block::new().style(Style::default().fg(app.theme.rest.base).bg(app.theme.bg.unwrap())),
            f.area(),
        );
        for (row, text) in glyphs.iter().enumerate() {
            f.render_widget(*text, Rect::new(2, row as u16 * 2 + 2, w - 4, 1));
        }
        f.render_widget(
            Paragraph::new("Repeated replay should leave no extra cells or style changes."),
            Rect::new(2, h - 2, w - 4, 1),
        );
    })
    .unwrap();
    out.push(("glyph widths".into(), 1400, term.backend().buffer().clone()));
    // Both mascot sizes; the companion's small face was absent from the
    // first replay and exposed the font-leading artifact most clearly.
    for ms in [0, 1500, 1700, 2900] {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        buf.set_style(area, ratatui::style::Style::default().bg(app.theme.bg.unwrap()));
        for (size, x) in [(crate::creature::Size::Small, 3), (crate::creature::Size::Medium, 23)] {
            let stage =
                crate::creature::stage_for(size, crate::creature::Anim::Seat, ms, &app.theme)
                    .unwrap();
            stage.paint(&mut buf, x, 3, area);
        }
        out.push((format!("mascot faces {ms} ms"), 1200, buf));
    }
    out.extend(crate::intro::comparison_frames(&app.theme, w, h));
    out
}

#[test]
fn terminal_comparison_export() {
    let mut sets = Vec::new();
    for flavor in [Flavor::Graphite, Flavor::Chalk] {
        for (w, h) in [(120, 30), (60, 20)] {
            for (endpoint, program) in
                [("native", Some("iTerm.app")), ("apple", Some("Apple_Terminal"))]
            {
                let colored = crate::underline::colors_supported(program);
                let out = Output::default();
                let area = Rect::new(0, 0, w, h);
                let mut diff = terminal(out.clone(), area, program);
                let mut captures = Vec::new();
                for (name, ms, buffer) in frames(flavor, colored, w, h) {
                    paint(&mut diff, &buffer);
                    let delta = out.take();
                    paint(&mut diff, &buffer);
                    assert!(out.take().is_empty(), "{endpoint}/{name}: idle output");
                    let full = Output::default();
                    paint(&mut terminal(full.clone(), area, program), &buffer);
                    let full = full.take();
                    if !colored {
                        assert!(!delta.contains("\x1b[58"), "{name}: unsupported underline");
                    }
                    assert!(!delta.contains("\x1b[6n"), "{name}: cursor query");
                    captures.push(json!({"name": name, "ms": ms, "diff": delta, "full": full, "text": lines_of(&buffer)}));
                }
                sets.push(json!({"theme": flavor.name(), "endpoint": endpoint, "width": w, "height": h, "frames": captures}));
            }
        }
    }
    let document = json!({"version": 1, "sets": sets});
    if let Some(path) = std::env::var_os("MESIMON_TERMINAL_COMPARISON") {
        std::fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
    }
    assert_eq!(document["sets"].as_array().map(Vec::<Value>::len), Some(8));
}

#[test]
fn plain_progress_preserves_the_completed_share_and_the_rgb_card() {
    for flavor in Flavor::ALL {
        for focused in [false, true] {
            let mut reference = app_summary();
            reference.theme = Theme::new(flavor, Profile::TrueColor);
            reference.cursor_col = if focused { 1 } else { 0 };
            let before = cells(&reference, 120, 30);
            reference.theme.underline_colors = false;
            let after = cells(&reference, 120, 30);
            let y = lines_of(&before).iter().position(|l| l.contains("Fix OSC-11")).unwrap() as u16;
            let mut run = 0;
            let mut track = 0;
            for x in 0..120 {
                let (a, b) = (&before[(x, y)], &after[(x, y)]);
                assert_eq!((a.symbol(), a.fg, a.bg), (b.symbol(), b.fg, b.bg));
                if a.modifier.contains(Modifier::UNDERLINED) {
                    let done = a.underline_color == reference.theme.calm;
                    run += usize::from(done);
                    track += usize::from(!done);
                    assert_eq!(
                        b.modifier.contains(Modifier::UNDERLINED),
                        done,
                        "{flavor:?}/{focused}/{x}"
                    );
                    assert_eq!(b.underline_color, Color::Reset);
                }
            }
            assert!(run > 0 && track > run, "two of five must remain a partial run");
        }
    }
}
