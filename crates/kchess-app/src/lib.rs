//! KChess's app services over GPUI: what the pages read and write besides their views. The core
//! as the app holds it (`core`), the Lichess and Chess.com accounts (`accounts`), Settings (`settings`), view preferences (`prefs`), the games
//! played (`played`), reviews (`reviews`), the study library (`studies`), whether the computer is
//! online (`network`), the log (`log`), saving everything before quitting (`quit`), loading states
//! (`loading`), timers (`timer`) and the watch on calls that block the UI thread (`slow_calls`).
//! It never depends on the app, and never names the core's own crate: the app opens the core and
//! hands it over.
pub mod accounts;
pub mod core;
pub mod loading;
pub mod log;
pub mod network;
pub mod played;
pub mod prefs;
pub mod quit;
pub mod reviews;
pub mod settings;
pub mod slow_calls;
pub mod studies;
pub mod timer;
