//! Another workspace package's program, built from a test: the CLI's tests start the service and
//! the scripted engine, the app's end-to-end tests the scripted engine.
//!
//! Nothing here reads Cargo's build folder layout. Cargo 1.100 moved test programs from
//! `target/<profile>/deps` to `target/<profile>/build/<package>/<hash>/out` (2026-11-20), and the
//! tests that went two folders up from their own program built the engine into
//! `target/debug/build/kchess-cli` and looked for it where it never was. So the profile folder is
//! the one holding Cargo's lock, which every build takes there (`CACHEDIR.TAG` marks only a target
//! folder Cargo made itself: cargo-mutants' copies have none), the target folder the one above
//! it, and the program's path is what Cargo's JSON messages say.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The program `name` of the workspace package `package`, built first into the target folder
/// this test runs from, in its profile: Cargo does nothing when it is fresh, and a program left
/// over from before a change (a service of another protocol) is never run.
///
/// The target folder is passed on: `cargo llvm-cov` moves it with a flag that a nested build does
/// not inherit.
pub fn workspace_binary(package: &str, name: &str) -> PathBuf {
    let exe =
        std::env::current_exe().unwrap_or_else(|error| panic!("the test's own program: {error}"));
    let locked = |folder: &Path| LOCKS.iter().any(|lock| folder.join(lock).is_file());
    let (target, profile) = target_of(&exe, locked)
        .unwrap_or_else(|| panic!("{} is in no Cargo profile folder", exe.display()));
    let mut build = Command::new(env!("CARGO"));
    build.args(build_args(package, name, &target, &profile));
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(is_cargo_test_var) {
            build.env_remove(key);
        }
    }
    let output = (build.stderr(Stdio::inherit()).output())
        .unwrap_or_else(|error| panic!("cargo does not run: {error}"));
    assert!(output.status.success(), "{name} builds");
    executable(&String::from_utf8_lossy(&output.stdout), name)
        .unwrap_or_else(|| panic!("cargo named no program {name}"))
}

/// What `cargo` is given to build the program `name` of `package` into `target`, in `profile`'s
/// folder: the dev profile's (`debug`) unless it is `release`.
pub fn build_args(
    package: &str,
    name: &str,
    target: &Path,
    profile: &str,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> =
        ["build", "-q", "--locked", "-p", package, "--bin", name]
            .into_iter()
            .chain([
                "--message-format",
                "json-render-diagnostics",
                "--target-dir",
            ])
            .map(Into::into)
            .collect();
    args.push(target.into());
    if profile == "release" {
        args.push("--release".into());
    }
    args
}

/// The lock files Cargo keeps in a profile folder (`target/debug`): `.cargo-lock` in every
/// version, the other two since 1.100.
pub const LOCKS: [&str; 3] = [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"];

/// The target folder holding `exe` and the name of its profile folder: the nearest folder above
/// `exe` that `is_profile` accepts, and the folder above that.
pub fn target_of(exe: &Path, is_profile: impl Fn(&Path) -> bool) -> Option<(PathBuf, String)> {
    let profile = exe.ancestors().skip(1).find(|folder| is_profile(folder))?;
    let name = profile.file_name()?.to_str()?.to_string();
    Some((profile.parent()?.to_path_buf(), name))
}

/// The program `name` Cargo built, from its JSON messages (`--message-format json`): the
/// `executable` of the `compiler-artifact` whose target is the binary `name`.
pub fn executable(messages: &str, name: &str) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| message["target"]["name"] == name)
        .filter(|message| {
            let mut kinds = message["target"]["kind"].as_array().into_iter().flatten();
            kinds.any(|kind| kind == "bin")
        })
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
}

/// The variables Cargo sets for the test's own package (`CARGO_MANIFEST_DIR`, `OUT_DIR`,
/// `CARGO_PKG_*`…), left out of a nested build: build scripts that watch one (ring's) would
/// rebuild for it, and the next outer build rebuild them back (2026-11-10: about 7 s for every
/// test that built).
pub fn is_cargo_test_var(key: &str) -> bool {
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
    use super::*;

    /// The layout of Cargo 1.100 and the one before it give the same target and profile.
    /// The layout of Cargo 1.100 and the one before it give the same target and profile, in a
    /// target folder with no `CACHEDIR.TAG` too (cargo-mutants' copies, 2026-11-20).
    #[test]
    fn the_profile_folder_is_the_locked_one_and_the_target_folder_above_it() {
        let locked = |folder: &Path| {
            [Path::new("/w/target/debug"), Path::new("/w/target/release")].contains(&folder)
        };
        let new = Path::new("/w/target/debug/build/kchess-cli/64f4467b/out/cli-64f4467b");
        let old = Path::new("/w/target/release/deps/cli-64f4467b");
        let expected = |profile: &str| Some((PathBuf::from("/w/target"), profile.to_string()));
        assert_eq!(target_of(new, locked), expected("debug"));
        assert_eq!(target_of(old, locked), expected("release"));
        // The nearest: `cargo llvm-cov`'s target folder inside the usual one.
        let cov_locked = |folder: &Path| folder.ends_with("debug");
        let cov = Path::new("/w/target/debug/cov-target/debug/build/x/1/out/x-1");
        let cov_target = Some((
            PathBuf::from("/w/target/debug/cov-target"),
            "debug".to_string(),
        ));
        assert_eq!(target_of(cov, cov_locked), cov_target);
        assert_eq!(target_of(new, |_| false), None);
        // Only a folder above the program: a program is never its own profile folder.
        assert_eq!(target_of(Path::new("/w/target/debug"), locked), None);
        // A profile folder with no folder above it has no target folder.
        assert_eq!(
            target_of(Path::new("/x"), |folder| folder == Path::new("/")),
            None
        );
        assert_eq!(
            LOCKS,
            [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"]
        );
    }

    /// Where a test of this package runs from, its profile folder holds one of Cargo's locks.
    #[test]
    fn this_tests_own_profile_folder_holds_a_lock() {
        let exe = std::env::current_exe().unwrap();
        let locked = |folder: &Path| LOCKS.iter().any(|lock| folder.join(lock).is_file());
        let (target, profile) = target_of(&exe, locked).expect("a locked profile folder");
        assert!(exe.starts_with(target.join(profile)));
    }

    #[test]
    fn the_build_is_of_the_one_program_into_the_tests_target_folder_and_profile() {
        let args = |profile: &str| -> Vec<String> {
            build_args(
                "kchess-core",
                "kchess-fake-uci",
                Path::new("/w/target"),
                profile,
            )
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect()
        };
        let debug = [
            "build",
            "-q",
            "--locked",
            "-p",
            "kchess-core",
            "--bin",
            "kchess-fake-uci",
            "--message-format",
            "json-render-diagnostics",
            "--target-dir",
            "/w/target",
        ];
        assert_eq!(args("debug"), debug);
        assert_eq!(args("release")[..debug.len()], debug);
        assert_eq!(args("release")[debug.len()..], ["--release"]);
    }

    /// A program that does not build fails the test there: nothing is run in its place.
    #[test]
    #[should_panic(expected = "no-such-program builds")]
    fn a_program_that_does_not_build_fails_the_test() {
        workspace_binary("kchess-testkit", "no-such-program");
    }

    #[test]
    fn the_program_is_the_executable_of_the_named_binary() {
        let artifact = |name: &str, kind: &str, exe: &str| {
            serde_json::json!({
                "reason": "compiler-artifact",
                "target": { "name": name, "kind": [kind] },
                "executable": exe,
            })
            .to_string()
        };
        let messages = [
            "not json".to_string(),
            serde_json::json!({ "reason": "build-script-executed" }).to_string(),
            artifact("kchess-core", "lib", "/t/lib"),
            artifact("kchess-fake-uci", "test", "/t/test"),
            serde_json::json!({
                "reason": "compiler-message",
                "target": { "name": "kchess-fake-uci", "kind": ["bin"] },
                "executable": "/t/message",
            })
            .to_string(),
            artifact("kchess-fake-uci", "bin", "/t/debug/kchess-fake-uci"),
            serde_json::json!({ "reason": "build-finished", "success": true }).to_string(),
        ]
        .join("\n");
        assert_eq!(
            executable(&messages, "kchess-fake-uci"),
            Some(PathBuf::from("/t/debug/kchess-fake-uci"))
        );
        assert_eq!(executable(&messages, "kchess-service"), None);
        assert_eq!(executable("", "kchess-fake-uci"), None);
    }

    #[test]
    fn only_the_variables_of_the_tests_own_package_are_left_out() {
        for key in [
            "CARGO_PKG_NAME",
            "CARGO_BIN_EXE_kchess-cli",
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
