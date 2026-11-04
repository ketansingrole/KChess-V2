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

pub mod ai;
pub mod archive;
pub mod capabilities;
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
pub mod secrets;
pub mod store;
mod tasks;
pub mod themes;
pub mod usage;
pub mod voice;

pub use crate::core::Core;
pub use error::{CoreError, ErrorKind, Result};
pub use host::{Config, Host, Level};
