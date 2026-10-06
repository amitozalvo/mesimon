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
//! - **iTerm2's tab in the theme's colour** (`OSC 6;1;bg`, T-528): the
//!   whole tab in the hint line's colour, so the tab strip reads as the
//!   board's chrome; the needs-you whole-tab colour paints over it and
//!   hands the tab back to it, never to the terminal's own;
//! - **iTerm2's subtitle** (`OSC 21337 status=`): how many need you and
//!   how many are working;
//! - **iTerm2's icon** (`OSC 1337;SetProfileProperty=Icon=…;Custom Icon
//!   Path=…`): the shin, resting (the notifier's own PNG) or, while any
//!   ticket needs you, wearing an attention-colour badge in its corner —
//!   a tab gives an icon sixteen pixels, and the pose's "!" is a fraction
//!   of one there, so the needs-you icon says it with the board's one
//!   saturated colour instead. iTerm2 re-reads a changed path (its cache
//!   keys on the path and a profile reload re-asks the tab graphic). The escape
//!   changes this SESSION's copy of the profile and never the saved one
//!   (README promise 2 holds); `finish` sets the icon mode back to
//!   iTerm2's default, built-in for the current app, since the profile's
//!   own choice cannot be read.
//!
//! The dot, the subtitle and the icon are iTerm2 3.7's (`OSC 21337` and
//! `SetProfileProperty` are new there); 3.6 ignores both escapes, so on it
//! those rows say so and nothing is written. `TERM_PROGRAM_VERSION` is how
//! iTerm2 says which it is.
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

/// A tab is a few dozen cells wide on every terminal that has one; a
/// ticket's title clipped here, behind the board's words, reads better
/// than one the terminal cuts mid-word. On a word boundary, with an
/// ellipsis, the notification's rule.
const FOCUS_TITLE_CHARS: usize = 32;

/// Which terminal the board's stdout reaches, as far as the iTerm2-only
/// rows are concerned. Resolved from the environment in `lib.rs::run`,
/// never in `App::new`, so no test app reads a developer's terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Terminal {
    /// iTerm2 and no tmux in between. `status`: 3.7 or later, where the
    /// session-status escape and `SetProfileProperty` exist.
    ITerm2 {
        status: bool,
    },
    /// The user's own tmux: `TERM_PROGRAM` is rewritten to `tmux` in every
    /// pane, which is the one reliable negative — `__CFBundleIdentifier`
    /// is inherited and stale in there.
    OuterTmux,
    /// The three other terminals that post a system notification from an
    /// escape of their own (T-676): kitty (OSC 99), WezTerm and Ghostty
    /// (OSC 777). Named for the notifications row only; every iTerm2 row
    /// treats them as `Other`.
    Kitty,
    WezTerm,
    Ghostty,
    #[default]
    Other,
}

impl Terminal {
    /// The escape this terminal posts a banner from, or `None` where it
    /// cannot (T-676): Terminal.app, an outer tmux, anything unknown.
    pub(crate) fn poster(self) -> Option<crate::notify::Poster> {
        use crate::notify::Poster;
        match self {
            Terminal::ITerm2 { .. } => Some(Poster::Osc9),
            Terminal::Kitty => Some(Poster::Osc99),
            Terminal::WezTerm | Terminal::Ghostty => Some(Poster::Osc777),
            Terminal::OuterTmux | Terminal::Other => None,
        }
    }
}

pub(crate) fn terminal() -> Terminal {
    let var = |k: &str| std::env::var(k).ok();
    classify(&Seen {
        term_program: var("TERM_PROGRAM").as_deref(),
        bundle: var("__CFBundleIdentifier").as_deref(),
        version: var("TERM_PROGRAM_VERSION").as_deref(),
        term: var("TERM").as_deref(),
        kitty_window: var("KITTY_WINDOW_ID").as_deref(),
    })
}

/// The environment words `classify` reads.
#[derive(Default)]
struct Seen<'a> {
    term_program: Option<&'a str>,
    bundle: Option<&'a str>,
    version: Option<&'a str>,
    term: Option<&'a str>,
    kitty_window: Option<&'a str>,
}

fn classify(e: &Seen) -> Terminal {
    let program = |name: &str| e.term_program.is_some_and(|v| v.eq_ignore_ascii_case(name));
    if program("tmux") {
        return Terminal::OuterTmux;
    }
    if e.bundle == Some("com.googlecode.iterm2") || e.term_program == Some("iTerm.app") {
        return Terminal::ITerm2 { status: e.version.is_some_and(|v| at_least(v, (3, 7))) };
    }
    // `TERM_PROGRAM`, where it is set, outranks the inherited words: VS
    // Code started from a kitty shell keeps `KITTY_WINDOW_ID` and the
    // bundle id, and is not kitty.
    let unnamed = e.term_program.is_none_or(str::is_empty);
    let kitty = e.kitty_window.is_some_and(|v| !v.is_empty())
        || e.term == Some("xterm-kitty")
        || e.bundle == Some("net.kovidgoyal.kitty");
    if program("kitty") || (unnamed && kitty) {
        return Terminal::Kitty;
    }
    if program("WezTerm") || (unnamed && e.bundle == Some("com.github.wez.wezterm")) {
        return Terminal::WezTerm;
    }
    let ghostty = e.term == Some("xterm-ghostty") || e.bundle == Some("com.mitchellh.ghostty");
    if program("ghostty") || (unnamed && ghostty) {
        return Terminal::Ghostty;
    }
    Terminal::Other
}

/// `3.6.11`, `3.7.0beta1`: the leading `major.minor` compared, the rest
/// ignored. An unparseable word is below everything.
fn at_least(version: &str, floor: (u32, u32)) -> bool {
    let mut parts = version.split('.');
    let num = |s: Option<&str>| -> Option<u32> {
        let digits: String = s?.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    match (num(parts.next()), num(parts.next())) {
        (Some(major), Some(minor)) => (major, minor) >= floor,
        _ => false,
    }
}

/// The word iTerm2 gives its version, for the doctor line.
pub(crate) fn iterm2_version_word() -> String {
    std::env::var("TERM_PROGRAM_VERSION").unwrap_or_else(|_| "of an unknown version".into())
}

/// The board runs in iTerm2 directly — what the notifier's bounce asks.
pub(crate) fn iterm2_direct() -> bool {
    matches!(terminal(), Terminal::ITerm2 { .. })
}

/// The two shin PNGs the tab icon shows, materialised under the
/// notifications dir (the same two files a banner attaches) — lazily, on
/// the first frame that wants them, never by a row that is off.
pub(crate) fn shin_icons(repo_root: &std::path::Path) -> std::io::Result<Icons> {
    let dir = mesimon_daemon::Paths::for_repo(repo_root)
        .map_err(std::io::Error::other)?
        .state_dir
        .parent()
        .map(|home| home.join("notifications"))
        .ok_or_else(|| std::io::Error::other("no state dir"))?;
    Ok(Icons {
        resting: crate::mascot::tab_icon(&dir, false)?,
        needs_you: crate::mascot::tab_icon(&dir, true)?,
    })
}

/// The two icon files, by pose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Icons {
    pub resting: std::path::PathBuf,
    pub needs_you: std::path::PathBuf,
}

/// `SetProfileProperty`'s value: base64 of the JSON of the value.
fn profile_value(json: &str) -> String {
    crate::osc::base64(json.as_bytes())
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

/// A focus handover's words: the board's own words first, the way the
/// title row shapes them, then the ticket's key and title — `mesimon ∙
/// api ∙ T-12 fix the parser`. The title is clipped shorter than a
/// board's line, since the board's words take the front of the tab.
pub(crate) fn focus(name: &str, app_word: bool, key: &str, ticket_title: &str) -> String {
    let title = text::clip_words(&text::scrub_text(ticket_title), FOCUS_TITLE_CHARS);
    let key = text::scrub_text(key);
    let ticket = if title.is_empty() { key } else { format!("{key} {title}") };
    format!("{} ∙ {ticket}", board(name, app_word, None))
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
    /// The whole tab's colour at rest (T-528): the theme's, under a
    /// `Mark::Tab`, which outranks it while it stands.
    pub tint: Option<u32>,
    /// `Some("")` clears the subtitle.
    pub subtitle: Option<String>,
    /// The icon file to show; `None` while the row is off.
    pub icon: Option<std::path::PathBuf>,
}

impl Frame {
    /// The colour the whole tab wears: the needs-you mark's, else the
    /// theme's tint, else the terminal's own.
    fn chrome(&self) -> Option<u32> {
        match self.mark {
            Some(Mark::Tab(rgb)) => Some(rgb),
            _ => self.tint,
        }
    }
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
            // empty indicator, the chrome below.
            if matches!(before, Mark::Dot(_)) && !matches!(after, Mark::Dot(_)) {
                out.write_all(b"\x1b]21337;indicator=\x07")?;
            }
            if let Mark::Dot(rgb) = after {
                write!(out, "\x1b]21337;indicator=#{rgb:06x}\x07")?;
            }
        }
        // The chrome is one colour with two writers: the needs-you mark,
        // and under it the theme's tint. `*;default` only when neither
        // wants it, so a mark that clears hands the tab back to the tint.
        let (before, after) = (self.last.chrome(), want.chrome());
        if after != before {
            match after {
                Some(rgb) => {
                    let [_, r, g, b] = rgb.to_be_bytes();
                    write!(
                        out,
                        "\x1b]6;1;bg;red;brightness;{r}\x07\x1b]6;1;bg;green;brightness;{g}\x07\x1b]6;1;bg;blue;brightness;{b}\x07"
                    )?;
                }
                None => out.write_all(b"\x1b]6;1;bg;*;default\x07")?,
            }
        }
        if want.subtitle != self.last.subtitle {
            let words = want.subtitle.as_deref().unwrap_or("");
            write!(out, "\x1b]21337;status={words}\x07")?;
        }
        if want.icon != self.last.icon {
            match &want.icon {
                // Icon mode 2 (custom) and the file, one sequence: the
                // path rides base64 JSON, so no byte of it can end the
                // OSC early or be read as a key.
                Some(path) => {
                    let json = serde_json::to_string(&path.to_string_lossy())
                        .unwrap_or_else(|_| "\"\"".into());
                    write!(
                        out,
                        "\x1b]1337;SetProfileProperty=Icon={};Custom Icon Path={}\x07",
                        profile_value("2"),
                        profile_value(&json)
                    )?;
                }
                // Mode 1: iTerm2's built-in icon for the current app, its
                // default — the profile's own choice cannot be read back.
                None => {
                    write!(out, "\x1b]1337;SetProfileProperty=Icon={}\x07", profile_value("1"))?
                }
            }
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
    fn a_focus_names_the_board_then_the_ticket_and_clips_a_long_title_on_a_word() {
        assert_eq!(
            focus("api", true, "T-12", "fix the parser"),
            "mesimon ∙ api ∙ T-12 fix the parser"
        );
        assert_eq!(focus("api", false, "T-12", "fix the parser"), "api ∙ T-12 fix the parser");
        assert_eq!(focus("api", true, "T-12", ""), "mesimon ∙ api ∙ T-12");
        let long = "a ".repeat(40) + "tail";
        let f = focus("api", false, "T-12", &long);
        assert!(f.ends_with('…'), "{f}");
        let ticket = f.trim_start_matches("api ∙ ");
        assert!(ticket.chars().count() <= FOCUS_TITLE_CHARS + "T-12 ".len() + 1, "{f}");
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
        let f = focus("api", true, "T-12", "fix\x07 the\x1b]0;x\x07 parser");
        assert!(!f.contains('\x07') && !f.contains('\x1b'), "{f}");
        assert_eq!(board("api\x1b", true, None), "mesimon ∙ api");
    }

    #[test]
    fn iterm2_is_named_directly_and_never_through_an_outer_tmux() {
        let old = Terminal::ITerm2 { status: false };
        let new = Terminal::ITerm2 { status: true };
        let c = |term_program, bundle, version| {
            classify(&Seen { term_program, bundle, version, ..Seen::default() })
        };
        assert_eq!(c(None, Some("com.googlecode.iterm2"), Some("3.6.11")), old);
        assert_eq!(c(Some("iTerm.app"), None, Some("3.7.0beta2")), new);
        assert_eq!(c(Some("iTerm.app"), None, Some("4.0")), new);
        assert_eq!(c(Some("iTerm.app"), None, None), old, "no version: the old escapes");
        let tmux = c(Some("tmux"), Some("com.googlecode.iterm2"), Some("3.7"));
        assert_eq!(tmux, Terminal::OuterTmux);
        assert_eq!(c(Some("Apple_Terminal"), None, Some("455")), Terminal::Other);
        assert!(!at_least("3", (3, 7)));
        assert!(!at_least("beta", (3, 7)));
    }

    /// The three other terminals that post a banner (T-676), each by its
    /// own words, and never through an outer tmux or under a program that
    /// names itself something else.
    #[test]
    fn kitty_wezterm_and_ghostty_are_named_and_post_banners() {
        use crate::notify::Poster;
        let kitty = Seen { kitty_window: Some("1"), term: Some("xterm-kitty"), ..Seen::default() };
        assert_eq!(classify(&kitty), Terminal::Kitty);
        assert_eq!(
            classify(&Seen { term: Some("xterm-kitty"), ..Seen::default() }),
            Terminal::Kitty
        );
        let wez = Seen { term_program: Some("WezTerm"), ..Seen::default() };
        assert_eq!(classify(&wez), Terminal::WezTerm);
        let ghostty =
            Seen { term_program: Some("ghostty"), version: Some("1.3.1"), ..Seen::default() };
        assert_eq!(classify(&ghostty), Terminal::Ghostty);
        let in_tmux = Seen { term_program: Some("tmux"), ..kitty };
        assert_eq!(classify(&in_tmux), Terminal::OuterTmux);
        let vscode =
            Seen { term_program: Some("vscode"), kitty_window: Some("1"), ..Seen::default() };
        assert_eq!(classify(&vscode), Terminal::Other, "an inherited kitty id is not kitty");
        assert_eq!(Terminal::ITerm2 { status: false }.poster(), Some(Poster::Osc9));
        assert_eq!(Terminal::Kitty.poster(), Some(Poster::Osc99));
        assert_eq!(Terminal::WezTerm.poster(), Some(Poster::Osc777));
        assert_eq!(Terminal::Ghostty.poster(), Some(Poster::Osc777));
        assert_eq!(Terminal::OuterTmux.poster(), None);
        assert_eq!(Terminal::Other.poster(), None);
    }

    /// The icon rides `SetProfileProperty` as base64 JSON, and off puts
    /// iTerm2's built-in icon back.
    #[test]
    fn the_icon_is_set_as_base64_json_and_given_back_as_the_builtin_one() {
        let mut out = Vec::new();
        let mut tab = Tab::default();
        let f = Frame { icon: Some("/s/shin-a.png".into()), ..Default::default() };
        tab.sync(&mut out, &f).unwrap();
        let s = String::from_utf8(out.clone()).unwrap();
        // `2` → `Mg==`; `"/s/shin-a.png"` → its base64.
        assert_eq!(
            s,
            "\x1b]1337;SetProfileProperty=Icon=Mg==;Custom Icon Path=Ii9zL3NoaW4tYS5wbmci\x07"
        );
        out.clear();
        tab.finish(&mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b]1337;SetProfileProperty=Icon=MQ==\x07");
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
            tint: None,
            subtitle: Some("3 working".into()),
            icon: None,
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

    /// The theme's tint (T-528) is the chrome at rest: the needs-you mark
    /// paints over it and gives the tab back to it, never to the
    /// terminal's own; a theme change repaints it; off and `finish` hand
    /// the tab back to the terminal.
    #[test]
    fn the_theme_tint_sits_under_the_needs_you_mark() {
        let chrome = |rgb: u32| {
            let [_, r, g, b] = rgb.to_be_bytes();
            format!(
                "\x1b]6;1;bg;red;brightness;{r}\x07\x1b]6;1;bg;green;brightness;{g}\x07\x1b]6;1;bg;blue;brightness;{b}\x07"
            )
        };
        let text = |out: &mut Vec<u8>| String::from_utf8(std::mem::take(out)).unwrap();
        let (dark, light, attn) = (0x2A2D33, 0xE4E1DA, 0xF0A93A);
        let mut out = Vec::new();
        let mut tab = Tab::default();
        let rest = Frame { tint: Some(dark), mark: Some(Mark::Off), ..Default::default() };
        tab.sync(&mut out, &rest).unwrap();
        assert_eq!(text(&mut out), chrome(dark));
        let blocked = Frame { mark: Some(Mark::Tab(attn)), ..rest.clone() };
        tab.sync(&mut out, &blocked).unwrap();
        assert_eq!(text(&mut out), chrome(attn), "needs-you paints over the tint");
        tab.sync(&mut out, &rest).unwrap();
        assert_eq!(text(&mut out), chrome(dark), "and hands the tab back to it");
        let dot = Frame { mark: Some(Mark::Dot(attn)), ..rest.clone() };
        tab.sync(&mut out, &dot).unwrap();
        assert_eq!(text(&mut out), "\x1b]21337;indicator=#f0a93a\x07", "a dot leaves the tint");
        tab.sync(&mut out, &Frame { tint: Some(light), ..dot.clone() }).unwrap();
        assert_eq!(text(&mut out), chrome(light), "a theme change repaints it");
        tab.sync(&mut out, &Frame { tint: None, mark: Some(Mark::Off), ..Default::default() })
            .unwrap();
        let s = text(&mut out);
        assert!(s.contains("\x1b]21337;indicator=\x07"), "{s:?}");
        assert!(s.ends_with("\x1b]6;1;bg;*;default\x07"), "off: the terminal's own: {s:?}");
        tab.sync(&mut out, &Frame { tint: Some(dark), ..Default::default() }).unwrap();
        out.clear();
        tab.finish(&mut out).unwrap();
        assert_eq!(text(&mut out), "\x1b]6;1;bg;*;default\x07");
    }
}
