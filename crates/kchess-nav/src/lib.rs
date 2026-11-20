//! The app's pages as the shell drives them, over GPUI: what a page is and declares (`page`),
//! how it reaches the shell to open another (`shell`) or Settings at a category (`sections`), back and forward by swipe (`swipe`), and
//! the board pages' shared layout (`play_layout`, `part`). It never depends on the app or on any
//! page: the app registers its pages (`page::set_registry`) and its workspace answers `Shell`.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub mod page;
pub mod part;
pub mod play_layout;
pub mod sections;
pub mod shell;
pub mod swipe;

// The app's names for the crates these modules use.
use kchess_board::widgets;

#[cfg(any(test, feature = "test-support"))]
// Test code, which other crates' tests build too.
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod testing;
#[cfg(test)]
mod tests;
