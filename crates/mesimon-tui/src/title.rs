//! The terminal's own tab, named after the board (T-492).
//!
//! Opt-in (`prefs.rs::tab_title`, per machine, off by default): a title is
//! the terminal's, and renaming somebody's tab is a thing they ask for. On,
//! the tab reads `mesimon ∙ <board>` — `2 need you ∙ <board>` while any
//! ticket does — and through a focus handover the ticket whose pane took
//! the terminal: `T-12 fix the parser`. The board WRITES the title and
//! never reads it: `CSI 21 t` (report the title) is refused by most
//! terminals for the same reason `OSC 52` reads are, and a reply would land
//! on stdin as keystrokes, the trap `osc.rs` exists for. So the terminal's
//! own title comes back through the xterm title STACK instead — `CSI 22;0
//! t` saves it before the first write and `CSI 23;0 t` restores it at the
//! end — which iTerm2, ghostty, kitty, WezTerm, foot, xterm and
//! Terminal.app honour, and a terminal that does not simply keeps the last
//! words the board set, which are still true.
//!
//! Three rules. **The pane's title never reaches the tab**: the private
//! tmux server keeps `set-titles` off, so an agent's own `OSC 0` (Claude
//! Code's `✳ …`) stops at `#{pane_title}` and the ticket's words stay up
//! for the whole focus. **Every write is a change**: `sync` compares with
//! the last words sent and writes nothing otherwise, so a 250 ms tick costs
//! the tty no bytes. **Every word crosses `scrub_text`**: a ticket title is
//! the user's, a board name is a directory's, and the BEL and ESC that
//! function strips are exactly what would close the sequence early.

use std::io::Write;

use mesimon_core::text;

/// A tab is a few dozen cells wide on every terminal that has one; a title
/// clipped here reads better than one the terminal cuts mid-word. On a
/// word boundary, with an ellipsis, the notification's rule.
const TITLE_CHARS: usize = 48;

/// The board's own words: the directory (or a joined board's title) with
/// the count of tickets that need you in front of it while any do.
pub(crate) fn board(name: &str, needs_you: usize) -> String {
    let name = text::scrub_text(name);
    if needs_you == 0 {
        format!("mesimon ∙ {name}")
    } else {
        format!("{needs_you} need you ∙ {name}")
    }
}

/// A focus handover's words: the ticket's key and title.
pub(crate) fn focus(key: &str, ticket_title: &str) -> String {
    let title = clip(&text::scrub_text(ticket_title), TITLE_CHARS);
    if title.is_empty() {
        text::scrub_text(key)
    } else {
        format!("{} {title}", text::scrub_text(key))
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    let cut = match head.rfind(char::is_whitespace) {
        Some(i) if i * 2 >= max => i,
        _ => head.len(),
    };
    format!("{}…", head[..cut].trim_end())
}

/// What the tab has been told, so a frame that changes nothing writes
/// nothing, and so the terminal's own title is saved exactly once.
#[derive(Debug, Default)]
pub(crate) struct Tab {
    /// `CSI 22;0 t` has been written and not yet popped.
    pushed: bool,
    /// The last words written.
    last: Option<String>,
}

impl Tab {
    /// Bring the tab to `words`, or with `None` (the preference is off)
    /// give the terminal its own title back. Writes only on a change.
    pub(crate) fn sync(
        &mut self,
        out: &mut impl Write,
        words: Option<&str>,
    ) -> std::io::Result<()> {
        let Some(words) = words else {
            return self.finish(out);
        };
        if self.last.as_deref() == Some(words) {
            return Ok(());
        }
        if !self.pushed {
            out.write_all(b"\x1b[22;0t")?;
            self.pushed = true;
        }
        // OSC 0: icon name and window title both, which is what a tab
        // shows on every terminal — iTerm2's tab takes the icon name.
        write!(out, "\x1b]0;{words}\x07")?;
        out.flush()?;
        self.last = Some(words.to_string());
        Ok(())
    }

    /// The terminal's own title back, if ours ever went up. Idempotent:
    /// the loop calls it before a suspend, a reload and the exit.
    pub(crate) fn finish(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        if !self.pushed {
            return Ok(());
        }
        out.write_all(b"\x1b[23;0t")?;
        out.flush()?;
        self.pushed = false;
        self.last = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_board_words_lead_with_the_count_when_any_need_you() {
        assert_eq!(board("api", 0), "mesimon ∙ api");
        assert_eq!(board("api", 2), "2 need you ∙ api");
    }

    #[test]
    fn a_focus_names_the_ticket_and_clips_a_long_title_on_a_word() {
        assert_eq!(focus("T-12", "fix the parser"), "T-12 fix the parser");
        assert_eq!(focus("T-12", ""), "T-12");
        let long = "a ".repeat(40) + "tail";
        let f = focus("T-12", &long);
        assert!(f.ends_with('…'), "{f}");
        assert!(f.chars().count() <= TITLE_CHARS + "T-12 ".len() + 1, "{f}");
    }

    /// A BEL or an ESC inside a title would end the sequence early and
    /// type the rest — the scrub is what makes the write safe.
    #[test]
    fn a_control_in_a_title_never_reaches_the_terminal() {
        let f = focus("T-12", "fix\x07 the\x1b]0;x\x07 parser");
        assert!(!f.contains('\x07') && !f.contains('\x1b'), "{f}");
        assert_eq!(board("api\x1b", 0), "mesimon ∙ api");
    }

    /// Push once before the first words, write only on a change, pop once
    /// at the end, and a pop with nothing up writes nothing.
    #[test]
    fn the_stack_is_pushed_once_and_popped_once_and_repeats_write_nothing() {
        let mut out = Vec::new();
        let mut tab = Tab::default();
        tab.finish(&mut out).unwrap();
        assert!(out.is_empty(), "nothing up, nothing to pop");
        tab.sync(&mut out, Some("mesimon ∙ api")).unwrap();
        assert_eq!(out, b"\x1b[22;0t\x1b]0;mesimon \xe2\x88\x99 api\x07");
        out.clear();
        tab.sync(&mut out, Some("mesimon ∙ api")).unwrap();
        assert!(out.is_empty(), "the same words again cost the tty nothing");
        tab.sync(&mut out, Some("1 need you ∙ api")).unwrap();
        assert_eq!(out, b"\x1b]0;1 need you \xe2\x88\x99 api\x07", "no second push");
        out.clear();
        tab.sync(&mut out, None).unwrap();
        assert_eq!(out, b"\x1b[23;0t", "off is the pop");
        out.clear();
        tab.finish(&mut out).unwrap();
        assert!(out.is_empty(), "already popped");
        // Back on: pushed again, so the stack stays balanced.
        tab.sync(&mut out, Some("mesimon ∙ api")).unwrap();
        assert!(out.starts_with(b"\x1b[22;0t"));
    }
}
