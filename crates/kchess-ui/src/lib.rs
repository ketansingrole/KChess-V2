//! KChess's design system over GPUI Kit: what every page paints with and nothing app-specific.
//! The theme and its colors (`theme`), the system's reduced motion (`motion`) and the line chart
//! (`chart`). It never depends on the app: what it needs from it (where log lines go, which theme
//! Settings chose) the app passes in.

pub mod chart;
pub mod motion;
pub mod theme;

use gpui_kit::{App, Global};
use kchess_contracts::Level;

/// Where this crate's log lines go: the app's log (`cx.set_global(Logger(log::write))`). Until
/// one is set they go to stderr.
#[derive(Clone, Copy)]
pub struct Logger(pub fn(Level, &str, &str));

impl Global for Logger {}

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
