//! The KChess headless core's services, migrating from `crates/kchess-node/js` (see
//! `RUST_MIGRATION.md`). Hosts drive it through `kchess-node`'s `NativeCore`.

pub mod archive;
pub mod capabilities;
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
