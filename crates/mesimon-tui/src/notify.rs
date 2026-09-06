//! Saying it out loud (T-282): the two ladders behind `core::notify`.
//!
//! The pure half decides WHAT is said and when; this decides HOW, and it is
//! the only file that knows a platform. Both ladders follow `opener.rs`: an
//! environment variable first (the explicit choice, and the seam that keeps a
//! test from ever making a noise), then the platform's own program, then a
//! rung that always exists. Resolved once per process in `lib.rs` — never in
//! `App::new`, so no test app and no golden reaches a real program.
//!
//! **The banner ladder** is `MESIMON_NOTIFY` (`off`, `osc`, or a program) →
//! `terminal-notifier` → `osascript` on macOS → `notify-send` on Linux →
//! **OSC 9**, written to our own terminal. The last rung cannot fail to
//! resolve, which is why it is last: on a terminal that draws it (iTerm2,
//! WezTerm, ghostty, kitty) the banner comes from the terminal itself, and on
//! one that does not, nothing happens — that is the rung's known limit and
//! the reason a helper program outranks it. An outer tmux of the user's own
//! swallows it too; the board itself never runs inside mesimon's private
//! server, so there is no DCS wrap to do here.
//!
//! **The sound ladder** is `MESIMON_SOUND` (`off` or a program) → `afplay` on
//! macOS → `paplay` / `pw-play` / `canberra-gtk-play` on Linux → the terminal
//! bell, which is the rung nothing can take away.
//!
//! Two rules hold across both, and both are load-bearing:
//!
//! - **the text rides argv, never a program's source.** A ticket key is ours,
//!   but a board's directory name is the user's, and `osascript` takes a
//!   PROGRAM. So the script is a constant with `on run argv` and the words go
//!   past it as arguments — `workspace.rs`'s "argv arrays always" rule.
//! - **everything crosses `text::scrub_text` first**, the boundary function
//!   for text leaving for another process. It is also what makes the OSC rung
//!   safe: the ESC and BEL it strips are exactly what would otherwise close
//!   the sequence early.

use std::io::Write;
use std::path::PathBuf;

use mesimon_core::notify::{Post, Sound};
use mesimon_core::text;

use crate::opener::launch;

/// A notification body is one line on somebody's screen. Both fields are
/// bounded here rather than trusted: the title is a directory name and the
/// body can carry an agent's own sentence. Wide enough that a raised hand's
/// full `board::RAISE_REASON_MAX_BYTES` (160) still arrives with `T-12 needs
/// you ∙ ` in front of it — the words are why the banner was worth sending.
const MAX_FIELD: usize = 240;

/// Where the freedesktop sound theme keeps the three events six names
/// collapse to (`core::notify::Sound::event_freedesktop`).
const FREEDESKTOP: &str = "/usr/share/sounds/freedesktop/stereo";

/// Who draws the banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    /// `MESIMON_NOTIFY=off`. The sound, if any, still plays.
    Off,
    /// OSC 9 to our own stdout — the terminal draws it, or nobody does.
    Osc,
    /// `MESIMON_NOTIFY=<program>`, called `<program> <title> <body>`.
    Custom(String),
    TerminalNotifier,
    Osascript,
    NotifySend,
}

impl Banner {
    /// The argv, or `None` where the rung writes an escape instead.
    fn argv(&self, title: &str, body: &str) -> Option<Vec<String>> {
        let v = |args: &[&str]| Some(args.iter().map(|s| (*s).to_string()).collect());
        match self {
            Banner::Off | Banner::Osc => None,
            Banner::Custom(prog) => v(&[prog, title, body]),
            Banner::TerminalNotifier => {
                v(&["terminal-notifier", "-title", title, "-message", body])
            }
            // The words are `argv`, the script is a constant. `item 1` is the
            // title and `item 2` the body, so neither can be read as code
            // however they are spelled.
            Banner::Osascript => v(&[
                "osascript",
                "-e",
                "on run argv",
                "-e",
                "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e",
                "end run",
                title,
                body,
            ]),
            // `--` so a title that begins with a dash is a title. Every
            // notify-send since 0.7 takes it.
            Banner::NotifySend => v(&["notify-send", "--", title, body]),
        }
    }

    /// What `doctor` calls this rung.
    pub fn word(&self) -> String {
        match self {
            Banner::Off => "off".into(),
            Banner::Osc => "OSC 9 (your terminal draws it)".into(),
            Banner::Custom(p) => format!("{p} ($MESIMON_NOTIFY)"),
            Banner::TerminalNotifier => "terminal-notifier".into(),
            Banner::Osascript => "osascript".into(),
            Banner::NotifySend => "notify-send".into(),
        }
    }
}

/// Who makes the sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Player {
    /// `MESIMON_SOUND=off`. The banner, if any, still shows.
    Off,
    /// The terminal bell — the rung nothing can take away, and the one every
    /// other rung falls back to when its file is missing.
    Bell,
    /// `MESIMON_SOUND=<program>`, called `<program> <sound>` with the name.
    Custom(String),
    Afplay,
    Paplay,
    PwPlay,
    Canberra,
}

impl Player {
    /// The argv, or `None` to fall back to the bell — which is what a rung
    /// whose file is not on this machine does.
    fn argv(&self, sound: Sound) -> Option<Vec<String>> {
        if sound.is_off() {
            return None;
        }
        let file = |name: &str| {
            let p = PathBuf::from(FREEDESKTOP).join(format!("{name}.oga"));
            std::fs::metadata(&p).is_ok().then(|| p.to_string_lossy().into_owned())
        };
        let v = |args: Vec<String>| Some(args);
        match self {
            Player::Off | Player::Bell => None,
            Player::Custom(prog) => v(vec![prog.clone(), sound.name().to_string()]),
            Player::Afplay => {
                let f = sound.file_macos()?;
                std::fs::metadata(&f).is_ok().then(|| vec!["afplay".to_string(), f])
            }
            Player::Paplay => v(vec!["paplay".into(), file(sound.event_freedesktop()?)?]),
            Player::PwPlay => v(vec!["pw-play".into(), file(sound.event_freedesktop()?)?]),
            // The theme-aware one: it takes the event, not a path, so it
            // needs no file check of ours.
            Player::Canberra => {
                v(vec!["canberra-gtk-play".into(), "-i".into(), sound.event_freedesktop()?.into()])
            }
        }
    }

    pub fn word(&self) -> String {
        match self {
            Player::Off => "off".into(),
            Player::Bell => "the terminal bell".into(),
            Player::Custom(p) => format!("{p} ($MESIMON_SOUND)"),
            Player::Afplay => "afplay".into(),
            Player::Paplay => "paplay".into(),
            Player::PwPlay => "pw-play".into(),
            Player::Canberra => "canberra-gtk-play".into(),
        }
    }
}

/// Both rungs, resolved once. `lib.rs` parks this on `App`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channels {
    pub banner: Banner,
    pub player: Player,
}

pub fn find() -> Channels {
    Channels {
        banner: find_banner(std::env::var("MESIMON_NOTIFY").ok().as_deref(), which_on_path),
        player: find_player(std::env::var("MESIMON_SOUND").ok().as_deref(), which_on_path),
    }
}

fn find_banner(env: Option<&str>, which: impl Fn(&str) -> Option<PathBuf>) -> Banner {
    match env.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.eq_ignore_ascii_case("off") => return Banner::Off,
        Some(v) if v.eq_ignore_ascii_case("osc") => return Banner::Osc,
        Some(v) => return Banner::Custom(v.to_string()),
        None => {}
    }
    if which("terminal-notifier").is_some() {
        return Banner::TerminalNotifier;
    }
    if cfg!(target_os = "macos") {
        return Banner::Osascript;
    }
    if which("notify-send").is_some() {
        return Banner::NotifySend;
    }
    Banner::Osc
}

fn find_player(env: Option<&str>, which: impl Fn(&str) -> Option<PathBuf>) -> Player {
    match env.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.eq_ignore_ascii_case("off") => return Player::Off,
        Some(v) => return Player::Custom(v.to_string()),
        None => {}
    }
    if cfg!(target_os = "macos") {
        return Player::Afplay;
    }
    for (name, rung) in [
        ("paplay", Player::Paplay),
        ("pw-play", Player::PwPlay),
        ("canberra-gtk-play", Player::Canberra),
    ] {
        if which(name).is_some() {
            return rung;
        }
    }
    Player::Bell
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|cand| std::fs::metadata(cand).is_ok_and(|m| m.is_file()))
}

/// Say it. A banner with no words is a sound-only post — the focus rule's
/// output, and a Settings row's preview.
///
/// Errors are the spawn's own (no such program) and the terminal's; whether
/// the notification was actually SEEN is not ours to know, the same reason
/// the board says `opening …` and never `opened`.
pub fn post(ch: &Channels, p: &Post) -> std::io::Result<()> {
    if !p.body.is_empty() {
        let title = field(&p.title);
        let body = field(&p.body);
        match ch.banner.argv(&title, &body) {
            Some(argv) => launch(&argv, None)?,
            None if ch.banner == Banner::Osc => write_osc9(&title, &body)?,
            None => {}
        }
    }
    if !p.sound.is_off() {
        match ch.player.argv(p.sound) {
            Some(argv) => launch(&argv, None)?,
            None if ch.player == Player::Off => {}
            None => ring_bell()?,
        }
    }
    Ok(())
}

/// One field of a notification, safe to hand to another process or to write
/// inside an escape sequence.
fn field(raw: &str) -> String {
    text::cap_bytes(&text::scrub_text(raw), MAX_FIELD).to_string()
}

/// OSC 9 has one text, not a title and a body — so they are joined with the
/// separator every other line of mesimon uses. Written straight to stdout the
/// way `osc::copy_to_clipboard` writes OSC 52: between draws, no `execute!`,
/// and nothing on screen moves, so no redraw is owed.
fn write_osc9(title: &str, body: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    if title.is_empty() {
        write!(out, "\x1b]9;{body}\x07")?;
    } else {
        write!(out, "\x1b]9;{title} ∙ {body}\x07")?;
    }
    out.flush()
}

fn ring_bell() -> std::io::Result<()> {
    let mut out = std::io::stdout();
    write!(out, "\x07")?;
    out.flush()
}

/// What `mesimon doctor` says: whether it is on, which rungs answered, and
/// both sounds. The rungs are named even while it is off, because "would it
/// work if I turned it on" is the question somebody reads this line to ask.
pub fn doctor_line() -> String {
    let p = &crate::prefs::load_home().prefs;
    let ch = find();
    if !p.notify {
        return format!(
            "off (Settings ∙ Notifications turns it on) — would use {} and {}",
            ch.banner.word(),
            ch.player.word()
        );
    }
    let mut parts = vec![format!("on ∙ {}", ch.banner.word())];
    parts.push(format!("needs-you {}", p.notify_sound_needs_you.name()));
    if p.notify_done {
        parts.push(format!("finished {}", p.notify_sound_done.name()));
    } else {
        parts.push("nothing when a turn finishes".into());
    }
    if !p.notify_sound_needs_you.is_off() || !p.notify_sound_done.is_off() {
        parts.push(format!("through {}", ch.player.word()));
    }
    parts.push(if p.notify_focused {
        "shown even while the board is focused".into()
    } else {
        "quiet while the board is focused".into()
    });
    parts.join(" ∙ ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<PathBuf> {
        None
    }
    fn all(n: &str) -> Option<PathBuf> {
        Some(PathBuf::from(format!("/usr/bin/{n}")))
    }

    #[test]
    fn the_env_wins_on_both_ladders() {
        assert_eq!(find_banner(Some(" off "), all), Banner::Off);
        assert_eq!(find_banner(Some("OSC"), all), Banner::Osc);
        assert_eq!(find_banner(Some("my-notifier"), all), Banner::Custom("my-notifier".into()));
        assert_eq!(find_banner(Some("   "), none), find_banner(None, none));
        assert_eq!(find_player(Some("off"), all), Player::Off);
        assert_eq!(find_player(Some(" aplay "), all), Player::Custom("aplay".into()));
    }

    #[test]
    fn the_ladders_end_on_a_rung_that_always_exists() {
        // Nothing on PATH: the banner falls to the terminal's own escape and
        // the sound to the bell — neither can fail to resolve.
        if cfg!(target_os = "macos") {
            assert_eq!(find_banner(None, none), Banner::Osascript);
            assert_eq!(find_player(None, none), Player::Afplay);
        } else {
            assert_eq!(find_banner(None, none), Banner::Osc);
            assert_eq!(find_player(None, none), Player::Bell);
            assert_eq!(find_banner(None, all), Banner::TerminalNotifier);
            assert_eq!(find_player(None, all), Player::Paplay);
            let send =
                |n: &str| (n == "notify-send").then(|| PathBuf::from("/usr/bin/notify-send"));
            assert_eq!(find_banner(None, send), Banner::NotifySend);
        }
        // terminal-notifier outranks the platform's own on every platform.
        let tn = |n: &str| (n == "terminal-notifier").then(|| PathBuf::from("/usr/bin/x"));
        assert_eq!(find_banner(None, tn), Banner::TerminalNotifier);
    }

    #[test]
    fn the_words_ride_argv_and_never_the_script() {
        let argv = Banner::Osascript.argv("board", "\" & do shell script \"boom").expect("argv");
        assert_eq!(argv.last().expect("the body"), "\" & do shell script \"boom");
        // Every `-e` fragment is a constant: none of them holds the words.
        for (i, a) in argv.iter().enumerate() {
            if a == "-e" {
                let script = &argv[i + 1];
                assert!(!script.contains("board"), "{script}");
                assert!(!script.contains("boom"), "{script}");
            }
        }
        assert_eq!(
            Banner::NotifySend.argv("-t", "b").expect("argv"),
            vec!["notify-send", "--", "-t", "b"],
            "a title that starts with a dash is still a title"
        );
        assert_eq!(
            Banner::Custom("ding".into()).argv("t", "b").expect("argv"),
            vec!["ding", "t", "b"]
        );
        assert!(Banner::Osc.argv("t", "b").is_none());
        assert!(Banner::Off.argv("t", "b").is_none());
    }

    #[test]
    fn a_field_is_scrubbed_and_bounded() {
        // The two characters that would close an OSC sequence early, and the
        // one that would end a line in an argument.
        let dirty = "T-1 \x1b]9;evil\x07 \n needs you";
        let out = field(dirty);
        assert!(!out.contains('\x1b'), "{out}");
        assert!(!out.contains('\x07'), "{out}");
        assert!(!out.contains('\n'), "{out}");
        assert!(field(&"x".repeat(1000)).len() <= MAX_FIELD);
        assert_eq!(field("board ∙ name"), "board ∙ name", "ordinary words survive");
    }

    #[test]
    fn off_means_off_on_each_channel_alone() {
        assert!(Player::Off.argv(Sound::Glass).is_none());
        assert!(Player::Afplay.argv(Sound::Off).is_none());
        assert!(Player::Bell.argv(Sound::Glass).is_none(), "the bell has no argv, it is a write");
        assert_eq!(
            Player::Canberra.argv(Sound::Glass).expect("argv"),
            vec!["canberra-gtk-play", "-i", "message"]
        );
        assert_eq!(
            Player::Custom("ding".into()).argv(Sound::Tink).expect("argv"),
            vec!["ding", "Tink"]
        );
    }

    #[test]
    fn a_silent_post_runs_nothing() {
        // Off on both channels with a real body: `post` must not try to spawn,
        // and `Off` is the one rung that writes no escape either.
        let ch = Channels { banner: Banner::Off, player: Player::Off };
        let p = Post { title: "t".into(), body: "b".into(), sound: Sound::Glass };
        assert!(post(&ch, &p).is_ok());
        assert!(post(&ch, &Post::sound_only(Sound::Off)).is_ok());
    }
}
