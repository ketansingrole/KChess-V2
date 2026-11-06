//! `check`, `check-fast` and the pre-push hook.

use std::io::Read;
use std::process::Command;

use crate::util::{Result, available, output, root, run, run_with, say};

/// Coverage floors for the workspace without the GPUI app and this tooling, as whole percentages
/// rounded down from the measured totals of `cargo llvm-cov` (2026-10-29: lines 88.30%, regions
/// 86.56%, functions 86.37%). The run fails below
/// a floor. Ratchet them up when the measured totals rise; do not lower them without a
/// measurement that explains the drop.
///
/// The GPUI app's views are checked by running the app (its end-to-end suite), and its unit
/// tests still run here, outside the coverage totals.
const FLOORS: [(&str, f64); 3] = [("lines", 88.0), ("regions", 86.0), ("functions", 86.0)];

/// The shipped binaries, whose code must not unwrap or expect outside tests.
const SHIPPED_BINS: [&str; 4] = [
    "kchess-voice",
    "kchess-service",
    "kchess-cli",
    "kchess-electron-bridge",
];

fn step(name: &str, work: impl FnOnce() -> Result) -> Result {
    say(&format!("Checking {name}…"));
    work().map_err(|cause| format!("{name}: {cause}"))
}

fn lints() -> Result {
    run("cargo", &["fmt", "--all", "--check"])?;
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--lib",
            "--locked",
            "--",
            "-D",
            "clippy::unwrap_used",
            "-D",
            "clippy::expect_used",
        ],
    )?;
    run("cargo", &shipped_bins_args())
}

/// One clippy run over every shipped binary, its `unwrap`/`expect` denied as the libraries' are.
fn shipped_bins_args() -> Vec<&'static str> {
    let mut args = vec!["clippy"];
    for bin in SHIPPED_BINS {
        args.extend(["-p", bin]);
    }
    args.extend([
        "--bins",
        "--locked",
        "--",
        "-D",
        "clippy::unwrap_used",
        "-D",
        "clippy::expect_used",
    ]);
    args
}

/// Whether cargo-nextest is installed (CI installs it). It runs every test in its own process and
/// every test binary at once, where `cargo test` runs one binary at a time (2026-11-06: the core's
/// tests in 7 s against 19 s); `.config/nextest.toml` holds its profiles.
fn nextest() -> bool {
    available("cargo", &["nextest", "--version"])
}

/// Run the tests of these packages, or of the whole workspace when there are none: with nextest
/// when it is installed, else with `cargo test`.
fn run_tests(packages: &[String]) -> Result {
    let mut command = Command::new("cargo");
    if nextest() {
        command.args(["nextest", "run", "--locked", "--no-tests=warn"]);
    } else {
        say(
            "cargo-nextest is not installed (`cargo install cargo-nextest`): running `cargo test`.",
        );
        command.args(["test", "--locked"]);
    }
    if packages.is_empty() {
        command.arg("--workspace");
    }
    for package in packages {
        command.args(["-p", package]);
    }
    run_with(&mut command)
}

/// The workspace's tests: under the coverage gate when cargo-llvm-cov is installed (CI installs
/// it), else plainly; then the doctests, which nextest does not run.
fn tests() -> Result {
    if available("cargo", &["llvm-cov", "--version"]) {
        coverage()?;
        // Outside the totals: the app's views are checked by its end-to-end suite.
        run_tests(&["kchess-gpui".into(), "xtask".into()])?;
    } else {
        say(
            "cargo-llvm-cov is not installed: running the tests without the coverage gate (CI runs it).",
        );
        run_tests(&[])?;
    }
    run("cargo", &["test", "--doc", "--workspace", "--locked"])
}

/// `cargo llvm-cov`'s arguments: through nextest when it is installed.
fn coverage_args(nextest: bool, report: &str) -> Vec<&str> {
    let mut args = vec!["llvm-cov"];
    if nextest {
        args.push("nextest");
    }
    args.extend([
        "--workspace",
        "--exclude",
        "kchess-gpui",
        "--exclude",
        "xtask",
        "--locked",
        "--summary-only",
        "--json",
        "--output-path",
        report,
    ]);
    args
}

fn coverage() -> Result {
    let report = root().join("test-results/rust-coverage/summary.json");
    std::fs::create_dir_all(report.parent().unwrap_or(&root()))
        .map_err(|cause| cause.to_string())?;
    let path = report.to_string_lossy().to_string();
    run("cargo", &coverage_args(nextest(), &path))?;
    let text = std::fs::read_to_string(&report).map_err(|cause| cause.to_string())?;
    let summary: serde_json::Value =
        serde_json::from_str(&text).map_err(|cause| cause.to_string())?;
    let totals = &summary["data"][0]["totals"];
    let mut below = Vec::new();
    let mut measured = Vec::new();
    for (metric, floor) in FLOORS {
        let percent = totals[metric]["percent"]
            .as_f64()
            .ok_or_else(|| format!("The coverage report has no {metric}."))?;
        measured.push(format!("{metric} {percent:.2}%"));
        if percent < floor {
            below.push(format!(
                "{metric} coverage {percent:.2}% is below the {floor}% floor"
            ));
        }
    }
    say(&format!("Rust coverage: {}.", measured.join(", ")));
    if below.is_empty() {
        Ok(())
    } else {
        Err(below.join("; "))
    }
}

/// `check`: everything a push to main must pass.
pub fn full() -> Result {
    step("format and lints", lints)?;
    step("notices", crate::licenses::check_notices)?;
    step("icons", crate::icons::check)?;
    step("release metadata", crate::release::verify)?;
    step("tests", tests)?;
    say("`check` passed.");
    Ok(())
}

/// The workspace package a changed file belongs to, by its nearest Cargo.toml.
fn package_of(file: &str) -> Option<String> {
    let mut dir = std::path::Path::new(file).parent();
    while let Some(current) = dir {
        let manifest = root().join(current).join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest)
            && let Some(name) = text
                .lines()
                .skip_while(|line| line.trim() != "[package]")
                .find_map(|line| line.trim().strip_prefix("name = "))
        {
            return Some(name.trim_matches('"').to_string());
        }
        dir = current.parent();
    }
    None
}

/// `check-fast`: format, lints and the tests of the packages changed since main (committed or
/// not). Never a substitute for `check` before a push to main.
pub fn fast() -> Result {
    let base = output("git", &["merge-base", "HEAD", "origin/main"])
        .map(|text| text.trim().to_string())
        .unwrap_or_else(|_| "HEAD".into());
    let mut files: Vec<String> = output("git", &["diff", "--name-only", &base])?
        .lines()
        .map(str::to_string)
        .collect();
    files.extend(
        output("git", &["ls-files", "--others", "--exclude-standard"])?
            .lines()
            .map(str::to_string),
    );
    let rust = files
        .iter()
        .any(|file| file.ends_with(".rs") || file.ends_with("Cargo.toml") || file == "Cargo.lock");
    if !rust {
        say("No Rust changes since main.");
        return Ok(());
    }
    step("format", || run("cargo", &["fmt", "--all", "--check"]))?;
    step("lints", || {
        run(
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        )
    })?;
    let mut changed: Vec<String> = files.iter().filter_map(|file| package_of(file)).collect();
    changed.sort();
    changed.dedup();
    if changed.is_empty()
        || files
            .iter()
            .any(|file| file == "Cargo.toml" || file == "Cargo.lock")
    {
        return step("tests", || run_tests(&[]));
    }
    let packages = with_dependents(&changed, &workspace_dependencies()?);
    step(&format!("tests of {}", packages.join(", ")), || {
        run_tests(&packages)
    })
}

/// Each workspace package with the workspace packages it depends on (normal, dev or build), from
/// `cargo metadata`.
fn workspace_dependencies() -> Result<Vec<(String, Vec<String>)>> {
    let text = output(
        "cargo",
        &["metadata", "--format-version", "1", "--no-deps", "--locked"],
    )?;
    let metadata: serde_json::Value =
        serde_json::from_str(&text).map_err(|cause| cause.to_string())?;
    let packages = metadata["packages"].as_array().cloned().unwrap_or_default();
    let names: Vec<String> = packages
        .iter()
        .filter_map(|package| package["name"].as_str().map(str::to_string))
        .collect();
    Ok(packages
        .iter()
        .filter_map(|package| {
            let name = package["name"].as_str()?.to_string();
            let dependencies = package["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|dependency| dependency["name"].as_str())
                .filter(|dependency| names.iter().any(|name| name == dependency))
                .map(str::to_string)
                .collect();
            Some((name, dependencies))
        })
        .collect())
}

/// The changed packages and every workspace package that depends on one of them, directly or not:
/// a change to the core runs the app's tests as well as its own.
fn with_dependents(changed: &[String], dependencies: &[(String, Vec<String>)]) -> Vec<String> {
    let mut packages = changed.to_vec();
    loop {
        let before = packages.len();
        for (name, uses) in dependencies {
            if !packages.contains(name) && uses.iter().any(|used| packages.contains(used)) {
                packages.push(name.clone());
            }
        }
        if packages.len() == before {
            break;
        }
    }
    packages.sort();
    packages
}

/// The local commits a push sends to main, from the hook's standard input.
pub fn main_pushes(input: &str) -> Vec<(String, String)> {
    input
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [local_ref, local_sha, remote_ref, ..] = fields.as_slice() else {
                return None;
            };
            (*remote_ref == "refs/heads/main" && !local_sha.bytes().all(|b| b == b'0'))
                .then(|| ((*local_ref).to_string(), (*local_sha).to_string()))
        })
        .collect()
}

/// Why `check` cannot vouch for this push: it runs on the working tree, so the pushed commit must
/// be HEAD and nothing may be uncommitted or untracked.
pub fn push_problem(pushes: &[(String, String)], head: &str, status: &str) -> Option<String> {
    if let Some((reference, sha)) = pushes.iter().find(|(_, sha)| sha != head) {
        return Some(format!(
            "{reference} ({}) is not the checked-out HEAD; check it out to verify it.",
            &sha[..sha.len().min(12)]
        ));
    }
    (!status.trim().is_empty()).then(|| {
        format!(
            "the working tree has uncommitted or untracked files, which `cargo xtask check` would include:\n{}",
            status.trim_end()
        )
    })
}

/// The pre-push hook: pushes to main must pass the full `check` first; other branches push at
/// once (CI still runs everything). Skip on purpose with `KCHESS_SKIP_PRE_PUSH=1`.
pub fn pre_push() -> Result {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|cause| cause.to_string())?;
    let pushes = main_pushes(&input);
    if pushes.is_empty() {
        return Ok(());
    }
    if std::env::var("KCHESS_SKIP_PRE_PUSH").as_deref() == Ok("1") {
        say("KCHESS_SKIP_PRE_PUSH=1: pushing to main without `cargo xtask check`.");
        return Ok(());
    }
    let head = output("git", &["rev-parse", "HEAD"])?;
    let status = output(
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
    )?;
    if let Some(problem) = push_problem(&pushes, head.trim(), &status) {
        return Err(format!(
            "Push to main refused: {problem}\nSkip on purpose with KCHESS_SKIP_PRE_PUSH=1 git push …"
        ));
    }
    say("Pushing to main: running `cargo xtask check`…");
    full().map_err(|cause| format!("Push to main refused: {cause}"))
}

/// Point Git at the repository's hooks, unless the clone already uses others on purpose.
pub fn install_hooks() -> Result {
    let current = output("git", &["config", "--get", "core.hooksPath"]).unwrap_or_default();
    if !current.trim().is_empty() && current.trim() != ".githooks" {
        say(&format!(
            "core.hooksPath is {}; leaving it.",
            current.trim()
        ));
        return Ok(());
    }
    run("git", &["config", "core.hooksPath", ".githooks"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pushes_to_main_are_checked() {
        let input = "refs/heads/main abc123 refs/heads/main 000\n\
                     refs/heads/topic def456 refs/heads/topic 000\n\
                     (delete) 0000000000 refs/heads/main 111\n";
        assert_eq!(
            main_pushes(input),
            [("refs/heads/main".to_string(), "abc123".to_string())]
        );
    }

    #[test]
    fn a_push_must_be_the_clean_head() {
        let pushes = [("refs/heads/main".to_string(), "abc123".to_string())];
        assert_eq!(push_problem(&pushes, "abc123", ""), None);
        assert!(
            push_problem(&pushes, "fff", "")
                .unwrap()
                .contains("not the checked-out HEAD")
        );
        assert!(
            push_problem(&pushes, "abc123", "?? new.rs\n")
                .unwrap()
                .contains("untracked")
        );
    }

    #[test]
    fn one_run_checks_every_shipped_binary() {
        let args = shipped_bins_args();
        for bin in SHIPPED_BINS {
            assert!(args.windows(2).any(|pair| pair == ["-p", bin]), "{bin}");
        }
        assert!(args.contains(&"--bins"));
        assert!(args.ends_with(&["-D", "clippy::expect_used"]));
    }

    #[test]
    fn a_change_tests_every_package_that_depends_on_it() {
        let dependencies = [
            (
                "app".to_string(),
                vec!["core".to_string(), "domain".to_string()],
            ),
            ("core".to_string(), vec!["domain".to_string()]),
            ("domain".to_string(), vec![]),
            ("tooling".to_string(), vec![]),
        ];
        assert_eq!(
            with_dependents(&["domain".into()], &dependencies),
            ["app", "core", "domain"]
        );
        assert_eq!(with_dependents(&["app".into()], &dependencies), ["app"]);
        assert_eq!(
            with_dependents(&["tooling".into(), "core".into()], &dependencies),
            ["app", "core", "tooling"]
        );
    }

    #[test]
    fn coverage_runs_through_nextest_when_it_is_installed() {
        assert_eq!(coverage_args(true, "r.json")[..2], ["llvm-cov", "nextest"]);
        assert_eq!(coverage_args(false, "r.json")[1], "--workspace");
        assert!(coverage_args(true, "r.json").ends_with(&["--output-path", "r.json"]));
    }

    #[test]
    fn files_belong_to_their_nearest_package() {
        assert_eq!(
            package_of("crates/kchess-domain/src/api.rs").as_deref(),
            Some("kchess-domain")
        );
        assert_eq!(
            package_of("tooling/xtask/src/check.rs").as_deref(),
            Some("xtask")
        );
        assert_eq!(package_of("README.md"), None);
    }
}
