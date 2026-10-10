//! What the core needs from its host while services are split between the languages:
//! somewhere to log, somewhere to send events, and the directories it owns.

use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// Host callbacks. They must not block: the core calls them from its worker threads.
pub trait Host: Send + Sync + 'static {
    /// One log line under a `[scope]`, as `logDebug/logInfo/logWarn/logError` write it.
    fn log(&self, level: Level, scope: &str, message: &str);
    /// An event for the host to deliver (`CoreEvents` names, or internal `host:*` requests).
    fn emit(&self, event: &str, payload: Value);
}

/// Where the core keeps its files.
#[derive(Clone, Debug)]
pub struct Config {
    /// KChess's data directory; it exists.
    pub data_dir: PathBuf,
    /// The main database of an earlier release, imported once into an empty profile
    /// (`CorePlatform.legacyDatabasePath`).
    pub legacy_database_path: Option<PathBuf>,
}
