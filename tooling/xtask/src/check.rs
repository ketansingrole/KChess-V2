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

fn step(name: &str, work: impl FnOnce() -> Result) -> Result {
    say(&format!("Checking {name}…"));
    work().map_err(|cause| format!("{name}: {cause}"))
}

fn format() -> Result {
    run("cargo", &["fmt", "--all", "--check"])
}

/// Clippy over every target of the workspace, its warnings denied.
fn clippy_workspace() -> Result {
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
}

fn lints() -> Result {
    format()?;
    clippy_workspace()?;
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

/// The shipped binaries whose code must not unwrap or expect outside tests: every shipped
/// program's crate but the app's.
fn shipped_bins() -> impl Iterator<Item = &'static str> {
    crate::package::SHIPPED
        .into_iter()
        .filter(|package| *package != APP)
}

/// One clippy run over every shipped binary, its `unwrap`/`expect` denied as the libraries' are.
fn shipped_bins_args() -> Vec<&'static str> {
    let mut args = vec!["clippy"];
    for bin in shipped_bins() {
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

/// The packages outside the coverage totals: the app's views are checked by its end-to-end
/// suite, and this tooling by running it. Their tests still run.
const NOT_COVERED: [&str; 2] = [APP, "xtask"];

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

/// The packages whose tests run outside the coverage gate: the app's unless `app` is false.
fn uncovered(app: bool) -> Vec<String> {
    NOT_COVERED
        .iter()
        .filter(|package| app || **package != APP)
        .map(|package| package.to_string())
        .collect()
}

/// The workspace's tests: under the coverage gate when cargo-llvm-cov is installed (CI installs
/// it), else plainly. No doctests: the workspace has none (`boundaries` refuses a doc-comment
/// code block), and `cargo test --doc` compiled every library again to run nothing. Without the
/// app's (`app` false) when another job runs them.
fn tests(app: bool) -> Result {
    if available("cargo", &["llvm-cov", "--version"]) {
        coverage()?;
        run_tests(&uncovered(app))?;
    } else {
        say(
            "cargo-llvm-cov is not installed: running the tests without the coverage gate (CI runs it).",
        );
        if app {
            run_tests(&[])?;
        } else {
            let others: Vec<String> = workspace_packages()?
                .members
                .into_iter()
                .filter(|name| name != APP)
                .collect();
            run_tests(&others)?;
        }
    }
    Ok(())
}

/// `cargo llvm-cov`'s arguments: through nextest when it is installed.
fn coverage_args(nextest: bool, report: &str) -> Vec<&str> {
    let mut args = vec!["llvm-cov"];
    if nextest {
        args.push("nextest");
    }
    args.push("--workspace");
    for package in NOT_COVERED {
        args.extend(["--exclude", package]);
    }
    args.extend([
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
    full_with(true)
}

/// `check [--without-app-tests]`: CI's check job leaves the app's tests to its `gpui` job, which
/// runs them on Linux beside it.
pub fn check(args: &[String]) -> Result {
    full_with(!without_app_tests(args))
}

/// Whether `check`'s arguments leave the app's tests out.
fn without_app_tests(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--without-app-tests")
}

fn full_with(app: bool) -> Result {
    crate::sweep::objects();
    step("format and lints", lints)?;
    step("notices", crate::licenses::check_notices)?;
    step("icons", crate::icons::check)?;
    step("release metadata", crate::release::verify)?;
    step("tests", || tests(app))?;
    say("`check` passed.");
    Ok(())
}

/// The package a changed file belongs to, by its nearest Cargo.toml: a workspace member, a
/// vendored crate (`vendor/`) or an excluded one (`fuzz/`).
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

/// What a change to a file asks `check-fast` to run.
#[derive(Debug, PartialEq)]
enum Scope {
    /// Nothing `check-fast` runs reads it: documents, the site, the mutants' exclusions
    /// (`.cargo/mutants.toml`, read only by `cargo mutants`), cargo-deny's and the hooks' settings.
    Nothing,
    /// Only the format and lints read it.
    Lints,
    /// Every package's build or tests read it.
    Everything,
    /// The tests of these packages and of every workspace member that depends on one of them.
    Packages(Vec<String>),
}

/// Files outside any package that every build or test run reads: the workspace's manifest and
/// lock, the toolchain, nextest's profiles, and Cargo's runner on macOS (`.cargo/config.toml`
/// starts every test binary through it). Anything else under `.cargo/` is Cargo's configuration
/// too, which can change every build's flags.
const EVERYTHING: [&str; 5] = [
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".config/nextest.toml",
    "tooling/macos-dev-runner.sh",
];

/// cargo-mutants' settings: under `.cargo/`, but no build or test reads them.
const MUTANTS_SETTINGS: &str = ".cargo/mutants.toml";

/// rustfmt's and clippy's settings: the format and lints read them, no test does.
const LINT_SETTINGS: [&str; 4] = [
    "rustfmt.toml",
    ".rustfmt.toml",
    "clippy.toml",
    ".clippy.toml",
];

/// The directories whose Rust sources xtask's tests read, wherever their package
/// (`boundaries.rs`: the frontends' boundaries, and no doctest anywhere).
const READ_BY_XTASK: [&str; 3] = ["apps/", "crates/", "tooling/"];

/// What a change to `file` asks `check-fast` to run, `package` being the package it belongs to
/// (`package_of`). Anything inside a package counts, as `include_str!`, `build.rs` or a test may
/// read it (a release key once skipped every test this way).
fn scope_of(file: &str, package: Option<String>) -> Scope {
    if EVERYTHING.contains(&file) || (file.starts_with(".cargo/") && file != MUTANTS_SETTINGS) {
        return Scope::Everything;
    }
    if LINT_SETTINGS.contains(&file) {
        return Scope::Lints;
    }
    let mut packages: Vec<String> = package.into_iter().collect();
    // xtask's tests check the workflows' scripts (`every_workflow_step_parses_in_bash`) and read
    // every Rust source of the repository's own packages.
    if file.starts_with(".github/workflows/")
        || (file.ends_with(".rs") && READ_BY_XTASK.iter().any(|dir| file.starts_with(dir)))
    {
        packages.push("xtask".into());
    }
    if packages.is_empty() {
        Scope::Nothing
    } else {
        Scope::Packages(packages)
    }
}

/// What `check-fast` runs for these changes.
#[derive(Debug, PartialEq)]
enum Selection {
    /// Nothing: no change reaches the workspace's build.
    Nothing,
    /// The format and lints.
    Lints,
    /// The format, lints and the tests of these workspace members.
    Packages(Vec<String>),
    /// The format, lints and every test.
    Everything,
}

/// The selection for these scopes, from the workspace's packages: a change to a vendored crate
/// tests the members that depend on it, and one to a crate outside the workspace (`fuzz/`) tests
/// nothing.
fn select(scopes: &[Scope], packages: &Packages) -> Selection {
    if scopes.contains(&Scope::Everything) {
        return Selection::Everything;
    }
    let changed: Vec<String> = scopes
        .iter()
        .filter_map(|scope| match scope {
            Scope::Packages(names) => Some(names.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let members: Vec<String> = with_dependents(&changed, &packages.uses)
        .into_iter()
        .filter(|name| packages.members.contains(name))
        .collect();
    if !members.is_empty() {
        Selection::Packages(members)
    } else if scopes.contains(&Scope::Lints) {
        Selection::Lints
    } else {
        Selection::Nothing
    }
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
    crate::sweep::objects();
    let base = crate::util::merge_base(&root())?;
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
    let scopes: Vec<Scope> = files
        .iter()
        .map(|file| scope_of(file, package_of(file)))
        .collect();
    let selection = select(&scopes, &workspace_packages()?);
    if selection == Selection::Nothing {
        say("No change since main reaches the workspace's build.");
        return Ok(());
    }
    step("format", format)?;
    step("lints", clippy_workspace)?;
    match selection {
        Selection::Everything => step("tests", || run_tests(&[])),
        Selection::Packages(packages) => step(&format!("tests of {}", packages.join(", ")), || {
            run_tests(&packages)
        }),
        Selection::Nothing | Selection::Lints => Ok(()),
    }
}

/// The workspace's members, and every package of its build (members, vendored crates and
/// dependencies) with the packages it uses (normal, dev or build, on any platform).
#[derive(Debug, Default)]
struct Packages {
    members: Vec<String>,
    uses: Vec<(String, Vec<String>)>,
}

/// The workspace's packages, from `cargo metadata` with its resolved dependencies (half a second).
fn workspace_packages() -> Result<Packages> {
    let text = output("cargo", &["metadata", "--format-version", "1", "--locked"])?;
    let metadata: serde_json::Value =
        serde_json::from_str(&text).map_err(|cause| cause.to_string())?;
    Ok(packages_in(&metadata))
}

/// The packages in `cargo metadata`'s answer.
fn packages_in(metadata: &serde_json::Value) -> Packages {
    let names: std::collections::HashMap<&str, &str> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|package| Some((package["id"].as_str()?, package["name"].as_str()?)))
        .collect();
    let name = |id: &serde_json::Value| id.as_str().and_then(|id| names.get(id)).copied();
    let members = metadata["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(name)
        .map(str::to_string)
        .collect();
    let uses = metadata["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| {
            let uses = node["deps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|dependency| name(&dependency["pkg"]))
                .map(str::to_string)
                .collect();
            Some((name(&node["id"])?.to_string(), uses))
        })
        .collect();
    Packages { members, uses }
}

/// The changed packages and every package that depends on one of them, directly or not: a change
/// to the core runs the app's tests as well as its own.
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
    packages.dedup();
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

/// The pre-push hook: pushes to main must pass the full `check` first; other branches the
/// mutants of their new lines (`branch_mutants`). Skip on purpose with `KCHESS_SKIP_PRE_PUSH=1`.
pub fn pre_push() -> Result {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|cause| cause.to_string())?;
    let pushes = main_pushes(&input);
    if pushes.is_empty() {
        return branch_mutants(&input);
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

/// A push of another branch: the mutants of the lines it sends, so a survivor stops the push here
/// rather than failing CI's `mutants` job a quarter of an hour later. Only the lines new since the
/// branch's last push, which was tested then; the app's are left out as in CI. It tests the working
/// tree, so a push of anything but a clean HEAD, or without cargo-mutants, skips it: CI still runs
/// them all.
fn branch_mutants(input: &str) -> Result {
    let head = output("git", &["rev-parse", "HEAD"])?;
    let Some(remote) = head_push(input, head.trim()) else {
        return Ok(());
    };
    if std::env::var("KCHESS_SKIP_PRE_PUSH").as_deref() == Ok("1") {
        say("KCHESS_SKIP_PRE_PUSH=1: pushing without testing the changed lines' mutants.");
        return Ok(());
    }
    let status = output(
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
    )?;
    if !status.trim().is_empty() {
        say("Uncommitted or untracked files: not testing mutants before the push (CI does).");
        return Ok(());
    }
    if !available("cargo", &["mutants", "--version"]) {
        say(
            "cargo-mutants is not installed (`cargo install cargo-mutants`): CI tests the mutants.",
        );
        return Ok(());
    }
    let is_ancestor = |remote: &str| {
        Command::new("git")
            .args(["merge-base", "--is-ancestor", remote, head.trim()])
            .status()
            .is_ok_and(|status| status.success())
    };
    let base = match mutants_base(&remote, is_ancestor) {
        Some(base) => base.to_string(),
        None => crate::util::merge_base(&root())?,
    };
    crate::mutants::run(&branch_mutants_args(&base)).map_err(|cause| {
        format!("Push refused: {cause}\nSkip on purpose with KCHESS_SKIP_PRE_PUSH=1 git push …")
    })
}

/// The remote's current commit for the pushed branch whose new commit is `head`, from the hook's
/// standard input; none when no such branch is pushed (a delete, a tag, another commit).
fn head_push(input: &str, head: &str) -> Option<String> {
    input.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [_, local_sha, remote_ref, remote_sha] = fields.as_slice() else {
            return None;
        };
        (*local_sha == head && remote_ref.starts_with("refs/heads/"))
            .then(|| (*remote_sha).to_string())
    })
}

/// What the push's mutants are measured from: the branch's previous push, when the remote has one
/// and this push builds on it (`is_ancestor`), since that push tested the lines before it; else
/// none, for where the branch left main.
fn mutants_base(remote: &str, is_ancestor: impl Fn(&str) -> bool) -> Option<&str> {
    (!remote.bytes().all(|b| b == b'0') && is_ancestor(remote)).then_some(remote)
}

/// `cargo xtask mutants`' arguments for a branch push from `base`: CI's, on one machine.
fn branch_mutants_args(base: &str) -> Vec<String> {
    ["--base", base, "--", "--exclude", "apps/gpui/**", "-j", "4"]
        .map(String::from)
        .to_vec()
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
    fn ci_can_leave_the_app_tests_to_another_job() {
        assert!(without_app_tests(&["--without-app-tests".to_string()]));
        assert!(!without_app_tests(&[]));
        assert_eq!(uncovered(true), [APP, "xtask"]);
        assert_eq!(uncovered(false), ["xtask"]);
    }

    #[test]
    fn a_branch_push_is_found_by_its_head() {
        let input = "refs/heads/topic abc123 refs/heads/topic 999\n\
                     refs/tags/v1 abc123 refs/tags/v1 000\n\
                     refs/heads/other def456 refs/heads/other 888\n";
        assert_eq!(head_push(input, "abc123").as_deref(), Some("999"));
        assert_eq!(head_push(input, "def456").as_deref(), Some("888"));
        assert_eq!(head_push(input, "fff"), None);
        assert_eq!(
            head_push("refs/tags/v1 abc123 refs/tags/v1 000\n", "abc123"),
            None
        );
        assert_eq!(head_push("refs/heads/topic abc123\n", "abc123"), None);
    }

    #[test]
    fn a_branch_push_tests_only_what_its_last_push_did_not() {
        assert_eq!(mutants_base("999", |_| true), Some("999"));
        // A rewritten branch: its previous push is not under this one.
        assert_eq!(mutants_base("999", |_| false), None);
        // A new branch: nothing pushed yet, measured from main.
        assert_eq!(
            mutants_base("0000", |_| panic!("no commit to ask about")),
            None
        );
        assert_eq!(mutants_base("0a00", |_| true), Some("0a00"));
    }

    #[test]
    fn a_branch_push_runs_cis_mutants_without_the_app() {
        assert_eq!(
            branch_mutants_args("abc"),
            [
                "--base",
                "abc",
                "--",
                "--exclude",
                "apps/gpui/**",
                "-j",
                "4"
            ]
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
        for bin in ["kchess-voice", "kchess-service", "kchess-cli"] {
            assert!(args.windows(2).any(|pair| pair == ["-p", bin]), "{bin}");
        }
        assert!(!args.contains(&APP));
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

    /// A build like the workspace's: the app uses the vendored GPUI, which uses its Apple backend.
    fn packages() -> Packages {
        let strings = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let uses = |name: &str, list: &[&str]| (name.to_string(), strings(list));
        Packages {
            members: strings(&[
                "kchess-cli",
                "kchess-core",
                "kchess-domain",
                "kchess-gpui",
                "xtask",
            ]),
            uses: vec![
                uses("kchess-gpui", &["gpui-pre", "kchess-core", "serde"]),
                uses("gpui-pre", &["gpui-pre-apple", "serde"]),
                uses("gpui-pre-apple", &[]),
                uses("kchess-cli", &["kchess-core"]),
                uses("kchess-core", &["kchess-domain", "serde"]),
                uses("kchess-domain", &["serde"]),
                uses("serde", &[]),
                uses("xtask", &["serde"]),
            ],
        }
    }

    /// 2026-11-07: committing the release key (`apps/gpui/update-key.pub`, read by
    /// `include_str!`) ran no tests, and CI found four the key broke.
    /// 2026-11-10: a change to `.cargo/config.toml`, `rust-toolchain.toml` or
    /// `.config/nextest.toml` alone ran nothing ("No Rust changes since main."), and one to
    /// `vendor/gpui-pre` tested the vendored crate instead of the app that builds it.
    #[test]
    fn each_change_tests_what_reads_it() {
        let all = |member: &[&str]| member.iter().map(|s| s.to_string()).collect();
        let domain = all(&["kchess-cli", "kchess-core", "kchess-domain", "kchess-gpui"]);
        let cases: [(&[&str], Selection); 21] = [
            (&[".cargo/config.toml"], Selection::Everything),
            (&[".cargo/config"], Selection::Everything),
            (&["rust-toolchain.toml"], Selection::Everything),
            (&[".config/nextest.toml"], Selection::Everything),
            (&["tooling/macos-dev-runner.sh"], Selection::Everything),
            (&["Cargo.toml"], Selection::Everything),
            (&["Cargo.lock", "docs/releasing.md"], Selection::Everything),
            (&[".cargo/mutants.toml"], Selection::Nothing),
            (&["docs/releasing.md", "AGENTS.md"], Selection::Nothing),
            (&["fuzz/fuzz_targets/pgn.rs"], Selection::Nothing),
            (&["clippy.toml"], Selection::Lints),
            (&["rustfmt.toml"], Selection::Lints),
            (
                &["vendor/gpui-pre/src/window.rs"],
                Selection::Packages(all(&["kchess-gpui"])),
            ),
            (
                &["vendor/gpui-pre-apple/KCHESS-PATCH.md"],
                Selection::Packages(all(&["kchess-gpui"])),
            ),
            (
                &[".github/workflows/ci.yml"],
                Selection::Packages(all(&["xtask"])),
            ),
            (
                &["apps/gpui/update-key.pub"],
                Selection::Packages(all(&["kchess-gpui"])),
            ),
            (
                &["crates/kchess-domain/tests/fixtures/rules-corpus.jsonl.gz"],
                Selection::Packages(domain),
            ),
            (
                &["crates/kchess-domain/src/api.rs"],
                Selection::Packages(all(&[
                    "kchess-cli",
                    "kchess-core",
                    "kchess-domain",
                    "kchess-gpui",
                    "xtask",
                ])),
            ),
            (
                &["apps/cli/src/main.rs", "clippy.toml"],
                Selection::Packages(all(&["kchess-cli", "xtask"])),
            ),
            (
                &["apps/gpui/update-key.pub", "apps/gpui/build.rs"],
                Selection::Packages(all(&["kchess-gpui", "xtask"])),
            ),
            (
                &["tooling/xtask/src/check.rs"],
                Selection::Packages(all(&["xtask"])),
            ),
        ];
        for (files, expected) in cases {
            let scopes: Vec<Scope> = files
                .iter()
                .map(|file| scope_of(file, package_of(file)))
                .collect();
            assert_eq!(select(&scopes, &packages()), expected, "{files:?}");
        }
    }

    #[test]
    fn the_packages_are_read_from_cargo_metadata() {
        let metadata = serde_json::json!({
            "packages": [
                {"id": "path+file:///r/apps/gpui#1", "name": "kchess-gpui"},
                {"id": "path+file:///r/vendor/gpui-pre#0.3.11", "name": "gpui-pre"},
                {"id": "registry+https://x#serde@1", "name": "serde"},
            ],
            "workspace_members": ["path+file:///r/apps/gpui#1"],
            "resolve": {"nodes": [
                {"id": "path+file:///r/apps/gpui#1", "deps": [
                    {"name": "gpui", "pkg": "path+file:///r/vendor/gpui-pre#0.3.11"},
                ]},
                {"id": "path+file:///r/vendor/gpui-pre#0.3.11", "deps": [
                    {"name": "serde", "pkg": "registry+https://x#serde@1"},
                ]},
                {"id": "registry+https://x#serde@1", "deps": []},
            ]},
        });
        let packages = packages_in(&metadata);
        assert_eq!(packages.members, ["kchess-gpui"]);
        assert_eq!(
            packages.uses,
            [
                ("kchess-gpui".to_string(), vec!["gpui-pre".to_string()]),
                ("gpui-pre".to_string(), vec!["serde".to_string()]),
                ("serde".to_string(), vec![]),
            ]
        );
    }

    /// The real workspace: its vendored GPUI is the app's.
    #[test]
    fn a_vendored_crate_tests_the_members_that_build_it() {
        let packages = workspace_packages().unwrap();
        assert!(packages.members.contains(&APP.to_string()));
        assert!(!packages.members.contains(&"gpui-pre".to_string()));
        let scopes = [scope_of(
            "vendor/gpui-pre-macos/src/events.rs",
            package_of("vendor/gpui-pre-macos/src/events.rs"),
        )];
        assert_eq!(
            select(&scopes, &packages),
            Selection::Packages(vec![APP.to_string()])
        );
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

    /// 2026-11-09: a plain `run:` value with `: ` in it made release.yml invalid YAML, so GitHub
    /// started no run for the v2026.11.6 tag; `bash_steps` reads lines and never noticed. Every
    /// workflow parses as one YAML document.
    #[test]
    fn every_workflow_is_valid_yaml() {
        let dir = crate::util::root().join(".github/workflows");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "yml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let docs = yaml_rust2::YamlLoader::load_from_str(&text)
                .unwrap_or_else(|error| panic!("{} is not valid YAML: {error}", path.display()));
            assert_eq!(docs.len(), 1, "{} is not one YAML document", path.display());
            checked += 1;
        }
        assert!(checked > 5, "only {checked} workflows found");
    }

    /// CI packages, builds the Snap and runs the Windows upgrade only when a file matching its
    /// `PACKAGING` pattern changed. A change to the code packaging runs (util.rs, the patches,
    /// kchess-delta) once skipped the jobs that test it: every such file must match.
    #[cfg(unix)]
    #[test]
    fn packaging_changes_run_the_package_jobs() {
        let ci = std::fs::read_to_string(root().join(".github/workflows/ci.yml")).unwrap();
        let pattern = ci
            .lines()
            .find_map(|line| line.trim().strip_prefix("PACKAGING: '"))
            .and_then(|rest| rest.strip_suffix('\''))
            .expect("ci.yml has a PACKAGING pattern");
        let mut files: Vec<String> = std::fs::read_dir(root().join("tooling/xtask/src/package"))
            .unwrap()
            .flatten()
            .map(|entry| {
                format!(
                    "tooling/xtask/src/package/{}",
                    entry.file_name().to_string_lossy()
                )
            })
            .collect();
        for module in [
            "deb", "deltas", "engines", "fairy", "licenses", "sums", "util", "voice",
        ] {
            files.push(format!("tooling/xtask/src/{module}.rs"));
        }
        files.extend(
            [
                "crates/kchess-delta/src/lib.rs",
                "crates/kchess-voice/src/main.rs",
                "Cargo.lock",
                ".cargo/config.toml",
            ]
            .map(String::from),
        );
        let matches = |file: &str| {
            let mut grep = Command::new("grep")
                .args(["-qE", pattern])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            use std::io::Write as _;
            grep.stdin
                .take()
                .unwrap()
                .write_all(format!("{file}\n").as_bytes())
                .unwrap();
            grep.wait().unwrap().success()
        };
        for file in &files {
            assert!(matches(file), "{file} does not run CI's packaging jobs");
        }
        for file in [
            "apps/gpui/src/explorer.rs",
            "tooling/xtask/src/bench.rs",
            "docs/releasing.md",
            // The mutants' settings: no package is built from them.
            ".cargo/mutants.toml",
        ] {
            assert!(!matches(file), "{file} runs CI's packaging jobs");
        }
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
