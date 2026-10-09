//! Engine status, identity and trust (formerly the status parts of `core/src/services/engine.ts`):
//! which engine would run, the command that starts it, a key that changes when the executable
//! does, the engine paths the renderer may persist, and whether the computer is playing.
//! Search orchestration (`bestMove`, `stopEngine`) belongs to the UCI controller port.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use tokio::process::Command;

use super::managed::{ManagedEngine, Target, can_download, executable_name, managed_engine};
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

/// The bundled Stockfish UCI script, relative to the application: the `stockfish` npm package's
/// lite multi-threaded WASM build, run as a UCI process by the host's own Node runtime. The host
/// resolves it to an absolute path for `EngineLocations::bundled_script`.
pub const BUNDLED_ENGINE_SCRIPT: &str = "node_modules/stockfish/bin/stockfish-19-lite.js";
/// The identity of the bundled engine, which never changes within a build.
pub const BUNDLED_ENGINE_IDENTITY: &str = "stockfish-19-lite";

/// What the host supplies to find and start engines.
#[derive(Clone, Debug)]
pub struct EngineLocations {
    /// The managed engine directory (`managed::default_dir`).
    pub managed_dir: PathBuf,
    /// The platform whose official build may be downloaded.
    pub target: Target,
    /// Absolute path of `BUNDLED_ENGINE_SCRIPT`.
    pub bundled_script: PathBuf,
    /// The host's Node executable, which runs the bundled script (`process.execPath`).
    pub node_path: PathBuf,
    /// Extra environment for running the bundled script, on top of the inherited one.
    pub node_env: BTreeMap<String, String>,
}

/// What `engine_status` reports (`EngineStatus` in the TypeScript contracts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
/// bundled script when it is readable. Not ready when neither is.
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
    if access(&locations.bundled_script, AccessMode::Read) {
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
            "Bundled engine check failed: {} is not readable",
            locations.bundled_script.display()
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
    if !access(path, AccessMode::Execute) {
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

/// The command that starts the engine `status` names: the bundled script under the host's Node
/// runtime with its extra environment, or the native executable. The command is not spawned; the
/// caller chooses its stdio.
pub fn engine_command(locations: &EngineLocations, status: &EngineStatus) -> Command {
    if status.bundled {
        let mut command = Command::new(&locations.node_path);
        command
            .arg(&locations.bundled_script)
            .envs(&locations.node_env);
        command
    } else {
        Command::new(&status.path)
    }
}

/// A key that changes when the selected executable does: the bundled identity, or the path with
/// its size and modification time.
pub async fn engine_identity(status: &EngineStatus) -> Result<String> {
    if status.bundled {
        return Ok(BUNDLED_ENGINE_IDENTITY.to_string());
    }
    let metadata = tokio::fs::metadata(&status.path)
        .await
        .map_err(|cause| CoreError::new(cause.to_string()))?;
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

/// The engine paths the renderer may persist: the managed engine, or one the user picked in the
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

#[derive(Clone, Copy)]
enum AccessMode {
    Read,
    Execute,
}

/// `access(2)` for the mode, as Node's `fs.access` checks it. Without `access`, the Windows
/// check is existence, which is also what Node does for `X_OK` there.
#[cfg(unix)]
fn access(path: &Path, mode: AccessMode) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let flag = match mode {
        AccessMode::Read => libc::R_OK,
        AccessMode::Execute => libc::X_OK,
    };
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c_path` is a valid NUL-terminated string that lives across the call.
    unsafe { libc::access(c_path.as_ptr(), flag) == 0 }
}

#[cfg(not(unix))]
fn access(path: &Path, _mode: AccessMode) -> bool {
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
            bundled_script: dir.join("bundled.js"),
            node_path: PathBuf::from("/usr/bin/node"),
            node_env: BTreeMap::from([("KCHESS_TEST".to_string(), "1".to_string())]),
        }
    }

    #[tokio::test]
    async fn bundled_engine_is_ready_when_its_script_is_readable() {
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        assert!(!engine_status(&SilentHost, &locations, "").await.ready);
        std::fs::write(&locations.bundled_script, "// engine").unwrap();
        let status = engine_status(&SilentHost, &locations, "").await;
        assert!(status.ready && status.bundled);
        assert_eq!(status.path, PathBuf::new());
        assert!(status.can_download);
        assert!(!status.managed.installed);
        assert_eq!(
            engine_identity(&status).await.unwrap(),
            BUNDLED_ENGINE_IDENTITY
        );
    }

    #[tokio::test]
    async fn bundled_command_runs_the_script_with_node_and_its_environment() {
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        let status = EngineStatus {
            ready: true,
            path: PathBuf::new(),
            bundled: true,
            managed: ManagedEngine {
                installed: false,
                path: managed_path(&locations.managed_dir),
                version: None,
            },
            can_download: true,
        };
        let command = engine_command(&locations, &status);
        let std_command = command.as_std();
        assert_eq!(std_command.get_program(), locations.node_path.as_os_str());
        let args: Vec<_> = std_command.get_args().collect();
        assert_eq!(args, [locations.bundled_script.as_os_str()]);
        let envs: Vec<_> = std_command
            .get_envs()
            .map(|(key, value)| (key.to_os_string(), value.map(|v| v.to_os_string())))
            .collect();
        assert!(envs.contains(&(OsString::from("KCHESS_TEST"), Some(OsString::from("1")))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn configured_executable_wins_and_identity_tracks_changes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        std::fs::write(&locations.bundled_script, "// engine").unwrap();
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
