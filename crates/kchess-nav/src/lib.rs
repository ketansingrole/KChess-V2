//! The app's pages as the shell drives them, over GPUI: what a page is and declares (`page`),
//! how it reaches the shell to open another (`shell`), back and forward by swipe (`swipe`), and
//! the board pages' shared layout (`play_layout`, `part`). It never depends on the app or on any
//! page: the app registers its pages (`page::set_registry`) and its workspace answers `Shell`.

pub mod page;
pub mod part;
pub mod play_layout;
pub mod shell;
pub mod swipe;

// The app's names for the crates these modules use.
use kchess_board::widgets;
