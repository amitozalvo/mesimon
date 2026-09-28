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
