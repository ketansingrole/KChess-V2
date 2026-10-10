//! What the core needs from its host while services are split between the languages:
//! somewhere to log, somewhere to send events, and the directories it owns.

use serde_json::Value;
use std::collections::BTreeMap;
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

/// Where the core keeps its files, and where the host keeps the engines.
#[derive(Clone, Debug)]
pub struct Config {
    /// KChess's data directory; it exists.
    pub data_dir: PathBuf,
    /// The main database of an earlier release, imported once into an empty profile
    /// (`CorePlatform.legacyDatabasePath`).
    pub legacy_database_path: Option<PathBuf>,
    /// Absolute path of the bundled Stockfish UCI script (`CorePlatform.bundledEnginePath`).
    /// `None` when the host bundles none; the bundled engine is then not ready.
    pub bundled_engine_path: Option<PathBuf>,
    /// The Node executable that runs the bundled script (`process.execPath`).
    pub node_path: Option<PathBuf>,
    /// Extra environment for running the bundled script (`CorePlatform.nodeEnv`).
    pub node_env: BTreeMap<String, String>,
    /// Where the downloaded Stockfish is kept (`CorePlatform.managedEngineDir`); the default is
    /// `engines/stockfish/current` in the data directory.
    pub managed_engine_dir: Option<PathBuf>,
}

impl Config {
    /// A configuration with only the data directory: no bundled engine and no legacy database.
    pub fn new(data_dir: PathBuf) -> Config {
        Config {
            data_dir,
            legacy_database_path: None,
            bundled_engine_path: None,
            node_path: None,
            node_env: BTreeMap::new(),
            managed_engine_dir: None,
        }
    }
}
