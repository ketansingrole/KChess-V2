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
const SHIPPED_BINS: [&str; 3] = ["kchess-voice", "kchess-service", "kchess-cli"];

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

/// The app, whose tests run under `cargo test`: they are one binary, so nextest gains no
/// parallelism there, and each test in its own process starts GPUI afresh (2026-11-06, 546 tests:
/// 60 s under `cargo test`, 74 s under nextest).
const APP: &str = "kchess-gpui";

/// The commands that run the tests of these packages, or of the whole workspace when there are
/// none: the app's under `cargo test`, the others under nextest when it is installed.
fn test_commands(packages: &[String], nextest: bool) -> Vec<Vec<String>> {
    let app = packages.is_empty() || packages.iter().any(|package| package == APP);
    let others: Vec<&String> = packages.iter().filter(|package| *package != APP).collect();
    let mut commands = Vec::new();
    if packages.is_empty() || !others.is_empty() {
        let mut command: Vec<String> = if nextest {
            ["nextest", "run", "--locked", "--no-tests=warn"]
                .map(String::from)
                .to_vec()
        } else {
            ["test", "--locked"].map(String::from).to_vec()
        };
        if packages.is_empty() {
            command.extend(["--workspace", "--exclude", APP].map(String::from));
        }
        for package in others {
            command.extend(["-p".to_string(), package.clone()]);
        }
        commands.push(command);
    }
    if app {
        commands.push(["test", "--locked", "-p", APP].map(String::from).to_vec());
    }
    commands
}

/// Run the tests of these packages, or of the whole workspace when there are none.
fn run_tests(packages: &[String]) -> Result {
    let nextest = nextest();
    if !nextest {
        say(
            "cargo-nextest is not installed (`cargo install cargo-nextest`): running `cargo test`.",
        );
    }
    for args in test_commands(packages, nextest) {
        run_with(Command::new("cargo").args(&args))?;
    }
    Ok(())
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
        // A test process killed while writing its profile leaves a corrupt one, and llvm-profdata
        // then refuses to merge any: CI's check failed twice with every test passed ("file header
        // is corrupt … no profile can be merged"). The rest still measure the code; only when
        // none can be merged is the run refused.
        "--failure-mode",
        "all",
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

/// Whether a change to `file` can change what a package builds or tests: Rust, Cargo files, and
/// anything inside a package, which `include_str!`, `build.rs` or a test may read (a release key
/// once skipped every test this way). Documents outside the packages do not.
fn builds_into_a_package(file: &str) -> bool {
    file.ends_with(".rs")
        || file.ends_with("Cargo.toml")
        || file == "Cargo.lock"
        || package_of(file).is_some()
}

/// Whether a change can add or drop a shipped dependency, which THIRD_PARTY_NOTICES.md must name.
fn touches_dependencies(files: &[String]) -> bool {
    files.iter().any(|file| {
        file == "THIRD_PARTY_NOTICES.md"
            || file == "Cargo.lock"
            || file == "Cargo.toml"
            || file.ends_with("/Cargo.toml")
    })
}

/// `check-fast`: format, lints, the notices when dependencies changed, and the tests of the packages changed since main (committed or
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
    if touches_dependencies(&files) {
        step("notices", crate::licenses::check_notices)?;
    }
    // xtask's tests check the workflows' scripts (`every_workflow_step_parses_in_bash`).
    let workflows = files
        .iter()
        .any(|file| file.starts_with(".github/workflows/"));
    if !workflows && !files.iter().any(|file| builds_into_a_package(file)) {
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
    if workflows {
        changed.push("xtask".into());
    }
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
    fn the_app_tests_under_cargo_test_and_the_rest_under_nextest() {
        let strings = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            test_commands(&strings(&["kchess-core", "kchess-gpui"]), true),
            [
                strings(&[
                    "nextest",
                    "run",
                    "--locked",
                    "--no-tests=warn",
                    "-p",
                    "kchess-core"
                ]),
                strings(&["test", "--locked", "-p", "kchess-gpui"]),
            ]
        );
        assert_eq!(
            test_commands(&[], true),
            [
                strings(&[
                    "nextest",
                    "run",
                    "--locked",
                    "--no-tests=warn",
                    "--workspace",
                    "--exclude",
                    "kchess-gpui"
                ]),
                strings(&["test", "--locked", "-p", "kchess-gpui"]),
            ]
        );
        assert_eq!(
            test_commands(&strings(&["kchess-gpui"]), true),
            [strings(&["test", "--locked", "-p", "kchess-gpui"])]
        );
        assert_eq!(
            test_commands(&strings(&["xtask"]), false),
            [strings(&["test", "--locked", "-p", "xtask"])]
        );
    }

    #[test]
    fn coverage_runs_through_nextest_when_it_is_installed() {
        assert_eq!(coverage_args(true, "r.json")[..2], ["llvm-cov", "nextest"]);
        assert_eq!(coverage_args(false, "r.json")[1], "--workspace");
        assert!(coverage_args(true, "r.json").ends_with(&["--output-path", "r.json"]));
        assert!(
            coverage_args(true, "r.json")
                .windows(2)
                .any(|pair| pair == ["--failure-mode", "all"])
        );
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

    /// 2026-11-07: committing the release key (`apps/gpui/update-key.pub`, read by
    /// `include_str!`) ran no tests, and CI found four the key broke.
    /// 2026-11-07: dropping kchess-electron-legacy took the notices' only `windows-sys` row with
    /// it while the app still ships it; `check-fast` passed and CI's notices check failed.
    #[test]
    fn a_dependency_change_checks_the_notices() {
        let changed = |files: &[&str]| {
            touches_dependencies(
                &files
                    .iter()
                    .map(|file| file.to_string())
                    .collect::<Vec<_>>(),
            )
        };
        assert!(changed(&["crates/kchess-core/Cargo.toml"]));
        assert!(changed(&["Cargo.lock"]));
        assert!(changed(&["THIRD_PARTY_NOTICES.md"]));
        assert!(!changed(&[
            "crates/kchess-core/src/core.rs",
            "docs/releasing.md"
        ]));
    }

    #[test]
    fn a_file_a_package_reads_is_a_change_to_test() {
        assert!(builds_into_a_package("apps/gpui/update-key.pub"));
        assert!(builds_into_a_package(
            "crates/kchess-domain/tests/fixtures/rules-corpus.jsonl.gz"
        ));
        assert!(builds_into_a_package("Cargo.lock"));
        assert!(!builds_into_a_package("docs/releasing.md"));
        assert!(!builds_into_a_package(".github/workflows/ci.yml"));
        assert!(!builds_into_a_package("AGENTS.md"));
    }

    /// Each workflow step's shell script that runs on bash, with its line: every step of a
    /// workflow that never runs on Windows, and elsewhere those that say `shell: bash` (a Windows
    /// runner's default shell is PowerShell).
    fn bash_steps(text: &str) -> Vec<(usize, String)> {
        let windows = text.contains("windows");
        let lines: Vec<&str> = text.lines().collect();
        let indent = |line: &str| line.len() - line.trim_start().len();
        let mut found = Vec::new();
        let mut start = 0;
        while start < lines.len() {
            let line = lines[start];
            if !line.trim_start().starts_with("- ") {
                start += 1;
                continue;
            }
            // The step runs to the next item at its indentation or less.
            let item = indent(line);
            let end = (start + 1..lines.len())
                .find(|&n| {
                    let next = lines[n];
                    !next.trim().is_empty()
                        && (indent(next) < item
                            || (indent(next) == item && next.trim_start().starts_with("- ")))
                })
                .unwrap_or(lines.len());
            let step = &lines[start..end];
            let key = |name: &str| {
                step.iter().position(|line| {
                    let trimmed = line.trim_start().trim_start_matches("- ");
                    trimmed.starts_with(&format!("{name}:"))
                })
            };
            let shell = key("shell").map(|n| step[n].split(':').nth(1).unwrap_or("").trim());
            let runs_on_bash = match shell {
                Some(shell) => shell == "bash",
                None => !windows,
            };
            if let Some(run) = key("run")
                && runs_on_bash
            {
                let head = step[run]
                    .split_once("run:")
                    .map(|(_, rest)| rest.trim())
                    .unwrap_or("");
                let script = if head == "|" || head == ">" {
                    let base = indent(step[run]);
                    step[run + 1..]
                        .iter()
                        .take_while(|line| line.trim().is_empty() || indent(line) > base)
                        .map(|line| line.get(base + 2..).unwrap_or("").to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    head.to_string()
                };
                found.push((start + run + 1, script));
            }
            start += 1;
        }
        found
    }

    /// 2026-11-07: a heredoc inside `$(...)` with a backtick in it parsed in zsh but not in bash,
    /// and the TestFlight step failed on the first release that ran it. Every bash step parses.
    #[cfg(unix)]
    #[test]
    fn every_workflow_step_parses_in_bash() {
        let dir = crate::util::root().join(".github/workflows");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "yml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (line, script) in bash_steps(&text) {
                let mut bash = std::process::Command::new("bash")
                    .arg("-n")
                    .stdin(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                use std::io::Write as _;
                bash.stdin
                    .take()
                    .unwrap()
                    .write_all(script.as_bytes())
                    .unwrap();
                let out = bash.wait_with_output().unwrap();
                assert!(
                    out.status.success(),
                    "{}:{line} does not parse in bash: {}",
                    path.display(),
                    String::from_utf8_lossy(&out.stderr)
                );
                checked += 1;
            }
        }
        assert!(checked > 20, "only {checked} steps found");
    }

    #[test]
    fn bash_steps_are_found_with_their_shell() {
        let text = "jobs:\n  a:\n    runs-on: macos-latest\n    steps:\n      - name: One\n        run: |\n          echo \"$(date)\"\n          echo two\n      - run: echo three\n";
        let steps = bash_steps(text);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].1, "echo \"$(date)\"\necho two");
        assert_eq!(steps[1].1, "echo three");
        let windows = "jobs:\n  a:\n    runs-on: windows-latest\n    steps:\n      - run: Get-Item x\n      - shell: bash\n        run: echo bash\n";
        assert_eq!(bash_steps(windows), vec![(7, "echo bash".to_string())]);
    }
}
