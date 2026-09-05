//! The wall clock as a number, once. Every stamp mesimon writes or compares
//! is Unix milliseconds (or seconds) as `u64`, and a clock before the epoch
//! reads as zero — the shape eleven private copies had each spelled.

use std::time::{SystemTime, UNIX_EPOCH};

/// Now, in Unix milliseconds.
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Now, in Unix seconds.
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A file time (an mtime, usually) in Unix milliseconds; `None` before the
/// epoch.
pub fn epoch_ms(t: SystemTime) -> Option<u64> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}
