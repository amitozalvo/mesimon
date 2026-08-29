//! Terminal capability detection (06 §2.9). Runs exactly once, at startup,
//! before the daemon connect and before raw mode — never while a session is
//! focused, and never in the daemon. The result is cached in `App` for the
//! process lifetime; focus handovers must not re-query (T-4: tmux itself
//! issues OSC 10/11 at attach and the replies race our stdin).
//!
//! Deferred rungs, deliberately: `CSI ? 996 n` + `CSI ? 2031` live re-theming
//! (one Contour-extension feature — with 2031 never armed there is no DSR to
//! disarm), and the two terminfo rungs (terminfo is a guaranteed false
//! negative under tmux; the env rungs carry the real weight).

use crate::theme::{Flavor, Profile, Theme};

pub(crate) fn detect() -> Theme {
    let profile = profile_from_env();
    let flavor = match profile {
        // Mono never queries the tty — there is nothing to colour.
        Profile::Mono => Flavor::Graphite,
        _ => flavor_ladder(),
    };
    Theme::new(flavor, profile)
}

/// Colour-profile ladder, first hit wins: NO_COLOR → MESIMON_COLOR (the
/// --color flag's env twin) → COLORTERM → TERM. Never `$COLORFGBG` (D19).
fn profile_from_env() -> Profile {
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return Profile::Mono;
    }
    if let Ok(v) = std::env::var("MESIMON_COLOR") {
        match v.as_str() {
            "never" | "mono" => return Profile::Mono,
            "8" => return Profile::Ansi8,
            "16" => return Profile::Ansi16,
            "256" => return Profile::Ansi256,
            "truecolor" | "24bit" => return Profile::TrueColor,
            _ => {}
        }
    }
    if let Ok(v) = std::env::var("COLORTERM") {
        if v == "truecolor" || v == "24bit" {
            return Profile::TrueColor;
        }
    }
    match std::env::var("TERM") {
        Ok(t) if t.contains("256color") => Profile::Ansi256,
        Ok(t) if t == "linux" => Profile::Ansi8,
        Ok(t) if t == "dumb" => Profile::Mono,
        _ => Profile::Ansi16,
    }
}

/// Light/dark ladder: `MESIMON_THEME` short-circuits (the config-file rung
/// until a config system exists) → OSC 11 via terminal-colorsaurus →
/// graphite. A reply that cannot be read is discarded silently (06 §2.9).
fn flavor_ladder() -> Flavor {
    if let Ok(v) = std::env::var("MESIMON_THEME") {
        match v.as_str() {
            "light" | "chalk" => return Flavor::Chalk,
            "dark" | "graphite" => return Flavor::Graphite,
            _ => {}
        }
    }
    let mut opts = terminal_colorsaurus::QueryOptions::default();
    // 06 §2.9: 150 ms budget. Terminals that can't answer are detected as
    // such well before the timeout; the ladder must not stall startup.
    opts.timeout = std::time::Duration::from_millis(150);
    match terminal_colorsaurus::theme_mode(opts) {
        Ok(terminal_colorsaurus::ThemeMode::Light) => Flavor::Chalk,
        Ok(terminal_colorsaurus::ThemeMode::Dark) | Err(_) => Flavor::Graphite,
    }
}
