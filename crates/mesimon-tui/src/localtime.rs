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

fn epoch_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The moment `secs` in local time, or `None` when libc cannot say.
fn local_of(secs: u64) -> Option<LocalTime> {
    let t = libc::time_t::try_from(secs).ok()?;
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

    /// The two calls invert each other, and the presets resolve to a moment
    /// ahead of now on the real clock — whatever zone the test runs in.
    #[test]
    fn the_clock_round_trips_and_the_presets_land_ahead() {
        let now = epoch_now();
        let local = now_local().expect("localtime_r answers");
        let back = to_epoch(&local).expect("mktime answers");
        assert_eq!(back, now, "localtime_r then mktime is the identity");
        for p in Preset::ALL {
            for w in Weekday::ALL {
                let d = deadline(p, now, &local, w, &to_epoch).expect("resolves");
                assert!(d > now, "{p:?} on {w:?} is ahead");
                if p.is_calendar() {
                    let at = local_of(d).expect("resolves");
                    assert_eq!((at.hour, at.min, at.sec), (9, 0, 0), "{p:?} is at nine");
                    if p == Preset::NextWeek9 {
                        assert_eq!(at.wday, w.wday(), "{p:?} lands on the week's first day");
                    }
                }
            }
        }
        assert_eq!(clock_word(now).map(|w| w.len()), Some(5));
    }
}
