//! `cargo xtask mutants`: mutation testing of the lines changed since main, so every change
//! arrives with its mutants killed. The weekly workflow (`.github/workflows/mutants.yml`) works
//! through the rest of the code. See docs/mutation-testing.md.

use std::path::Path;
use std::process::Command;

use crate::members::{APP, APP_FILES};
use crate::util::{Result, git, merge_base, root, say, succeeds, without_git_location};

/// Where the diff cargo-mutants reads is written, under the (ignored) target directory.
const DIFF: &str = "target/mutants-diff.patch";

/// The nextest profile each mutant's tests run under: it stops at the first failure.
const NEXTEST_PROFILE: &str = "mutants";

/// The base to diff against: `--base <ref>`, else where this branch left `origin/main`.
fn base_of(repo: &Path, args: &[String]) -> Result<String> {
    let own = own(args);
    match own.iter().position(|arg| arg == "--base") {
        Some(at) => own
            .get(at + 1)
            .cloned()
            .ok_or_else(|| "--base needs a commit or branch.".to_string()),
        None => merge_base(repo),
    }
}

fn own(args: &[String]) -> &[String] {
    match args.iter().position(|arg| arg == "--") {
        Some(at) => &args[..at],
        None => args,
    }
}

fn has(args: &[String], flag: &str) -> bool {
    own(args).iter().any(|arg| arg == flag)
}

fn passed_through(args: &[String]) -> &[String] {
    match args.iter().position(|arg| arg == "--") {
        Some(at) => &args[at + 1..],
        None => &[],
    }
}

/// cargo-mutants' arguments for the files a run of changed lines covers: everything but the app,
/// or with `--app` the app alone. Each of the app's mutants rebuilds the app, so a run of changed
/// lines (a pull request's, a branch push's) leaves them out, and the weekly run's `gpui` job
/// tests them (`--app`). A run of `--all` names its own files or packages.
fn files_of(args: &[String]) -> Vec<String> {
    if has(args, "--all") {
        return Vec::new();
    }
    let flag = if has(args, "--app") {
        "--file"
    } else {
        "--exclude"
    };
    vec![flag.to_string(), APP_FILES.to_string()]
}

/// cargo-mutants' arguments: the files the run covers, then those after `--`.
fn cargo_mutants_args(args: &[String]) -> Vec<String> {
    let mut out = files_of(args);
    out.extend(passed_through(args).iter().cloned());
    out
}

/// Whether a unified diff changes any Rust source, the only files cargo-mutants mutates.
fn changes_rust(diff: &str) -> bool {
    diff.lines()
        .any(|line| line.starts_with("+++ b/") && line.ends_with(".rs"))
}

/// Git colours a moved block's lines with these, and no other line: `-c` settings that override
/// whatever colours the user's configuration picks.
const MOVED_COLOURS: [&str; 12] = [
    "-c",
    "color.diff.new=green",
    "-c",
    "color.diff.old=red",
    "-c",
    "color.diff.newMoved=cyan",
    "-c",
    "color.diff.oldMoved=magenta",
    "-c",
    "color.diff.newMovedAlternative=cyan",
    "-c",
    "color.diff.oldMovedAlternative=magenta",
];
/// How a line moved to here, and a line moved away, start in the coloured diff.
const MOVED_IN: &str = "\x1b[36m+";
const MOVED_OUT: &str = "\x1b[35m-";

/// The Rust changes of `repo`'s working tree since `base`: committed, staged and unstaged alike,
/// and the new files Git does not track yet, which `git diff` leaves out. Code that only moved
/// (`without_moves`) is not a change.
fn diff_since(repo: &Path, base: &str) -> Result<String> {
    let plain = git(repo, &["diff", "--no-ext-diff", base, "--", "*.rs"], &[0])?;
    let mut coloured_args = MOVED_COLOURS.to_vec();
    coloured_args.extend([
        "diff",
        "--no-ext-diff",
        "--color=always",
        "--color-moved=blocks",
        "--color-moved-ws=allow-indentation-change",
        base,
        "--",
        "*.rs",
    ]);
    let coloured = git(repo, &coloured_args, &[0])?;
    let mut diff = without_moves(&plain, &coloured);
    let untracked = git(
        repo,
        &["ls-files", "--others", "--exclude-standard", "--", "*.rs"],
        &[0],
    )?;
    for file in untracked.lines() {
        // `--no-index` exits 1 when the files differ, which they always do here.
        let args = [
            "diff",
            "--no-ext-diff",
            "--no-index",
            "--",
            "/dev/null",
            file,
        ];
        diff.push_str(&git(repo, &args, &[0, 1])?);
    }
    Ok(diff)
}

/// `plain` without the code that only moved, as `coloured` (the same diff, coloured with
/// `MOVED_COLOURS` and `--color-moved`) marks it: a moved block's new lines become context, its
/// old lines go, and each hunk's counts follow. Git marks a block as moved when it holds at least
/// 20 letters and digits, whatever its indentation, so a split file or a function moved into a
/// module is no change, while an edited line in it, or a new one, still is. Moved code was tested
/// where it was, and its mutants that were missed there are the weekly run's.
fn without_moves(plain: &str, coloured: &str) -> String {
    let marks: Vec<&str> = coloured.lines().collect();
    let lines: Vec<&str> = plain.lines().collect();
    if marks.len() != lines.len() {
        // The two diffs disagree: keep every change rather than guess.
        return plain.to_string();
    }
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    for (line, mark) in lines.iter().zip(&marks) {
        if mark.starts_with(MOVED_OUT) {
            continue;
        }
        match line.strip_prefix('+') {
            Some(moved) if mark.starts_with(MOVED_IN) => kept.push(format!(" {moved}")),
            _ => kept.push((*line).to_string()),
        }
    }
    let mut diff = recounted(&kept).join("\n");
    if plain.ends_with('\n') {
        diff.push('\n');
    }
    diff
}

/// `lines` with each hunk's header counting the lines that follow it: the old side's context
/// and removed lines, the new side's context and added lines. A hunk that gained or lost old
/// lines (moved code turned to context, or dropped) moves the old starts of the file's later
/// hunks by as much, so the old side stays one file in order: a block moved within a file left
/// a later hunk starting inside an earlier one, and cargo-mutants refused the whole diff.
fn recounted(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut hunk: Option<Hunk> = None;
    // Old lines the rewritten hunks so far in this file gained, less those they lost.
    let mut shift: isize = 0;
    for line in lines {
        if let Some((old_start, old_count, new_start, rest)) = hunk_header(line) {
            shift += close(&mut out, hunk.take());
            hunk = Some(Hunk {
                at: out.len(),
                old_start: old_start.saturating_add_signed(shift),
                old_count,
                new_start,
                rest: rest.to_string(),
                old: 0,
                new: 0,
            });
            out.push(line.clone());
            continue;
        }
        match (hunk.as_mut(), line.as_bytes().first()) {
            (Some(hunk), Some(b' ')) => {
                hunk.old += 1;
                hunk.new += 1;
            }
            (Some(hunk), Some(b'-')) => hunk.old += 1,
            (Some(hunk), Some(b'+')) => hunk.new += 1,
            (Some(_), Some(b'\\')) => {}
            // Outside a hunk is a file's header: the next file starts afresh.
            _ => {
                close(&mut out, hunk.take());
                shift = 0;
            }
        }
        out.push(line.clone());
    }
    close(&mut out, hunk);
    out
}

/// A hunk being counted: its header's place in `out`, its old start (moved by the hunks before
/// it) and count as Git wrote them, its new start, the rest of its header, and its counts so far.
struct Hunk {
    at: usize,
    old_start: usize,
    old_count: usize,
    new_start: usize,
    rest: String,
    old: usize,
    new: usize,
}

/// Writes a counted hunk's header in place; how many old lines it gained (or, negative, lost).
fn close(out: &mut [String], hunk: Option<Hunk>) -> isize {
    let Some(Hunk {
        at,
        old_start,
        old_count,
        new_start,
        rest,
        old,
        new,
    }) = hunk
    else {
        return 0;
    };
    // A new file's hunk starts at line 0 of nothing; once it has context it starts at 1.
    let old_start = if old > 0 { old_start.max(1) } else { old_start };
    out[at] = format!("@@ -{old_start},{old} +{new_start},{new} @@{rest}");
    old as isize - old_count as isize
}

/// A hunk header's old start and count, its new start, and what follows its closing `@@`.
fn hunk_header(line: &str) -> Option<(usize, usize, usize, &str)> {
    let ranges = line.strip_prefix("@@ -")?;
    let (ranges, rest) = ranges.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let start = |range: &str| range.split(',').next()?.parse::<usize>().ok();
    // A count left out is one line.
    let count = |range: &str| match range.split_once(',') {
        Some((_, count)) => count.parse::<usize>().ok(),
        None => Some(1),
    };
    Some((start(old)?, count(old)?, start(new)?, rest))
}

/// cargo-mutants' arguments: only mutants on the diff's lines (every mutant without one), each
/// tested with the tests of its own package, under nextest when it is installed.
fn mutants_args(diff: Option<&str>, nextest: bool, extra: &[String]) -> Vec<String> {
    let mut args: Vec<String> = vec!["mutants".into()];
    if let Some(diff) = diff {
        args.extend(["--in-diff".into(), diff.into()]);
    }
    args.push("--no-shuffle".into());
    if nextest {
        args.extend(["--test-tool", "nextest"].map(String::from));
    }
    args.extend(extra.iter().cloned());
    args
}

/// Whether `cargo <args>` runs and succeeds in `repo`.
fn cargo_has(cargo: &str, repo: &Path, args: &[&str]) -> bool {
    succeeds(Command::new(cargo).args(args).current_dir(repo))
}

/// Write the Rust changes since `base` where cargo-mutants reads them (`DIFF`); whether there are any.
fn write_diff(repo: &Path, base: &str) -> Result<bool> {
    let diff = diff_since(repo, base)?;
    if !changes_rust(&diff) {
        return Ok(false);
    }
    let path = repo.join(DIFF);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|cause| format!("{}: {cause}", dir.display()))?;
    }
    std::fs::write(&path, &diff).map_err(|cause| format!("{}: {cause}", path.display()))?;
    Ok(true)
}

/// How many mutants a CI shard tests: about eight minutes of a kchess-core change's (each
/// rebuilds the core and runs its tests), less for the lighter packages.
const PER_SHARD: usize = 15;

/// The most shards a pull request's mutants take: each holds a runner, and the account runs
/// about twenty jobs at once.
const MAX_SHARDS: usize = 8;

/// How many CI shards `count` mutants need: none for none.
fn shards_for(count: usize) -> usize {
    count.div_ceil(PER_SHARD).min(MAX_SHARDS)
}

/// The GitHub step outputs of a plan for `count` mutants: the count, the shard numbers as a JSON
/// list for the job matrix, and how many there are.
fn plan_outputs(count: usize) -> String {
    let shards = shards_for(count);
    let numbers: Vec<String> = (0..shards).map(|shard| shard.to_string()).collect();
    format!(
        "count={count}\nshards=[{}]\ntotal={shards}\n",
        numbers.join(",")
    )
}

/// `mutants --plan [--base <ref>] [-- <cargo-mutants arguments>]`: how many mutants the changed
/// lines have and the CI shards they need, as GitHub step outputs on standard output. Listing them
/// builds nothing, so CI's mutants jobs start only when there is something to test, and as many
/// as it takes.
fn plan(args: &[String]) -> Result {
    let repo = root();
    let base = base_of(&repo, args)?;
    let count = if write_diff(&repo, &base)? {
        let mut list: Vec<String> = ["mutants", "--in-diff", DIFF, "--list"]
            .map(String::from)
            .to_vec();
        list.extend(cargo_mutants_args(args));
        let out = Command::new("cargo")
            .args(&list)
            .current_dir(&repo)
            .output()
            .map_err(|cause| format!("Could not run cargo mutants: {cause}"))?;
        if !out.status.success() {
            return Err(format!(
                "cargo mutants --list failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        count_listed(&String::from_utf8_lossy(&out.stdout))
    } else {
        0
    };
    say(&format!(
        "{count} mutants on the lines changed since {base}."
    ));
    print!("{}", plan_outputs(count));
    Ok(())
}

fn count_listed(list: &str) -> usize {
    list.lines().filter(|line| !line.trim().is_empty()).count()
}

/// `mutants [--plan | --warm] [--app] [--base <ref> | --all] [--in-place] [-- <cargo-mutants
/// arguments>]`.
pub fn run(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("--plan") => plan(&args[1..]),
        Some("--warm") => warm(),
        _ => run_in(&root(), "cargo", args),
    }
}

/// `--warm`: build the tests of every package a run of changed lines mutates (all but the app),
/// one package at a time, as cargo-mutants builds each mutant's package, so the features each is
/// built with are the ones a mutant of it asks for (CI's mutants cache).
fn warm() -> Result {
    for package in crate::check::members()? {
        if package == APP {
            continue;
        }
        crate::util::run(
            "cargo",
            &[
                "nextest",
                "run",
                "--no-run",
                "--locked",
                "--package",
                &package,
            ],
        )?;
    }
    Ok(())
}

/// What a run tests: the mutants of the lines changed since a base, or (`--all`) every mutant
/// of the code the arguments after `--` select, as the weekly run does.
fn scope_of(repo: &Path, args: &[String]) -> Result<Option<String>> {
    if !has(args, "--all") {
        return base_of(repo, args).map(Some);
    }
    if has(args, "--base") {
        return Err("--all tests every mutant: it takes no --base.".into());
    }
    Ok(None)
}

/// cargo-mutants' arguments and environment for a run in place (`--in-place`): each mutant is
/// built and tested in the checkout itself, one at a time, incrementally, as the dev profile
/// builds the workspace's own crates. Incrementally, a mutant's build takes a third of the time
/// (2026-11-09, 43 kchess-domain mutants: 6.0 s against 2.1 s each, the same outcomes). CI's
/// rust-cache turns incremental builds off (`CARGO_INCREMENTAL=0`), so the variable goes; it is
/// not set to 1, which sccache refuses ("incremental compilation is prohibited"), failing every
/// build where it is the wrapper. The checkout must be one nobody else is using: CI's, or the
/// pre-push hook's own tree (`mutants_tree`).
fn in_place(mutants: &mut Vec<String>, command: &mut Command) {
    mutants.push("--in-place".into());
    command.env_remove("CARGO_INCREMENTAL");
}

/// `run` in `repo`, with `cargo` as the cargo to run (a stand-in in the tests).
pub(crate) fn run_in(repo: &Path, cargo: &str, args: &[String]) -> Result {
    if !cargo_has(cargo, repo, &["mutants", "--version"]) {
        return Err("cargo-mutants is not installed: `cargo install cargo-mutants`.".into());
    }
    let scope = scope_of(repo, args)?;
    let nextest = cargo_has(cargo, repo, &["nextest", "--version"]);
    let mut mutants = match &scope {
        Some(base) => {
            if !write_diff(repo, base)? {
                say(&format!(
                    "No Rust changes since {base}: no mutants to test."
                ));
                return Ok(());
            }
            say(&format!(
                "Testing the mutants of the lines changed since {base}…"
            ));
            mutants_args(Some(DIFF), nextest, &cargo_mutants_args(args))
        }
        None => mutants_args(None, nextest, &cargo_mutants_args(args)),
    };
    let mut command = Command::new(cargo);
    // From the pre-push hook, Git's variables name the pushing checkout, not `repo`.
    without_git_location(&mut command)
        .current_dir(repo)
        // The run ends at a mutant's first failing test (`.config/nextest.toml`).
        .env("NEXTEST_PROFILE", NEXTEST_PROFILE);
    if has(args, "--in-place") {
        in_place(&mut mutants, &mut command);
    }
    let status = command
        .args(&mutants)
        .status()
        .map_err(|cause| format!("Could not run {cargo}: {cause}"))?;
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "cargo mutants failed ({status}). A mutant on a changed line survived or timed out: \
         mutants.out/missed.txt and timeout.txt name it. Write the test that tells it apart, or, \
         if the program behaves the same, exclude it in .cargo/mutants.toml with the reason \
         (docs/mutation-testing.md)."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    /// A run of changed lines leaves the app out unless `--app` asks for it alone; `--all` names
    /// its own.
    #[test]
    fn the_app_is_left_out_unless_asked_for() {
        assert_eq!(
            cargo_mutants_args(&strings(&["--base", "abc", "--", "--shard", "1/4"])),
            ["--exclude", APP_FILES, "--shard", "1/4"]
        );
        assert_eq!(
            cargo_mutants_args(&strings(&["--app", "--base", "abc"])),
            ["--file", APP_FILES]
        );
        assert_eq!(
            cargo_mutants_args(&strings(&["--all", "--", "-p", "kchess-domain"])),
            ["-p", "kchess-domain"]
        );
        // After `--`, a flag is cargo-mutants', not this command's.
        assert_eq!(
            cargo_mutants_args(&strings(&["--", "--app"])),
            ["--exclude", APP_FILES, "--app"]
        );
    }

    #[test]
    fn a_plan_takes_a_shard_per_fifteen_mutants_up_to_eight() {
        assert_eq!(shards_for(0), 0);
        assert_eq!(shards_for(1), 1);
        assert_eq!(shards_for(15), 1);
        assert_eq!(shards_for(16), 2);
        assert_eq!(shards_for(120), 8);
        assert_eq!(shards_for(665), 8);
        assert_eq!(plan_outputs(0), "count=0\nshards=[]\ntotal=0\n");
        assert_eq!(plan_outputs(23), "count=23\nshards=[0,1]\ntotal=2\n");
    }

    #[test]
    fn each_listed_line_is_a_mutant() {
        assert_eq!(
            count_listed("a.rs:1:1: replace x\nb.rs:2:2: delete !\n\n"),
            2
        );
        assert_eq!(count_listed(""), 0);
    }

    #[test]
    fn the_base_is_the_one_given() {
        let here = Path::new(".");
        assert_eq!(
            base_of(here, &strings(&["--base", "abc123"])).unwrap(),
            "abc123"
        );
        assert!(base_of(here, &strings(&["--base"])).is_err());
    }

    #[test]
    fn arguments_after_the_separator_reach_cargo_mutants() {
        let args = strings(&["--base", "main", "--", "-p", "kchess-domain", "-j", "4"]);
        assert_eq!(
            passed_through(&args),
            strings(&["-p", "kchess-domain", "-j", "4"])
        );
        assert!(passed_through(&strings(&["--base", "main"])).is_empty());
    }

    #[test]
    fn only_a_diff_of_rust_sources_has_mutants() {
        let rust = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-a\n+b\n";
        assert!(changes_rust(rust));
        let deleted = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ /dev/null\n";
        assert!(!changes_rust(deleted));
        assert!(!changes_rust(""));
        assert!(!changes_rust("+++ b/notes.md\n++++ b/x.rs\n"));
    }

    #[test]
    fn mutants_run_on_the_diff_under_nextest_when_installed() {
        let extra = strings(&["-j", "2"]);
        assert_eq!(
            mutants_args(Some("d.patch"), true, &extra),
            strings(&[
                "mutants",
                "--in-diff",
                "d.patch",
                "--no-shuffle",
                "--test-tool",
                "nextest",
                "-j",
                "2"
            ])
        );
        assert_eq!(
            mutants_args(Some("d.patch"), false, &[]),
            strings(&["mutants", "--in-diff", "d.patch", "--no-shuffle"])
        );
    }

    /// A Git repository with one committed Rust file, and its first commit.
    fn repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| git(dir.path(), args, &[0]).unwrap();
        git(&["init", "-q"]);
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        git(&["add", "a.rs"]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "a",
        ]);
        let base = git(&["rev-parse", "HEAD"]).trim().to_string();
        (dir, base)
    }

    #[test]
    fn the_diff_holds_changed_and_untracked_rust_files_only() {
        let (dir, base) = repo();
        assert_eq!(diff_since(dir.path(), &base).unwrap(), "");
        std::fs::write(dir.path().join("a.rs"), "fn a() { 1; }\n").unwrap();
        std::fs::write(dir.path().join("new.rs"), "fn new() {}\n").unwrap();
        std::fs::write(dir.path().join("notes.md"), "notes\n").unwrap();
        let diff = diff_since(dir.path(), &base).unwrap();
        assert!(diff.contains("+++ b/a.rs\n"), "{diff}");
        assert!(diff.contains("+fn a() { 1; }"), "{diff}");
        assert!(diff.contains("+++ b/new.rs\n"), "{diff}");
        assert!(diff.contains("+fn new() {}"), "{diff}");
        assert!(!diff.contains("notes"), "{diff}");
        assert!(diff_since(dir.path(), "no-such-commit").is_err());
    }

    /// `repo`'s working tree as a commit, so `diff_since` sees committed changes as CI does.
    fn commit(dir: &Path) {
        let git = |args: &[&str]| git(dir, args, &[0]).unwrap();
        git(&["add", "-A"]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "change",
        ]);
    }

    const MOVED_FN: &str = "fn moved(value: u32) -> u32 {\n    \
                            let doubled = value.saturating_mul(2);\n    \
                            doubled.saturating_add(value_offset())\n}\n";

    #[test]
    fn code_that_only_moved_is_no_change() {
        let (dir, _) = repo();
        std::fs::write(
            dir.path().join("a.rs"),
            format!("fn a() {{}}\n\n{MOVED_FN}"),
        )
        .unwrap();
        commit(dir.path());
        let base = git(dir.path(), &["rev-parse", "HEAD"], &[0])
            .unwrap()
            .trim()
            .to_string();
        // The function moves to b.rs, indented into a module, beside a new one.
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        let indented: String = MOVED_FN
            .lines()
            .map(|line| format!("    {line}\n"))
            .collect();
        std::fs::write(
            dir.path().join("b.rs"),
            format!("mod inner {{\n{indented}}}\n\nfn brand_new() -> u32 {{\n    7\n}}\n"),
        )
        .unwrap();
        commit(dir.path());
        let diff = diff_since(dir.path(), &base).unwrap();
        assert!(!diff.contains("-fn moved"), "{diff}");
        assert!(!diff.contains("+    fn moved"), "{diff}");
        assert!(
            diff.contains("\n     fn moved(value: u32) -> u32 {\n"),
            "{diff}"
        );
        assert!(diff.contains("+fn brand_new() -> u32 {"), "{diff}");
        assert!(diff.contains("+mod inner {"), "{diff}");
        // The new file's hunk counts its moved lines as context on both sides.
        assert!(diff.contains("@@ -1,4 +1,10 @@"), "{diff}");
    }

    /// Each file's hunks as (old start, old count, new start, new count), in diff order.
    fn hunks(diff: &str) -> Vec<Vec<(usize, usize, usize, usize)>> {
        let mut files = Vec::new();
        for line in diff.lines() {
            if line.starts_with("diff ") {
                files.push(Vec::new());
            }
            let Some((old_start, old_count, new_start, _)) = hunk_header(line) else {
                continue;
            };
            let new_count = line
                .split(" +")
                .nth(1)
                .and_then(|range| range.split(' ').next()?.split(',').nth(1)?.parse().ok())
                .unwrap_or(1);
            if let Some(file) = files.last_mut() {
                file.push((old_start, old_count, new_start, new_count));
            }
        }
        files
    }

    /// Regression: a function moved from the end of a file to its start became context in the
    /// first hunk (one more old line) and left the last one (one fewer), but the hunks after the
    /// first kept their old starts, so one started inside the one before it and cargo-mutants
    /// refused the diff ("hunks not in order or overlap"). Each file's hunks now stay in order.
    #[test]
    fn code_moved_within_a_file_keeps_its_hunks_in_order() {
        let (dir, _) = repo();
        let filler: String = (0..40).map(|n| format!("fn filler_{n}() {{}}\n")).collect();
        std::fs::write(
            dir.path().join("a.rs"),
            format!("fn first() {{}}\n{filler}\n{MOVED_FN}"),
        )
        .unwrap();
        commit(dir.path());
        let base = git(dir.path(), &["rev-parse", "HEAD"], &[0])
            .unwrap()
            .trim()
            .to_string();
        let edited = filler.replace("fn filler_12() {}", "fn filler_12() { edited(); }");
        std::fs::write(
            dir.path().join("a.rs"),
            format!("fn first() {{}}\n{MOVED_FN}\n{edited}"),
        )
        .unwrap();
        commit(dir.path());
        let diff = diff_since(dir.path(), &base).unwrap();
        assert!(!diff.contains("+fn moved"), "{diff}");
        assert!(!diff.contains("-fn moved"), "{diff}");
        assert!(diff.contains("+fn filler_12() { edited(); }"), "{diff}");
        assert!(
            hunks(&diff)[0].len() > 1,
            "the move and the edit are hunks apart: {diff}"
        );
        assert_hunks_in_order(&diff);
    }

    /// Regression: the old lines a file's hunks gained (moved code turned to context) moved the
    /// next file's hunks too. A function moved from b.rs to the top of a.rs now leaves b.rs's
    /// hunks where they are.
    #[test]
    fn a_move_between_files_shifts_only_its_own_file() {
        let (dir, _) = repo();
        let filler: String = (0..40).map(|n| format!("fn filler_{n}() {{}}\n")).collect();
        std::fs::write(
            dir.path().join("a.rs"),
            format!("fn first() {{}}\n{filler}"),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.rs"),
            format!("fn b() {{}}\n{filler}\n{MOVED_FN}"),
        )
        .unwrap();
        commit(dir.path());
        let base = git(dir.path(), &["rev-parse", "HEAD"], &[0])
            .unwrap()
            .trim()
            .to_string();
        std::fs::write(
            dir.path().join("a.rs"),
            format!(
                "fn first() {{}}\n{MOVED_FN}{}",
                filler.replace("fn filler_20() {}", "fn filler_20() { edited(); }")
            ),
        )
        .unwrap();
        std::fs::write(dir.path().join("b.rs"), format!("fn b() {{}}\n{filler}")).unwrap();
        commit(dir.path());
        let diff = diff_since(dir.path(), &base).unwrap();
        assert!(
            !diff.contains("+fn moved") && !diff.contains("-fn moved"),
            "{diff}"
        );
        assert_eq!(
            hunks(&diff).iter().map(Vec::len).collect::<Vec<_>>(),
            [2, 1],
            "the move and the edit in a.rs, what is left in b.rs: {diff}"
        );
        assert_hunks_in_order(&diff);
    }

    /// Each file's hunks start where the two sides agree, and follow one another in order with
    /// the same lines between them on both sides.
    fn assert_hunks_in_order(diff: &str) {
        for file in hunks(diff) {
            // Before its first hunk a file is the same on both sides.
            if let Some(&(old, old_count, new, _)) = file.first()
                && old_count > 0
            {
                assert_eq!(old, new, "{diff}");
            }
            for pair in file.windows(2) {
                let ((old, old_count, new, new_count), (next_old, _, next_new, _)) =
                    (pair[0], pair[1]);
                assert!(old + old_count <= next_old, "old sides overlap: {diff}");
                assert!(new + new_count <= next_new, "new sides overlap: {diff}");
                // Between hunks the two sides are the same lines.
                assert_eq!(
                    next_old - old - old_count,
                    next_new - new - new_count,
                    "{diff}"
                );
            }
        }
    }

    #[test]
    fn an_edit_inside_moved_code_is_a_change() {
        let (dir, _) = repo();
        std::fs::write(
            dir.path().join("a.rs"),
            format!("fn a() {{}}\n\n{MOVED_FN}"),
        )
        .unwrap();
        commit(dir.path());
        let base = git(dir.path(), &["rev-parse", "HEAD"], &[0])
            .unwrap()
            .trim()
            .to_string();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(
            dir.path().join("b.rs"),
            MOVED_FN.replace("saturating_mul(2)", "saturating_mul(3)"),
        )
        .unwrap();
        commit(dir.path());
        let diff = diff_since(dir.path(), &base).unwrap();
        assert!(
            diff.contains("+    let doubled = value.saturating_mul(3);"),
            "{diff}"
        );
    }

    #[test]
    fn a_hunk_counts_the_lines_it_keeps() {
        let plain =
            "--- a/x.rs\n+++ b/x.rs\n@@ -0,0 +1,2 @@ fn x\n+a\n+b\n\\ No newline at end of file\n";
        let coloured = "\x1b[1m--- a/x.rs\x1b[m\n\x1b[1m+++ b/x.rs\x1b[m\n\x1b[36m@@ -0,0 +1,2 @@\x1b[m fn x\n\x1b[36m+a\x1b[m\n\x1b[32m+b\x1b[m\n\\ No newline at end of file\n";
        assert_eq!(
            without_moves(plain, coloured),
            "--- a/x.rs\n+++ b/x.rs\n@@ -1,1 +1,2 @@ fn x\n a\n+b\n\\ No newline at end of file\n"
        );
        // A new file with nothing moved keeps its empty old side.
        let added = "@@ -0,0 +1,1 @@\n+a\n";
        assert_eq!(
            without_moves(added, "\x1b[36m@@ -0,0 +1,1 @@\x1b[m\n\x1b[32m+a\x1b[m\n"),
            added
        );
        // Counting goes on past a missing final newline of the old side, to the next file.
        let newline =
            "@@ -1,2 +1,2 @@\n x\n-a\n\\ No newline at end of file\n+b\ndiff --git a/y.rs b/y.rs\n";
        let marked = "\x1b[36m@@ -1,2 +1,2 @@\x1b[m\n x\n\x1b[31m-a\x1b[m\n\\ No newline at end of file\n\x1b[36m+b\x1b[m\n\x1b[1mdiff --git a/y.rs b/y.rs\x1b[m\n";
        assert_eq!(
            without_moves(newline, marked),
            "@@ -1,3 +1,2 @@\n x\n-a\n\\ No newline at end of file\n b\ndiff --git a/y.rs b/y.rs\n"
        );
        let moved_away = "@@ -3,2 +3,0 @@\n-gone\n-also\n";
        let marked = "\x1b[36m@@ -3,2 +3,0 @@\x1b[m\n\x1b[35m-gone\x1b[m\n\x1b[31m-also\x1b[m\n";
        assert_eq!(
            without_moves(moved_away, marked),
            "@@ -3,1 +3,0 @@\n-also\n"
        );
        // Diffs that disagree keep every change.
        assert_eq!(without_moves("+a\n", ""), "+a\n");
        assert_eq!(
            hunk_header("@@ -12 +13,4 @@ fn f"),
            Some((12, 1, 13, " fn f"))
        );
        assert_eq!(hunk_header("@@ -12,5 +13,4 @@"), Some((12, 5, 13, "")));
        assert_eq!(hunk_header("@@ broken"), None);
        assert_eq!(hunk_header("+@@ -1 +1 @@"), None);
    }

    /// A stand-in cargo in `dir`: it answers `--version` when `installed`, records any other
    /// call's arguments and the diff it names, and exits with `status`.
    #[cfg(unix)]
    fn fake_cargo(dir: &Path, installed: bool, status: i32) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-cargo");
        let version = if installed { 0 } else { 1 };
        let script = format!(
            "#!/bin/sh\ncase \"$2\" in --version) exit {version};; esac\n\
             echo \"$@\" > \"{calls}\"\necho \"$NEXTEST_PROFILE\" > \"{profile}\"\n\
             echo \"${{CARGO_INCREMENTAL-unset}}\" > \"{incremental}\"\n\
             cat \"$3\" > \"{seen}\" 2>/dev/null\nexit {status}\n",
            calls = dir.join("calls").display(),
            incremental = dir.join("incremental").display(),
            seen = dir.join("seen.patch").display(),
            profile = dir.join("profile").display(),
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[cfg(unix)]
    #[test]
    fn without_cargo_mutants_it_says_how_to_install_it() {
        let (dir, base) = repo();
        let tools = tempfile::tempdir().unwrap();
        let cargo = fake_cargo(tools.path(), false, 0);
        let refused = run_in(dir.path(), &cargo, &strings(&["--base", &base])).unwrap_err();
        assert!(refused.contains("cargo install cargo-mutants"), "{refused}");
        assert!(!tools.path().join("calls").exists());
    }

    #[cfg(unix)]
    #[test]
    fn without_rust_changes_no_mutants_run() {
        let (dir, base) = repo();
        std::fs::write(dir.path().join("notes.md"), "notes\n").unwrap();
        let tools = tempfile::tempdir().unwrap();
        let cargo = fake_cargo(tools.path(), true, 2);
        run_in(dir.path(), &cargo, &strings(&["--base", &base])).unwrap();
        assert!(!tools.path().join("calls").exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_changed_lines_go_to_cargo_mutants_and_its_verdict_is_the_run_s() {
        let (dir, base) = repo();
        std::fs::write(dir.path().join("new.rs"), "fn new() {}\n").unwrap();
        let tools = tempfile::tempdir().unwrap();
        let args = strings(&["--base", &base, "--", "-j", "2"]);

        let cargo = fake_cargo(tools.path(), true, 0);
        run_in(dir.path(), &cargo, &args).unwrap();
        let calls = std::fs::read_to_string(tools.path().join("calls")).unwrap();
        assert_eq!(
            calls.trim(),
            format!(
                "mutants --in-diff {DIFF} --no-shuffle --test-tool nextest --exclude {APP_FILES} -j 2"
            )
        );
        let seen = std::fs::read_to_string(tools.path().join("seen.patch")).unwrap();
        assert!(seen.contains("+fn new() {}"), "{seen}");
        // Each mutant's tests stop at their first failure.
        let profile = std::fs::read_to_string(tools.path().join("profile")).unwrap();
        assert_eq!(profile.trim(), "mutants");

        // A missed mutant (cargo-mutants exits 2) fails the run and says what to do.
        let cargo = fake_cargo(tools.path(), true, 2);
        let failed = run_in(dir.path(), &cargo, &args).unwrap_err();
        assert!(failed.contains("mutants.out/missed.txt"), "{failed}");
    }

    /// What the stand-in cargo recorded in `tools`: its arguments and `CARGO_INCREMENTAL`.
    #[cfg(unix)]
    fn recorded(tools: &Path) -> (String, String) {
        let read = |name: &str| {
            std::fs::read_to_string(tools.join(name))
                .unwrap()
                .trim()
                .to_string()
        };
        (read("calls"), read("incremental"))
    }

    #[cfg(unix)]
    #[test]
    fn in_place_each_mutant_builds_incrementally_in_the_checkout() {
        let (dir, base) = repo();
        std::fs::write(dir.path().join("new.rs"), "fn new() {}\n").unwrap();
        let tools = tempfile::tempdir().unwrap();
        let cargo = fake_cargo(tools.path(), true, 0);
        let args = strings(&["--base", &base, "--in-place", "--", "--shard", "1/2"]);
        run_in(dir.path(), &cargo, &args).unwrap();
        let (calls, incremental) = recorded(tools.path());
        assert_eq!(
            calls,
            format!(
                "mutants --in-diff {DIFF} --no-shuffle --test-tool nextest --exclude {APP_FILES} --shard 1/2 --in-place"
            )
        );
        assert_eq!(incremental, "unset");

        // Without it, the run is in cargo-mutants' copies, built as the caller's environment says.
        run_in(dir.path(), &cargo, &strings(&["--base", &base])).unwrap();
        let (calls, incremental) = recorded(tools.path());
        assert!(!calls.contains("--in-place"), "{calls}");
        let inherited = std::env::var("CARGO_INCREMENTAL").unwrap_or_else(|_| "unset".into());
        assert_eq!(incremental, inherited);
    }

    #[cfg(unix)]
    #[test]
    fn all_tests_every_mutant_the_arguments_select_without_a_diff() {
        // No `origin/main` here: a run that looked for a base would fail.
        let (dir, base) = repo();
        let tools = tempfile::tempdir().unwrap();
        let cargo = fake_cargo(tools.path(), true, 0);
        let args = strings(&["--all", "--in-place", "--", "-p", "kchess-domain"]);
        run_in(dir.path(), &cargo, &args).unwrap();
        let (calls, incremental) = recorded(tools.path());
        assert_eq!(
            calls,
            "mutants --no-shuffle --test-tool nextest -p kchess-domain --in-place"
        );
        assert_eq!(incremental, "unset");
        assert!(!dir.path().join(DIFF).exists());

        let both = strings(&["--all", "--base", &base]);
        let refused = run_in(dir.path(), &cargo, &both).unwrap_err();
        assert!(refused.contains("takes no --base"), "{refused}");
    }

    #[test]
    fn only_arguments_before_the_separator_are_the_command_s_own() {
        let args = strings(&["--in-place", "--", "--all", "--base", "x"]);
        assert_eq!(own(&args), strings(&["--in-place"]));
        assert!(has(&args, "--in-place"));
        assert!(!has(&args, "--all"));
        assert_eq!(own(&strings(&["--all"])), strings(&["--all"]));
        // A `--base` meant for cargo-mutants is not this command's base.
        let (dir, _) = repo();
        assert!(base_of(dir.path(), &strings(&["--", "--base", "x"])).is_err());
    }

    /// What `command` does with `variable`: `Some(None)` when it removes it.
    fn env_of(command: &Command, variable: &str) -> Option<Option<String>> {
        command
            .get_envs()
            .find(|(key, _)| *key == variable)
            .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
    }

    #[test]
    fn in_place_lifts_ci_s_incremental_off_without_forcing_it_on() {
        let mut command = Command::new("cargo");
        command.env("CARGO_INCREMENTAL", "0");
        let mut args = strings(&["mutants"]);
        in_place(&mut args, &mut command);
        assert_eq!(args, strings(&["mutants", "--in-place"]));
        assert_eq!(env_of(&command, "CARGO_INCREMENTAL"), Some(None));
    }

    #[test]
    fn cargo_mutants_finds_its_repository_from_its_own_folder() {
        let mut command = Command::new("cargo");
        command.env("GIT_DIR", "/elsewhere/.git");
        without_git_location(&mut command);
        assert_eq!(env_of(&command, "GIT_DIR"), Some(None));
        assert_eq!(env_of(&command, "GIT_INDEX_FILE"), Some(None));
    }
}
