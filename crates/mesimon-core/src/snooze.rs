//! Snooze presets and the calendar arithmetic behind them (T-74).
//!
//! `z` on a card cycles a FIXED ladder — `1h`, `4h`, `tomorrow 9:00`, `next
//! Monday 9:00` — and Enter turns the pick into one absolute deadline the
//! daemon compares against its clock. The last rung is "the start of next
//! week", and which day that is is the user's (`Weekday`, a Settings row):
//! Monday for most of the world, Sunday in Israel and the US, Saturday
//! across the Middle East. The ring stays FOUR rungs whichever it is.
//!
//! The two relative steps are plain seconds; the two calendar steps are "the
//! next 09:00 local on that day", which needs a local wall clock this crate
//! does not have. So the pure part lives here — what day, what hour — over
//! a `LocalTime` in `struct tm`'s own conventions, and the TUI supplies
//! `localtime_r` in and `mktime` out. `mday` is deliberately left
//! un-normalised (`32` is fine): `mktime` rolls it into the next month, and
//! that keeps every leap-year and month-length rule out of this file.
//!
//! The labels are `&'static str` because a footer hint is one (the keymap's
//! rule: a hint comes from a fixed set). The exact clock time the pick
//! resolves to is the status line's and the card's to say.

/// The day a week starts on — the preference behind the ring's last rung.
/// Three inhabitants, the three first days in use anywhere; a `struct tm`
/// `wday` each, so the arithmetic needs no table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Weekday {
    #[default]
    Monday,
    Sunday,
    Saturday,
}

impl Weekday {
    /// In the order the Settings row cycles them: the ISO default first,
    /// then the two that precede it on the calendar.
    pub const ALL: [Weekday; 3] = [Weekday::Monday, Weekday::Sunday, Weekday::Saturday];

    /// The one after this, wrapping — the Settings row's Enter.
    pub fn next(self) -> Weekday {
        let i = Weekday::ALL.iter().position(|d| *d == self).unwrap_or(0);
        Weekday::ALL[(i + 1) % Weekday::ALL.len()]
    }

    /// `struct tm`'s `tm_wday`: 0 = Sunday.
    pub fn wday(self) -> i32 {
        match self {
            Weekday::Sunday => 0,
            Weekday::Monday => 1,
            Weekday::Saturday => 6,
        }
    }

    /// The day's name, as the ring's rung and the Settings row spell it.
    pub fn name(self) -> &'static str {
        match self {
            Weekday::Monday => "Monday",
            Weekday::Sunday => "Sunday",
            Weekday::Saturday => "Saturday",
        }
    }

    /// The `prefs.json` spelling: the name, lower-case.
    pub fn key(self) -> &'static str {
        match self {
            Weekday::Monday => "monday",
            Weekday::Sunday => "sunday",
            Weekday::Saturday => "saturday",
        }
    }

    /// `key`'s inverse, for the file; case-insensitive because a hand edit
    /// is the other writer. `None` is an unknown day, and the caller's
    /// default.
    pub fn from_key(s: &str) -> Option<Weekday> {
        Weekday::ALL.into_iter().find(|d| d.key().eq_ignore_ascii_case(s))
    }
}

/// One rung of the ladder, in the order `z` walks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    OneHour,
    FourHours,
    Tomorrow9,
    /// 09:00 on the first day of next week — `Weekday` says which day.
    NextWeek9,
}

impl Preset {
    pub const ALL: [Preset; 4] =
        [Preset::OneHour, Preset::FourHours, Preset::Tomorrow9, Preset::NextWeek9];

    /// The rung after this one, wrapping: a repeated `z` walks the ring.
    pub fn next(self) -> Preset {
        let i = Preset::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Preset::ALL[(i + 1) % Preset::ALL.len()]
    }

    /// The word the footer and the card use. Static still — the week can
    /// start on one of three days, so the last rung has three spellings,
    /// each a literal.
    pub fn label(self, week_start: Weekday) -> &'static str {
        match self {
            Preset::OneHour => "1h",
            Preset::FourHours => "4h",
            Preset::Tomorrow9 => "tomorrow 9:00",
            Preset::NextWeek9 => match week_start {
                Weekday::Monday => "next Monday 9:00",
                Weekday::Sunday => "next Sunday 9:00",
                Weekday::Saturday => "next Saturday 9:00",
            },
        }
    }

    /// A calendar rung resolves through the local wall clock; a relative
    /// one is seconds from now.
    pub fn is_calendar(self) -> bool {
        matches!(self, Preset::Tomorrow9 | Preset::NextWeek9)
    }
}

/// The confirm key's hint for a label — one static per rung (per spelling
/// of the last), so the keymap can name the pick without a `format!`. An
/// unknown word (the chord is not armed) falls to the bare verb.
pub fn hint_for_label(label: &str) -> &'static str {
    match label {
        "1h" => "snooze 1h",
        "4h" => "snooze 4h",
        "tomorrow 9:00" => "snooze until tomorrow 9:00",
        "next Monday 9:00" => "snooze until next Monday 9:00",
        "next Sunday 9:00" => "snooze until next Sunday 9:00",
        "next Saturday 9:00" => "snooze until next Saturday 9:00",
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
/// Monday" on a Monday is the one a week out (so for whichever day the week
/// starts on). `mday` may run past the month; the caller's `mktime`
/// normalises it.
pub fn target(preset: Preset, now: &LocalTime, week_start: Weekday) -> LocalTime {
    let days = match preset {
        Preset::Tomorrow9 => 1,
        Preset::NextWeek9 => {
            let d = (week_start.wday() - now.wday).rem_euclid(7);
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
    week_start: Weekday,
    to_epoch: &dyn Fn(&LocalTime) -> Option<u64>,
) -> Option<u64> {
    match preset {
        Preset::OneHour => Some(now_secs + 3600),
        Preset::FourHours => Some(now_secs + 4 * 3600),
        Preset::Tomorrow9 | Preset::NextWeek9 => to_epoch(&target(preset, now_local, week_start)),
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
            for d in Weekday::ALL {
                let hint = hint_for_label(p.label(d));
                assert!(hint.starts_with("snooze "), "{p:?} on {d:?}: {hint}");
                assert!(hint.ends_with(p.label(d)), "{p:?} on {d:?}: the hint names the rung");
            }
        }
        assert_eq!(hint_for_label(""), "snooze");
    }

    /// The week-start preference: three days on a ring, each with a stable
    /// file spelling that reads back, and an unknown one refused.
    #[test]
    fn the_week_start_is_a_ring_with_a_file_spelling() {
        let mut d = Weekday::default();
        assert_eq!(d, Weekday::Monday, "ISO's default");
        let mut seen = vec![d];
        for _ in 0..2 {
            d = d.next();
            seen.push(d);
        }
        assert_eq!(seen, Weekday::ALL.to_vec());
        assert_eq!(d.next(), Weekday::Monday, "wraps");
        for d in Weekday::ALL {
            assert_eq!(Weekday::from_key(d.key()), Some(d));
            assert_eq!(Weekday::from_key(d.name()), Some(d), "case does not matter");
            assert_eq!(d.key(), d.name().to_ascii_lowercase());
            assert!(Preset::NextWeek9.label(d).contains(d.name()));
        }
        assert_eq!(Weekday::from_key("wednesday"), None);
        assert_eq!(Weekday::from_key(""), None);
    }

    #[test]
    fn relative_rungs_never_touch_the_calendar() {
        let panic = |_: &LocalTime| -> Option<u64> { panic!("asked the clock") };
        let d = Weekday::Monday;
        assert_eq!(deadline(Preset::OneHour, 1000, &THU, d, &panic), Some(4600));
        assert_eq!(deadline(Preset::FourHours, 1000, &THU, d, &panic), Some(15400));
    }

    #[test]
    fn tomorrow_is_the_next_day_at_nine() {
        let d = Weekday::Monday;
        let t = target(Preset::Tomorrow9, &THU, d);
        assert_eq!((t.mday, t.hour, t.min, t.sec), (4, 9, 0, 0));
        // Late at night is still "tomorrow", never "in a few hours".
        let late = LocalTime { hour: 23, min: 59, ..THU };
        assert_eq!(target(Preset::Tomorrow9, &late, d).mday, 4);
        // And a morning before nine still skips to the NEXT morning.
        let early = LocalTime { hour: 7, ..THU };
        assert_eq!(target(Preset::Tomorrow9, &early, d).mday, 4);
        // The week start has no say in "tomorrow".
        for d in Weekday::ALL {
            assert_eq!(target(Preset::Tomorrow9, &THU, d).mday, 4);
        }
    }

    #[test]
    fn next_week_is_always_ahead_whichever_day_starts_it() {
        // wday 0..=6 → days to add. Monday: Sun 1, Mon 7, Tue 6, Wed 5,
        // Thu 4, Fri 3, Sat 2. Sunday: Sun 7, Mon 6 … Sat 1. Saturday: Sun 6,
        // Mon 5 … Fri 1, Sat 7.
        let want = [
            (Weekday::Monday, [1, 7, 6, 5, 4, 3, 2]),
            (Weekday::Sunday, [7, 6, 5, 4, 3, 2, 1]),
            (Weekday::Saturday, [6, 5, 4, 3, 2, 1, 7]),
        ];
        for (start, days_by_wday) in want {
            for (wday, days) in days_by_wday.iter().enumerate() {
                let now = LocalTime { wday: wday as i32, ..THU };
                let t = target(Preset::NextWeek9, &now, start);
                assert_eq!(t.mday - THU.mday, *days, "{start:?} from wday {wday}");
                assert_eq!(t.hour, 9);
                // Landing on the day itself is the one a week out, never today.
                assert!((1..=7).contains(days));
                // And the day landed on IS the start of the week.
                assert_eq!((wday as i32 + days) % 7, start.wday(), "{start:?} from wday {wday}");
            }
        }
    }

    #[test]
    fn month_and_year_ends_roll_through_mktime() {
        // 2026-12-31 (Thursday): tomorrow is 2027-01-01, mday 32 un-normalised.
        let nye = LocalTime { year: 126, mon: 11, mday: 31, hour: 10, min: 0, sec: 0, wday: 4 };
        let t = target(Preset::Tomorrow9, &nye, Weekday::Monday);
        assert_eq!(t.mday, 32);
        let epoch = fake_epoch(&t).expect("resolves");
        let jan1 = LocalTime { year: 127, mon: 0, mday: 1, hour: 9, min: 0, sec: 0, wday: 5 };
        assert_eq!(epoch, fake_epoch(&jan1).expect("resolves"));
        // And the deadline road agrees, and is after now.
        let now = fake_epoch(&nye).expect("now");
        let d =
            deadline(Preset::Tomorrow9, now, &nye, Weekday::Monday, &fake_epoch).expect("deadline");
        assert_eq!(d, epoch);
        assert!(d > now);
    }

    #[test]
    fn every_rung_lands_in_the_future() {
        let now = fake_epoch(&THU).expect("now");
        for p in Preset::ALL {
            for w in Weekday::ALL {
                let d = deadline(p, now, &THU, w, &fake_epoch).expect("deadline");
                assert!(d > now, "{p:?} on {w:?}");
            }
        }
        // A clock that cannot answer refuses rather than guesses.
        let mute = |_: &LocalTime| -> Option<u64> { None };
        assert_eq!(deadline(Preset::Tomorrow9, now, &THU, Weekday::Monday, &mute), None);
    }
}
