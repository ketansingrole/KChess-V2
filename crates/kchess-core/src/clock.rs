//! The core's wall clock. Durations and deadlines use `std::time::Instant` instead, which never
//! goes back.

use std::time::{SystemTime, UNIX_EPOCH};

/// `Date.now()`: whole milliseconds since the Unix epoch, 0 for a clock set before it.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}
