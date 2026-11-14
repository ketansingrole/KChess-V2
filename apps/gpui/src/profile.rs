//! Where the app keeps its data: the profile every KChess release has used, with its games,
//! settings and accounts. `KCHESS_USER_DATA_DIR` picks another profile (isolated tests,
//! development with fresh data); `kchess_service::Layout` resolves it, as it does for the CLI.

use std::path::PathBuf;

/// The product name, which names the profile folder.
pub const APP_NAME: &str = kchess_service::layout::APP_NAME;

#[cfg(test)]
thread_local! {
    /// The test's own profile (`e2e::Harness`): tests run in parallel on their own threads, so the
    /// process-wide `KCHESS_USER_DATA_DIR` cannot give each its own.
    pub static TEST_DATA_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The profile folder, created if it is missing.
pub fn data_dir() -> std::io::Result<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = TEST_DATA_DIR.with_borrow(Clone::clone) {
        std::fs::create_dir_all(&dir)?;
        return Ok(dir);
    }
    let dir = kchess_service::Layout::current().profile();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    /// A test's profile (as any isolated one) has its own downloaded engine, never the one
    /// every other app on the account shares.
    #[test]
    fn an_isolated_profile_keeps_its_own_engine() {
        let folder = tempfile::tempdir().expect("a profile");
        let config = kchess_service::Layout::current().config(folder.path(), true);
        let engine = config.managed_engine_dir.expect("a managed engine folder");
        assert!(engine.starts_with(folder.path()), "{}", engine.display());
    }

    /// macOS names a development build after its executable: it read "kchess-gpui" in the menu
    /// bar and Dock until the executable was named as the app.
    #[test]
    fn the_executable_is_named_as_the_app() {
        assert_eq!(env!("CARGO_BIN_NAME"), super::APP_NAME);
    }
}
