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

/// One OSC 11 round-trip through terminal-colorsaurus (which reads
/// `/dev/tty`, sidestepping the T-4 stdin race). `None` is "no usable
/// answer" — unsupported, timed out, or unparseable — and is discarded
/// silently (06 §2.9).
fn query_ground() -> Option<Ground> {
    let mut opts = terminal_colorsaurus::QueryOptions::default();
    // 06 §2.9: 150 ms budget. Terminals that can't answer are detected as
    // such well before the timeout; the ladder must not stall startup.
    opts.timeout = Duration::from_millis(150);
    match terminal_colorsaurus::theme_mode(opts) {
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
}
