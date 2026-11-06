//! The KChess headless core's services (see `RUST_MIGRATION.md`). Hosts drive it through
//! `Core`.

/// How KChess names itself to the services it calls (Lichess refuses a request that names no
/// program with a 403 page; GitHub asks every API caller for one).
pub const USER_AGENT: &str = "KChess";

/// Whether this is the Mac App Store's KChess. Only that build runs in the App Sandbox, which
/// sets `APP_SANDBOX_CONTAINER_ID` for the app and every program it starts. The App Store updates
/// it and forbids downloading programs, so its updater and the Stockfish download stay off.
pub fn app_store() -> bool {
    cfg!(target_os = "macos") && std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

/// Whether this build honours the development overrides: the test switch (`KCHESS_STORE_DEBUG`)
/// and what it unlocks (test Lichess, Chess.com, OpenAI and voice origins), and the frontends'
/// `KCHESS_ENGINE_PATH`, `KCHESS_VARIANT_ENGINE_PATH`, `KCHESS_VOICE_HELPER`,
/// `KCHESS_SERVICE_BINARY` and `KCHESS_UPDATE_FEED`. A packaged build (`cargo xtask package`
/// builds with the `packaged` feature) never does, so software that can set the user's
/// environment cannot make a signed KChess start its programs or send its logins elsewhere.
pub const fn dev_overrides() -> bool {
    !cfg!(feature = "packaged")
}

/// The development override `name` (see `dev_overrides`), when this build honours it and it is
/// set to something.
pub fn dev_override(name: &str) -> Option<std::ffi::OsString> {
    if !dev_overrides() {
        return None;
    }
    std::env::var_os(name).filter(|value| !value.is_empty())
}

pub mod ai;
pub mod archive;
pub mod capabilities;
pub mod chesscom;
pub mod connection;
pub mod core;
pub mod engine;
pub mod error;
pub mod facade;
pub mod host;
pub mod lichess;
pub mod loopback;
mod panics;
mod process;
pub mod puzzles;
pub mod redact;
pub mod secrets;
pub mod store;
mod tasks;
pub mod themes;
pub mod usage;
pub mod voice;

pub use crate::core::Core;
pub use error::{CoreError, ErrorKind, Result};
pub use host::{Config, Host, Level};
