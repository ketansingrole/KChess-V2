//! The `kchess-voice` helper's pipe protocol, shared by the helper, the core that runs it and the
//! core's scripted stand-in for its tests.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub mod protocol;

/// The libvosk file the helper loads from beside its executable.
#[cfg(target_os = "macos")]
pub const LIBRARY: &str = "libvosk.dylib";
#[cfg(target_os = "windows")]
pub const LIBRARY: &str = "libvosk.dll";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const LIBRARY: &str = "libvosk.so";
