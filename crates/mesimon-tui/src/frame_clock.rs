//! During composition, a time-dependent cell records when it next changes.
//! Card layout measures off-screen cards too, so their clocks are collected
//! separately and only joined to the frame for cards inside the viewport.
use std::cell::Cell;
use std::time::Duration;

thread_local! {
    static NEXT: Cell<Option<Duration>> = const { Cell::new(None) };
}

pub(crate) fn collect<T>(draw: impl FnOnce() -> T) -> (T, Option<Duration>) {
    let outer = NEXT.take();
    let value = draw();
    (value, NEXT.replace(outer))
}

pub(crate) fn after(delay: Duration) {
    NEXT.set(Some(NEXT.get().map_or(delay, |old| old.min(delay))));
}

pub(crate) fn after_ms(ms: u64) {
    after(Duration::from_millis(ms));
}

/// A time-limited treatment needs one more frame to remove it.
pub(crate) fn before(now: u64, end: u64) -> bool {
    if now >= end {
        return false;
    }
    after_ms(end - now);
    true
}

pub(crate) fn during(start: std::time::Instant, duration: Duration) -> bool {
    if let Some(left) = duration.checked_sub(start.elapsed()) {
        after(left);
        true
    } else {
        false
    }
}

pub(crate) fn phase(frame: usize, steps: usize) -> usize {
    after_ms((steps - frame % steps) as u64 * crate::glyphs::SPIN_STEP_MS);
    frame / steps
}
