//! The core's contract types as Rust (serde) shapes, shared by the core, the GPUI app and the CLI,
//! and the errors and log levels that cross between them (`error`).

pub mod calls;
pub mod core_api;
pub mod defaults;
pub mod error;
pub mod library;
pub mod raw;
pub mod settings;
pub mod types;
pub mod variant;
pub mod voice;

pub use error::{CoreError, ErrorKind, Level};

/// How KChess names itself to the services it calls (Lichess refuses a request that names no
/// program with a 403 page; GitHub asks every API caller for one).
pub const USER_AGENT: &str = "KChess";
