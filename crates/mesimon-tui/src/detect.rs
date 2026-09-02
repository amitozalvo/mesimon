//! Terminal capability detection (06 §2.9). The colour-profile and light/dark
//! ladders run once, at startup, before the daemon connect and before raw
//! mode — never while a session is focused, and never in the daemon.
//!
//! Light/dark can then be WATCHED for the process lifetime (`GroundWatch`,
//! opt-in via `MESIMON_GROUND_WATCH=1` — see `watch_enabled`): the
//! OS flips appearance at sunset, the terminal follows it, and a board that
//! keeps painting graphite on a now-white terminal is unreadable until it is
//! restarted. The watch re-asks the terminal — never the OS — because the
//! terminal's background is the thing the palette has to sit on: a terminal
//! pinned to a dark profile must keep its dark pick through an OS flip, and
//! it does, because its answer never changes.
//!
//! The answer is a GROUND, not a flavor: the terminal only knows light or
//! dark, and which theme sits on each is the user's (`prefs.rs`, two slots).
//! Resolving one to the other is `lib.rs`'s job at startup and
//! `App::watch_flavor`'s afterwards.
//!
//! Deferred rungs, deliberately: `CSI ? 996 n` + `CSI ? 2031` (one Contour
//! extension; with 2031 never armed there is no unsolicited DSR to disarm
//! before a PTY attach, which is the trap 06 §2.9 names), and the two
//! terminfo rungs (terminfo is a guaranteed false negative under tmux; the
//! env rungs carry the real weight).

use std::time::{Duration, Instant};

use crate::theme::{Flavor, Ground, Profile};

/// How often the ground is re-asked. Slow on purpose: an appearance flip is
/// a once-a-day event, and every query is a read off the tty (see
/// `GroundWatch::poll`).
const RECHECK_EVERY: Duration = Duration::from_secs(3);

/// Consecutive unanswered queries that end the watch. A terminal that
/// answered at startup and has now gone quiet three times running is not
/// coming back, and asking forever means reading the tty forever.
const STRIKES: u8 = 3;

pub(crate) struct Detected {
    pub profile: Profile,
    /// What the terminal said, or `Dark` when it could not say.
    pub ground: Ground,
    /// `MESIMON_THEME`: the user's word, pinned for the process.
    pub forced: Option<Flavor>,
    /// Armed only when the terminal actually answered the light/dark query
    /// and nothing is pinned: a forced flavor is not a starting point to be
    /// corrected three seconds later, and a terminal that could not answer
    /// once will not answer later either.
    pub watch: Option<GroundWatch>,
}

pub(crate) fn detect() -> Detected {
    let profile = profile_from_env();
    // Mono never queries the tty — there is nothing to colour, so there is
    // nothing to watch either, and no flavor to pin.
    if profile == Profile::Mono {
        return Detected { profile, ground: Ground::Dark, forced: None, watch: None };
    }
    let forced = forced_flavor();
    // Asked even under a pin: the picker sets the slot the terminal is on,
    // and a one-shot query corrects nothing. Only the WATCH stands down.
    let answer = query_ground();
    Detected {
        profile,
        ground: answer.unwrap_or(Ground::Dark),
        forced,
        watch: if forced.is_none() && watch_enabled() {
            answer.map(GroundWatch::new)
        } else {
            None
        },
    }
}

/// The live re-ask is OFF unless `MESIMON_GROUND_WATCH=1` (author,
/// 2026-09-02). Every query is a write to the tty and a timed read back, and
/// a reply that arrives after the budget lands on stdin as keystrokes —
/// `osc::ReplySwallow` catches the common shape, but a reply split at its
/// first byte still typed `1 1 ; r …` into the board and opened rename on a
/// ticket. One query at startup is one exposure; one every 3 s for the life
/// of the process was the bug. The deferred root fix (the watch parses its
/// own reply, no blocking read) is what would earn the default back.
fn watch_enabled() -> bool {
    std::env::var_os("MESIMON_GROUND_WATCH").is_some_and(|v| v == "1")
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
/// two aliases `dark`/`light`. It short-circuits the watch — an explicit
/// choice is not a starting point to be corrected — but not the prefs file,
/// which a pick in the menu still writes for the next launch.
fn forced_flavor() -> Option<Flavor> {
    Flavor::from_name(&std::env::var("MESIMON_THEME").ok()?)
}

/// One OSC 11 round-trip through terminal-colorsaurus (which reads
/// `/dev/tty`, sidestepping the T-4 stdin race). `None` is "no usable
/// answer" — unsupported, timed out, or unparseable — and is discarded
/// silently (06 §2.9).
pub(crate) fn query_ground() -> Option<Ground> {
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

/// The live half of the light/dark ladder: re-ask on a slow cadence, report
/// only a change.
///
/// The query is a write to the tty and a read back off it, so the caller
/// decides WHEN it is safe (`due` is cheap, `poll` is not): never with a
/// keypress already waiting and never under a text field, because the read
/// discards whatever it finds ahead of the reply — which would be the key
/// that was just typed.
pub(crate) struct GroundWatch {
    current: Ground,
    last: Instant,
    /// Consecutive silences. Reset by any answer; `STRIKES` of them ends the
    /// watch for good.
    strikes: u8,
}

impl GroundWatch {
    fn new(current: Ground) -> Self {
        Self { current, last: Instant::now(), strikes: 0 }
    }

    /// Cheap enough to ask every frame: is another query due, and is the
    /// watch still alive?
    pub(crate) fn due(&self) -> bool {
        self.strikes < STRIKES && self.last.elapsed() >= RECHECK_EVERY
    }

    /// Run one query. `Some` only when the ground actually changed — a board
    /// that repaints in the same colours is a repaint for nothing.
    pub(crate) fn poll(&mut self, query: impl FnOnce() -> Option<Ground>) -> Option<Ground> {
        self.last = Instant::now();
        let Some(ground) = query() else {
            self.strikes = self.strikes.saturating_add(1);
            return None;
        };
        self.strikes = 0;
        (ground != std::mem::replace(&mut self.current, ground)).then_some(ground)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch() -> GroundWatch {
        GroundWatch::new(Ground::Dark)
    }

    #[test]
    fn an_unchanged_answer_repaints_nothing() {
        let mut w = watch();
        assert_eq!(w.poll(|| Some(Ground::Dark)), None);
        assert_eq!(w.poll(|| Some(Ground::Dark)), None);
    }

    #[test]
    fn a_flip_is_reported_once_and_becomes_the_new_resting_state() {
        let mut w = watch();
        assert_eq!(w.poll(|| Some(Ground::Light)), Some(Ground::Light));
        assert_eq!(w.poll(|| Some(Ground::Light)), None);
        assert_eq!(w.poll(|| Some(Ground::Dark)), Some(Ground::Dark));
    }

    #[test]
    fn one_missed_answer_is_forgiven() {
        let mut w = watch();
        assert_eq!(w.poll(|| None), None);
        assert_eq!(w.poll(|| None), None);
        assert_eq!(w.poll(|| Some(Ground::Light)), Some(Ground::Light));
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
