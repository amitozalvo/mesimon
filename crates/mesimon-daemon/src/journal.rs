//! The daemon's journal: `daemon.log` in the state dir — what the PROCESS did,
//! where the activity feed is what the board did. Three kinds of line and no
//! more: `started` (pid, build, exe stamp, repo, how it was started),
//! `stopping: <why>` (a `Shutdown` and which client asked, or `SIGTERM`) then
//! `stopped`, and `slow turn` — a writer-thread turn that outlasted
//! [`SLOW_TURN`], named by the message it handled and, for a tick, the
//! slowest stage inside it. Built after 2026-09-05, when a daemon left for
//! four minutes with a clean exit and nothing said why: the feed's sequence
//! restarting was the only trace, and it names a start, never a reason.
//!
//! Lines are written at once (they are rare, and a crash a moment later must
//! still find them), UTC-stamped, and the file rotates by size to
//! `daemon.log.1`. The detached spawn also points the daemon's stdout and
//! stderr at this file, so a panic lands beside the journal's own lines —
//! though after a rotation a panic follows the OLD inode into `.1`; the
//! cap is generous enough that a rotation is a once-a-year event.
//!
//! A journal that cannot open writes nothing and fails nothing: the board
//! is more important than its diary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Rotate at 1 MiB — ten thousand lines of a file that gains a handful a day.
pub const ROTATE_BYTES: u64 = 1024 * 1024;

/// A writer turn this long earns a line. The TUI's request timeout is 10 s
/// and a snapshot round-trip is usually under a millisecond, so one second
/// is already a board that felt it.
pub const SLOW_TURN: Duration = Duration::from_secs(1);

pub struct Journal {
    path: PathBuf,
    file: Option<std::fs::File>,
    bytes: u64,
    rotate_at: u64,
    slow_at: Duration,
}

impl Journal {
    pub fn open(path: &Path) -> Self {
        Self::open_with(path, ROTATE_BYTES, SLOW_TURN)
    }

    pub fn open_with(path: &Path, rotate_at: u64, slow_at: Duration) -> Self {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
        let bytes = file.as_ref().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len());
        Self { path: path.to_path_buf(), file, bytes, rotate_at, slow_at }
    }

    /// One stamped line, written now. Newlines inside `text` are flattened:
    /// a line is the unit a reader greps for.
    pub fn line(&mut self, text: &str) {
        let Some(file) = self.file.as_mut() else { return };
        let flat: String =
            text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
        let out = format!("{} {}\n", iso_utc(now_ms()), flat.trim_end());
        if file.write_all(out.as_bytes()).is_err() {
            return;
        }
        self.bytes += out.len() as u64;
        if self.bytes >= self.rotate_at {
            let old = self.path.with_extension("log.1");
            let _ = std::fs::rename(&self.path, old);
            self.file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path).ok();
            self.bytes = 0;
        }
    }

    /// A writer turn that began at `started` just ended: a line if it ran
    /// past the threshold, naming `what` it handled and, when given, the
    /// slowest stage inside it. True when a line was written.
    pub fn slow_turn(
        &mut self,
        started: Instant,
        what: &str,
        slowest: Option<(&str, Duration)>,
    ) -> bool {
        let took = started.elapsed();
        if took < self.slow_at {
            return false;
        }
        let inner = match slowest {
            Some((stage, d)) if !d.is_zero() => {
                format!(" ∙ slowest stage {stage} {} ms", d.as_millis())
            }
            _ => String::new(),
        };
        self.line(&format!("slow turn: {what} took {} ms{inner}", took.as_millis()));
        true
    }

    /// The last `stopping:` line in the journal at `path`, for `doctor` — the
    /// one question a restarted board asks of the daemon that went before.
    pub fn last_stop(path: &Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        text.lines().rev().find(|l| l.contains(" stopping: ")).map(str::to_string)
    }
}

use mesimon_core::clock::now_ms;

/// `2026-09-05T14:29:21.123Z` from Unix milliseconds — UTC, so two machines'
/// journals compare, and no dependency for one line a day. Proleptic
/// Gregorian, days-to-civil after Hinnant.
pub fn iso_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let millis = ms % 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.{millis:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("msmn-journal-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("daemon.log")
    }

    #[test]
    fn iso_utc_reads_the_calendar_right() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        // 2026-09-05 14:29:21.123 UTC (the afternoon this module was written).
        assert_eq!(iso_utc(1_788_618_561_123), "2026-09-05T14:29:21.123Z");
        // A leap day, and the last second of a year.
        assert_eq!(iso_utc(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_utc(1_704_067_199_999), "2023-12-31T23:59:59.999Z");
    }

    /// A line lands at once, stamped, one per call, newlines flattened.
    #[test]
    fn a_line_is_written_now_and_stamped() {
        let path = tmp("line");
        let mut j = Journal::open(&path);
        j.line("started pid 1");
        j.line("stopping: shutdown asked by\nmesimon-tui/0.1");
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("Z started pid 1"), "{}", lines[0]);
        assert!(lines[1].contains("stopping: shutdown asked by mesimon-tui/0.1"), "{}", lines[1]);
        assert_eq!(Journal::last_stop(&path).as_deref(), Some(lines[1]));
    }

    /// Only a turn past the threshold is recorded, and a tick's line names
    /// the stage that took the time.
    #[test]
    fn only_a_slow_turn_earns_a_line() {
        let path = tmp("slow");
        let mut j = Journal::open_with(&path, ROTATE_BYTES, Duration::from_millis(20));
        assert!(!j.slow_turn(Instant::now(), "tick", None), "a quick turn says nothing");
        let long_ago = Instant::now() - Duration::from_millis(500);
        assert!(j.slow_turn(
            long_ago,
            "tick",
            Some(("probe_activity", Duration::from_millis(480)))
        ));
        assert!(j.slow_turn(long_ago, "request snapshot", None));
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.contains("slow turn: tick took "), "{text}");
        assert!(text.contains(" ms ∙ slowest stage probe_activity 480 ms"), "{text}");
        assert!(text.contains("slow turn: request snapshot took "), "{text}");
    }

    #[test]
    fn rotates_by_size_and_keeps_writing() {
        let path = tmp("rotate");
        let mut j = Journal::open_with(&path, 200, SLOW_TURN);
        for i in 0..8 {
            j.line(&format!("started pid {i} build 0.1.0-alpha.14 detached"));
        }
        assert!(path.with_extension("log.1").is_file(), "rotated file exists");
        j.line("stopping: SIGTERM");
        assert!(std::fs::read_to_string(&path).unwrap().contains("stopping: SIGTERM"));
    }

    /// No file, no failure: the board outranks its diary.
    #[test]
    fn an_unopenable_journal_is_silent() {
        let path = Path::new("/dev/null/not-a-dir/daemon.log");
        let mut j = Journal::open(path);
        j.line("started");
        j.slow_turn(Instant::now() - Duration::from_secs(5), "tick", None);
        assert!(Journal::last_stop(path).is_none());
    }
}
