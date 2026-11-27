//! Running commands and reading the repository.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kchess_delta::REPOSITORY;

pub type Result<T = ()> = std::result::Result<T, String>;

/// The repository's root (this crate lives in `tooling/xtask`).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

/// The repository the release tooling works on: `GH_REPO` (CI sets it to the one it runs in, so
/// `gh` and these agree), else KChess's.
pub fn repository() -> String {
    repository_from(std::env::var("GH_REPO").ok())
}

fn repository_from(gh_repo: Option<String>) -> String {
    gh_repo
        .filter(|repo| !repo.is_empty())
        .unwrap_or_else(|| REPOSITORY.to_string())
}

pub fn say(message: &str) {
    eprintln!("[xtask] {message}");
}

/// When the file or folder at `path` last changed, if it is there.
pub fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path)
        .and_then(|file| file.modified())
        .ok()
}

/// The folders right inside `folder`; none when it cannot be read.
pub fn subfolders(folder: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// Runs `program` in the repository, its output going to this terminal and nothing for its
/// input: a tool that asks a question (cargo-llvm-cov offering to install llvm-tools) then fails
/// at once rather than wait for an answer, as a local `check` once did for hours.
pub fn run(program: &str, args: &[&str]) -> Result {
    run_with(Command::new(program).args(args))
}

pub fn run_with(command: &mut Command) -> Result {
    run_shown(command, &[])
}

/// Runs `command` like `run_with`, letting it print nothing for `quiet` (or the longer limit
/// `KCHESS_QUIET_MINUTES` sets): for a step known to be quieter than any other.
pub fn run_quiet_for(command: &mut Command, quiet: Duration) -> Result {
    run_watched(command, &[], quiet_limit().max(quiet))
}

/// Runs `program` with `args` like `run`, but never shows the `secrets` among them: a failure
/// message (printed and kept in build logs) names them `<redacted>`.
// Only notarization (macOS) passes secrets; its test runs on any Unix.
#[cfg(any(target_os = "macos", all(test, unix)))]
pub fn run_redacted(program: &str, args: &[&str], secrets: &[&str]) -> Result {
    run_shown(Command::new(program).args(args), secrets)
}

/// How long a command may print nothing before it is taken for hung and stopped: longer than the
/// quietest step xtask runs but the release build's (`RELEASE_BUILD_QUIET`), far shorter than the
/// hours a stuck one cost. `KCHESS_QUIET_MINUTES` sets another limit for a step known to be slower.
const QUIET: Duration = Duration::from_secs(20 * 60);

/// The release build's: its link optimises the whole program as one (fat LTO, one codegen unit),
/// printing nothing until it ends. On CI's macOS runner that took 8 minutes for 2026.11.9's
/// package (2026-11-22) and 20 for 2026.11.10's five days later, when the second run passed the
/// 20-minute limit and was stopped. The job's own `timeout-minutes` still ends a real hang.
pub const RELEASE_BUILD_QUIET: Duration = Duration::from_secs(45 * 60);

fn quiet_limit() -> Duration {
    std::env::var("KCHESS_QUIET_MINUTES")
        .ok()
        .and_then(|minutes| minutes.parse::<u64>().ok())
        .map_or(QUIET, |minutes| Duration::from_secs(minutes * 60))
}

fn run_shown(command: &mut Command, secrets: &[&str]) -> Result {
    run_watched(command, secrets, quiet_limit())
}

/// Runs `command` like `run_shown`, its output passed through as it comes, and stops it once it
/// has printed nothing for `quiet`: a tool that hangs without a word (a download from a server
/// that stopped sending, a process waiting on another) fails with its name instead of holding
/// the run for hours.
fn run_watched(command: &mut Command, secrets: &[&str], quiet: Duration) -> Result {
    let mut shown = format!("{command:?}");
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        shown = shown.replace(secret, "<redacted>");
    }
    // Piped, cargo and its tools would print without colour; keep it when this terminal shows it.
    if std::io::stderr().is_terminal() && std::env::var_os("CARGO_TERM_COLOR").is_none() {
        command.env("CARGO_TERM_COLOR", "always");
    }
    let mut child = command
        .current_dir(root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|cause| format!("Could not run {shown}: {cause}"))?;
    let started = Instant::now();
    let last = Arc::new(AtomicU64::new(0));
    if let Some(out) = child.stdout.take() {
        pass_through(out, std::io::stdout(), started, Arc::clone(&last));
    }
    if let Some(err) = child.stderr.take() {
        pass_through(err, std::io::stderr(), started, Arc::clone(&last));
    }
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|cause| format!("Could not wait for {shown}: {cause}"))?
        {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("{shown} failed ({status})."))
            };
        }
        let silent = started
            .elapsed()
            .saturating_sub(Duration::from_millis(last.load(Ordering::Relaxed)));
        if silent >= quiet {
            // Only the command itself: what it started, it is left to stop as its parent goes.
            child.kill().ok();
            child.wait().ok();
            return Err(format!(
                "{shown} printed nothing for {} s and was stopped as hung.",
                silent.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(100).min(quiet));
    }
}

/// Copies `from` to `to` as it arrives, noting in `last` when (since `started`) it last did.
fn pass_through(
    mut from: impl Read + Send + 'static,
    mut to: impl Write + Send + 'static,
    started: Instant,
    last: Arc<AtomicU64>,
) {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        while let Ok(read) = from.read(&mut buffer) {
            if read == 0 {
                break;
            }
            last.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
            if to
                .write_all(&buffer[..read])
                .and_then(|()| to.flush())
                .is_err()
            {
                break;
            }
        }
    });
}

/// Runs `program` and answers its standard output.
pub fn output(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(root())
        .output()
        .map_err(|cause| format!("Could not run {program}: {cause}"))?;
    if !out.status.success() {
        return Err(format!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs()) as i64
}

/// The UTC calendar date of `seconds` since 1970: (year, month, day), by Howard Hinnant's
/// civil_from_days.
pub fn civil_date(seconds: i64) -> (i64, u32, u32) {
    let z = seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

/// `seconds` since 1970 as an ISO 8601 UTC timestamp with milliseconds.
pub fn iso_timestamp(seconds: i64) -> String {
    let (year, month, day) = civil_date(seconds);
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Whether `program <args>` runs and succeeds, its output hidden: whether a tool is installed.
pub fn available(program: &str, args: &[&str]) -> bool {
    succeeds(Command::new(program).args(args))
}

/// Whether `command` runs and succeeds, its output hidden.
pub fn succeeds(command: &mut Command) -> bool {
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The variables that tell Git which repository to use. A Git hook sets them for its own
/// repository: from a worktree, `GIT_DIR` is the absolute path of the shared one, so a test's
/// `git init` in a temporary folder, run by the pre-push hook, re-initialized the real repository
/// as bare (2026-11-10).
const GIT_LOCATION: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];

/// `command` without the variables a hook's Git set (`GIT_LOCATION`), so it, and any Git it runs,
/// finds the repository from its own folder.
pub fn without_git_location(command: &mut Command) -> &mut Command {
    for variable in GIT_LOCATION {
        command.env_remove(variable);
    }
    command
}

/// Git run in `repo`, finding the repository from there whatever its caller's Git set.
fn git_in(repo: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(repo);
    without_git_location(&mut command);
    command
}

/// Git's output in `repo`, when it exits with one of the `accepted` statuses.
pub fn git(repo: &Path, args: &[&str], accepted: &[i32]) -> Result<String> {
    let out = git_in(repo)
        .args(args)
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

/// Where `repo`'s HEAD left `origin/main`: what "changed since main" is measured from. Without
/// `origin/main` there is no telling, which is refused rather than taken as "nothing changed".
pub fn merge_base(repo: &Path) -> Result<String> {
    git(repo, &["merge-base", "HEAD", "origin/main"], &[0])
        .map(|text| text.trim().to_string())
        .map_err(|cause| {
            format!(
                "Cannot tell what changed since main ({cause}). Fetch it with \
                 `git fetch origin main` and try again."
            )
        })
}

/// The workspace's release version (`[workspace.package] version` in Cargo.toml).
pub fn version() -> Result<String> {
    let text = std::fs::read_to_string(root().join("Cargo.toml"))
        .map_err(|cause| format!("Cannot read Cargo.toml: {cause}"))?;
    workspace_version(&text).ok_or_else(|| "Cargo.toml has no workspace version.".into())
}

pub fn workspace_version(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
            continue;
        }
        if in_package
            && let Some(value) = line.strip_prefix("version")
            && let Some(value) = value.trim_start().strip_prefix('=')
        {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Downloads `url` to `path` with curl (on every CI image and developer machine).
pub fn download(url: &str, path: &Path) -> Result {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|cause| cause.to_string())?;
    }
    let target = path.to_string_lossy().to_string();
    run_with(Command::new("curl").args(curl_args(url, &target, 60)))
}

/// curl's arguments for `download`: it gives up on a server that does not answer in 30 s or
/// sends under 1 KB/s for `stall_seconds`, which a slow but working connection never does, rather
/// than wait on it for ever. Each try is retried up to three times.
fn curl_args(url: &str, target: &str, stall_seconds: u32) -> Vec<String> {
    [
        "-fsSL",
        "--retry",
        "3",
        "--connect-timeout",
        "30",
        "--speed-limit",
        "1024",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .chain([
        "--speed-time".to_string(),
        stall_seconds.to_string(),
        "-o".to_string(),
        target.to_string(),
        url.to_string(),
    ])
    .collect()
}

/// Makes `path` hold the file pinned by `sha256`: a cached copy that matches is kept, anything
/// else is fetched from `url` with `fetch` (`download`) and checked. A fetched file that does not
/// match is removed and refused, so a tampered or changed upstream file never gets used.
pub fn pinned(
    what: &str,
    url: &str,
    sha256: &str,
    path: &Path,
    fetch: impl FnOnce(&str, &Path) -> Result,
) -> Result {
    if path.exists() && crate::sums::file(path)? == sha256 {
        return Ok(());
    }
    say(&format!("Downloading {what}…"));
    fetch(url, path)?;
    let found = crate::sums::file(path)?;
    if found == sha256 {
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    Err(format!(
        "{what} from {url} has SHA-256 {found}, not the pinned {sha256}, so it was removed. \
         If upstream changed the file, re-check it and update the pin."
    ))
}

/// The Rust sources under `dir` of `repo`, as (path from `repo`, text), in path order.
pub fn sources_under(repo: &Path, dir: &str, into: &mut Vec<(String, String)>) -> Result {
    let full = repo.join(dir);
    let entries =
        std::fs::read_dir(&full).map_err(|cause| format!("{}: {cause}", full.display()))?;
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        let path = format!("{dir}/{name}");
        let full = repo.join(&path);
        if full.is_dir() {
            sources_under(repo, &path, into)?;
        } else if name.ends_with(".rs") {
            let text = std::fs::read_to_string(&full)
                .map_err(|cause| format!("{}: {cause}", full.display()))?;
            into.push((path, text));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_command_that_goes_quiet_is_stopped_as_hung() {
        let started = Instant::now();
        let failure = run_watched(
            Command::new("sh").args(["-c", "echo working; sleep 30"]),
            &[],
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(failure.contains("printed nothing for 1 s"), "{failure}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    /// The release build waits longer for its silent link than any other step waits.
    #[test]
    fn the_release_build_may_be_quiet_longest() {
        assert!(RELEASE_BUILD_QUIET > QUIET);
        if std::env::var_os("KCHESS_QUIET_MINUTES").is_none() {
            assert_eq!(quiet_limit().max(RELEASE_BUILD_QUIET), RELEASE_BUILD_QUIET);
        }
    }

    #[test]
    fn a_command_that_keeps_printing_runs_past_the_quiet_limit() {
        run_watched(
            Command::new("sh").args(["-c", "for i in 1 2 3 4 5 6; do echo $i; sleep 0.3; done"]),
            &[],
            Duration::from_secs(1),
        )
        .unwrap();
        let failure =
            run_watched(Command::new("sh").args(["-c", "exit 4"]), &[], QUIET).unwrap_err();
        assert!(failure.contains("exit status: 4"), "{failure}");
    }

    #[test]
    fn a_download_from_a_server_that_sends_nothing_gives_up() {
        // Accepted by the system and never answered: curl's request waits on a silent server.
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/stockfish.tar", silent.local_addr().unwrap());
        let target = scratch("silent-download");
        let args = curl_args(&url, &target.to_string_lossy(), 1);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let started = Instant::now();
        let failure = run_watched(Command::new("curl").args(&args), &[], QUIET).unwrap_err();
        assert!(failure.contains("failed"), "{failure}");
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{:?}",
            started.elapsed()
        );
        assert!(args.windows(2).any(|pair| pair == ["--speed-time", "1"]));
        assert_eq!(curl_args("u", "t", 60)[7..9], ["--speed-time", "60"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_command_that_asks_a_question_fails_instead_of_waiting() {
        // `read` waits for a line from the terminal; with no input it reads nothing at once.
        let failure = run("sh", &["-c", "read answer || exit 3"]).unwrap_err();
        assert!(failure.contains("exit status: 3"), "{failure}");
    }

    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-pinned-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("file")
    }

    fn writes(bytes: &'static [u8]) -> impl FnOnce(&str, &Path) -> Result {
        move |_, path| std::fs::write(path, bytes).map_err(|cause| cause.to_string())
    }

    #[test]
    fn git_finds_its_repository_from_its_folder_not_a_hooks_variables() {
        let command = git_in(Path::new("/some/repo"));
        assert_eq!(command.get_current_dir(), Some(Path::new("/some/repo")));
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        for variable in GIT_LOCATION {
            assert!(
                removed.iter().any(|name| name == variable),
                "{variable} is kept"
            );
        }
    }

    #[test]
    fn a_cached_file_that_matches_its_pin_is_kept_without_a_download() {
        let path = scratch("kept");
        std::fs::write(&path, b"hello").unwrap();
        let fetch =
            |_: &str, _: &Path| -> Result { panic!("a matching file was downloaded again") };
        pinned(
            "the file",
            "https://example.test/file",
            HELLO_SHA256,
            &path,
            fetch,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn a_missing_or_tampered_cached_file_is_fetched_again() {
        for cached in [None, Some(&b"tampered"[..])] {
            let path = scratch("refetched");
            if let Some(bytes) = cached {
                std::fs::write(&path, bytes).unwrap();
            }
            let fetched = std::cell::Cell::new(None);
            let fetch = |url: &str, to: &Path| -> Result {
                fetched.set(Some(url.to_string()));
                writes(b"hello")(url, to)
            };
            pinned(
                "the file",
                "https://example.test/file",
                HELLO_SHA256,
                &path,
                fetch,
            )
            .unwrap();
            assert_eq!(fetched.take().as_deref(), Some("https://example.test/file"));
            assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        }
    }

    #[test]
    fn a_download_that_does_not_match_its_pin_is_removed_and_refused() {
        let path = scratch("refused");
        std::fs::write(&path, b"tampered").unwrap();
        let error = pinned(
            "the AppImage runtime",
            "https://example.test/runtime",
            HELLO_SHA256,
            &path,
            writes(b"changed upstream"),
        )
        .unwrap_err();
        assert!(!path.exists(), "the mismatching download was kept");
        assert!(
            error
                .starts_with("the AppImage runtime from https://example.test/runtime has SHA-256 ")
        );
        assert!(error.contains(&format!("not the pinned {HELLO_SHA256}")));
    }

    #[test]
    fn a_failed_download_is_reported() {
        let path = scratch("failed");
        let fetch = |_: &str, _: &Path| -> Result { Err("HTTP 504".into()) };
        let error = pinned(
            "the file",
            "https://example.test/file",
            HELLO_SHA256,
            &path,
            fetch,
        );
        assert_eq!(error, Err("HTTP 504".to_string()));
    }

    #[test]
    fn dates_fall_on_the_right_day_across_leap_rules() {
        for (seconds, date) in [
            (0, (1970, 1, 1)),
            (-1, (1969, 12, 31)),
            (951_782_400, (2000, 2, 29)),
            (951_868_800, (2000, 3, 1)),
            (1_709_164_800, (2024, 2, 29)),
            (4_107_456_000, (2100, 2, 28)),
            (4_107_542_400, (2100, 3, 1)),
            (1_798_761_599, (2026, 12, 31)),
            (1_798_761_600, (2027, 1, 1)),
        ] {
            assert_eq!(civil_date(seconds), date, "{seconds}");
        }
    }

    #[test]
    fn timestamps_carry_the_time_of_day() {
        assert_eq!(iso_timestamp(1_762_605_296), "2025-11-08T12:34:56.000Z");
        assert_eq!(iso_timestamp(-1), "1969-12-31T23:59:59.000Z");
        assert_eq!(iso_timestamp(0), "1970-01-01T00:00:00.000Z");
    }

    /// A clone without `origin/main` made `check-fast` diff only the uncommitted changes, say
    /// "No Rust changes since main." and pass without a test: no base is an error.
    #[test]
    fn changes_are_measured_from_origin_main_or_refused() {
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
        let refused = merge_base(dir.path()).unwrap_err();
        assert!(refused.contains("git fetch origin main"), "{refused}");
        git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        assert_eq!(
            merge_base(dir.path()).unwrap(),
            git(&["rev-parse", "HEAD"]).trim()
        );
    }

    #[test]
    fn the_repository_is_gh_repo_else_kchess() {
        assert_eq!(repository_from(Some("fork/KChess".into())), "fork/KChess");
        assert_eq!(repository_from(Some(String::new())), REPOSITORY);
        assert_eq!(repository_from(None), "ketansingrole/KChess-V2");
        assert_eq!(repository(), repository_from(std::env::var("GH_REPO").ok()));
    }

    #[test]
    fn a_files_time_is_read_only_while_it_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        assert_eq!(modified(&file), None);
        std::fs::write(&file, "x").unwrap();
        assert_eq!(
            modified(&file),
            std::fs::metadata(&file).unwrap().modified().ok()
        );
    }

    #[test]
    fn a_command_succeeds_only_when_it_exits_zero() {
        assert!(available("git", &["--version"]));
        assert!(!available("git", &["no-such-git-command"]));
        assert!(!available("no-such-program-xtask", &[]));
    }

    #[test]
    fn reads_the_workspace_version_only() {
        let manifest = "[workspace]\nmembers = []\n\n[workspace.package]\n# The release\nversion = \"2026.10.15\"\nedition = \"2024\"\n\n[package]\nversion = \"0.1.0\"\n";
        assert_eq!(workspace_version(manifest).as_deref(), Some("2026.10.15"));
        assert_eq!(workspace_version("[package]\nversion = \"1.0.0\"\n"), None);
    }

    /// A failed notarization must not print the app-specific password.
    #[cfg(unix)]
    #[test]
    fn a_failed_command_does_not_show_its_secret() {
        let failed = run_redacted(
            "sh",
            &["-c", "exit 3", "--password", "abcd-efgh"],
            &["abcd-efgh"],
        )
        .unwrap_err();
        assert!(!failed.contains("abcd-efgh"), "{failed}");
        assert!(failed.contains("<redacted>"), "{failed}");
    }
}
