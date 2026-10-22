//! Where the app keeps its data: the Electron app's profile, so either app opens the other's
//! games, settings and accounts. `KCHESS_USER_DATA_DIR` picks another profile, as it does for
//! Electron (isolated tests, development with fresh data).

use std::path::PathBuf;

/// Matches `build.productName` in `apps/desktop/package.json`, which names Electron's profile.
pub const APP_NAME: &str = "KChess";

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Whether `KCHESS_USER_DATA_DIR` chose the profile.
pub fn isolated() -> bool {
    std::env::var_os("KCHESS_USER_DATA_DIR").is_some_and(|dir| !dir.is_empty())
}

/// Electron's `app.getPath('userData')` for this app, created if it is missing.
pub fn data_dir() -> std::io::Result<PathBuf> {
    let dir = match std::env::var_os("KCHESS_USER_DATA_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => default_dir(),
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(target_os = "macos")]
fn default_dir() -> PathBuf {
    home().join("Library/Application Support").join(APP_NAME)
}

#[cfg(target_os = "windows")]
fn default_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("AppData/Roaming"))
        .join(APP_NAME)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn default_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
        .join(APP_NAME)
}

/// Where earlier releases installed the downloaded engine (`CorePlatform.managedEngineDir`).
pub fn managed_engine_dir() -> PathBuf {
    home().join(".kchess/engines/stockfish/current")
}

/// The database earlier macOS releases kept outside the profile; an isolated profile never imports it.
pub fn legacy_database(_data_dir: &std::path::Path) -> Option<PathBuf> {
    (cfg!(target_os = "macos") && !isolated()).then(|| home().join(".kchess/kchess.db"))
}
