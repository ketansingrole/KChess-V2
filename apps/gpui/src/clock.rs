//! The app's two clocks, as JavaScript-style millisecond numbers: the wall clock for times that
//! are saved or shown, and a monotonic one for measuring how long something took.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// `Date.now()`: milliseconds since the Unix epoch, 0 for a clock set before it.
pub fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0., |since| since.as_millis() as f64)
}

/// `performance.now()`: milliseconds on a clock that only goes forward, from its first use.
pub fn monotonic_ms() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.
}
