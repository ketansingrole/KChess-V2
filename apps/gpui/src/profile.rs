//! Where the app keeps its data: the Electron app's profile, so either app opens the other's
//! games, settings and accounts. `KCHESS_USER_DATA_DIR` picks another profile, as it does for
//! Electron (isolated tests, development with fresh data).

use std::path::PathBuf;

/// The Electron app's product name, which named the profile folder both apps use.
pub const APP_NAME: &str = "KChess";

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Whether `KCHESS_USER_DATA_DIR` chose the profile.
pub fn isolated() -> bool {
    #[cfg(test)]
    if TEST_DATA_DIR.with_borrow(Option::is_some) {
        return true;
    }
    std::env::var_os("KCHESS_USER_DATA_DIR").is_some_and(|dir| !dir.is_empty())
}

#[cfg(test)]
thread_local! {
    /// The test's own profile (`e2e::Harness`): tests run in parallel on their own threads, so the
    /// process-wide `KCHESS_USER_DATA_DIR` cannot give each its own.
    pub static TEST_DATA_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Electron's `app.getPath('userData')` for this app, created if it is missing.
pub fn data_dir() -> std::io::Result<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = TEST_DATA_DIR.with_borrow(Clone::clone) {
        std::fs::create_dir_all(&dir)?;
        return Ok(dir);
    }
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

/// Where earlier releases installed the downloaded engine (`CorePlatform.managedEngineDir`),
/// shared by every app on this account. An isolated profile (a scratch one, a test's) keeps its
/// own, so installing or deleting it there never touches the person's engine.
pub fn managed_engine_dir() -> PathBuf {
    if isolated()
        && let Ok(dir) = data_dir()
    {
        return dir.join("engines/stockfish/current");
    }
    home().join(".kchess/engines/stockfish/current")
}

/// The database earlier macOS releases kept outside the profile; an isolated profile never imports it.
pub fn legacy_database(_data_dir: &std::path::Path) -> Option<PathBuf> {
    (cfg!(target_os = "macos") && !isolated()).then(|| home().join(".kchess/kchess.db"))
}

/// The `kchess-voice` speech helper, libvosk beside it: `KCHESS_VOICE_HELPER`, else the package's
/// `voice` folder beside the app (in `Contents/Resources` in the macOS bundle), else (a build in
/// this repository) the helper `cargo xtask voice-runtime` stages in `target/voice-runtime`.
pub fn voice_helper() -> Option<PathBuf> {
    if let Some(path) = kchess_core::dev_override("KCHESS_VOICE_HELPER") {
        return Some(PathBuf::from(path));
    }
    let name = if cfg!(windows) {
        "kchess-voice.exe"
    } else {
        "kchess-voice"
    };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    // Beside the executable; in a Mac bundle, in Contents/Resources.
    for packaged in [
        dir.join("voice").join(name),
        dir.join("../Resources/voice").join(name),
    ] {
        if packaged.exists() {
            return Some(packaged);
        }
    }
    let staged = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/voice-runtime")
        .join(name);
    staged.exists().then_some(staged)
}

/// The Stockfish this app ships: a native executable beside the app's own (`Contents/MacOS` in the
/// macOS bundle), started without a runtime. `KCHESS_ENGINE_PATH` names another one (development
/// builds, which ship none).
pub fn bundled_engine() -> Option<PathBuf> {
    if let Some(path) = kchess_core::dev_override("KCHESS_ENGINE_PATH") {
        return Some(PathBuf::from(path));
    }
    let name = if cfg!(windows) {
        "stockfish.exe"
    } else {
        "stockfish"
    };
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(name))
}

/// The Fairy-Stockfish this app ships for variants: `KCHESS_VARIANT_ENGINE_PATH`, else beside
/// the app's executable (`Contents/MacOS` in the macOS bundle), else (a build in this repository)
/// the one `cargo xtask fairy-stockfish` builds in `target/fairy-stockfish/bin`.
pub fn variant_engine() -> Option<PathBuf> {
    if let Some(path) = kchess_core::dev_override("KCHESS_VARIANT_ENGINE_PATH") {
        return Some(PathBuf::from(path));
    }
    let name = if cfg!(windows) {
        "fairy-stockfish.exe"
    } else {
        "fairy-stockfish"
    };
    let packaged = std::env::current_exe().ok()?.parent()?.join(name);
    if packaged.exists() {
        return Some(packaged);
    }
    let built = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fairy-stockfish/bin")
        .join(name);
    built.exists().then_some(built)
}

#[cfg(test)]
mod tests {
    /// macOS names a development build after its executable: it read "kchess-gpui" in the menu
    /// bar and Dock until the executable was named as the app.
    /// A test's profile (as any isolated one) has its own downloaded engine, never the one
    /// every other app on the account shares.
    #[test]
    fn an_isolated_profile_keeps_its_own_engine() {
        let folder = tempfile::tempdir().expect("a profile");
        super::TEST_DATA_DIR.with_borrow_mut(|dir| *dir = Some(folder.path().to_path_buf()));
        let engine = super::managed_engine_dir();
        super::TEST_DATA_DIR.with_borrow_mut(|dir| *dir = None);
        assert!(engine.starts_with(folder.path()), "{}", engine.display());
    }

    #[test]
    fn the_executable_is_named_as_the_app() {
        assert_eq!(env!("CARGO_BIN_NAME"), super::APP_NAME);
    }
}
