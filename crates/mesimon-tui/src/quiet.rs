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
    inner: CrosstermBackend<crate::underline::UnderlineColors<W>>,
    /// What the terminal was last told each cell is (T-696). iTerm2 does
    /// not repaint a cell whose only change is its underline colour — a
    /// whole line rewritten shows the new colour, one cell rewritten keeps
    /// the old (measured 2026-10-08, iTerm2 3.6) — and a sweep on the
    /// underline channel is exactly that, cell by cell. Such a cell is
    /// written twice in the frame: first without its underline, then as it
    /// should be, two attribute changes the terminal honours, inside the
    /// same synchronized update so neither is seen on its own.
    shadow: std::collections::HashMap<(u16, u16), Cell>,
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
        Self::for_terminal(writer, std::env::var("TERM_PROGRAM").ok().as_deref())
    }

    pub(crate) fn for_terminal(writer: W, terminal_program: Option<&str>) -> Self {
        Self {
            inner: CrosstermBackend::new(crate::underline::UnderlineColors::new(
                writer,
                terminal_program,
            )),
            shadow: std::collections::HashMap::new(),
            hidden: None,
            at: None,
            framing: false,
            open: false,
        }
    }

    /// A hidden cursor goes to the bottom-right cell, the same cell every
    /// frame (T-760). It is still where macOS draws the input-source
    /// indicator: iTerm2 reports the caret to the text system hidden or
    /// not, and the indicator stands under it. Left where the last changed
    /// cell put it, the caret sat beside a working card's spinner and moved
    /// with every tick, and the indicator, which fades only once the caret
    /// is still, stayed on the board. Parked, it moves only when the window
    /// does, and it leaves the indicator below the last row, off the board.
    /// A frame that wrote nothing costs nothing: the cursor is already there.
    fn park(&mut self, area: ratatui::layout::Rect) -> io::Result<()> {
        let corner =
            Position { x: area.right().saturating_sub(1), y: area.bottom().saturating_sub(1) };
        if self.at == Some(corner) {
            return Ok(());
        }
        self.begin()?;
        self.inner.set_cursor_position(corner)?;
        self.at = Some(corner);
        Ok(())
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
    let mut area = ratatui::layout::Rect::default();
    let drawn = terminal
        .draw(|f| {
            area = f.area();
            render(f)
        })
        .map(drop);
    let backend = terminal.backend_mut();
    if drawn.is_ok() && backend.hidden == Some(true) {
        backend.park(area)?;
    }
    backend.framing = false;
    if std::mem::take(&mut backend.open) {
        queue!(backend.inner, EndSynchronizedUpdate)?;
        Backend::flush(&mut backend.inner)?;
    }
    drawn
}

/// Clear our fullscreen viewport and invalidate ratatui's previous frame.
/// `Terminal::clear` preserves the cursor by querying it (CSI 6n); a slow
/// or non-answering terminal stalls there for up to two seconds. Our next
/// frame places the cursor itself. Resize clears without that round trip;
/// size is a local ioctl. Also used by the intro's no-reply pty test.
pub(crate) fn clear<B: Backend>(terminal: &mut ratatui::Terminal<B>) -> Result<(), B::Error> {
    let size = terminal.size()?;
    terminal.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height))
}

/// A wide glyph's covered columns, written before the glyph (T-540).
///
/// ratatui follows a VS16 emoji (the header's `☕️`) with its covered column
/// whenever that column's symbol changed, and crossterm's backend writes a
/// cell at the column after the last one without moving the cursor. A
/// terminal that drew the emoji two columns wide is already past that
/// column, so the blank and every changed cell after it land one column
/// right — and stay there, mixed with later writes that do move the cursor:
/// the memory chip read `0190GiB` for `0.9GiB`. Written first, every covered
/// column is reached by a cursor move; the glyph then covers it in a terminal
/// that draws it wide and leaves it cleared in one that draws it narrow,
/// which is what ratatui wrote it for.
fn covered_first<'a>(
    cells: impl Iterator<Item = (u16, u16, &'a Cell)>,
) -> Vec<(u16, u16, &'a Cell)> {
    use ratatui::buffer::CellWidth;
    let mut out: Vec<(u16, u16, &Cell)> = Vec::new();
    // Where in `out` the last wide glyph sits, while its columns may follow.
    let mut wide: Option<usize> = None;
    for (x, y, cell) in cells {
        if let Some(at) = wide {
            let (wx, wy, glyph) = out[at];
            if y == wy && x < wx.saturating_add(glyph.cell_width()) {
                out.insert(at, (x, y, cell));
                wide = Some(at + 1);
                continue;
            }
        }
        wide = (cell.cell_width() > 1).then_some(out.len());
        out.push((x, y, cell));
    }
    out
}

/// The same glyph, inks and attributes, and a different underline colour:
/// the one change iTerm2 does not draw.
fn only_underline_colour_differs(was: &Cell, now: &Cell) -> bool {
    was.underline_color != now.underline_color
        && was.symbol() == now.symbol()
        && was.fg == now.fg
        && was.bg == now.bg
        && was.modifier == now.modifier
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
        let mut cells: Vec<(u16, u16, Cell)> = Vec::new();
        for (x, y, cell) in covered_first(content) {
            if self.shadow.get(&(x, y)).is_some_and(|was| only_underline_colour_differs(was, cell))
            {
                let mut bare = cell.clone();
                bare.modifier.remove(ratatui::style::Modifier::UNDERLINED);
                cells.push((x, y, bare));
            }
            cells.push((x, y, cell.clone()));
            self.shadow.insert((x, y), cell.clone());
        }
        self.inner.draw(cells.iter().map(|(x, y, c)| (*x, *y, c)))
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
        self.shadow.clear();
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
        (Terminal::with_options(Quiet::for_terminal(tty.clone(), None), options).unwrap(), tty)
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
    fn fullscreen_clear_repaints_an_unchanged_frame_after_external_damage() {
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(12, 2)).unwrap();
        let paint = |f: &mut ratatui::Frame| f.render_widget("board", f.area());
        terminal.draw(paint).unwrap();
        let expected = terminal.backend().buffer().clone();
        // An escaped write or a handover has invalidated what ratatui
        // remembers. Clearing only the backend would leave this blank.
        terminal.backend_mut().clear().unwrap();
        clear(&mut terminal).unwrap();
        terminal.draw(paint).unwrap();
        assert_eq!(terminal.backend().buffer(), &expected);
    }

    /// A cell whose only change is its underline colour is written twice
    /// (T-696): bare first, then underlined in the new colour — iTerm2
    /// repaints on the attribute and not on the colour alone. A cell that
    /// changes anything else is written once, as ever.
    #[test]
    fn an_underline_colour_change_is_written_as_two_attribute_changes() {
        // This fixture tests colour bytes. Agent shells often set NO_COLOR;
        // crossterm otherwise suppresses the very output asserted below.
        // All byte-colour fixtures use the same override; no env mutation.
        ratatui::crossterm::style::force_color_output(true);
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        let (mut t, tty) = term();
        let under =
            |c: Color| Style::default().add_modifier(Modifier::UNDERLINED).underline_color(c);
        let red = Color::Rgb(255, 60, 60);
        let blue = Color::Rgb(60, 60, 255);
        draw(&mut t, |f| f.render_widget(Line::from(Span::styled("ab cd", under(red))), f.area()))
            .unwrap();
        let first = tty.take();
        assert_eq!(first.matches("\x1b[4m").count(), 1, "{first:?}");
        draw(&mut t, |f| f.render_widget(Line::from(Span::styled("ab cd", under(blue))), f.area()))
            .unwrap();
        let recoloured = tty.take();
        // Five cells, each bare then underlined: the underline is taken per
        // cell and dropped before the next (the first bare write starts
        // from nothing, so four drops), and the blue colour rides along.
        assert_eq!(recoloured.matches("\x1b[24m").count(), 4, "{recoloured:?}");
        assert_eq!(recoloured.matches("\x1b[4m").count(), 5, "{recoloured:?}");
        assert!(recoloured.contains("58:2::60:60:255"), "{recoloured:?}");
        assert!(!recoloured.contains("58;"), "ungrouped color components: {recoloured:?}");
        assert!(recoloured.starts_with(BEGIN) && recoloured.ends_with(END), "{recoloured:?}");
        // Unchanged: nothing. A glyph change: once.
        draw(&mut t, |f| f.render_widget(Line::from(Span::styled("ab cd", under(blue))), f.area()))
            .unwrap();
        assert_eq!(tty.take(), "");
        draw(&mut t, |f| f.render_widget(Line::from(Span::styled("ab cD", under(blue))), f.area()))
            .unwrap();
        let glyph = tty.take();
        assert!(glyph.contains('D') && !glyph.contains("\x1b[24m"), "{glyph:?}");
    }

    #[test]
    fn leaving_a_card_on_apple_terminal_keeps_its_text_style_and_plain_underline() {
        ratatui::crossterm::style::force_color_output(true);
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        let actual = Tty::default();
        let reference = Tty::default();
        let options = || TerminalOptions { viewport: Viewport::Fixed(Rect::new(0, 0, 24, 2)) };
        let mut apple = Terminal::with_options(
            Quiet::for_terminal(actual.clone(), Some("Apple_Terminal")),
            options(),
        )
        .unwrap();
        let mut plain =
            Terminal::with_options(Quiet::for_terminal(reference.clone(), None), options())
                .unwrap();
        for (frame, focused) in [true, false, false].into_iter().enumerate() {
            let style = Style::default()
                .fg(Color::Rgb(80, 200, 230))
                .bg(if focused { Color::Rgb(65, 40, 75) } else { Color::Rgb(30, 30, 40) })
                .add_modifier(Modifier::UNDERLINED)
                .add_modifier(if focused { Modifier::BOLD } else { Modifier::empty() });
            for (terminal, custom) in [(&mut apple, true), (&mut plain, false)] {
                draw(terminal, |f| {
                    let color = |rgb| if custom { style.underline_color(rgb) } else { style };
                    f.render_widget(
                        Line::from(vec![
                            Span::styled("prepare pro", color(Color::Rgb(41, 46, 107))),
                            Span::styled("duction", color(Color::Rgb(46, 41, 107))),
                        ]),
                        f.area(),
                    );
                })
                .unwrap();
            }
            let bytes = actual.take();
            // Exactly the stream produced without custom underline colors:
            // RGB ink, surface, bold removal, ordinary underline, and cursor.
            assert_eq!(bytes, reference.take(), "frame {frame}");
            assert!(!bytes.contains("\x1b[58"), "{bytes:?}");
            if frame == 2 {
                assert!(bytes.is_empty(), "the settled card stays silent");
            } else {
                assert!(bytes.contains("\x1b[4m"), "ordinary underline remains: {bytes:?}");
                assert!(bytes.contains("38;2;80;200;230"), "RGB ink remains: {bytes:?}");
            }
        }
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
        // Hidden, it is parked in the corner (T-760).
        frame(&mut t, "titlE", None);
        assert_eq!(tty.take(), format!("{BEGIN}\x1b[?25l\x1b[2;12H{END}"));
        frame(&mut t, "titlE", None);
        assert_eq!(tty.take(), "");
    }

    /// T-760: a hidden cursor rests in the same cell after every frame, so
    /// a spinner's tick never moves the caret macOS hangs its input-source
    /// indicator from.
    #[test]
    fn a_hidden_cursor_rests_in_the_corner_whatever_the_frame_wrote() {
        let (mut t, tty) = term();
        let mut seen = String::new();
        for text in ["⠋ working", "⠙ working", "⠹ working"] {
            frame(&mut t, text, None);
            let bytes = tty.take();
            assert!(bytes.ends_with(&format!("\x1b[2;12H{END}")), "{bytes:?}");
            seen += &bytes;
        }
        assert_eq!(play(&seen, (12, 2))[0], "⠹ working   ");
        // Nothing changed: not a byte, the cursor is already parked.
        frame(&mut t, "⠹ working", None);
        assert_eq!(tty.take(), "");
    }

    /// The tty's bytes played onto a grid as a terminal plays them: `CUP`
    /// places the cursor, any other CSI is ignored, a glyph fills as many
    /// columns as it is wide and moves the cursor past all of them, and a
    /// write into either half of a wide glyph erases the whole of it.
    fn play(bytes: &str, (width, height): (usize, usize)) -> Vec<String> {
        use unicode_segmentation::UnicodeSegmentation;
        // A covered column is "": the row's concatenation is what shows.
        let mut grid = vec![vec![" ".to_string(); width]; height];
        let (mut x, mut y) = (0, 0);
        let mut rest = bytes;
        while !rest.is_empty() {
            if let Some(csi) = rest.strip_prefix("\x1b[") {
                let end = csi.find(|c: char| ('@'..='~').contains(&c)).unwrap();
                if csi[end..].starts_with('H') {
                    let mut at = csi[..end].split(';').map(|n| n.parse::<usize>().unwrap_or(1));
                    y = at.next().unwrap_or(1) - 1;
                    x = at.next().unwrap_or(1) - 1;
                }
                rest = &csi[end + 1..];
                continue;
            }
            let run = rest.find('\x1b').unwrap_or(rest.len());
            for g in rest[..run].graphemes(true) {
                let w = unicode_width::UnicodeWidthStr::width(g).max(1);
                let row = &mut grid[y];
                if row[x].is_empty() {
                    row[x - 1] = " ".into();
                }
                if row.get(x + w).is_some_and(String::is_empty) {
                    row[x + w] = " ".into();
                }
                row[x] = g.into();
                for covered in &mut row[x + 1..x + w] {
                    covered.clear();
                }
                x += w;
            }
            rest = &rest[run..];
        }
        grid.into_iter().map(|row| row.concat()).collect()
    }

    /// T-540: the header's `☕️` moved onto a column that held text and the
    /// chip after it read `0190GiB`. ratatui follows a VS16 emoji with its
    /// covered column whenever that column changed, and crossterm writes a
    /// cell one past the last without moving the cursor — so the blank and
    /// every changed cell after it landed one column right.
    #[test]
    fn a_cell_after_an_emoji_lands_in_its_own_column() {
        let (mut t, tty) = term();
        frame(&mut t, "abcdefghij", None);
        let mut seen = tty.take();
        frame(&mut t, "☕️bcdefghi", None);
        seen += &tty.take();
        assert_eq!(play(&seen, (12, 2))[0], "☕️bcdefghi  ");
        // And back: the text that replaces it writes over both columns.
        frame(&mut t, "xyzdefghi", None);
        seen += &tty.take();
        assert_eq!(play(&seen, (12, 2))[0], "xyzdefghi   ");
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
