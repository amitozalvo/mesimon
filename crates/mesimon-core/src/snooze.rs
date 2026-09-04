//! Snooze presets and the calendar arithmetic behind them (T-74).
//!
//! `z` on a card cycles a FIXED ladder — `1h`, `4h`, `tomorrow 9:00`, `next
//! Monday 9:00` — and Enter turns the pick into one absolute deadline the
//! daemon compares against its clock. The two relative steps are plain
//! seconds; the two calendar steps are "the next 09:00 local on that day",
//! which needs a local wall clock this crate does not have. So the pure
//! part lives here — what day, what hour — over a `LocalTime` in `struct
//! tm`'s own conventions, and the TUI supplies `localtime_r` in and `mktime`
//! out. `mday` is deliberately left un-normalised (`32` is fine): `mktime`
//! rolls it into the next month, and that keeps every leap-year and
//! month-length rule out of this file.
//!
//! The labels are `&'static str` because a footer hint is one (the keymap's
//! rule: a hint comes from a fixed set). The exact clock time the pick
//! resolves to is the status line's and the card's to say.

/// One rung of the ladder, in the order `z` walks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    OneHour,
    FourHours,
    Tomorrow9,
    NextMonday9,
}

impl Preset {
    pub const ALL: [Preset; 4] =
        [Preset::OneHour, Preset::FourHours, Preset::Tomorrow9, Preset::NextMonday9];

    /// The rung after this one, wrapping: a repeated `z` walks the ring.
    pub fn next(self) -> Preset {
        let i = Preset::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Preset::ALL[(i + 1) % Preset::ALL.len()]
    }

    /// The word the footer and the card use.
    pub fn label(self) -> &'static str {
        match self {
            Preset::OneHour => "1h",
            Preset::FourHours => "4h",
            Preset::Tomorrow9 => "tomorrow 9:00",
            Preset::NextMonday9 => "next Monday 9:00",
        }
    }

    /// A calendar rung resolves through the local wall clock; a relative
    /// one is seconds from now.
    pub fn is_calendar(self) -> bool {
        matches!(self, Preset::Tomorrow9 | Preset::NextMonday9)
    }
}

/// The confirm key's hint for a label — one static per rung, so the keymap
/// can name the pick without a `format!`. An unknown word (the chord is not
/// armed) falls to the bare verb.
pub fn hint_for_label(label: &str) -> &'static str {
    match label {
        "1h" => "snooze 1h",
        "4h" => "snooze 4h",
        "tomorrow 9:00" => "snooze until tomorrow 9:00",
        "next Monday 9:00" => "snooze until next Monday 9:00",
        _ => "snooze",
    }
}

/// A broken-down local time in `struct tm`'s conventions: `year` is years
/// since 1900, `mon` is 0–11, `wday` is 0 = Sunday. Kept that way so the
/// libc glue is a field copy with nothing to get off by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub mon: i32,
    pub mday: i32,
    pub hour: i32,
    pub min: i32,
    pub sec: i32,
    pub wday: i32,
}

const WAKE_HOUR: i32 = 9;

/// The local wall-clock moment a calendar rung means, given now. Always in
/// the future by at least a day: "tomorrow" is never today, and "next
/// Monday" on a Monday is the one a week out. `mday` may run past the
/// month; the caller's `mktime` normalises it.
pub fn target(preset: Preset, now: &LocalTime) -> LocalTime {
    let days = match preset {
        Preset::Tomorrow9 => 1,
        Preset::NextMonday9 => {
            let d = (8 - now.wday).rem_euclid(7);
            if d == 0 {
                7
            } else {
                d
            }
        }
        Preset::OneHour | Preset::FourHours => 0,
    };
    LocalTime { mday: now.mday + days, hour: WAKE_HOUR, min: 0, sec: 0, ..*now }
}

/// The absolute deadline, unix seconds. `to_epoch` is the wall clock's
/// inverse (`mktime`), asked only for the calendar rungs; `None` means the
/// clock could not say, and the caller refuses rather than guesses.
pub fn deadline(
    preset: Preset,
    now_secs: u64,
    now_local: &LocalTime,
    to_epoch: &dyn Fn(&LocalTime) -> Option<u64>,
) -> Option<u64> {
    match preset {
        Preset::OneHour => Some(now_secs + 3600),
        Preset::FourHours => Some(now_secs + 4 * 3600),
        Preset::Tomorrow9 | Preset::NextMonday9 => to_epoch(&target(preset, now_local)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake `mktime` over a fixed offset: civil date → days since the
    /// epoch, with `mday` past the month rolling forward the way the real
    /// one does. Enough to prove the arithmetic without a wall clock.
    fn fake_epoch(t: &LocalTime) -> Option<u64> {
        let month_days = |y: i32, m: i32| -> i32 {
            match m {
                0 | 2 | 4 | 6 | 7 | 9 | 11 => 31,
                3 | 5 | 8 | 10 => 30,
                _ => {
                    if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                        29
                    } else {
                        28
                    }
                }
            }
        };
        let (mut y, mut m, mut d) = (t.year + 1900, t.mon, t.mday);
        while d > month_days(y, m) {
            d -= month_days(y, m);
            m += 1;
            if m == 12 {
                m = 0;
                y += 1;
            }
        }
        // Howard Hinnant's days_from_civil, m as 1-12.
        let m1 = m + 1;
        let y2 = if m1 <= 2 { y - 1 } else { y };
        let era = y2.div_euclid(400);
        let yoe = y2 - era * 400;
        let doy = (153 * (m1 + if m1 > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146097 + doe - 719468;
        let secs = days as i64 * 86400 + (t.hour * 3600 + t.min * 60 + t.sec) as i64;
        u64::try_from(secs).ok()
    }

    // Thursday 2026-09-03 15:42:07 local.
    const THU: LocalTime =
        LocalTime { year: 126, mon: 8, mday: 3, hour: 15, min: 42, sec: 7, wday: 4 };

    #[test]
    fn the_ladder_is_a_ring() {
        let mut p = Preset::OneHour;
        let mut seen = vec![p];
        for _ in 0..3 {
            p = p.next();
            seen.push(p);
        }
        assert_eq!(seen, Preset::ALL.to_vec());
        assert_eq!(p.next(), Preset::OneHour, "wraps");
        for p in Preset::ALL {
            assert_eq!(hint_for_label(p.label()), hint_for_label(p.label()));
            assert!(hint_for_label(p.label()).starts_with("snooze "), "{:?}", p);
        }
        assert_eq!(hint_for_label(""), "snooze");
    }

    #[test]
    fn relative_rungs_never_touch_the_calendar() {
        let panic = |_: &LocalTime| -> Option<u64> { panic!("asked the clock") };
        assert_eq!(deadline(Preset::OneHour, 1000, &THU, &panic), Some(4600));
        assert_eq!(deadline(Preset::FourHours, 1000, &THU, &panic), Some(15400));
    }

    #[test]
    fn tomorrow_is_the_next_day_at_nine() {
        let t = target(Preset::Tomorrow9, &THU);
        assert_eq!((t.mday, t.hour, t.min, t.sec), (4, 9, 0, 0));
        // Late at night is still "tomorrow", never "in a few hours".
        let late = LocalTime { hour: 23, min: 59, ..THU };
        assert_eq!(target(Preset::Tomorrow9, &late).mday, 4);
        // And a morning before nine still skips to the NEXT morning.
        let early = LocalTime { hour: 7, ..THU };
        assert_eq!(target(Preset::Tomorrow9, &early).mday, 4);
    }

    #[test]
    fn next_monday_is_always_ahead() {
        // wday 0..=6 → days to add: Sun 1, Mon 7, Tue 6, Wed 5, Thu 4, Fri 3, Sat 2.
        let want = [1, 7, 6, 5, 4, 3, 2];
        for (wday, days) in want.iter().enumerate() {
            let now = LocalTime { wday: wday as i32, ..THU };
            let t = target(Preset::NextMonday9, &now);
            assert_eq!(t.mday - THU.mday, *days, "wday {wday}");
            assert_eq!(t.hour, 9);
        }
    }

    #[test]
    fn month_and_year_ends_roll_through_mktime() {
        // 2026-12-31 (Thursday): tomorrow is 2027-01-01, mday 32 un-normalised.
        let nye = LocalTime { year: 126, mon: 11, mday: 31, hour: 10, min: 0, sec: 0, wday: 4 };
        let t = target(Preset::Tomorrow9, &nye);
        assert_eq!(t.mday, 32);
        let epoch = fake_epoch(&t).expect("resolves");
        let jan1 = LocalTime { year: 127, mon: 0, mday: 1, hour: 9, min: 0, sec: 0, wday: 5 };
        assert_eq!(epoch, fake_epoch(&jan1).expect("resolves"));
        // And the deadline road agrees, and is after now.
        let now = fake_epoch(&nye).expect("now");
        let d = deadline(Preset::Tomorrow9, now, &nye, &fake_epoch).expect("deadline");
        assert_eq!(d, epoch);
        assert!(d > now);
    }

    #[test]
    fn every_rung_lands_in_the_future() {
        let now = fake_epoch(&THU).expect("now");
        for p in Preset::ALL {
            let d = deadline(p, now, &THU, &fake_epoch).expect("deadline");
            assert!(d > now, "{p:?}");
        }
        // A clock that cannot answer refuses rather than guesses.
        let mute = |_: &LocalTime| -> Option<u64> { None };
        assert_eq!(deadline(Preset::Tomorrow9, now, &THU, &mute), None);
    }
}
