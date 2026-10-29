//! Engine status, identity and trust: which engine would run, the command that starts it, a key that changes when the executable
//! does, the engine paths the frontends may persist, and whether the computer is playing.
//! Search orchestration (`bestMove`, `stopEngine`) belongs to the UCI controller port.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::managed::{ManagedEngine, Target, can_download, executable_name, managed_engine};
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

/// The identity of the bundled engine, the official Stockfish shipped beside the app. It never
/// changes within a build, and differs from the WebAssembly build's (`stockfish-19-lite`) that the
/// Electron app bundled, so analyses cached in a profile it used never mix the two engines.
pub const BUNDLED_ENGINE_IDENTITY: &str = "stockfish-native";

/// What the host supplies to find and start engines.
#[derive(Clone, Debug)]
pub struct EngineLocations {
    /// The managed engine directory (`managed::default_dir`).
    pub managed_dir: PathBuf,
    /// The platform whose official build may be downloaded.
    pub target: Target,
    /// The Stockfish executable shipped beside the app.
    pub bundled: PathBuf,
}

/// What `engine_status` reports (`EngineStatus` in the TypeScript contracts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub ready: bool,
    /// The executable in use (a downloaded or chosen one), or empty when the bundled engine is.
    pub path: PathBuf,
    pub bundled: bool,
    /// The Stockfish KChess downloaded, whether or not it is the one in use.
    pub managed: ManagedEngine,
    /// An official native download exists for this platform and architecture.
    pub can_download: bool,
}

/// The engine the computer would use: a configured executable that is executable, else the
/// bundled one. Not ready when neither is.
pub async fn engine_status(
    host: &dyn Host,
    locations: &EngineLocations,
    configured: &str,
) -> EngineStatus {
    let managed = managed_engine(host, &locations.managed_dir).await;
    let can_download = can_download(locations.target.platform, locations.target.arch);
    if !configured.is_empty() && is_executable_file(host, Path::new(configured)).await {
        return EngineStatus {
            ready: true,
            path: PathBuf::from(configured),
            bundled: false,
            managed,
            can_download,
        };
    }
    if is_executable_file(host, &locations.bundled).await {
        return EngineStatus {
            ready: true,
            path: PathBuf::new(),
            bundled: true,
            managed,
            can_download,
        };
    }
    host.log(
        Level::Debug,
        "engine",
        &format!(
            "Bundled engine check failed: {} is not executable",
            locations.bundled.display()
        ),
    );
    EngineStatus {
        ready: false,
        path: PathBuf::from(configured),
        bundled: false,
        managed,
        can_download,
    }
}

async fn is_executable_file(host: &dyn Host, path: &Path) -> bool {
    if !executable(path) {
        return false;
    }
    match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata.is_file(),
        Err(cause) => {
            host.log(
                Level::Debug,
                "engine",
                &format!("Engine executable check failed: {} {cause}", path.display()),
            );
            false
        }
    }
}

/// The command that starts the engine `status` names: the bundled executable or the configured
/// one. The command is not spawned; the caller chooses its stdio.
pub fn engine_command(locations: &EngineLocations, status: &EngineStatus) -> Command {
    let mut command = if status.bundled {
        Command::new(&locations.bundled)
    } else {
        Command::new(&status.path)
    };
    crate::process::no_window(&mut command);
    command
}

/// A key that changes when the selected executable does: the bundled identity, or the path with
/// its size and modification time.
pub async fn engine_identity(status: &EngineStatus) -> Result<String> {
    if status.bundled {
        return Ok(BUNDLED_ENGINE_IDENTITY.to_string());
    }
    let metadata = tokio::fs::metadata(&status.path)
        .await
        .map_err(|cause| CoreError::wrap("Could not read the engine executable", cause))?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0);
    Ok(format!(
        "{}:{}:{}",
        status.path.display(),
        metadata.len(),
        modified_ms
    ))
}

/// The downloaded engine's executable in `managed_dir`.
pub fn managed_path(managed_dir: &Path) -> PathBuf {
    managed_dir.join(executable_name())
}

/// Whether a search is running, or a move was computed within `within_ms` of `now` (milliseconds
/// since the epoch). `last_move_at` is 0 before any move.
pub fn computer_playing(active: bool, last_move_at: i64, within_ms: i64, now: i64) -> bool {
    active || now.saturating_sub(last_move_at) < within_ms
}

/// The engine paths the frontends may persist: the managed engine, or one the user picked in the
/// native file dialog this session (or that is already saved).
#[derive(Default)]
pub struct EngineTrust {
    paths: Mutex<HashSet<OsString>>,
}

impl EngineTrust {
    pub fn trust(&self, path: &Path) {
        self.with_paths(|paths| {
            paths.insert(path.as_os_str().to_os_string());
        });
    }

    /// True for the managed executable and for every path trusted since the last `clear`.
    pub fn is_trusted(&self, path: &Path, managed_dir: &Path) -> bool {
        if path.as_os_str() == managed_path(managed_dir).as_os_str() {
            return true;
        }
        self.with_paths(|paths| paths.contains(path.as_os_str()))
    }

    pub fn clear(&self) {
        self.with_paths(HashSet::clear);
    }

    fn with_paths<T>(&self, action: impl FnOnce(&mut HashSet<OsString>) -> T) -> T {
        let mut paths = match self.paths.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        action(&mut paths)
    }
}

/// Whether `path` may be executed (`access(2)` with `X_OK`). Windows has no such bit: there the
/// check is existence.
#[cfg(unix)]
fn executable(path: &Path) -> bool {
    rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok()
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    struct SilentHost;

    impl Host for SilentHost {
        fn log(&self, _level: Level, _scope: &str, _message: &str) {}
        fn emit(&self, _event: &str, _payload: Value) {}
    }

    fn locations(dir: &Path) -> EngineLocations {
        EngineLocations {
            managed_dir: dir.join("managed"),
            target: Target {
                platform: "linux",
                arch: "x64",
            },
            bundled: dir.join("stockfish"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bundled_engine_is_ready_when_it_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        assert!(!engine_status(&SilentHost, &locations, "").await.ready);
        std::fs::write(&locations.bundled, "#!/bin/sh\n").unwrap();
        assert!(
            !engine_status(&SilentHost, &locations, "").await.ready,
            "a file that cannot run is not an engine"
        );
        std::fs::set_permissions(&locations.bundled, std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let status = engine_status(&SilentHost, &locations, "").await;
        assert!(status.ready && status.bundled);
        assert_eq!(status.path, PathBuf::new());
        assert!(status.can_download);
        assert!(!status.managed.installed);
        assert_eq!(
            engine_identity(&status).await.unwrap(),
            BUNDLED_ENGINE_IDENTITY
        );
        let command = engine_command(&locations, &status);
        assert_eq!(
            command.as_std().get_program(),
            locations.bundled.as_os_str()
        );
        assert_eq!(command.as_std().get_args().count(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn configured_executable_wins_and_identity_tracks_changes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        std::fs::write(&locations.bundled, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&locations.bundled, std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let engine = dir.path().join("my-stockfish");
        std::fs::write(&engine, "#!/bin/sh\n").unwrap();
        let configured = engine.display().to_string();

        // Not executable yet: the bundled engine is used.
        let status = engine_status(&SilentHost, &locations, &configured).await;
        assert!(status.bundled);

        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        let status = engine_status(&SilentHost, &locations, &configured).await;
        assert!(status.ready && !status.bundled);
        assert_eq!(status.path, engine);
        let first = engine_identity(&status).await.unwrap();

        std::fs::write(&engine, "#!/bin/sh\necho changed\n").unwrap();
        let second = engine_identity(&status).await.unwrap();
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn missing_configured_engine_falls_back_and_reports_the_configured_path() {
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        let status = engine_status(&SilentHost, &locations, "/no/such/engine").await;
        assert!(!status.ready);
        assert_eq!(status.path, PathBuf::from("/no/such/engine"));
    }

    #[test]
    fn computer_playing_while_searching_or_just_after_a_move() {
        assert!(computer_playing(true, 0, 1_000, 5_000));
        assert!(computer_playing(false, 4_500, 1_000, 5_000));
        assert!(!computer_playing(false, 3_000, 1_000, 5_000));
        assert!(!computer_playing(false, 0, 1_000, 5_000));
    }

    #[test]
    fn trusts_only_the_managed_engine_and_chosen_paths() {
        let trust = EngineTrust::default();
        let managed = Path::new("/data/engines/stockfish/current");
        assert!(trust.is_trusted(&managed_path(managed), managed));
        assert!(!trust.is_trusted(Path::new("/usr/bin/stockfish"), managed));
        trust.trust(Path::new("/usr/bin/stockfish"));
        assert!(trust.is_trusted(Path::new("/usr/bin/stockfish"), managed));
        trust.clear();
        assert!(!trust.is_trusted(Path::new("/usr/bin/stockfish"), managed));
    }
}
