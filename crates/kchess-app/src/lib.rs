//! KChess's app services over GPUI: what the pages read and write besides their views. The core
//! as the app holds it (`core`), Settings (`settings`) and view preferences (`prefs`), the log
//! (`log`), saving everything before quitting (`quit`), loading states (`loading`), timers
//! (`timer`) and the watch on calls that block the UI thread (`slow_calls`). It never depends on
//! the app, and never names the core's own crate: the app opens the core and hands it over.

pub mod core;
pub mod loading;
pub mod log;
pub mod prefs;
pub mod quit;
pub mod settings;
pub mod slow_calls;
pub mod timer;
