//! The board's backend: crossterm's, minus every byte a frame that changes
//! nothing would write (T-496).
//!
//! The loop draws every tick — 100 ms, the working spinner's pace — and
//! ratatui sends only the cells that changed. But an unchanged frame still
//! wrote about thirty bytes: crossterm's backend ends every `draw` with an
//! SGR reset even when the diff is empty, ratatui hides (or shows and
//! moves) the cursor after every frame, and the loop bracketed each frame
//! in `DECSET 2026`. iTerm2 counts any byte as output: a session that read
//! one in the last two seconds (the `idleTimeSeconds` advanced setting) is
//! "processing", and a BACKGROUND tab with a processing session wears the
//! activity spinner (`PTYTab.isProcessing`, unless Settings › Appearance ›
//! Tabs hides it). So a board with nothing running spun in the tab strip
//! for as long as its tab was not the front one, beside a progress ring
//! (`title.rs`) that was correctly off.
//!
//! `Quiet` holds the tab's own rule — a tick costs the tty no bytes — for
//! the frame too: an empty diff writes nothing, a cursor call that repeats
//! what the terminal was last told writes nothing, and the synchronized
//! update opens on a frame's first real write and closes at its end, so it
//! is spent only on a frame that changes something. Outside `draw` (the
//! ^L clear, the one inside `init_terminal`) nothing opens one: a
//! synchronized update left open across a handover would freeze the pane
//! the terminal is handed to.

use std::io::{self, Write};

use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::crossterm::queue;
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::layout::{Position, Size};

pub(crate) struct Quiet<W: Write> {
    inner: CrosstermBackend<W>,
    /// What the cursor was last told: hidden or shown. `None` until it has
    /// been told anything, so the first frame always says.
    hidden: Option<bool>,
    /// Where the cursor was last put. `None` once a draw or a clear has
    /// moved it, so a frame that wants it somewhere puts it there again.
    at: Option<Position>,
    /// Inside `draw`: the first write opens a synchronized update.
    framing: bool,
    /// A synchronized update is open and `draw` closes it.
    open: bool,
}

impl<W: Write> Quiet<W> {
    pub(crate) fn new(writer: W) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
            hidden: None,
            at: None,
            framing: false,
            open: false,
        }
    }

    /// Called before every write: inside a frame, the first one opens the
    /// synchronized update.
    fn begin(&mut self) -> io::Result<()> {
        if self.framing && !self.open {
            queue!(self.inner, BeginSynchronizedUpdate)?;
            self.open = true;
        }
        Ok(())
    }
}

/// One frame of the board: rendered every time, written only where it
/// differs from the last, and in one synchronized update when it writes
/// anything — a terminal that honours `DECSET 2026` (iTerm2, kitty,
/// ghostty, WezTerm, foot, tmux ≥3.4) shows it whole, and one that does not
/// ignores the two sequences.
pub(crate) fn draw<W: Write>(
    terminal: &mut ratatui::Terminal<Quiet<W>>,
    render: impl FnOnce(&mut ratatui::Frame),
) -> io::Result<()> {
    terminal.backend_mut().framing = true;
    let drawn = terminal.draw(render).map(drop);
    let backend = terminal.backend_mut();
    backend.framing = false;
    if std::mem::take(&mut backend.open) {
        queue!(backend.inner, EndSynchronizedUpdate)?;
        Backend::flush(&mut backend.inner)?;
    }
    drawn
}

impl<W: Write> Backend for Quiet<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut content = content.peekable();
        if content.peek().is_none() {
            return Ok(());
        }
        self.begin()?;
        self.at = None;
        self.inner.draw(content)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.begin()?;
        self.at = None;
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        if self.hidden == Some(true) {
            return Ok(());
        }
        self.begin()?;
        self.inner.hide_cursor()?;
        self.hidden = Some(true);
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        if self.hidden == Some(false) {
            return Ok(());
        }
        self.begin()?;
        self.inner.show_cursor()?;
        self.hidden = Some(false);
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let position = position.into();
        if self.at == Some(position) {
            return Ok(());
        }
        self.begin()?;
        self.inner.set_cursor_position(position)?;
        self.at = Some(position);
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.clear_region(ClearType::All)
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.begin()?;
        self.at = None;
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use ratatui::layout::Rect;
    use ratatui::{Terminal, TerminalOptions, Viewport};

    use super::*;

    /// The tty: every byte the backend wrote since the last `take`.
    #[derive(Clone, Default)]
    struct Tty(Rc<RefCell<Vec<u8>>>);

    impl Write for Tty {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Tty {
        fn take(&self) -> String {
            String::from_utf8(std::mem::take(&mut *self.0.borrow_mut())).unwrap()
        }
    }

    const BEGIN: &str = "\x1b[?2026h";
    const END: &str = "\x1b[?2026l";

    /// A fixed viewport: no size query, so no tty is needed.
    fn term() -> (Terminal<Quiet<Tty>>, Tty) {
        let tty = Tty::default();
        let options = TerminalOptions { viewport: Viewport::Fixed(Rect::new(0, 0, 12, 2)) };
        (Terminal::with_options(Quiet::new(tty.clone()), options).unwrap(), tty)
    }

    fn frame(t: &mut Terminal<Quiet<Tty>>, text: &str, cursor: Option<(u16, u16)>) {
        draw(t, |f| {
            f.render_widget(text, f.area());
            if let Some(at) = cursor {
                f.set_cursor_position(at);
            }
        })
        .unwrap();
    }

    #[test]
    fn a_frame_that_changes_nothing_writes_nothing() {
        let (mut t, tty) = term();
        frame(&mut t, "board", None);
        let first = tty.take();
        assert!(first.starts_with(BEGIN) && first.ends_with(END), "{first:?}");
        assert!(first.contains("board") && first.contains("\x1b[?25l"), "{first:?}");
        // The idle board, a hundred ticks: the tab never hears from it.
        for _ in 0..100 {
            frame(&mut t, "board", None);
        }
        assert_eq!(tty.take(), "");
        frame(&mut t, "boarD", None);
        let changed = tty.take();
        assert!(changed.starts_with(BEGIN) && changed.ends_with(END), "{changed:?}");
        assert!(changed.contains('D') && !changed.contains("board"), "only the cell: {changed:?}");
        assert!(!changed.contains("\x1b[?25l"), "still hidden: {changed:?}");
    }

    #[test]
    fn the_cursor_is_told_only_what_it_was_not_told_last() {
        let (mut t, tty) = term();
        frame(&mut t, "title", Some((2, 1)));
        let first = tty.take();
        assert!(first.contains("\x1b[?25h") && first.ends_with(&format!("\x1b[2;3H{END}")));
        frame(&mut t, "title", Some((2, 1)));
        assert_eq!(tty.take(), "", "same cells, same cursor");
        // A cursor step with no cell changed: the move alone.
        frame(&mut t, "title", Some((3, 1)));
        assert_eq!(tty.take(), format!("{BEGIN}\x1b[2;4H{END}"));
        // A cell drawn moves the terminal's cursor, so it is put back even
        // though the frame asks for the same place.
        frame(&mut t, "titlE", Some((3, 1)));
        let typed = tty.take();
        assert!(typed.contains('E') && typed.ends_with(&format!("\x1b[2;4H{END}")), "{typed:?}");
        frame(&mut t, "titlE", None);
        assert_eq!(tty.take(), format!("{BEGIN}\x1b[?25l{END}"));
        frame(&mut t, "titlE", None);
        assert_eq!(tty.take(), "");
    }

    #[test]
    fn a_clear_outside_a_frame_opens_no_synchronized_update() {
        let (mut t, tty) = term();
        frame(&mut t, "board", Some((1, 0)));
        tty.take();
        t.backend_mut().clear_region(ClearType::All).unwrap();
        let cleared = tty.take();
        assert!(!cleared.contains(BEGIN) && !cleared.contains(END), "{cleared:?}");
        // The clear may have moved the cursor: the next frame places it.
        frame(&mut t, "board", Some((1, 0)));
        assert_eq!(tty.take(), format!("{BEGIN}\x1b[1;2H{END}"));
    }
}
