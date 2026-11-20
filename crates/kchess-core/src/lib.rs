//! The KChess headless core's services. Hosts drive it through `Core`.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub use kchess_contracts::USER_AGENT;

pub mod ai;
pub mod archive;
pub mod capabilities;
pub mod chesscom;
pub mod connection;
pub mod core;
pub mod download;
pub mod engine;
pub mod facade;
pub mod host;
pub mod ids;
pub mod lichess;
pub mod loopback;
pub mod memory;
mod panics;
pub mod power;
mod process;
pub mod profile_lock;
pub mod puzzles;
pub mod secrets;
pub mod store;
pub mod tasks;
#[doc(hidden)]
pub mod test_host;
pub mod themes;
pub mod time;
pub mod usage;
pub mod voice;

pub use crate::core::Core;
pub use host::{Config, Host, Level, TestSwitches, TestVoiceModel};
