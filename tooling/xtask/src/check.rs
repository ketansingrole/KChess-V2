//! `check`, `check-fast` and the pre-push hook.

use std::io::Read;
use std::path::Path;
use std::process::Command;

use crate::members::{self, APP};
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
    members::shipped().filter(|package| *package != APP)
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

/// The commands that run the tests of these packages, or of the whole workspace when there are
/// none: the app's under `cargo test`, the others under nextest when it is installed. The app's
/// tests are one binary, so nextest gains no parallelism there, and each test in its own process
/// starts GPUI afresh (2026-11-06, 546 tests: 60 s under `cargo test`, 74 s under nextest).
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

fn uncovered(app: bool) -> Vec<String> {
    members::uncovered()
        .filter(|package| app || *package != APP)
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
    for package in members::uncovered() {
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

/// `check [--without-app-tests]`: everything a push to main must pass. CI's check job leaves the
/// app's tests to its `gpui` job, which runs them on Linux beside it.
pub fn check(args: &[String]) -> Result {
    let app = with_app_tests(args);
    crate::sweep::build_folder(false)?;
    step("format and lints", lints)?;
    step("notices", crate::licenses::check_notices)?;
    step("workspace hack", crate::hack::check)?;
    step("fuzz targets", fuzz_targets)?;
    step("icons", crate::icons::check)?;
    step("release metadata", crate::release::verify)?;
    step("tests", || tests(app))?;
    say("`check` passed.");
    Ok(())
}

/// Whether `check` runs the app's tests: unless `--without-app-tests` says another job does.
fn with_app_tests(args: &[String]) -> bool {
    !args.iter().any(|arg| arg == "--without-app-tests")
}

/// The fuzz targets (`fuzz/`), their own workspace, which nothing else builds: they call the
/// rules, and a change to the rules once left every target but one failing to compile until the
/// weekly Fuzz run. Stable Cargo checks them; only `cargo fuzz run` needs `fuzz/`'s nightly. In
/// `target/fuzz`, so CI's build cache keeps their dependencies.
fn fuzz_targets() -> Result {
    let target = root().join("target/fuzz");
    run(
        "cargo",
        &[
            "check",
            "--manifest-path",
            "fuzz/Cargo.toml",
            "--bins",
            "--locked",
            "--target-dir",
            &target.to_string_lossy(),
        ],
    )
}

/// The member the fuzz targets build on.
const FUZZED: &str = "kchess-domain";

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
    // xtask's tests check the workflows' and actions' scripts (`every_workflow_step_parses_in_bash`)
    // and read every Rust source of the repository's own packages.
    if is_workflow(file)
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

impl Selection {
    /// Whether it runs the tests of `package`.
    fn tests(&self, package: &str) -> bool {
        match self {
            Selection::Everything => true,
            Selection::Packages(packages) => packages.iter().any(|name| name == package),
            Selection::Nothing | Selection::Lints => false,
        }
    }
}

/// Whether `file` is a workspace manifest or the lock, which add and drop dependencies.
pub fn is_manifest(file: &str) -> bool {
    file == "Cargo.lock" || file == "Cargo.toml" || file.ends_with("/Cargo.toml")
}

type Check = fn() -> Result;

/// The steps beside the format, lints and tests that `check-fast` runs for these changes: each
/// when a file it reads changed (the notices, the workspace hack), and the fuzz targets when
/// they or the rules they call did.
fn steps_for(files: &[String], selection: &Selection) -> Vec<(&'static str, Check)> {
    let any = |reads: fn(&str) -> bool| files.iter().any(|file| reads(file));
    let mut steps: Vec<(&'static str, Check)> = Vec::new();
    if any(crate::licenses::reads) {
        steps.push(("notices", crate::licenses::check_notices));
    }
    if any(crate::hack::reads) {
        steps.push(("workspace hack", crate::hack::check));
    }
    if any(|file| file.starts_with("fuzz/")) || selection.tests(FUZZED) {
        steps.push(("fuzz targets", fuzz_targets));
    }
    steps
}

/// The files changed since `base`: committed, and with `uncommitted` also those in the working
/// tree, untracked ones included.
fn changed_files(repo: &Path, base: &str, uncommitted: bool) -> Result<Vec<String>> {
    let range = if uncommitted {
        base.to_string()
    } else {
        format!("{base}..HEAD")
    };
    let git = |args: &[&str]| crate::util::git(repo, args, &[0]);
    let mut files: Vec<String> = git(&["diff", "--name-only", &range])?
        .lines()
        .map(str::to_string)
        .collect();
    if uncommitted {
        files.extend(
            git(&["ls-files", "--others", "--exclude-standard"])?
                .lines()
                .map(str::to_string),
        );
    }
    Ok(files)
}

/// `check-fast`: format, lints, the steps that read a changed file, and the tests of the packages
/// changed since main (committed or not). Never a substitute for `check` before a push to main.
pub fn fast() -> Result {
    crate::sweep::build_folder(false)?;
    let base = crate::util::merge_base(&root())?;
    let files = changed_files(&root(), &base, true)?;
    let scopes: Vec<Scope> = files
        .iter()
        .map(|file| scope_of(file, package_of(file)))
        .collect();
    let selection = select(&scopes, &workspace_packages()?);
    for (name, check) in steps_for(&files, &selection) {
        step(name, check)?;
    }
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

pub fn members() -> Result<Vec<String>> {
    let text = output(
        "cargo",
        &["metadata", "--no-deps", "--format-version", "1", "--locked"],
    )?;
    let metadata: serde_json::Value =
        serde_json::from_str(&text).map_err(|cause| cause.to_string())?;
    Ok(packages_in(&metadata).members)
}

/// The workspace's packages, from `cargo metadata` with its resolved dependencies (half a second).
fn workspace_packages() -> Result<Packages> {
    let text = output("cargo", &["metadata", "--format-version", "1", "--locked"])?;
    let metadata: serde_json::Value =
        serde_json::from_str(&text).map_err(|cause| cause.to_string())?;
    Ok(packages_in(&metadata))
}

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
    check(&[]).map_err(|cause| format!("Push to main refused: {cause}"))
}

/// A push of another branch: the mutants of the lines it sends, so a survivor stops the push here
/// rather than failing CI's `mutants` job a quarter of an hour later. Only the lines new since the
/// branch's last push, which was tested then; the app's are left out as in CI. They are tested in
/// the checkout's own mutants tree (`mutants_tree`), set to the pushed commit, so each push
/// rebuilds only what changed since the last: one at a time in place as in CI, or a big push's
/// several at once in copies of the built tree (`jobs_for`); a push of anything but
/// a clean HEAD, or without cargo-mutants, skips it: CI still runs them all.
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
    let refused = |cause| {
        format!("Push refused: {cause}\nSkip on purpose with KCHESS_SKIP_PRE_PUSH=1 git push …")
    };
    let checkout = root();
    let tree = crate::mutants_tree::prepare(&checkout, head.trim()).map_err(refused)?;
    for message in crate::sweep::mutants_tree(&tree.target(), false) {
        say(&message);
    }
    // The same lines and files as the run: the app's left out.
    let count = crate::mutants::count_since(&tree.path, "cargo", &base, &[]).map_err(refused)?;
    let cpus = std::thread::available_parallelism().map_or(1, |cpus| cpus.get());
    let jobs = jobs_for(count, cpus);
    if jobs > 1 {
        say(&format!(
            "{count} mutants: testing {jobs} at a time, each in a copy of the built tree."
        ));
    }
    let args = branch_mutants_args(&base, &checkout, jobs);
    crate::mutants::run_in(&tree.path, "cargo", &args).map_err(refused)
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

/// `cargo xtask mutants`' arguments for a branch push from `base`: CI's (the app left out), with
/// the results in `checkout`'s `mutants.out`, where the pusher looks for them.
///
/// One job (`jobs_for`) tests them one at a time, in place, in the tree kept built: for a few
/// mutants that beats copies built afresh (2026-11-10, the 4 kchess-core mutants of 9e39cc7f took
/// 92 s on the next push against 337 s four at once, each build 8–19 s against 38–254 s). More test
/// that many at once, each in cargo-mutants' own copy of the tree with its build (`--copy-target`,
/// which APFS and other copy-on-write file systems clone rather than copy), so a job rebuilds
/// the workspace's crates (at their new path) but not their dependencies.
fn branch_mutants_args(base: &str, checkout: &Path, jobs: usize) -> Vec<String> {
    let out = checkout.to_string_lossy();
    if jobs <= 1 {
        return ["--base", base, "--in-place", "--", "--output", &out]
            .map(String::from)
            .to_vec();
    }
    let jobs = jobs.to_string();
    [
        "--base",
        base,
        "--",
        "--output",
        &out,
        "--jobs",
        &jobs,
        "--copy-target",
        "true",
    ]
    .map(String::from)
    .to_vec()
}

/// From how many mutants a branch push tests several at once (`jobs_for`). 2026-11-11, the 25
/// kchess-domain mutants of 1efd5324 on a busy 10-core Mac: 294 s in place (about 12 s each),
/// 174 s five at once with copied builds (the same outcomes). The copies paid about 115 s up
/// front (28 s for the baseline, then 65–91 s for each copy's first build, which rebuilds the
/// workspace's crates at their new path) and then 13 s a mutant per job, so they win from about
/// 11 kchess-domain mutants. The core's first builds in a copy take longer (38–254 s, 2026-11-10,
/// `branch_mutants_args`'s note), which moves its break-even towards 30.
const PARALLEL_FROM: usize = 30;

/// The most mutants a branch push tests at once: each job's builds and tests take several cores
/// and a few gigabytes, and the machine is still in use.
const MAX_JOBS: usize = 6;

/// How many mutants a branch push of `count` tests at once on a machine of `cpus`: one, in place,
/// below `PARALLEL_FROM`, where a copy's first build costs more than it saves; then half the
/// CPUs, leaving the rest to the jobs' own builds and to whoever uses the machine, up to
/// `MAX_JOBS`.
fn jobs_for(count: usize, cpus: usize) -> usize {
    if count < PARALLEL_FROM {
        return 1;
    }
    (cpus / 2).clamp(1, MAX_JOBS)
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

fn is_workflow(file: &str) -> bool {
    file.starts_with(".github/workflows/") || file.starts_with(".github/actions/")
}

/// Whether `file` is a document that no build, check or CI job reads: `docs/`, the site and the
/// top-level Markdown, but the notices (`licenses`).
pub fn is_document(file: &str) -> bool {
    file.starts_with("docs/")
        || file.starts_with("site/")
        || (!file.contains('/') && file.ends_with(".md") && !crate::licenses::reads(file))
}

/// Files outside xtask's sources that packaging is made from.
const PACKAGING_FILES: [&str; 10] = [
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".cargo/config.toml",
    "apps/gpui/build.rs",
    "crates/kchess-core/src/engine/managed/shipped.rs",
    "tooling/windows-upgrade-test.ps1",
    ".github/workflows/ci.yml",
    ".github/workflows/release.yml",
    ".github/workflows/windows-upgrade.yml",
];

/// Folders packaging is made from: the app's icons and bundle files, the shipped speech helper,
/// the release crate, xtask's packaging and the repository's own actions.
const PACKAGING_DIRS: [&str; 5] = [
    "apps/gpui/build/",
    "crates/kchess-voice/",
    "crates/kchess-delta/",
    "tooling/xtask/src/package/",
    ".github/actions/",
];

/// The xtask modules packaging runs: those `package/` and `deltas` (the update patches, which
/// the Windows upgrade test applies) name (`crate::<module>`), and those they name in turn, read
/// from the sources in `repo`.
fn packaging_modules(repo: &Path) -> Result<Vec<String>> {
    let src = repo.join("tooling/xtask/src");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(src.join("package"))
        .map_err(|cause| cause.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .chain([src.join("deltas.rs")])
        .collect();
    let mut modules = vec!["deltas".to_string()];
    while let Some(file) = files.pop() {
        let text = std::fs::read_to_string(&file).map_err(|cause| cause.to_string())?;
        for (at, _) in text.match_indices("crate::") {
            let rest = &text[at + "crate::".len()..];
            let names: Vec<&str> = match rest.strip_prefix('{') {
                Some(list) => list
                    .split('}')
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .map(|name| name.split("::").next().unwrap_or_default().trim())
                    .collect(),
                None => rest
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .into_iter()
                    .collect(),
            };
            for name in names {
                let path = src.join(format!("{name}.rs"));
                if name != "package" && path.is_file() && !modules.iter().any(|m| m == name) {
                    modules.push(name.to_string());
                    files.push(path);
                }
            }
        }
    }
    modules.sort();
    Ok(modules)
}

/// Whether a change to `file` may change what packaging makes, `modules` being xtask's
/// packaging modules (`packaging_modules`).
fn is_packaging(file: &str, modules: &[String]) -> bool {
    PACKAGING_FILES.contains(&file)
        || PACKAGING_DIRS.iter().any(|dir| file.starts_with(dir))
        || file
            .strip_prefix("tooling/xtask/src/")
            .and_then(|name| name.strip_suffix(".rs"))
            .is_some_and(|name| modules.iter().any(|module| module == name))
}

/// `scope <base>`: what the commits since `base` change, for CI's `changes` job: `code=true`
/// when one changes more than documents (`is_document`), `packaging=true` when one may change a
/// package (`is_packaging`).
pub fn scope(args: &[String]) -> Result {
    let base = args
        .first()
        .ok_or("scope needs the commit to compare with.")?;
    print!("{}", scope_report(&root(), base)?);
    Ok(())
}

/// What `scope` prints for the commits of `repo` since `base`.
fn scope_report(repo: &Path, base: &str) -> Result<String> {
    let files = changed_files(repo, base, false)?;
    let modules = packaging_modules(repo)?;
    let code = files.iter().any(|file| !is_document(file));
    let packaging = files.iter().any(|file| is_packaging(file, &modules));
    Ok(format!("code={code}\npackaging={packaging}\n"))
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
    fn a_push_tests_many_mutants_at_once_and_few_in_place() {
        assert_eq!(jobs_for(0, 10), 1);
        assert_eq!(jobs_for(PARALLEL_FROM - 1, 10), 1);
        assert_eq!(jobs_for(PARALLEL_FROM, 10), 5);
        assert_eq!(jobs_for(5282, 10), 5);
        // Half the CPUs, at least one and at most `MAX_JOBS`.
        assert_eq!(jobs_for(5282, 7), 3);
        assert_eq!(jobs_for(5282, 1), 1);
        assert_eq!(jobs_for(5282, 64), MAX_JOBS);
    }

    #[test]
    fn a_push_s_mutants_land_in_the_checkout_one_job_in_place() {
        let checkout = Path::new("/work/kchess");
        assert_eq!(
            branch_mutants_args("abc", checkout, 1),
            [
                "--base",
                "abc",
                "--in-place",
                "--",
                "--output",
                "/work/kchess"
            ]
        );
        assert_eq!(
            branch_mutants_args("abc", checkout, 4),
            [
                "--base",
                "abc",
                "--",
                "--output",
                "/work/kchess",
                "--jobs",
                "4",
                "--copy-target",
                "true"
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
    fn files_belong_to_their_nearest_package() {
        assert_eq!(
            package_of("crates/kchess-domain/src/lib.rs").as_deref(),
            Some("kchess-domain")
        );
        assert_eq!(
            package_of("tooling/xtask/src/check.rs").as_deref(),
            Some("xtask")
        );
        assert_eq!(package_of("README.md"), None);
    }

    /// 2026-11-07: dropping kchess-electron-legacy took the notices' only `windows-sys` row with
    /// it while the app still ships it; `check-fast` passed and CI's notices check failed.
    /// 2026-11-11: an edit to `.github/dependabot.yml` or `.config/hakari.toml`, both read by the
    /// workspace hack's check, reported "No change since main reaches the workspace's build".
    #[test]
    fn a_change_runs_each_step_that_reads_it() {
        let steps = |files: &[&str], selection: Selection| {
            let files: Vec<String> = files.iter().map(|file| file.to_string()).collect();
            steps_for(&files, &selection)
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        };
        let both = ["notices", "workspace hack"];
        assert_eq!(
            steps(&["crates/kchess-core/Cargo.toml"], Selection::Nothing),
            both
        );
        assert_eq!(steps(&["Cargo.lock"], Selection::Nothing), both);
        assert_eq!(
            steps(&["THIRD_PARTY_NOTICES.md"], Selection::Nothing),
            ["notices"]
        );
        assert_eq!(
            steps(&[".github/dependabot.yml"], Selection::Nothing),
            ["workspace hack"]
        );
        assert_eq!(
            steps(&[".config/hakari.toml"], Selection::Nothing),
            ["workspace hack"]
        );
        assert_eq!(
            steps(&["fuzz/fuzz_targets/pgn.rs"], Selection::Nothing),
            ["fuzz targets"]
        );
        assert_eq!(
            steps(
                &["crates/kchess-domain/src/tree.rs"],
                Selection::Packages(vec!["kchess-domain".into(), "kchess-core".into()])
            ),
            ["fuzz targets"]
        );
        assert_eq!(steps(&[], Selection::Everything), ["fuzz targets"]);
        assert!(
            steps(
                &["crates/kchess-core/src/core.rs", "docs/releasing.md"],
                Selection::Packages(vec!["kchess-core".into()])
            )
            .is_empty()
        );
    }

    /// CI's `changes` job asks `scope`: documents alone run none of CI's code jobs, and each file
    /// packaging is made from runs the package jobs. A change to the code packaging runs (util.rs,
    /// the patches, kchess-delta) once skipped the jobs that test it.
    #[test]
    fn scope_tells_documents_and_packaging_apart() {
        for file in [
            "docs/releasing.md",
            "site/index.html",
            "AGENTS.md",
            "README.md",
        ] {
            assert!(is_document(file), "{file}");
            assert_eq!(scope_of(file, package_of(file)), Scope::Nothing, "{file}");
        }
        for file in [
            "THIRD_PARTY_NOTICES.md",
            "apps/gpui/WHATS_NEW.md",
            ".cargo/mutants.toml",
            "deny.toml",
        ] {
            assert!(!is_document(file), "{file}");
        }
        let modules = packaging_modules(&root()).unwrap();
        for module in ["deltas", "fairy", "pinned", "util", "voice"] {
            assert!(modules.iter().any(|m| m == module), "{module}: {modules:?}");
        }
        for file in [
            "tooling/xtask/src/package/mac.rs",
            "tooling/xtask/src/util.rs",
            "crates/kchess-delta/src/lib.rs",
            "crates/kchess-voice/src/main.rs",
            "crates/kchess-core/src/engine/managed/shipped.rs",
            "Cargo.lock",
            ".cargo/config.toml",
            ".github/actions/engines/action.yml",
            ".github/workflows/release.yml",
        ] {
            assert!(is_packaging(file, &modules), "{file}");
        }
        for file in [
            "apps/gpui/src/explorer.rs",
            "tooling/xtask/src/bench.rs",
            "docs/releasing.md",
            ".cargo/mutants.toml",
            ".github/workflows/fuzz.yml",
        ] {
            assert!(!is_packaging(file, &modules), "{file}");
        }
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
        let cases: [(&[&str], Selection); 22] = [
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
                &[".github/actions/snap/action.yml"],
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
                &["crates/kchess-domain/src/lib.rs"],
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

    #[test]
    fn check_runs_the_app_tests_unless_told_another_job_does() {
        assert!(with_app_tests(&[]));
        assert!(with_app_tests(&["--other".to_string()]));
        assert!(!with_app_tests(&["--without-app-tests".to_string()]));
    }

    /// One clippy run denies `unwrap` in every shipped program but the app's.
    #[test]
    fn the_shipped_programs_but_the_app_are_linted_for_unwrap() {
        let args = shipped_bins_args();
        let packages: Vec<&str> = args
            .windows(2)
            .filter(|pair| pair[0] == "-p")
            .map(|pair| pair[1])
            .collect();
        assert_eq!(packages, ["kchess-service", "kchess-cli", "kchess-voice"]);
        assert_eq!(args[0], "clippy");
        assert!(args.ends_with(&["-D", "clippy::unwrap_used", "-D", "clippy::expect_used"]));
    }

    /// The members outside coverage run their tests on their own, the app's only when asked.
    #[test]
    fn the_uncovered_members_test_the_app_only_when_asked() {
        assert_eq!(uncovered(true), [APP, "xtask"]);
        assert_eq!(uncovered(false), ["xtask"]);
    }

    #[test]
    fn coverage_leaves_out_the_uncovered_members() {
        let args = coverage_args(true, "summary.json");
        assert_eq!(args[..3], ["llvm-cov", "nextest", "--workspace"]);
        for package in [APP, "xtask"] {
            assert!(
                args.windows(2).any(|pair| pair == ["--exclude", package]),
                "{args:?}"
            );
        }
        assert!(
            args.ends_with(&["--output-path", "summary.json"]),
            "{args:?}"
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--failure-mode", "all"])
        );
        let plain = coverage_args(false, "r.json");
        assert_eq!(plain[..2], ["llvm-cov", "--workspace"]);
    }

    /// A scratch repository with xtask's packaging sources: `package/` names a module by path,
    /// in a list, the package itself and a type that is no module.
    fn scratch_repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("tooling/xtask/src");
        std::fs::create_dir_all(src.join("package")).unwrap();
        let write = |path: &str, text: &str| std::fs::write(src.join(path), text).unwrap();
        write(
            "package/mod.rs",
            "use crate::package::mac;\nuse crate::{util::run, Result};\nfn f() { crate::helper::go() }\n",
        );
        write("deltas.rs", "use crate::sums;\n");
        write("util.rs", "use crate::Result;\n");
        write("helper.rs", "use crate::util;\n");
        write("sums.rs", "\n");
        write("unrelated.rs", "\n");
        dir
    }

    fn git_in(dir: &Path, args: &[&str]) -> String {
        crate::util::git(dir, args, &[0]).unwrap()
    }

    fn commit(dir: &Path, message: &str) {
        git_in(dir, &["add", "-A"]);
        git_in(
            dir,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                message,
            ],
        );
    }

    #[test]
    fn packaging_modules_follow_what_packaging_names() {
        let repo = scratch_repository();
        assert_eq!(
            packaging_modules(repo.path()).unwrap(),
            ["deltas", "helper", "sums", "util"]
        );
    }

    /// The files changed since a commit, committed or not, and what `scope` says of them.
    #[test]
    fn changes_since_a_commit_are_read_from_git() {
        let repo = scratch_repository();
        let dir = repo.path();
        git_in(dir, &["init", "-q"]);
        commit(dir, "first");
        let base = git_in(dir, &["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(
            changed_files(dir, &base, false).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            scope_report(dir, &base).unwrap(),
            "code=false\npackaging=false\n"
        );

        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/a.md"), "a\n").unwrap();
        commit(dir, "docs");
        assert_eq!(changed_files(dir, &base, false).unwrap(), ["docs/a.md"]);
        assert_eq!(
            scope_report(dir, &base).unwrap(),
            "code=false\npackaging=false\n"
        );

        std::fs::write(dir.join("tooling/xtask/src/unrelated.rs"), "fn a() {}\n").unwrap();
        commit(dir, "code");
        assert_eq!(
            scope_report(dir, &base).unwrap(),
            "code=true\npackaging=false\n"
        );

        std::fs::write(dir.join("tooling/xtask/src/helper.rs"), "fn b() {}\n").unwrap();
        commit(dir, "packaging");
        assert_eq!(
            scope_report(dir, &base).unwrap(),
            "code=true\npackaging=true\n"
        );

        // Uncommitted: an edit and a new file, beside the commits.
        let head = git_in(dir, &["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(dir.join("tooling/xtask/src/sums.rs"), "fn c() {}\n").unwrap();
        std::fs::write(dir.join("new.rs"), "\n").unwrap();
        assert_eq!(
            changed_files(dir, &head, true).unwrap(),
            ["tooling/xtask/src/sums.rs", "new.rs"]
        );
        assert_eq!(
            changed_files(dir, &head, false).unwrap(),
            Vec::<String>::new()
        );
    }
}
