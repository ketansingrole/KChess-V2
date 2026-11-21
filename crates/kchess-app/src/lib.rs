//! KChess's app services over GPUI: what the pages read and write besides their views. The core
//! as the app holds it (`core`), the Lichess and Chess.com accounts (`accounts`), Settings (`settings`), view preferences (`prefs`), the games
//! played (`played`), reviews (`reviews`), timed runs (`runs`), the study library (`studies`), whether the computer is
//! online (`network`), the log (`log`), what a crash leaves in it (`crashes`), saving everything before quitting (`quit`), loading states
//! (`loading`), timers (`timer`), the writers that save to the core in order (`writer`) and the watch on calls that block the UI thread (`slow_calls`).
//! It never depends on the app, and never names the core's own crate: the app opens the core and
//! hands it over.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub mod accounts;
pub mod core;
pub mod crashes;
pub mod loading;
pub mod log;
pub mod network;
pub mod played;
pub mod prefs;
pub mod quit;
pub mod reviews;
pub mod runs;
pub mod settings;
pub mod slow_calls;
pub mod studies;
pub mod timer;
pub mod writer;

/// A real core for the tests of the crates the app is built from (`test-support`).
#[cfg(any(test, feature = "test-support"))]
// Test code, which other crates' tests build too.
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod tests;
