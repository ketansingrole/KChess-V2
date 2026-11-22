//! What keeps KChess running without its window: the icon in the menu bar (macOS) or the system
//! tray (Windows, Linux) with its menu (`tray`), and the entry that opens KChess when the person
//! logs in (`autostart`).
//!
//! Both are the app's, not the core's (`docs/shared-core.md`, "What stays in the apps"): the
//! icon belongs to the app's process, and the login entry starts the app's own executable, which
//! only the app knows, from a folder of the person's own session rather than the profile.
//!
//! The crate leaves GPUI out, so its platform code checks for every target from any machine.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub mod autostart;
pub mod tray;
