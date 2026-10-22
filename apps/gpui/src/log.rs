//! The app's log: core and app lines, `[scope] message`, to stderr. Debug lines appear only with
//! `KCHESS_DEBUG=1`.

use kchess_core::Level;

pub fn write(level: Level, scope: &str, message: &str) {
    if level == Level::Debug && std::env::var("KCHESS_DEBUG").as_deref() != Ok("1") {
        return;
    }
    eprintln!("{} [{scope}] {message}", level.name());
}
