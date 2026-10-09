//! The KChess headless core's services, migrating from `core/src/services` (see
//! `docs/rust-migration.md`). Hosts drive it through `kchess-node`'s `NativeCore`.

pub mod core;
pub mod error;
pub mod host;
pub mod puzzles;

pub use crate::core::Core;
pub use error::{CoreError, Result};
pub use host::{Config, Host, Level};
