//! Terminal capability detection (06 §2.9). The colour-profile and light/dark
//! ladders run once, at startup, before the daemon connect and before raw
//! mode — never while a session is focused, and never in the daemon.
//!
//! Light/dark is then WATCHED for the process lifetime (`FlavorWatch`): the
//! OS flips appearance at sunset, the terminal follows it, and a board that
//! keeps painting graphite on a now-white terminal is unreadable until it is
//! restarted. The watch re-asks the terminal — never the OS — because the
//! terminal's background is the thing the palette has to sit on: a terminal
//! pinned to a dark profile must keep graphite through an OS flip, and it
//! does, because its answer never changes.
//!
//! Deferred rungs, deliberately: `CSI ? 996 n` + `CSI ? 2031` (one Contour
//! extension; with 2031 never armed there is no unsolicited DSR to disarm
//! before a PTY attach, which is the trap 06 §2.9 names), and the two
//! terminfo rungs (terminfo is a guaranteed false negative under tmux; the
//! env rungs carry the real weight).

use std::time::{Duration, Instant};

use crate::theme::{Flavor, Profile, Theme};

/// How often the flavor is re-asked. Slow on purpose: an appearance flip is
/// a once-a-day event, and every query is a read off the tty (see
/// `FlavorWatch::poll`).
const RECHECK_EVERY: Duration = Duration::from_secs(3);

/// Consecutive unanswered queries that end the watch. A terminal that
/// answered at startup and has now gone quiet three times running is not
/// coming back, and asking forever means reading the tty forever.
const STRIKES: u8 = 3;

pub(crate) struct Detected {
    pub theme: Theme,
    /// Armed only when the terminal actually answered the light/dark query:
    /// a forced flavor is the user's word (never overridden), and a terminal
    /// that could not answer once will not answer later either.
    pub watch: Option<FlavorWatch>,
}

pub(crate) fn detect() -> Detected {
    let profile = profile_from_env();
    // Mono never queries the tty — there is nothing to colour, so there is
    // nothing to watch either.
    if profile == Profile::Mono {
        return Detected { theme: Theme::new(Flavor::Graphite, profile), watch: None };
    }
    if let Some(flavor) = forced_flavor() {
        return Detected { theme: Theme::new(flavor, profile), watch: None };
    }
    match query_flavor() {
        Some(flavor) => {
            Detected { theme: Theme::new(flavor, profile), watch: Some(FlavorWatch::new(flavor)) }
        }
        None => Detected { theme: Theme::new(Flavor::Graphite, profile), watch: None },
    }
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

/// The top of the light/dark ladder: `MESIMON_THEME` (the config-file rung
/// until a config system exists). It short-circuits the query AND the watch —
/// an explicit choice is not a starting point to be corrected.
fn forced_flavor() -> Option<Flavor> {
    match std::env::var("MESIMON_THEME").ok()?.as_str() {
        "light" | "chalk" => Some(Flavor::Chalk),
        "dark" | "graphite" => Some(Flavor::Graphite),
        _ => None,
    }
}

/// One OSC 11 round-trip through terminal-colorsaurus (which reads
/// `/dev/tty`, sidestepping the T-4 stdin race). `None` is "no usable
/// answer" — unsupported, timed out, or unparseable — and is discarded
/// silently (06 §2.9).
pub(crate) fn query_flavor() -> Option<Flavor> {
    let mut opts = terminal_colorsaurus::QueryOptions::default();
    // 06 §2.9: 150 ms budget. Terminals that can't answer are detected as
    // such well before the timeout; the ladder must not stall startup.
    opts.timeout = Duration::from_millis(150);
    match terminal_colorsaurus::theme_mode(opts) {
        Ok(terminal_colorsaurus::ThemeMode::Light) => Some(Flavor::Chalk),
        Ok(terminal_colorsaurus::ThemeMode::Dark) => Some(Flavor::Graphite),
        Err(_) => None,
    }
}

/// The live half of the light/dark ladder: re-ask on a slow cadence, report
/// only a change.
///
/// The query is a write to the tty and a read back off it, so the caller
/// decides WHEN it is safe (`due` is cheap, `poll` is not): never with a
/// keypress already waiting and never under a text field, because the read
/// discards whatever it finds ahead of the reply — which would be the key
/// that was just typed.
pub(crate) struct FlavorWatch {
    current: Flavor,
    last: Instant,
    /// Consecutive silences. Reset by any answer; `STRIKES` of them ends the
    /// watch for good.
    strikes: u8,
}

impl FlavorWatch {
    fn new(current: Flavor) -> Self {
        Self { current, last: Instant::now(), strikes: 0 }
    }

    /// Cheap enough to ask every frame: is another query due, and is the
    /// watch still alive?
    pub(crate) fn due(&self) -> bool {
        self.strikes < STRIKES && self.last.elapsed() >= RECHECK_EVERY
    }

    /// Run one query. `Some` only when the flavor actually changed — a board
    /// that repaints in the same colours is a repaint for nothing.
    pub(crate) fn poll(&mut self, query: impl FnOnce() -> Option<Flavor>) -> Option<Flavor> {
        self.last = Instant::now();
        let Some(flavor) = query() else {
            self.strikes = self.strikes.saturating_add(1);
            return None;
        };
        self.strikes = 0;
        (flavor != std::mem::replace(&mut self.current, flavor)).then_some(flavor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch() -> FlavorWatch {
        FlavorWatch::new(Flavor::Graphite)
    }

    #[test]
    fn an_unchanged_answer_repaints_nothing() {
        let mut w = watch();
        assert_eq!(w.poll(|| Some(Flavor::Graphite)), None);
        assert_eq!(w.poll(|| Some(Flavor::Graphite)), None);
    }

    #[test]
    fn a_flip_is_reported_once_and_becomes_the_new_resting_state() {
        let mut w = watch();
        assert_eq!(w.poll(|| Some(Flavor::Chalk)), Some(Flavor::Chalk));
        assert_eq!(w.poll(|| Some(Flavor::Chalk)), None);
        assert_eq!(w.poll(|| Some(Flavor::Graphite)), Some(Flavor::Graphite));
    }

    #[test]
    fn one_missed_answer_is_forgiven() {
        let mut w = watch();
        assert_eq!(w.poll(|| None), None);
        assert_eq!(w.poll(|| None), None);
        assert_eq!(w.poll(|| Some(Flavor::Chalk)), Some(Flavor::Chalk));
        // The answer cleared the strikes, so a fresh run of silence gets its
        // own full allowance.
        for _ in 0..STRIKES - 1 {
            assert_eq!(w.poll(|| None), None);
        }
        w.last = Instant::now() - RECHECK_EVERY;
        assert!(w.due());
    }

    #[test]
    fn a_terminal_that_goes_quiet_ends_the_watch() {
        let mut w = watch();
        for _ in 0..STRIKES {
            assert_eq!(w.poll(|| None), None);
        }
        w.last = Instant::now() - RECHECK_EVERY;
        assert!(!w.due(), "a silent terminal must stop being read");
    }

    #[test]
    fn the_cadence_holds_the_query_back() {
        let w = watch();
        assert!(!w.due(), "a query just ran; the next one waits out the cadence");
    }
}
