//! The local wall clock, for the snooze presets that name a day (T-74).
//!
//! `core::snooze` decides WHAT "tomorrow 9:00" is over a broken-down local
//! time and hands back another one; this is the libc on either side of it —
//! `localtime_r` in, `mktime` out — kept to a field copy each way. There is
//! no chrono in the workspace and this is the one place that needs a time
//! zone, so two C calls it is. `mktime` with `tm_isdst = -1` also does the
//! DST arithmetic and rolls an over-long `mday` into the next month, which
//! is exactly what the pure side leaves to it.

use mesimon_core::snooze::LocalTime;

use mesimon_core::clock::now_secs as epoch_now;

/// The moment `secs` in local time, or `None` when libc cannot say.
pub(crate) fn local_of(secs: u64) -> Option<LocalTime> {
    // `t`'s type is inferred from `localtime_r`'s parameter and never spelled:
    // the libc crate deprecates the `time_t` alias on musl (its width is due
    // to change there), and naming it warns in the Linux release builds only.
    let t = secs.try_into().ok()?;
    // SAFETY: `localtime_r` writes only into the `tm` we hand it, which is
    // zero-initialised and lives for the call; a null return means it
    // could not, and we read nothing then.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    ok.then_some(LocalTime {
        year: tm.tm_year,
        mon: tm.tm_mon,
        mday: tm.tm_mday,
        hour: tm.tm_hour,
        min: tm.tm_min,
        sec: tm.tm_sec,
        wday: tm.tm_wday,
    })
}

/// A moment in unix seconds, broken down locally — the board's clock for a
/// quota's reset time (T-327), handed to `App::clock` by `lib.rs::run`.
pub(crate) fn at(secs: u64) -> Option<LocalTime> {
    local_of(secs)
}

/// Now, broken down locally.
pub(crate) fn now_local() -> Option<LocalTime> {
    local_of(epoch_now())
}

/// The local wall-clock moment back to unix seconds. `tm_isdst = -1` asks
/// libc to work out daylight time for that date itself.
pub(crate) fn to_epoch(t: &LocalTime) -> Option<u64> {
    // SAFETY: a zeroed `tm` with the fields we set is a valid input to
    // `mktime`, which reads it and normalises it in place; `-1` is its one
    // error value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = t.year;
    tm.tm_mon = t.mon;
    tm.tm_mday = t.mday;
    tm.tm_hour = t.hour;
    tm.tm_min = t.min;
    tm.tm_sec = t.sec;
    tm.tm_isdst = -1;
    let secs = unsafe { libc::mktime(&mut tm) };
    if secs == -1 {
        return None;
    }
    u64::try_from(secs).ok()
}

/// `HH:MM`, local, for a moment in unix seconds — the confirm status names
/// the clock the ticket comes back at.
pub(crate) fn clock_word(secs: u64) -> Option<String> {
    local_of(secs).map(|t| format!("{:02}:{:02}", t.hour, t.min))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::snooze::{deadline, Preset, Weekday};

    /// Every rung from `now` lands ahead of it: a relative one by its offset
    /// in instants, a calendar one at nine on the wall clock.
    fn presets_land_ahead(now: u64) {
        let local = local_of(now).expect("localtime_r answers");
        for p in Preset::ALL {
            for w in Weekday::ALL {
                let d = deadline(p, now, &local, w, &to_epoch).expect("resolves");
                assert!(d > now, "{p:?} on {w:?} is ahead");
                match p {
                    Preset::OneHour => assert_eq!(d - now, 3600, "{p:?} is an hour of instants"),
                    Preset::FourHours => assert_eq!(d - now, 4 * 3600, "{p:?} is four hours"),
                    Preset::Tomorrow9 | Preset::NextWeek9 => {
                        let at = local_of(d).expect("resolves");
                        assert_eq!((at.hour, at.min, at.sec), (9, 0, 0), "{p:?} is at nine");
                        if p == Preset::NextWeek9 {
                            assert_eq!(at.wday, w.wday(), "{p:?} lands on the week's first day");
                        }
                    }
                }
            }
        }
    }

    /// `mktime` answers an instant whose wall clock reads as the one asked,
    /// and the presets resolve ahead of now on the real clock, whatever zone
    /// the test runs in. Not `to_epoch(local_of(now)) == now`: in the hour a
    /// fall-back repeats, two instants share the wall time and `mktime` may
    /// name the other (T-972). The code asks `mktime` only for a nine o'clock.
    #[test]
    fn the_clock_round_trips_and_the_presets_land_ahead() {
        let now = epoch_now();
        let local = now_local().expect("localtime_r answers");
        let back = to_epoch(&local).expect("mktime answers");
        assert_eq!(local_of(back), Some(local), "mktime names an instant of the same wall time");
        presets_land_ahead(now);
        assert_eq!(clock_word(now).map(|w| w.len()), Some(5));
    }

    /// Israel's 2026 rule as a POSIX string, so no tzdata is needed: summer
    /// time from 02:00 on Friday 27 March, back at 02:00 on Sunday 25 October.
    const ISRAEL_2026: &str = "IST-2IDT,M3.5.5,M10.5.0";

    /// The two pinned cases run in a child under `TZ`, so the zone is never
    /// a global the other tests in this process read.
    #[test]
    fn the_presets_hold_across_a_repeated_and_a_missing_hour() {
        let out = std::process::Command::new(mesimon_core::exe::current_exe().expect("exe"))
            .args(["--exact", "localtime::tests::zone_child", "--ignored", "--nocapture"])
            .env("LOCALTIME_ZONE_CHILD", "1")
            .env("TZ", ISRAEL_2026)
            .output()
            .expect("spawn the child");
        assert!(
            out.status.success(),
            "the child failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(said.contains("1 passed"), "the child ran its test:\n{said}");
    }

    #[test]
    #[ignore = "the child of the_presets_hold_across_a_repeated_and_a_missing_hour"]
    fn zone_child() {
        assert_eq!(std::env::var("LOCALTIME_ZONE_CHILD").as_deref(), Ok("1"), "run by the parent");
        let wall = |secs| local_of(secs).map(|t| (t.mon + 1, t.mday, t.hour, t.min));

        // The repeated hour: 01:30 on 25 October is 22:30Z in summer time
        // and 23:30Z an hour later in winter time.
        let (summer, winter) = (1_792_881_000, 1_792_884_600);
        assert_eq!(wall(summer), Some((10, 25, 1, 30)), "the zone took");
        assert_eq!(wall(winter), Some((10, 25, 1, 30)), "the wall time repeats");
        let back = to_epoch(&local_of(winter).expect("resolves")).expect("mktime answers");
        assert!([summer, winter].contains(&back), "mktime names one of the two, not {back}");
        let tomorrow9 = 1_792_998_000; // 26 October 09:00 winter time, 07:00Z
        for now in [summer, winter] {
            presets_land_ahead(now);
            let local = local_of(now).expect("resolves");
            let d = deadline(Preset::Tomorrow9, now, &local, Weekday::Monday, &to_epoch);
            assert_eq!(d, Some(tomorrow9), "tomorrow 9:00 is one instant from either 01:30");
        }

        // The missing hour: 02:00 winter time on 27 March is 03:00 summer
        // time, and no instant reads 02:30.
        let before = 1_774_567_800; // 01:30 winter time, 23:30Z on the 26th
        assert_eq!(wall(before), Some((3, 27, 1, 30)));
        assert_eq!(wall(before + 1800), Some((3, 27, 3, 0)), "02:00 is 03:00");
        let gap = LocalTime { hour: 2, min: 30, ..local_of(before).expect("resolves") };
        let named = to_epoch(&gap).expect("mktime answers a wall time that never was");
        assert!([before, before + 3600].contains(&named), "an hour either side, not {named}");
        presets_land_ahead(before);
        let d = deadline(
            Preset::Tomorrow9,
            before,
            &local_of(before).expect("resolves"),
            Weekday::Monday,
            &to_epoch,
        );
        assert_eq!(d, Some(1_774_677_600), "28 March 09:00 summer time, 06:00Z");
    }
}
