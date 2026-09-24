//! First-attach practice, running inside the real private tmux server.
//! No key exits this process: only tmux's detach binding proves the way back.

use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::{cursor, event, execute, terminal};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{Frame, Terminal};

use crate::theme::Theme;

/// Internal `mesimon detach-guide` entry point. Uses the same preferences and
/// palette as the board, but opens no daemon connection and writes no settings.
pub fn run() -> Result<()> {
    let detected = crate::detect::detect();
    let prefs = crate::prefs::prefs_path()
        .as_deref()
        .map(crate::prefs::load)
        .map(|loaded| loaded.prefs)
        .unwrap_or_default();
    let board = crate::prefs::board_prefs_path(&std::env::current_dir()?)
        .as_deref()
        .map(crate::prefs::load_board)
        .unwrap_or_default();
    let resolved = prefs.overlay(&board.prefs);
    let theme = Theme::new(
        detected.forced.unwrap_or(resolved.for_ground(detected.ground)),
        detected.profile,
    );

    terminal::enable_raw_mode()?;
    let _restore = Restore;
    let mut stdout = std::io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    terminal.clear()?;
    let start = Instant::now();
    loop {
        terminal.draw(|f| draw(f, &theme, start.elapsed()))?;
        // Drain input without letting Enter, Escape or Ctrl+C stand in for
        // a real detach. Poll also wakes the drawing loop on terminal resize.
        if event::poll(Duration::from_millis(100))? {
            let _ = event::read()?;
        }
    }
}

struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(std::io::stdout(), terminal::LeaveAlternateScreen, cursor::Show);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Ready,
    Hold,
    Tap,
    Back,
}

impl Step {
    fn at(elapsed: Duration) -> Self {
        match elapsed.as_millis() % 4800 {
            0..1200 => Self::Ready,
            1200..2200 => Self::Hold,
            2200..3000 => Self::Tap,
            _ => Self::Back,
        }
    }
}

fn draw(f: &mut Frame, theme: &Theme, elapsed: Duration) {
    let area = f.area();
    let mut base = theme.base();
    if let Some(bg) = theme.bg {
        base = base.bg(bg);
    }
    f.render_widget(Block::default().style(base), area);
    if area.is_empty() {
        return;
    }
    let step = Step::at(elapsed);
    // Whitespace and painted surfaces, like the board's screen chips and
    // footer. No dialog border or separate visual vocabulary.
    let key = |label: &'static str, pressed: bool| {
        let style = if pressed {
            theme.selected_row().fg(theme.sel.base).add_modifier(Modifier::BOLD)
        } else {
            theme.dim1()
        };
        Span::styled(label, style)
    };
    let route = Line::from(vec![
        Span::styled("SESSION", if step == Step::Back { theme.dim2() } else { theme.base() }),
        Span::styled(
            if theme.glyph_tier() == crate::glyphs::Tier::Ascii { "   >   " } else { "   →   " },
            theme.dim2(),
        ),
        Span::styled(
            "BOARD",
            if step == Step::Back {
                theme.base().add_modifier(Modifier::BOLD)
            } else {
                theme.dim2()
            },
        ),
    ]);
    let keys = Line::from(vec![
        key(" Ctrl ", matches!(step, Step::Hold | Step::Tap)),
        Span::styled(" + ", theme.dim2()),
        key(" 5 ", step == Step::Tap),
    ]);
    let rows = vec![
        Line::styled("Before we connect you,", theme.base().add_modifier(Modifier::BOLD)),
        Line::styled("here's your shortcut back to the board.", theme.dim1()),
        Line::default(),
        route,
        Line::default(),
        keys,
        Line::styled(
            match step {
                Step::Ready => "Try it once",
                Step::Hold => "Hold Ctrl",
                Step::Tap => "Tap 5",
                Step::Back => "Back to the board",
            },
            theme.dim1(),
        ),
        Line::default(),
        Line::styled("Ctrl + ] works too", theme.dim2()),
        Line::styled("Give it a try, then you're in.", theme.dim2()),
    ];
    // A resized, short pane keeps the actionable key and its destination.
    let rows = if area.height < 10 || area.width < 40 {
        vec![
            Line::styled("Back to your board", theme.base().add_modifier(Modifier::BOLD)),
            rows[5].clone(),
            rows[8].clone(),
        ]
    } else {
        rows
    };
    let height = (rows.len() as u16).min(area.height);
    let content = Rect::new(area.x, area.y + (area.height - height) / 2, area.width, height);
    f.render_widget(Paragraph::new(rows).centered(), content);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};
    use ratatui::backend::TestBackend;

    #[test]
    fn animation_holds_control_while_tapping_then_releases() {
        for (ms, step) in [
            (0, Step::Ready),
            (1200, Step::Hold),
            (2200, Step::Tap),
            (3000, Step::Back),
            (4800, Step::Ready),
        ] {
            assert_eq!(Step::at(Duration::from_millis(ms)), step);
        }
    }

    #[test]
    fn animation_paints_the_keys_together_and_then_the_destination() {
        let theme = Theme::new(Flavor::Graphite, Profile::Mono);
        for (ms, ctrl, five, board) in [
            (0, false, false, false),
            (1200, true, false, false),
            (2200, true, true, false),
            (3000, false, false, true),
        ] {
            let mut t = Terminal::new(TestBackend::new(60, 19)).unwrap();
            t.draw(|f| draw(f, &theme, Duration::from_millis(ms))).unwrap();
            let b = t.backend().buffer();
            for (y, word, bold) in [(9, "Ctrl", ctrl), (9, "5", five), (7, "BOARD", board)] {
                let row: String = (0..60).map(|x| b[(x, y)].symbol()).collect();
                let x = row.find(word).unwrap() as u16;
                assert_eq!(b[(x, y)].modifier.contains(Modifier::BOLD), bold, "{ms}: {word}");
            }
        }
    }

    #[test]
    fn guide_preserves_keys_on_small_panes_and_uses_only_theme_ink() {
        for profile in
            [Profile::TrueColor, Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono]
        {
            for flavor in Flavor::ALL {
                let theme = Theme::new(flavor, profile);
                for (w, h) in [(80, 24), (60, 19), (30, 8), (1, 1), (0, 0)] {
                    for ms in [0, 1200, 2200, 3000] {
                        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                        t.draw(|f| draw(f, &theme, Duration::from_millis(ms))).unwrap();
                        let b = t.backend().buffer();
                        let text: String = b.content.iter().map(|c| c.symbol()).collect();
                        if w >= 30 {
                            assert!(text.contains(" Ctrl  +  5 "));
                            assert!(text.contains("Ctrl + ] works too"));
                        }
                        for cell in &b.content {
                            assert!([
                                theme.rest.base,
                                theme.rest.dim1,
                                theme.rest.dim2,
                                theme.sel.base
                            ]
                            .contains(&cell.fg));
                            assert!(!cell.modifier.intersects(
                                Modifier::SLOW_BLINK | Modifier::RAPID_BLINK | Modifier::DIM
                            ));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn guide_golden() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let mut t = Terminal::new(TestBackend::new(60, 19)).unwrap();
        t.draw(|f| draw(f, &theme, Duration::from_millis(2200))).unwrap();
        let text = t
            .backend()
            .buffer()
            .content
            .chunks(60)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>().trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_owned()
            + "\n";
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/golden/detach_guide.txt");
        if std::env::var_os("MESIMON_UPDATE_GOLDEN").is_some() {
            std::fs::write(path, text).unwrap();
        } else {
            assert_eq!(text, std::fs::read_to_string(path).unwrap());
        }
    }
}
