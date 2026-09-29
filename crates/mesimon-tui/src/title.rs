//! The terminal's own tab, named and marked after the board (T-492).
//!
//! Every row is opt-in (`prefs.rs`, per machine, off by default): a tab is
//! the terminal's, and renaming somebody's tab is a thing they ask for. The
//! board WRITES to the terminal and never reads it: `CSI 21 t` (report the
//! title) is refused by most terminals for the reason `OSC 52` reads are,
//! and a reply would land on stdin as keystrokes, the trap `osc.rs` exists
//! for. So the terminal's own title comes back through the xterm title
//! STACK instead — `CSI 22;0 t` saves it before the first write and `CSI
//! 23;0 t` restores it at the end — which iTerm2, ghostty, kitty, WezTerm,
//! foot, xterm and Terminal.app honour, and a terminal that does not simply
//! keeps the last words the board set, which are still true.
//!
//! What can be written, each behind its own row:
//!
//! - **the title** (`OSC 0`): `<board>` or `mesimon ∙ <board>`, `2 need you
//!   ∙ <board>` while any ticket does, and through a focus handover the
//!   ticket whose pane took the terminal (`T-12 fix the parser`);
//! - **a progress ring** (`OSC 9;4`, ConEmu's, drawn by iTerm2 3.6.6+,
//!   ghostty 1.2+, kitty, WezTerm and Windows Terminal): indeterminate
//!   while an agent is mid-turn, a full red bar while one needs you,
//!   cleared otherwise;
//! - **iTerm2's needs-you colour**: the tab's indicator dot (`OSC 21337
//!   indicator=`) or the whole tab's chrome (`OSC 6;1;bg`), in the theme's
//!   attention colour — the board's one-saturated-colour rule, on the tab
//!   strip;
//! - **iTerm2's subtitle** (`OSC 21337 status=`): how many need you and
//!   how many are working.
//!
//! Three rules. **The pane's title never reaches the tab**: the private
//! tmux server keeps `set-titles` off, so an agent's own `OSC 0` (Claude
//! Code's `✳ …`) stops at `#{pane_title}` and the ticket's words stay up
//! for the whole focus. **Every write is a change**: `sync` compares each
//! field with the last one sent and writes nothing otherwise, so a 250 ms
//! tick costs the tty no bytes. **Every word crosses `scrub_text`**: a
//! ticket title is the user's, a board name is a directory's, and the BEL
//! and ESC that function strips are exactly what would close the sequence
//! early. The iTerm2-only sequences are written only where the board runs
//! in iTerm2 directly (`terminal()`): every other terminal ignores `OSC
//! 1337`-family codes, but an outer tmux swallows them all, and a
//! `__CFBundleIdentifier` inherited through one names whatever started the
//! server — `notify.rs`'s veto, kept here.

use std::io::Write;

use mesimon_core::text;

/// A tab is a few dozen cells wide on every terminal that has one; a title
/// clipped here reads better than one the terminal cuts mid-word. On a
/// word boundary, with an ellipsis, the notification's rule.
const TITLE_CHARS: usize = 48;

/// Which terminal the board's stdout reaches, as far as the iTerm2-only
/// rows are concerned. Resolved from the environment in `lib.rs::run`,
/// never in `App::new`, so no test app reads a developer's terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Terminal {
    /// iTerm2 and no tmux in between.
    ITerm2,
    /// The user's own tmux: `TERM_PROGRAM` is rewritten to `tmux` in every
    /// pane, which is the one reliable negative — `__CFBundleIdentifier`
    /// is inherited and stale in there.
    OuterTmux,
    #[default]
    Other,
}

pub(crate) fn terminal() -> Terminal {
    classify(
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var("__CFBundleIdentifier").ok().as_deref(),
    )
}

fn classify(term_program: Option<&str>, bundle: Option<&str>) -> Terminal {
    if term_program.is_some_and(|v| v.eq_ignore_ascii_case("tmux")) {
        return Terminal::OuterTmux;
    }
    if bundle == Some("com.googlecode.iterm2") || term_program == Some("iTerm.app") {
        return Terminal::ITerm2;
    }
    Terminal::Other
}

/// The board runs in iTerm2 directly — what the doctor line and the row
/// details ask.
pub(crate) fn iterm2_direct() -> bool {
    terminal() == Terminal::ITerm2
}

/// The board's own words: the directory (or a joined board's title), with
/// `mesimon ∙ ` in front when asked, and the count of tickets that need
/// you in front of that while any do (and the row says to count).
pub(crate) fn board(name: &str, app_word: bool, needs_you: Option<usize>) -> String {
    let name = text::scrub_text(name);
    match needs_you {
        Some(n) if n > 0 => format!("{n} need you ∙ {name}"),
        _ if app_word => format!("mesimon ∙ {name}"),
        _ => name,
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

/// The subtitle's words: what needs you first, then what is working.
/// Empty when nothing is either, which clears the subtitle.
pub(crate) fn subtitle(needs_you: usize, working: usize) -> String {
    let mut parts = Vec::new();
    if needs_you > 0 {
        parts.push(format!("{needs_you} need you"));
    }
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    parts.join(" ∙ ")
}

/// The progress ring's state, precedence downward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Progress {
    /// Something needs you: a full bar in the error colour.
    Blocked,
    /// An agent is mid-turn: indeterminate.
    Working,
    #[default]
    None,
}

/// How iTerm2 is asked to mark the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Mark {
    #[default]
    Off,
    /// The indicator dot, in the given colour.
    Dot(u32),
    /// The whole tab's chrome, in the given colour.
    Tab(u32),
}

/// Everything one frame asks of the tab. `None` in a field means the row
/// is off: the terminal's own state is restored for it and nothing else
/// is written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Frame {
    pub title: Option<String>,
    pub progress: Option<Progress>,
    /// `Some(Mark::Off)` while the row is on and nothing needs you.
    pub mark: Option<Mark>,
    /// `Some("")` clears the subtitle.
    pub subtitle: Option<String>,
}

/// What the tab has been told, so a frame that changes nothing writes
/// nothing, and so the terminal's own title is saved exactly once.
#[derive(Debug, Default)]
pub(crate) struct Tab {
    /// `CSI 22;0 t` has been written and not yet popped.
    pushed: bool,
    last: Frame,
}

impl Tab {
    /// Bring the tab to `want`. Writes only what changed; a field that
    /// went from `Some` to `None` is given back to the terminal.
    pub(crate) fn sync(&mut self, out: &mut impl Write, want: &Frame) -> std::io::Result<()> {
        if *want == self.last {
            return Ok(());
        }
        if want.title != self.last.title {
            match &want.title {
                Some(words) => {
                    if !self.pushed {
                        out.write_all(b"\x1b[22;0t")?;
                        self.pushed = true;
                    }
                    // OSC 0: icon name and window title both, which is what
                    // a tab shows on every terminal.
                    write!(out, "\x1b]0;{words}\x07")?;
                }
                None => self.pop(out)?,
            }
        }
        if want.progress != self.last.progress {
            match want.progress.unwrap_or(Progress::None) {
                Progress::Blocked => out.write_all(b"\x1b]9;4;2;100\x07")?,
                Progress::Working => out.write_all(b"\x1b]9;4;3\x07")?,
                Progress::None => out.write_all(b"\x1b]9;4;0\x07")?,
            }
        }
        if want.mark != self.last.mark {
            let before = self.last.mark.unwrap_or(Mark::Off);
            let after = want.mark.unwrap_or(Mark::Off);
            // Each kind is reset on its own road: a dot is cleared by an
            // empty indicator, the chrome by `*;default`.
            if matches!(before, Mark::Dot(_)) && !matches!(after, Mark::Dot(_)) {
                out.write_all(b"\x1b]21337;indicator=\x07")?;
            }
            if matches!(before, Mark::Tab(_)) && !matches!(after, Mark::Tab(_)) {
                out.write_all(b"\x1b]6;1;bg;*;default\x07")?;
            }
            match after {
                Mark::Off => {}
                Mark::Dot(rgb) => write!(out, "\x1b]21337;indicator=#{rgb:06x}\x07")?,
                Mark::Tab(rgb) => {
                    let [_, r, g, b] = rgb.to_be_bytes();
                    write!(
                        out,
                        "\x1b]6;1;bg;red;brightness;{r}\x07\x1b]6;1;bg;green;brightness;{g}\x07\x1b]6;1;bg;blue;brightness;{b}\x07"
                    )?;
                }
            }
        }
        if want.subtitle != self.last.subtitle {
            let words = want.subtitle.as_deref().unwrap_or("");
            write!(out, "\x1b]21337;status={words}\x07")?;
        }
        out.flush()?;
        self.last = want.clone();
        Ok(())
    }

    /// The terminal's own state back, for every field that was ever set.
    /// Idempotent: the loop calls it before a suspend, a reload and the
    /// exit, and the next `sync` starts from nothing.
    pub(crate) fn finish(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        self.sync(out, &Frame::default())
    }

    fn pop(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        if self.pushed {
            out.write_all(b"\x1b[23;0t")?;
            self.pushed = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_board_words_follow_the_three_switches() {
        assert_eq!(board("api", false, None), "api");
        assert_eq!(board("api", true, None), "mesimon ∙ api");
        assert_eq!(board("api", true, Some(0)), "mesimon ∙ api");
        assert_eq!(board("api", true, Some(2)), "2 need you ∙ api");
        assert_eq!(board("api", false, Some(2)), "2 need you ∙ api");
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

    #[test]
    fn the_subtitle_counts_needs_you_first_and_is_empty_when_idle() {
        assert_eq!(subtitle(0, 0), "");
        assert_eq!(subtitle(2, 0), "2 need you");
        assert_eq!(subtitle(0, 3), "3 working");
        assert_eq!(subtitle(1, 3), "1 need you ∙ 3 working");
    }

    /// A BEL or an ESC inside a title would end the sequence early and
    /// type the rest — the scrub is what makes the write safe.
    #[test]
    fn a_control_in_a_title_never_reaches_the_terminal() {
        let f = focus("T-12", "fix\x07 the\x1b]0;x\x07 parser");
        assert!(!f.contains('\x07') && !f.contains('\x1b'), "{f}");
        assert_eq!(board("api\x1b", true, None), "mesimon ∙ api");
    }

    #[test]
    fn iterm2_is_named_directly_and_never_through_an_outer_tmux() {
        assert_eq!(classify(None, Some("com.googlecode.iterm2")), Terminal::ITerm2);
        assert_eq!(classify(Some("iTerm.app"), None), Terminal::ITerm2);
        assert_eq!(classify(Some("tmux"), Some("com.googlecode.iterm2")), Terminal::OuterTmux);
        assert_eq!(classify(Some("ghostty"), None), Terminal::Other);
    }

    /// Push once before the first words, write only on a change, pop once
    /// at the end, and a pop with nothing up writes nothing.
    #[test]
    fn the_title_stack_is_pushed_once_and_popped_once_and_repeats_write_nothing() {
        let mut out = Vec::new();
        let mut tab = Tab::default();
        tab.finish(&mut out).unwrap();
        assert!(out.is_empty(), "nothing up, nothing to pop");
        let words = |s: &str| Frame { title: Some(s.into()), ..Default::default() };
        tab.sync(&mut out, &words("mesimon ∙ api")).unwrap();
        assert_eq!(out, b"\x1b[22;0t\x1b]0;mesimon \xe2\x88\x99 api\x07");
        out.clear();
        tab.sync(&mut out, &words("mesimon ∙ api")).unwrap();
        assert!(out.is_empty(), "the same words again cost the tty nothing");
        tab.sync(&mut out, &words("1 need you ∙ api")).unwrap();
        assert_eq!(out, b"\x1b]0;1 need you \xe2\x88\x99 api\x07", "no second push");
        out.clear();
        tab.sync(&mut out, &Frame::default()).unwrap();
        assert_eq!(out, b"\x1b[23;0t", "off is the pop");
        out.clear();
        tab.finish(&mut out).unwrap();
        assert!(out.is_empty(), "already popped");
        tab.sync(&mut out, &words("mesimon ∙ api")).unwrap();
        assert!(out.starts_with(b"\x1b[22;0t"), "back on: pushed again, so the stack balances");
    }

    /// The ring, the mark and the subtitle each write on a
    /// change only, and `finish` gives every one of them back.
    #[test]
    fn every_other_field_writes_on_change_and_is_given_back_by_finish() {
        let mut out = Vec::new();
        let mut tab = Tab::default();
        let f = Frame {
            title: None,
            progress: Some(Progress::Working),
            mark: Some(Mark::Dot(0xF0A93A)),
            subtitle: Some("3 working".into()),
        };
        tab.sync(&mut out, &f).unwrap();
        let s = String::from_utf8(out.clone()).unwrap();
        assert!(s.contains("\x1b]9;4;3\x07"), "{s:?}");
        assert!(s.contains("\x1b]21337;indicator=#f0a93a\x07"), "{s:?}");
        assert!(s.contains("\x1b]21337;status=3 working\x07"), "{s:?}");
        assert!(!s.contains("\x1b[22;0t"), "no title, no push");
        out.clear();
        tab.sync(&mut out, &f).unwrap();
        assert!(out.is_empty());
        // Blocked: the bar goes red, the mark moves to the chrome and the
        // dot is cleared on its own road.
        let g = Frame { progress: Some(Progress::Blocked), mark: Some(Mark::Tab(0xF0A93A)), ..f };
        tab.sync(&mut out, &g).unwrap();
        let s = String::from_utf8(out.clone()).unwrap();
        assert!(s.contains("\x1b]9;4;2;100\x07"), "{s:?}");
        assert!(s.contains("\x1b]21337;indicator=\x07"), "{s:?}");
        assert!(s.contains("red;brightness;240\x07"), "{s:?}");
        assert!(s.contains("green;brightness;169\x07"), "{s:?}");
        assert!(s.contains("blue;brightness;58\x07"), "{s:?}");
        assert!(!s.contains("status="), "the subtitle did not change");
        out.clear();
        tab.finish(&mut out).unwrap();
        let s = String::from_utf8(out.clone()).unwrap();
        assert!(s.contains("\x1b]9;4;0\x07"), "{s:?}");
        assert!(s.contains("\x1b]6;1;bg;*;default\x07"), "{s:?}");
        assert!(s.contains("\x1b]21337;status=\x07"), "{s:?}");
        assert!(!s.contains("\x1b[23;0t"), "nothing was pushed");
    }
}
