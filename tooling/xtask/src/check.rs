//! `check`, `check-fast` and the pre-push hook.

use std::io::Read;
use std::process::Command;

use crate::util::{Result, available, output, root, run, run_with, say};

/// Coverage floors for the workspace without the GPUI app and this tooling, as whole percentages
/// rounded down from the measured totals of `cargo llvm-cov` (2026-10-29: lines 81.88%, regions
/// 79.78%, functions 80.58%). The run fails below
/// a floor. Ratchet them up when the measured totals rise; do not lower them without a
/// measurement that explains the drop.
///
/// The GPUI app's views are checked by running the app (its end-to-end suite), and its unit
/// tests still run here, outside the coverage totals.
const FLOORS: [(&str, f64); 3] = [("lines", 81.0), ("regions", 79.0), ("functions", 80.0)];

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

/// The workspace's tests: under the coverage gate when cargo-llvm-cov is installed (CI installs
/// it), else plainly.
fn tests() -> Result {
    if !available("cargo", &["llvm-cov", "--version"]) {
        say(
            "cargo-llvm-cov is not installed: running the tests without the coverage gate (CI runs it).",
        );
        return run("cargo", &["test", "--workspace", "--locked"]);
    }
    coverage()?;
    // Outside the totals: the app's views are checked by its end-to-end suite.
    run(
        "cargo",
        &["test", "--locked", "-p", "kchess-gpui", "-p", "xtask"],
    )
}

fn coverage() -> Result {
    let report = root().join("test-results/rust-coverage/summary.json");
    std::fs::create_dir_all(report.parent().unwrap_or(&root()))
        .map_err(|cause| cause.to_string())?;
    let path = report.to_string_lossy().to_string();
    run(
        "cargo",
        &[
            "llvm-cov",
            "--workspace",
            "--exclude",
            "kchess-gpui",
            "--exclude",
            "xtask",
            "--locked",
            "--summary-only",
            "--json",
            "--output-path",
            &path,
        ],
    )?;
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
    let mut packages: Vec<String> = files.iter().filter_map(|file| package_of(file)).collect();
    packages.sort();
    packages.dedup();
    if packages.is_empty()
        || files
            .iter()
            .any(|file| file == "Cargo.toml" || file == "Cargo.lock")
    {
        return step("tests", || {
            run("cargo", &["test", "--workspace", "--locked"])
        });
    }
    let mut command = Command::new("cargo");
    command.args(["test", "--locked"]);
    for package in &packages {
        command.args(["-p", package]);
    }
    step(&format!("tests of {}", packages.join(", ")), || {
        run_with(&mut command)
    })
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
