//! `cargo xtask mutants`: mutation testing of the lines changed since main, so every change
//! arrives with its mutants killed. The weekly workflow (`.github/workflows/mutants.yml`) works
//! through the rest of the code. See docs/mutation-testing.md.

use std::path::Path;
use std::process::Command;

use crate::util::{Result, root, say};

/// Where the diff cargo-mutants reads is written, under the (ignored) target directory.
const DIFF: &str = "target/mutants-diff.patch";

/// The base to diff against: `--base <ref>`, else where this branch left `origin/main`.
fn base_of(repo: &Path, args: &[String]) -> Result<String> {
    match args.iter().position(|arg| arg == "--base") {
        Some(at) => args
            .get(at + 1)
            .cloned()
            .ok_or_else(|| "--base needs a commit or branch.".to_string()),
        None => git(repo, &["merge-base", "HEAD", "origin/main"], &[0])
            .map(|text| text.trim().to_string()),
    }
}

/// The arguments after `--`, passed to cargo-mutants as they are.
fn passed_through(args: &[String]) -> &[String] {
    match args.iter().position(|arg| arg == "--") {
        Some(at) => &args[at + 1..],
        None => &[],
    }
}

/// Whether a unified diff changes any Rust source, the only files cargo-mutants mutates.
fn changes_rust(diff: &str) -> bool {
    diff.lines()
        .any(|line| line.starts_with("+++ b/") && line.ends_with(".rs"))
}

/// Git's output in `repo`, when it exits with one of the `accepted` statuses.
fn git(repo: &Path, args: &[&str], accepted: &[i32]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|cause| format!("Could not run git: {cause}"))?;
    match out.status.code() {
        Some(code) if accepted.contains(&code) => {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        }
        _ => Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// The Rust changes of `repo`'s working tree since `base`: committed, staged and unstaged alike,
/// and the new files Git does not track yet, which `git diff` leaves out.
fn diff_since(repo: &Path, base: &str) -> Result<String> {
    let mut diff = git(repo, &["diff", "--no-ext-diff", base, "--", "*.rs"], &[0])?;
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

/// cargo-mutants' arguments: only mutants on the diff's lines, each tested with the tests of its
/// own package, under nextest when it is installed.
fn mutants_args(diff: &str, nextest: bool, extra: &[String]) -> Vec<String> {
    let mut args: Vec<String> = ["mutants", "--in-diff", diff, "--no-shuffle"]
        .map(String::from)
        .to_vec();
    if nextest {
        args.extend(["--test-tool", "nextest"].map(String::from));
    }
    args.extend(extra.iter().cloned());
    args
}

/// Whether `cargo <args>` runs and succeeds in `repo`, quietly.
fn cargo_has(cargo: &str, repo: &Path, args: &[&str]) -> bool {
    Command::new(cargo)
        .args(args)
        .current_dir(repo)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `mutants [--base <ref>] [-- <cargo-mutants arguments>]`.
pub fn run(args: &[String]) -> Result {
    run_in(&root(), "cargo", args)
}

/// `mutants` in `repo`, with `cargo` as the cargo to run.
fn run_in(repo: &Path, cargo: &str, args: &[String]) -> Result {
    if !cargo_has(cargo, repo, &["mutants", "--version"]) {
        return Err("cargo-mutants is not installed: `cargo install cargo-mutants`.".into());
    }
    let base = base_of(repo, args)?;
    let diff = diff_since(repo, &base)?;
    if !changes_rust(&diff) {
        say(&format!(
            "No Rust changes since {base}: no mutants to test."
        ));
        return Ok(());
    }
    let path = repo.join(DIFF);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|cause| format!("{}: {cause}", dir.display()))?;
    }
    std::fs::write(&path, &diff).map_err(|cause| format!("{}: {cause}", path.display()))?;
    let nextest = cargo_has(cargo, repo, &["nextest", "--version"]);
    let mutants = mutants_args(DIFF, nextest, passed_through(args));
    say(&format!(
        "Testing the mutants of the lines changed since {base}…"
    ));
    let status = Command::new(cargo)
        .args(&mutants)
        .current_dir(repo)
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
            mutants_args("d.patch", true, &extra),
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
            mutants_args("d.patch", false, &[]),
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

    /// A stand-in cargo in `dir`: it answers `--version` when `installed`, records any other
    /// call's arguments and the diff it names, and exits with `status`.
    #[cfg(unix)]
    fn fake_cargo(dir: &Path, installed: bool, status: i32) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-cargo");
        let version = if installed { 0 } else { 1 };
        let script = format!(
            "#!/bin/sh\ncase \"$2\" in --version) exit {version};; esac\n\
             echo \"$@\" > \"{calls}\"\ncat \"$3\" > \"{seen}\"\nexit {status}\n",
            calls = dir.join("calls").display(),
            seen = dir.join("seen.patch").display(),
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
            format!("mutants --in-diff {DIFF} --no-shuffle --test-tool nextest -j 2")
        );
        let seen = std::fs::read_to_string(tools.path().join("seen.patch")).unwrap();
        assert!(seen.contains("+fn new() {}"), "{seen}");

        // A missed mutant (cargo-mutants exits 2) fails the run and says what to do.
        let cargo = fake_cargo(tools.path(), true, 2);
        let failed = run_in(dir.path(), &cargo, &args).unwrap_err();
        assert!(failed.contains("mutants.out/missed.txt"), "{failed}");
    }
}
