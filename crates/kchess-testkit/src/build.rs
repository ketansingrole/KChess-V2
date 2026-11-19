//! Another workspace package's program, built from a test: the CLI's tests start the service and
//! the scripted engine, the app's end-to-end tests the scripted engine.
//!
//! Nothing here reads Cargo's build folder layout. Cargo 1.100 moved test programs from
//! `target/<profile>/deps` to `target/<profile>/build/<package>/<hash>/out` (2026-11-20), and the
//! tests that went two folders up from their own program built the engine into
//! `target/debug/build/kchess-cli` and looked for it where it never was. So the target folder is
//! the one Cargo marks (`CACHEDIR.TAG`), and the program's path is what Cargo's JSON messages say.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The program `name` of the workspace package `package`, built first into the target folder
/// this test runs from, in its profile: Cargo does nothing when it is fresh, and a program left
/// over from before a change (a service of another protocol) is never run.
///
/// The target folder is passed on: `cargo llvm-cov` moves it with a flag that a nested build does
/// not inherit.
pub fn workspace_binary(package: &str, name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("the test's own program");
    let (target, profile) = target_of(&exe, |folder| folder.join("CACHEDIR.TAG").is_file())
        .unwrap_or_else(|| panic!("{} is in no Cargo target folder", exe.display()));
    let mut build = Command::new(env!("CARGO"));
    build.args(["build", "-q", "--locked", "-p", package, "--bin", name]);
    build.args(["--message-format", "json-render-diagnostics"]);
    build.arg("--target-dir").arg(&target);
    if profile == "release" {
        build.arg("--release");
    }
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(is_cargo_test_var) {
            build.env_remove(key);
        }
    }
    let output = build.stderr(Stdio::inherit()).output().expect("cargo runs");
    assert!(output.status.success(), "{name} builds");
    executable(&String::from_utf8_lossy(&output.stdout), name)
        .unwrap_or_else(|| panic!("cargo named no program {name}"))
}

/// The target folder holding `exe` (the nearest folder above it that `is_target` accepts) and the
/// profile folder's name under it, the first one on the way down to `exe`.
pub fn target_of(exe: &Path, is_target: impl Fn(&Path) -> bool) -> Option<(PathBuf, String)> {
    let target = exe.ancestors().skip(1).find(|folder| is_target(folder))?;
    let mut below = exe.strip_prefix(target).ok()?.components();
    let profile = below.next()?.as_os_str().to_str()?.to_string();
    // A program right in the target folder is in no profile.
    below.next()?;
    Some((target.to_path_buf(), profile))
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
    #[test]
    fn the_target_folder_is_the_marked_one_and_the_profile_the_folder_under_it() {
        let marked = |folder: &Path| folder == Path::new("/w/target");
        let new = Path::new("/w/target/debug/build/kchess-cli/64f4467b/out/cli-64f4467b");
        let old = Path::new("/w/target/release/deps/cli-64f4467b");
        let expected = |profile: &str| Some((PathBuf::from("/w/target"), profile.to_string()));
        assert_eq!(target_of(new, marked), expected("debug"));
        assert_eq!(target_of(old, marked), expected("release"));
        // The nearest marked folder: `cargo llvm-cov`'s target folder inside the usual one.
        let inner = |folder: &Path| folder.ends_with("target") || folder.ends_with("cov-target");
        let cov = Path::new("/w/target/cov-target/debug/build/x/1/out/x-1");
        let cov_target = Some((PathBuf::from("/w/target/cov-target"), "debug".to_string()));
        assert_eq!(target_of(cov, inner), cov_target);
        assert_eq!(target_of(Path::new("/w/target/cli"), marked), None);
        assert_eq!(target_of(new, |_| false), None);
        // Only a folder above the program: a program is never its own target folder.
        assert_eq!(target_of(Path::new("/w/target"), marked), None);
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
