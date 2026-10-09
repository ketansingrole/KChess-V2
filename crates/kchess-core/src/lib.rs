//! The KChess headless core's services, migrating from `core/src/services` (see
//! `RUST_MIGRATION.md`). Hosts drive it through `kchess-node`'s `NativeCore`.

pub mod core;
pub mod error;
pub mod host;
pub mod puzzles;
pub mod store;

pub use crate::core::Core;
pub use error::{CoreError, Result};
pub use host::{Config, Host, Level};
