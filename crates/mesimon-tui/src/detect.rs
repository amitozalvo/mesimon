//! Terminal capability detection (06 §2.9). The colour-profile and light/dark
//! ladders run once, at startup, before the daemon connect and before raw
//! mode — never while a session is focused, and never in the daemon.
//!
//! The light/dark answer is asked of the terminal EXACTLY ONCE, and only
//! when nothing else has answered it. A board that follows the OS
//! appearance (`appearance.rs`, T-485) arrives with the OS's answer and the
//! terminal is not asked at all; the live re-ask that once lived here
//! (`GroundWatch`, `MESIMON_GROUND_WATCH=1`) is gone for good — every query
//! was a write to the tty and a timed read back, and a reply that came in
//! late landed on stdin as keystrokes. `osc::ReplySwallow` still stands in
//! front of the keymap for the startup reply.
//!
//! The answer is a GROUND, not a flavor: the terminal only knows light or
//! dark, and which theme sits on each is the user's (`prefs.rs`, two slots).
//! Resolving one to the other is `lib.rs`'s job at startup and
//! `App::follow_appearance`'s afterwards.
//!
//! Deferred rungs, deliberately: `CSI ? 996 n` + `CSI ? 2031` (one Contour
//! extension; with 2031 never armed there is no unsolicited DSR to disarm
//! before a PTY attach, which is the trap 06 §2.9 names), and the two
//! terminfo rungs (terminfo is a guaranteed false negative under tmux; the
//! env rungs carry the real weight).

use std::time::Duration;

use crate::theme::{Flavor, Ground, Profile};

pub(crate) struct Detected {
    pub profile: Profile,
    /// What the OS said, else what the terminal said, else `Dark`.
    pub ground: Ground,
    /// `MESIMON_THEME`: the user's word, pinned for the process.
    pub forced: Option<Flavor>,
}

/// `known` is a ground somebody else already answered (the OS, for a board
/// that follows it); with one, the terminal is not asked.
pub(crate) fn detect(known: Option<Ground>) -> Detected {
    let profile = profile_from_env();
    // Mono never queries the tty — there is nothing to colour, so there is
    // no flavor to pin either.
    if profile == Profile::Mono {
        return Detected { profile, ground: Ground::Dark, forced: None };
    }
    let forced = forced_flavor();
    // Asked even under a pin: the picker sets the slot the terminal is on,
    // and a one-shot query corrects nothing.
    let ground = known.or_else(query_ground).unwrap_or(Ground::Dark);
    Detected { profile, ground, forced }
}

#[derive(Default)]
struct ColorEnv<'a> {
    no_color: bool,
    forced: Option<&'a str>,
    colorterm: Option<&'a str>,
    term: Option<&'a str>,
    program: Option<&'a str>,
    version: Option<&'a str>,
    multiplexed: bool,
}

/// The board and doctor use the same decision, without a terminal query.
fn color_from_env() -> (Profile, &'static str) {
    let var = |key| std::env::var(key).ok();
    color_profile(ColorEnv {
        no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
        forced: var("MESIMON_COLOR").as_deref(),
        colorterm: var("COLORTERM").as_deref(),
        term: var("TERM").as_deref(),
        program: var("TERM_PROGRAM").as_deref(),
        version: var("TERM_PROGRAM_VERSION").as_deref(),
        multiplexed: std::env::var_os("TMUX").is_some() || std::env::var_os("STY").is_some(),
    })
}

fn profile_from_env() -> Profile {
    color_from_env().0
}

/// First hit wins: explicit choices, COLORTERM, terminal evidence, TERM's
/// indexed fallback. An inherited outer emulator is not evidence that a
/// multiplexer passes RGB. Never `$COLORFGBG` or a macOS bundle id (D19).
fn color_profile(e: ColorEnv<'_>) -> (Profile, &'static str) {
    if e.no_color {
        return (Profile::Mono, "NO_COLOR");
    }
    if let Some(v) = e.forced {
        let profile = match v {
            "never" | "mono" => Some(Profile::Mono),
            "8" => Some(Profile::Ansi8),
            "16" => Some(Profile::Ansi16),
            "256" => Some(Profile::Ansi256),
            "truecolor" | "24bit" => Some(Profile::TrueColor),
            _ => None,
        };
        if let Some(profile) = profile {
            return (profile, "MESIMON_COLOR / --color");
        }
    }
    if matches!(e.colorterm, Some("truecolor" | "24bit")) {
        return (Profile::TrueColor, "COLORTERM");
    }
    let term = e.term.unwrap_or("");
    // These describe the actual endpoint and veto an inherited GUI name.
    match term {
        "dumb" => return (Profile::Mono, "TERM=dumb"),
        "linux" => return (Profile::Ansi8, "TERM=linux"),
        _ => {}
    }
    let multiplexed = e.multiplexed
        || term.starts_with("screen")
        || term.starts_with("tmux")
        || matches!(e.program, Some("tmux" | "screen"));
    if term.ends_with("-direct") || term.ends_with("-truecolor") {
        return (Profile::TrueColor, "TERM RGB entry");
    }
    if !multiplexed {
        if matches!(term, "xterm-ghostty" | "xterm-kitty" | "alacritty" | "foot") {
            return (Profile::TrueColor, "TERM RGB terminal");
        }
        if matches!(e.program, Some("ghostty" | "kitty" | "WezTerm" | "cmux")) {
            return (Profile::TrueColor, "TERM_PROGRAM RGB terminal");
        }
        let major = e.version.and_then(|v| v.split('.').next()?.parse::<u32>().ok());
        if e.program == Some("iTerm.app") && major.is_some_and(|v| v >= 3) {
            return (Profile::TrueColor, "iTerm2 3+");
        }
        // Terminal.app 2.15 / build 465 gained RGB in macOS Tahoe.
        // TERM_PROGRAM_VERSION is its build, not its marketing version.
        // See https://www.ratatui.rs/examples/layout/flex/ . Do not use the
        // host OS version: over SSH that is not the terminal's machine.
        if e.program == Some("Apple_Terminal") && major.is_some_and(|v| v >= 465) {
            return (Profile::TrueColor, "Terminal.app build 465+");
        }
    }
    if term.contains("256color") {
        (Profile::Ansi256, "TERM 256-color entry")
    } else {
        (Profile::Ansi16, "conservative fallback")
    }
}

/// Read-only diagnostics: the profile the board will use, and why. This
/// must work in pipes and never query the terminal or read its preferences.
pub fn doctor_line() -> String {
    let (profile, reason) = color_from_env();
    color_words(profile, reason)
}

fn color_words(profile: Profile, reason: &str) -> String {
    let name = match profile {
        Profile::TrueColor => "24-bit RGB",
        Profile::Ansi256 => "256 colors",
        Profile::Ansi16 => "16 colors",
        Profile::Ansi8 => "8 colors",
        Profile::Mono => "monochrome",
    };
    format!("{name} ({reason})")
}

/// The top of the theme ladder: `MESIMON_THEME`, any `Flavor::name` or the
/// two aliases `dark`/`light`. It pins the process — an explicit choice is
/// not a starting point to be corrected by an OS flip — but not the prefs
/// file, which a pick in the menu still writes for the next launch.
fn forced_flavor() -> Option<Flavor> {
    Flavor::from_name(&std::env::var("MESIMON_THEME").ok()?)
}

/// One colour round-trip through terminal-colorsaurus (which reads
/// `/dev/tty`, sidestepping the T-4 stdin race). `None` is "no usable
/// answer" — unsupported, timed out, or unparseable — and is discarded
/// silently (06 §2.9).
fn query_ground() -> Option<Ground> {
    let mut opts = terminal_colorsaurus::QueryOptions::default();
    // 06 §2.9: 150 ms budget. Terminals that can't answer are detected as
    // such well before the timeout; the ladder must not stall startup.
    opts.timeout = Duration::from_millis(150);
    // `OSC 10` and `OSC 11`, then `DA1`: a reply that misses the budget is
    // still coming, and `osc::ReplySwallow` takes it out of the key stream.
    let asked = std::time::Instant::now();
    let answer = terminal_colorsaurus::theme_mode(opts);
    crate::osc::log(&format!("asked OSC 10, OSC 11, DA1: {answer:?} in {:?}", asked.elapsed()));
    match answer {
        Ok(terminal_colorsaurus::ThemeMode::Light) => Some(Ground::Light),
        Ok(terminal_colorsaurus::ThemeMode::Dark) => Some(Ground::Dark),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_terminals_keep_the_palette_without_colorterm() {
        for term in ["xterm-ghostty", "xterm-kitty", "alacritty", "foot", "xterm-direct"] {
            assert_eq!(
                color_profile(ColorEnv { term: Some(term), ..Default::default() }).0,
                Profile::TrueColor,
                "{term}"
            );
        }
        for program in ["ghostty", "kitty", "WezTerm", "cmux"] {
            assert_eq!(
                color_profile(ColorEnv {
                    program: Some(program),
                    term: Some("xterm-256color"),
                    ..Default::default()
                })
                .0,
                Profile::TrueColor,
                "{program}"
            );
        }
    }

    #[test]
    fn apple_terminal_rgb_depends_on_its_build_not_the_host_os() {
        for (version, expected) in [
            (None, Profile::Ansi256),
            (Some(""), Profile::Ansi256),
            (Some("unknown"), Profile::Ansi256),
            (Some("2.15"), Profile::Ansi256),
            (Some("464"), Profile::Ansi256),
            (Some("465"), Profile::TrueColor),
            (Some("470.2"), Profile::TrueColor),
            (Some("488"), Profile::TrueColor),
        ] {
            assert_eq!(
                color_profile(ColorEnv {
                    program: Some("Apple_Terminal"),
                    version,
                    term: Some("xterm-256color"),
                    ..Default::default()
                })
                .0,
                expected,
                "{version:?}"
            );
        }
        for (version, expected) in [("2.9", Profile::Ansi256), ("3.6.11", Profile::TrueColor)] {
            assert_eq!(
                color_profile(ColorEnv {
                    program: Some("iTerm.app"),
                    version: Some(version),
                    term: Some("xterm-256color"),
                    ..Default::default()
                })
                .0,
                expected
            );
        }
    }

    #[test]
    fn outer_terminal_names_do_not_upgrade_multiplexers_or_basic_terminals() {
        for (term, multiplexed, expected) in [
            ("screen-256color", false, Profile::Ansi256),
            ("tmux-256color", false, Profile::Ansi256),
            ("xterm-256color", true, Profile::Ansi256),
            ("xterm-kitty", true, Profile::Ansi16),
            ("dumb", false, Profile::Mono),
            ("linux", false, Profile::Ansi8),
        ] {
            assert_eq!(
                color_profile(ColorEnv {
                    term: Some(term),
                    program: Some("ghostty"),
                    multiplexed,
                    ..Default::default()
                })
                .0,
                expected,
                "{term}"
            );
        }
        assert_eq!(
            color_profile(ColorEnv {
                term: Some("xterm-256color"),
                program: Some("tmux"),
                version: Some("488"),
                ..Default::default()
            })
            .0,
            Profile::Ansi256
        );
    }

    #[test]
    fn explicit_choices_and_colorterm_keep_their_precedence() {
        let rgb = || ColorEnv {
            program: Some("ghostty"),
            term: Some("xterm-ghostty"),
            colorterm: Some("truecolor"),
            ..Default::default()
        };
        assert_eq!(
            color_profile(ColorEnv { no_color: true, forced: Some("truecolor"), ..rgb() }),
            (Profile::Mono, "NO_COLOR")
        );
        for (forced, expected) in [
            ("never", Profile::Mono),
            ("mono", Profile::Mono),
            ("8", Profile::Ansi8),
            ("16", Profile::Ansi16),
            ("256", Profile::Ansi256),
            ("24bit", Profile::TrueColor),
            ("truecolor", Profile::TrueColor),
        ] {
            assert_eq!(
                color_profile(ColorEnv { forced: Some(forced), ..rgb() }),
                (expected, "MESIMON_COLOR / --color")
            );
        }
        assert_eq!(
            color_profile(ColorEnv { forced: Some("auto"), ..rgb() }),
            (Profile::TrueColor, "COLORTERM")
        );
        assert_eq!(
            color_profile(ColorEnv {
                term: Some("tmux-256color"),
                multiplexed: true,
                colorterm: Some("24bit"),
                ..Default::default()
            }),
            (Profile::TrueColor, "COLORTERM")
        );
    }

    #[test]
    fn generic_term_names_do_not_promise_rgb_and_doctor_explains_the_choice() {
        for (term, expected) in [("xterm-256color", Profile::Ansi256), ("xterm", Profile::Ansi16)] {
            let (profile, reason) = color_profile(ColorEnv {
                term: Some(term),
                program: Some("unknown"),
                ..Default::default()
            });
            assert_eq!(profile, expected);
            let words = color_words(profile, reason);
            assert!(words.is_ascii() && words.contains(reason));
            assert!(!words.contains("24-bit"));
        }
    }

    /// Every flavor name pins, and so do the two old aliases.
    #[test]
    fn the_env_var_names_every_flavor() {
        for f in Flavor::ALL {
            assert_eq!(Flavor::from_name(f.name()), Some(f));
        }
        assert_eq!(Flavor::from_name("dark"), Some(Flavor::Graphite));
        assert_eq!(Flavor::from_name("light"), Some(Flavor::Chalk));
    }

    /// The pty test's child: the startup colour query on the terminal it was
    /// born on, then the board's key road — crossterm under the reply swallow
    /// — until a `z`, printing every key that got through.
    #[test]
    #[ignore = "run by a_colour_reply_after_the_budget_is_never_keys, inside its pty"]
    fn pty_child() {
        if std::env::var_os("DETECT_PTY_CHILD").is_none() {
            return;
        }
        use ratatui::crossterm::event::{self, Event, KeyCode};
        use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};
        let ground = query_ground();
        enable_raw_mode().expect("raw mode");
        print!("asked {ground:?}, reading\r\n");
        let mut swallow = crate::osc::ReplySwallow::default();
        let mut passed = Vec::new();
        'read: while event::poll(Duration::from_secs(10)).expect("poll") {
            let Event::Key(key) = event::read().expect("read") else { continue };
            if let crate::osc::Feed::Pass(keys) = swallow.feed(key.code, key.modifiers) {
                for k in keys {
                    passed.push(k);
                    if k.0 == KeyCode::Char('z') {
                        break 'read;
                    }
                }
            }
        }
        print!("passed {passed:?}\r\n");
        disable_raw_mode().expect("cooked mode");
    }

    /// T-953: the colour query at startup misses its 150 ms budget, and the
    /// terminal's answer — `OSC 10` first, then `OSC 11`, then `DA1`, as
    /// iTerm2 gives it — lands in the key stream afterwards. Not one of its
    /// bytes is a key: the `z` typed after it is the only thing that passes.
    #[test]
    fn a_colour_reply_after_the_budget_is_never_keys() {
        use std::io::{Read, Write};
        use std::os::fd::{FromRawFd, OwnedFd};
        use std::os::unix::process::CommandExt;
        use std::sync::{Arc, Mutex};
        use std::time::Instant;
        let (mut m, mut s) = (0, 0);
        let (name, termp, winp) =
            (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(unsafe { libc::openpty(&mut m, &mut s, name, termp, winp) }, 0, "openpty");
        let mut master = unsafe { std::fs::File::from_raw_fd(m) };
        let slave = unsafe { OwnedFd::from_raw_fd(s) };
        let tty = || std::process::Stdio::from(slave.try_clone().expect("slave"));
        let mut cmd = std::process::Command::new(mesimon_core::exe::current_exe().expect("exe"));
        cmd.args(["--exact", "detect::tests::pty_child", "--ignored", "--nocapture"])
            .env("DETECT_PTY_CHILD", "1")
            .env("TERM", "xterm-256color")
            .env_remove("TMUX")
            .stdin(tty())
            .stdout(tty())
            .stderr(tty());
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn().expect("spawn the child");
        drop(cmd);
        drop(slave);
        let out = Arc::new(Mutex::new(Vec::new()));
        let (mut reader, sink) = (master.try_clone().expect("master"), out.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                sink.lock().expect("out").extend_from_slice(&buf[..n]);
            }
        });
        let shows = |what: &str| {
            let until = Instant::now() + Duration::from_secs(30);
            while Instant::now() < until {
                let out = out.lock().expect("out");
                if out.windows(what.len()).any(|w| w == what.as_bytes()) {
                    return true;
                }
                drop(out);
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        };
        assert!(shows("\x1b]10;?\x07\x1b]11;?\x07\x1b[c"), "the child never asked");
        // The child gave up on its own: the answer comes after its budget.
        assert!(shows("asked None, reading"), "the query did not time out");
        master
            .write_all(b"\x1b]10;rgb:dcaa/dcab/dcaa\x07\x1b]11;rgb:53ec/53ec/53ec\x07\x1b[?62;4c")
            .expect("answer");
        master.flush().expect("flush");
        std::thread::sleep(Duration::from_millis(200));
        master.write_all(b"z").expect("type");
        assert!(shows("passed ["), "the child never read the z");
        let _ = child.wait();
        let text = String::from_utf8_lossy(&out.lock().expect("out")).into_owned();
        let passed = text.lines().find(|l| l.starts_with("passed ")).expect("the passed line");
        assert_eq!(passed.trim_end(), r#"passed [(Char('z'), KeyModifiers(0x0))]"#, "{text}");
    }
}
