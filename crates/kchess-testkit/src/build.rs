//! A binary of another workspace package, built for the tests that run it: the scripted engine
//! (`kchess-core`'s `kchess-fake-uci`) for the app's end-to-end tests and the command line's, the
//! core service for the command line's.
//!
//! Built once per test run, under a lock every test process of the target folder shares. A fresh
//! `cargo build` is not free of effects: on macOS Cargo copies each binary out of `deps` rather
//! than linking it (rust-lang/cargo#10060), and it deletes the old copy first, on every build.
//! The end-to-end tests built in each of their processes, a thousand times a run, and one test's
//! copy of the engine then and again found it gone mid-build ("copy the scripted engine:
//! NotFound", the Stress workflow on macOS, 2026-11-12 and 2026-11-18).

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// `name`, a binary of the workspace's `package`, as this test run built it. The lock it holds
/// keeps every other test process from building until it is dropped: copy it before then.
pub fn workspace_binary(package: &str, name: &str) -> Built {
    let profile = profile_dir();
    let command = build_command(Path::new(env!("CARGO")), &profile, package, name);
    built(&profile, name, &run_key(), || {
        let mut command = command;
        let status = command.status().expect("cargo runs");
        assert!(status.success(), "{name} builds");
    })
}

/// A binary as this run built it, and the build lock, held until it is dropped.
pub struct Built {
    path: PathBuf,
    _lock: File,
}

impl Built {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Copy the binary to `dest`, unless it is a copy already: whole, under another name first,
    /// so a test starting at the same time never runs half a copy, and only when it differs, as
    /// Windows refuses to replace a program that is running.
    pub fn copy_to(&self, dest: &Path) {
        let same = std::fs::read(&self.path)
            .ok()
            .zip(std::fs::read(dest).ok())
            .is_some_and(|(binary, copy)| binary == copy);
        if same {
            return;
        }
        let mut partial = dest.as_os_str().to_owned();
        partial.push(format!(".{}.partial", std::process::id()));
        let partial = PathBuf::from(partial);
        std::fs::copy(&self.path, &partial).expect("copy the binary");
        std::fs::rename(&partial, dest).expect("name the binary's copy");
    }
}

/// `name` in `profile`, once `build` has made it, unless this run (`run`) did already and it is
/// still there. Holds the profile's build lock, taken first.
fn built(profile: &Path, name: &str, run: &str, build: impl FnOnce()) -> Built {
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(profile.join(".kchess-test-builds.lock"))
        .expect("the build lock");
    lock.lock().expect("take the build lock");
    let path = profile.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let stamp = profile.join(format!("{name}.test-run"));
    let current = std::fs::read_to_string(&stamp).is_ok_and(|built| built == run);
    if !current || !path.exists() {
        build();
        std::fs::write(&stamp, run).expect("note the run that built it");
    }
    Built { path, _lock: lock }
}

/// The test run this process belongs to: nextest's for its run, whose tests each have a process,
/// else this process alone (`cargo test` runs a package's tests in one).
fn run_key() -> String {
    static KEY: OnceLock<String> = OnceLock::new();
    KEY.get_or_init(|| {
        std::env::var("NEXTEST_RUN_ID").unwrap_or_else(|_| {
            // With the time, so a later process given the same ID builds again.
            let started = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("process {} at {started}", std::process::id())
        })
    })
    .clone()
}

/// The folder of the profile the running test was built in, `target/<profile>`, whose `deps`
/// holds the test.
fn profile_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.to_path_buf()))
        .expect("the test executable is in target/<profile>/deps")
}

/// `cargo build` of `package`'s `name` into the target folder `profile` is in, in its profile.
fn build_command(cargo: &Path, profile: &Path, package: &str, name: &str) -> Command {
    let mut build = Command::new(cargo);
    build.args(["build", "-q", "--locked", "-p", package, "--bin", name]);
    // Without the variables Cargo set for the test's own package: build scripts that watch one
    // (ring's) would rebuild for it, and the next outer build rebuild them back (2026-11-10:
    // about 7 s for every test that built).
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(is_cargo_test_var) {
            build.env_remove(key);
        }
    }
    // Into the target folder the test runs from, which `cargo llvm-cov` moves with a flag that a
    // nested build does not inherit.
    build
        .arg("--target-dir")
        .arg(profile.parent().expect("the target folder"));
    if profile.file_name().is_some_and(|name| name == "release") {
        build.arg("--release");
    }
    build
}

/// A variable Cargo sets for the package it builds or tests (`CARGO_MANIFEST_DIR`, `OUT_DIR`,
/// `CARGO_PKG_*`…), not for every build.
fn is_cargo_test_var(key: &str) -> bool {
    key.starts_with("CARGO_PKG_")
        || key.starts_with("CARGO_BIN_EXE_")
        || matches!(
            key,
            "CARGO_MANIFEST_DIR"
                | "CARGO_MANIFEST_PATH"
                | "CARGO_CRATE_NAME"
                | "CARGO_PRIMARY_PACKAGE"
                | "CARGO_TARGET_TMPDIR"
                | "CARGO_BIN_NAME"
                | "OUT_DIR"
        )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::OsStr;
    use std::time::{Duration, SystemTime};

    use super::*;

    /// A build that writes `content` as the binary and counts itself.
    fn making<'a>(
        profile: &'a Path,
        content: &'a str,
        builds: &'a Cell<usize>,
    ) -> impl FnOnce() + 'a {
        move || {
            builds.set(builds.get() + 1);
            let path = profile.join(format!("tool{}", std::env::consts::EXE_SUFFIX));
            std::fs::write(path, content).unwrap();
        }
    }

    #[test]
    fn a_run_builds_once() {
        let profile = tempfile::tempdir().unwrap();
        let builds = Cell::new(0);
        let first = built(
            profile.path(),
            "tool",
            "run 1",
            making(profile.path(), "1", &builds),
        );
        assert_eq!(std::fs::read_to_string(first.path()).unwrap(), "1");
        drop(first);
        let again = built(
            profile.path(),
            "tool",
            "run 1",
            making(profile.path(), "2", &builds),
        );
        assert_eq!(builds.get(), 1, "the run built it already");
        assert_eq!(std::fs::read_to_string(again.path()).unwrap(), "1");
        drop(again);
        // The next run builds it again: a change since is never missed.
        let next = built(
            profile.path(),
            "tool",
            "run 2",
            making(profile.path(), "3", &builds),
        );
        assert_eq!(builds.get(), 2);
        assert_eq!(std::fs::read_to_string(next.path()).unwrap(), "3");
    }

    #[test]
    fn a_binary_gone_since_is_built_again() {
        let profile = tempfile::tempdir().unwrap();
        let builds = Cell::new(0);
        let first = built(
            profile.path(),
            "tool",
            "run",
            making(profile.path(), "1", &builds),
        );
        std::fs::remove_file(first.path()).unwrap();
        drop(first);
        let again = built(
            profile.path(),
            "tool",
            "run",
            making(profile.path(), "2", &builds),
        );
        assert_eq!(builds.get(), 2);
        assert!(again.path().exists());
    }

    /// Every other process waits while one builds and copies: none sees a binary half made.
    #[test]
    fn the_lock_is_held_until_the_binary_is_dropped() {
        let profile = tempfile::tempdir().unwrap();
        let builds = Cell::new(0);
        let held = built(
            profile.path(),
            "tool",
            "run",
            making(profile.path(), "1", &builds),
        );
        let lock = File::open(profile.path().join(".kchess-test-builds.lock")).unwrap();
        assert!(matches!(
            lock.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(held);
        lock.try_lock().expect("free once dropped");
    }

    #[test]
    fn a_copy_is_whole_and_written_only_when_it_differs() {
        let profile = tempfile::tempdir().unwrap();
        let builds = Cell::new(0);
        let binary = built(
            profile.path(),
            "tool",
            "run",
            making(profile.path(), "1", &builds),
        );
        let dest = profile.path().join("copy");
        binary.copy_to(&dest);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "1");
        let leftovers: Vec<_> = std::fs::read_dir(profile.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().ends_with(".partial"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        // The same: left alone, as a running copy must be on Windows.
        let old = SystemTime::now() - Duration::from_secs(3600);
        File::options()
            .write(true)
            .open(&dest)
            .unwrap()
            .set_modified(old)
            .unwrap();
        binary.copy_to(&dest);
        let modified = std::fs::metadata(&dest).unwrap().modified().unwrap();
        assert_eq!(modified, old);

        // Different: replaced.
        std::fs::write(&dest, "stale").unwrap();
        binary.copy_to(&dest);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "1");
    }

    #[test]
    fn the_run_is_nextests_or_this_process() {
        let key = run_key();
        assert_eq!(run_key(), key, "the same for the whole process");
        match std::env::var("NEXTEST_RUN_ID") {
            Ok(run) => assert_eq!(key, run),
            Err(_) => assert!(key.starts_with(&format!("process {} at ", std::process::id()))),
        }
    }

    #[test]
    fn the_profile_is_the_tests_own() {
        let profile = profile_dir();
        let exe = std::env::current_exe().unwrap();
        assert_eq!(exe.parent().unwrap(), profile.join("deps"));
    }

    #[test]
    fn the_build_is_of_the_binary_in_the_tests_target_folder() {
        let target = Path::new("/work/target");
        let args = |profile: &str| -> Vec<String> {
            build_command(Path::new("cargo"), &target.join(profile), "pkg", "tool")
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect()
        };
        let mut debug = vec!["build", "-q", "--locked", "-p", "pkg", "--bin", "tool"];
        debug.extend(["--target-dir", "/work/target"]);
        assert_eq!(args("debug"), debug);
        let mut release = debug.clone();
        release.push("--release");
        assert_eq!(args("release"), release);
    }

    /// The test's own package's variables are left out; Cargo's others reach the build.
    #[test]
    fn the_build_leaves_out_the_tests_package() {
        let build = build_command(Path::new("cargo"), Path::new("/t/debug"), "pkg", "tool");
        let removed: Vec<&OsStr> = build
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key)
            .collect();
        // Cargo sets this for every test it runs.
        assert!(
            removed.contains(&OsStr::new("CARGO_MANIFEST_DIR")),
            "{removed:?}"
        );
        for key in removed {
            assert!(is_cargo_test_var(key.to_str().unwrap()), "{key:?}");
        }
    }

    #[test]
    fn cargos_package_variables_are_told_apart() {
        for key in [
            "CARGO_PKG_NAME",
            "CARGO_BIN_EXE_tool",
            "CARGO_MANIFEST_DIR",
            "CARGO_MANIFEST_PATH",
            "CARGO_CRATE_NAME",
            "CARGO_PRIMARY_PACKAGE",
            "CARGO_TARGET_TMPDIR",
            "CARGO_BIN_NAME",
            "OUT_DIR",
        ] {
            assert!(is_cargo_test_var(key), "{key}");
        }
        for key in [
            "CARGO",
            "CARGO_HOME",
            "CARGO_TARGET_DIR",
            "PATH",
            "RUSTFLAGS",
        ] {
            assert!(!is_cargo_test_var(key), "{key}");
        }
    }
}
