//! The KChess headless core's services, migrating from `crates/kchess-node/js` (see
//! `RUST_MIGRATION.md`). Hosts drive it through `kchess-node`'s `NativeCore`.

/// How KChess names itself to the services it calls (Lichess refuses a request that names no
/// program with a 403 page; GitHub asks every API caller for one).
pub const USER_AGENT: &str = "KChess";

pub mod archive;
pub mod capabilities;
pub mod connection;
pub mod core;
pub mod engine;
pub mod error;
pub mod facade;
pub mod host;
pub mod lichess;
mod panics;
pub mod puzzles;
pub mod secrets;
pub mod store;
mod tasks;
pub mod themes;
pub mod usage;
pub mod voice;

pub use crate::core::Core;
pub use error::{CoreError, Result};
pub use host::{Config, Host, Level};
