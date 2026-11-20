//! The shared KChess core (`docs/shared-core.md`): one core per profile, which every KChess
//! window and the command line use over a private local socket.
//!
//! - `server`: the service (`kchess-service serve`);
//! - `embedded`: the core in a frontend's own process (`embed`), for the programs that host it.
//!
//! What a frontend holds of its core (the connection, the protocol, the handle, the profile's
//! layout) is `kchess-client`, re-exported here for the service and the programs that host the
//! core; the crates a frontend is built from name `kchess-client` alone.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub mod embedded;
pub mod server;

pub use embedded::embed;
pub use kchess_client::*;
