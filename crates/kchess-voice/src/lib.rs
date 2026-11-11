//! The `kchess-voice` helper's pipe protocol, shared by the helper, the core that runs it and the
//! core's scripted stand-in for its tests.

pub mod protocol;

/// The libvosk file the helper loads from beside its executable.
#[cfg(target_os = "macos")]
pub const LIBRARY: &str = "libvosk.dylib";
#[cfg(target_os = "windows")]
pub const LIBRARY: &str = "libvosk.dll";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const LIBRARY: &str = "libvosk.so";
