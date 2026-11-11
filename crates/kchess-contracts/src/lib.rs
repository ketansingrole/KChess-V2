//! The core's contract types as Rust (serde) shapes, shared by the core, the GPUI app and the CLI:
//! its methods (`core_api`, with their typed `calls`), its events (`events`), and the errors and
//! log levels that cross between them (`error`).

mod api;
pub mod core_api;
pub mod defaults;
pub mod error;
pub mod events;
pub mod library;
pub mod types;
pub mod variant;
pub mod voice;

pub use core_api::calls;
pub use error::{CoreError, ErrorKind, Level};
pub use events::CoreEvent;

/// How KChess names itself to the services it calls (Lichess refuses a request that names no
/// program with a 403 page; GitHub asks every API caller for one).
pub const USER_AGENT: &str = "KChess";
