//! The core's test switch.

/// Whether the test-only behaviour is on: `KCHESS_STORE_DEBUG=1` in the process (tests set it).
///
/// Intentionally runtime-gated, not `cfg(test/debug_assertions)`: tests and end-to-end runs
/// drive a *release build*, where `debug_assertions` is off.
/// `core.rs` refuses the test-only Lichess/voice overrides unless it is set, and a packaged build
/// ignores it altogether (`crate::dev_overrides`).
pub fn enabled() -> bool {
    crate::dev_overrides() && crate::env::var("KCHESS_STORE_DEBUG").as_deref() == Some("1")
}
