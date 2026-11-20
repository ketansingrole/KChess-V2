//! KChess's design system over GPUI Kit: what every page paints with and nothing app-specific.
//! The theme and its colors (`theme`), the system's reduced motion (`motion`), the line chart
//! (`chart`), the generic widgets (`widgets`), dates as the app shows them (`dates`) and the keys
//! bound to an action (`keys`),
//! Quick Pick's search (`quick_pick`), the dialog that asks for a name (`name_dialog`) and every
//! widget drawn in a gallery for development (`gallery`). It never depends on the app: what it needs from it (where log
//! lines go, which theme Settings chose) the app passes in.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub mod chart;
pub mod dates;
pub mod gallery;
pub mod keys;
pub mod motion;
pub mod name_dialog;
pub mod quick_pick;
pub mod theme;
pub mod widgets;

use gpui_kit::component::Icon;
use gpui_kit::{App, Global};
use kchess_contracts::Level;

/// Where this crate's log lines go: the app's log (`cx.set_global(Logger(log::write))`). Until
/// one is set they go to stderr.
#[derive(Clone, Copy)]
pub struct Logger(pub fn(Level, &str, &str));

impl Global for Logger {}

/// An icon by its name (`circle-dot`), drawn from the app's `icons/` assets. `cargo xtask icons`
/// finds every `icon("…")` in the app's sources and this crate's, and every string literal that
/// is a name in its `ICONS`, so a name kept in a table is written there as a literal.
pub fn icon(name: &'static str) -> Icon {
    Icon::empty().path(format!("icons/{name}.svg"))
}

/// Write `message` under `scope` to the app's log.
pub(crate) fn log(cx: &App, level: Level, scope: &str, message: &str) {
    match cx.try_global::<Logger>() {
        Some(Logger(write)) => write(level, scope, message),
        None => {
            // Not `eprintln!`: it panics when stderr is gone.
            use std::io::Write as _;
            let _ = writeln!(std::io::stderr(), "{} [{scope}] {message}", level.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn remember(level: Level, scope: &str, message: &str) {
        LINES
            .lock()
            .unwrap()
            .push(format!("{} [{scope}] {message}", level.name()));
    }

    #[gpui_kit::test]
    fn lines_go_to_the_logger_the_app_set(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            // Before the app sets one, a line goes to stderr and nowhere else.
            log(cx, Level::Warn, "test", "unseen");
            cx.set_global(Logger(remember));
            log(cx, Level::Warn, "test", "seen");
        });
        assert_eq!(*LINES.lock().unwrap(), ["warn [test] seen"]);
    }
}
