//! Cell-budget text handling (07 §4.1): truncation is by grapheme cluster to a
//! measured display-cell budget, marked with ASCII `~` — one cell, every font,
//! every locale. A string that fits exactly is returned untouched.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Truncate `s` to at most `max` display cells. When truncation happens the
/// last cell is the `~` marker. Never splits a grapheme cluster; a wide
/// cluster that would straddle the budget is dropped entirely.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let budget = max - 1; // reserve the marker cell
    let mut width = 0usize;
    let mut out = String::new();
    for g in s.graphemes(true) {
        let w = g.width();
        if width + w > budget {
            break;
        }
        width += w;
        out.push_str(g);
    }
    out.push('~');
    out
}

/// The 3-cell age slot (06 §5.4): fixed vocabulary, right-aligned by the
/// caller. `now` under 10 s, then s/m/h/d/w buckets, `>1y` past a year.
pub(crate) fn age_slot(now_ms: u64, then_ms: u64) -> String {
    let secs = now_ms.saturating_sub(then_ms) / 1000;
    match secs {
        0..=9 => "now".into(),
        10..=99 => format!("{secs}s"),
        100..=5_999 => format!("{}m", (secs / 60).max(1)),
        6_000..=86_399 => format!("{}h", secs / 3600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=31_535_999 => format!("{}w", secs / 604_800),
        _ => ">1y".into(),
    }
}

/// A marquee window into `s`: skip `offset` display cells (whole grapheme
/// clusters), then hard-clip to `max` cells with no truncation marker — the
/// motion itself says "there is more". `offset == 0` with an overflowing
/// string is the caller's cue to use `truncate` instead.
pub(crate) fn marquee_window(s: &str, max: usize, offset: usize) -> String {
    let mut skipped = 0usize;
    let mut width = 0usize;
    let mut out = String::new();
    for g in s.graphemes(true) {
        let w = g.width();
        if skipped < offset {
            skipped += w;
            continue;
        }
        if width + w > max {
            break;
        }
        width += w;
        out.push_str(g);
    }
    out
}

/// How far a marquee has scrolled at `elapsed_ms`, for a title `overflow`
/// cells too wide: hold at the start, walk one cell per step to the end,
/// hold, loop.
pub(crate) fn marquee_offset(elapsed_ms: u64, overflow: usize) -> usize {
    const STEP_MS: u64 = 200;
    const HOLD_STEPS: u64 = 6;
    if overflow == 0 {
        return 0;
    }
    let cycle = HOLD_STEPS + overflow as u64 + HOLD_STEPS;
    let step = (elapsed_ms / STEP_MS) % cycle;
    // Steps 0..HOLD hold at 0; each step after walks one cell.
    (step.saturating_sub(HOLD_STEPS - 1) as usize).min(overflow)
}

/// `created_at` → epoch ms. The daemon writes `@<epoch-secs>` (server.rs
/// `now_iso`); RFC3339 (`2026-08-29T17:04:41Z`, offset forms) is accepted
/// for compatibility. None on anything unparsable — no age renders then.
pub(crate) fn created_at_epoch_ms(s: &str) -> Option<u64> {
    if let Some(rest) = s.strip_prefix('@') {
        return rest.parse::<u64>().ok().map(|secs| secs * 1000);
    }
    let bytes = s.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Timezone tail: 'Z', or ±HH:MM after an optional fractional part.
    let tail = &s[19..];
    let tz_pos = tail.find(['Z', '+', '-']);
    let tz_secs = match tz_pos.map(|p| &tail[p..]) {
        Some("Z") | None => 0i64,
        Some(t) if t.len() >= 6 => {
            let sign = if t.starts_with('-') { -1 } else { 1 };
            let th = t.get(1..3)?.parse::<i64>().ok()?;
            let tm = t.get(4..6)?.parse::<i64>().ok()?;
            sign * (th * 3600 + tm * 60)
        }
        _ => return None,
    };
    // Howard Hinnant's days_from_civil.
    let (y, mo) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (mo - 3) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - tz_secs;
    u64::try_from(secs).ok().map(|s| s * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_fit_is_untouched() {
        // The old char-based truncate could never return a string of width
        // == max; this is the off-by-one regression test.
        assert_eq!(truncate("abcde", 5), "abcde");
        assert_eq!(truncate("abcde", 6), "abcde");
    }

    #[test]
    fn overflow_gets_ascii_marker() {
        assert_eq!(truncate("abcdef", 5), "abcd~");
        assert_eq!(truncate("abcdef", 1), "~");
        assert_eq!(truncate("abcdef", 0), "");
    }

    #[test]
    fn wide_cluster_never_straddles() {
        // "你" is 2 cells; budget 4 → marker at cell 4, cluster dropped whole.
        assert_eq!(truncate("你好吗", 4), "你~");
        assert_eq!(truncate("你好吗", 6), "你好吗");
    }

    #[test]
    fn combining_cluster_stays_whole() {
        // e + combining acute is one cluster, one cell.
        let s = "cafe\u{301} bar";
        assert_eq!(truncate(s, 5), "cafe\u{301}~");
    }

    #[test]
    fn hebrew_stays_within_budget() {
        use unicode_width::UnicodeWidthStr;
        let s = "תיקון הפניית אימות";
        let t = truncate(s, 10);
        assert!(t.width() <= 10);
        assert!(t.ends_with('~'));
    }

    #[test]
    fn marquee_walks_and_holds() {
        // 6-step hold, then one cell per 200 ms, capped at overflow.
        assert_eq!(marquee_offset(0, 5), 0);
        assert_eq!(marquee_offset(1199, 5), 0); // still holding
        assert_eq!(marquee_offset(1200, 5), 1);
        assert_eq!(marquee_offset(2000, 5), 5);
        assert_eq!(marquee_offset(2199, 5), 5); // end hold
        assert_eq!(marquee_offset(0, 0), 0);
        assert_eq!(marquee_window("abcdefgh", 4, 2), "cdef");
        assert_eq!(marquee_window("你好吗x", 4, 2), "好吗");
    }

    #[test]
    fn created_at_parses() {
        // The daemon's own form.
        assert_eq!(created_at_epoch_ms("@0"), Some(0));
        assert_eq!(created_at_epoch_ms("@86400"), Some(86_400_000));
        // RFC3339 compatibility.
        assert_eq!(created_at_epoch_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(created_at_epoch_ms("1970-01-02T00:00:00Z"), Some(86_400_000));
        assert_eq!(
            created_at_epoch_ms("2026-08-29T00:00:00Z"),
            Some(1_787_961_600_000)
        );
        assert_eq!(created_at_epoch_ms("1970-01-01T02:00:00+02:00"), Some(0));
        assert_eq!(created_at_epoch_ms("1970-01-01T00:00:00.123Z"), Some(0));
        assert_eq!(created_at_epoch_ms("garbage"), None);
        assert_eq!(created_at_epoch_ms("@garbage"), None);
    }

    #[test]
    fn age_vocabulary() {
        let s = 1000u64;
        assert_eq!(age_slot(9 * s, 0), "now");
        assert_eq!(age_slot(45 * s, 0), "45s");
        assert_eq!(age_slot(180 * s, 0), "3m");
        assert_eq!(age_slot(4 * 3600 * s, 0), "4h");
        assert_eq!(age_slot(2 * 86_400 * s, 0), "2d");
        assert_eq!(age_slot(21 * 86_400 * s, 0), "3w");
        assert_eq!(age_slot(400 * 86_400 * s, 0), ">1y");
        // Every word fits the 3-cell slot.
        for t in [0, 9, 45, 180, 14_400, 172_800, 1_814_400, 40_000_000] {
            assert!(age_slot(t * s, 0).len() <= 3, "{}", age_slot(t * s, 0));
        }
    }
}
