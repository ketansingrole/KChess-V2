//! `app`: build and start the GPUI app on a scratch profile with a native Stockfish, in the
//! background, and stop it again by its process ID (never by the binary's path, which would also
//! stop a copy someone runs on their real profile). `--watch` stays in the foreground and does it
//! again whenever a source changes: GPUI views are compiled, so a change shows after a restart.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::util::{Result, root, say};

/// The scratch profile development runs use unless `--profile` names another.
const PROFILE: &str = ".data/gpui-profile";

/// Where the running app's process ID and output are kept.
fn pid_file() -> PathBuf {
    root().join(".data/gpui.pid")
}

fn log_file() -> PathBuf {
    root().join(".data/gpui.log")
}

/// Where a watched app keeps its page, to open on it after a restart.
fn page_file() -> PathBuf {
    root().join(".data/gpui-page")
}

/// The profiles of real installations, which a development run must never open.
fn real_profiles() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut profiles: Vec<PathBuf> = home
        .iter()
        .flat_map(|home| {
            [
                home.join("Library/Application Support/KChess"),
                home.join(".config/KChess"),
            ]
        })
        .collect();
    profiles.extend(std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("KChess")));
    profiles
}

fn is_real_profile(profile: &Path, real: &[PathBuf]) -> bool {
    let profile = profile
        .canonicalize()
        .unwrap_or_else(|_| profile.to_path_buf());
    real.iter()
        .any(|real| real.canonicalize().unwrap_or_else(|_| real.clone()) == profile)
}

/// `KCHESS_ENGINE_PATH`, else the Stockfish in `.data/stockfish`, fetched there when missing.
pub fn engine() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("KCHESS_ENGINE_PATH") {
        return Ok(PathBuf::from(path));
    }
    let dir = root().join(".data/stockfish");
    let name = if cfg!(windows) {
        "stockfish.exe"
    } else {
        "stockfish"
    };
    if dir.join(name).is_file() {
        return Ok(dir.join(name));
    }
    say("Fetching Stockfish into .data/stockfish…");
    let out = Command::new("cargo")
        .args([
            "run",
            "-q",
            "--locked",
            "-p",
            "kchess-core",
            "--example",
            "fetch_stockfish",
            "--",
        ])
        .arg(&dir)
        .current_dir(root())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|cause| format!("Could not fetch Stockfish: {cause}"))?;
    if !out.status.success() {
        return Err("Could not fetch Stockfish.".into());
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// Builds the app and the core service it starts, and answers the app's executable, as Cargo
/// reports it.
fn build() -> Result<PathBuf> {
    let out = Command::new("cargo")
        .args([
            "build",
            "-q",
            "--locked",
            "-p",
            "kchess-gpui",
            "--bin",
            "KChess",
            "-p",
            "kchess-service",
            "--bin",
            "kchess-service",
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(root())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|cause| format!("Could not run cargo build: {cause}"))?;
    if !out.status.success() {
        return Err("The app did not build.".into());
    }
    executable(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| "cargo build reported no KChess executable.".into())
}

/// The KChess executable in `cargo build --message-format=json` output.
fn executable(messages: &str) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["target"]["name"] == "KChess")
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
}

/// The app's process ID from the PID file, if that process still runs.
fn running() -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pid_file())
        .ok()?
        .trim()
        .parse()
        .ok()?;
    alive(pid, "KChess").then_some(pid)
}

/// Whether `pid` is a running `name` (a reused process ID is someone else's).
fn alive(pid: u32, name: &str) -> bool {
    if cfg!(windows) {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(name))
    } else {
        Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim().ends_with(name))
    }
}

fn terminate(pid: u32) -> Result {
    let status = if cfg!(windows) {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .status()
    } else {
        Command::new("kill").arg(pid.to_string()).status()
    };
    status
        .ok()
        .filter(|status| status.success())
        .map(|_| ())
        .ok_or_else(|| format!("Could not stop pid {pid}."))
}

fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// The process ID in the profile's `core.json`, when that core started before `built`.
fn outdated_service(info: &str, built: SystemTime) -> Option<u32> {
    let info: serde_json::Value = serde_json::from_str(info).ok()?;
    let started = info["startedAt"].as_u64()?;
    (started < millis(built)).then_some(info["pid"].as_u64()? as u32)
}

/// Stops the profile's core service if it runs a build older than `binary`'s, which the next
/// client then starts: it outlives the app, so its code would otherwise stay as it was.
fn stop_outdated_service(profile: &Path, binary: &Path) -> Result {
    let service = binary.with_file_name(format!("kchess-service{}", std::env::consts::EXE_SUFFIX));
    let (Ok(info), Ok(built)) = (
        std::fs::read_to_string(profile.join("core.json")),
        std::fs::metadata(&service).and_then(|file| file.modified()),
    ) else {
        return Ok(());
    };
    let Some(pid) = outdated_service(&info, built) else {
        return Ok(());
    };
    if !alive(pid, "kchess-service") {
        return Ok(());
    }
    terminate(pid)?;
    // The profile is its lock's until the process has gone.
    for _ in 0..100 {
        if !alive(pid, "kchess-service") {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!("The core service (pid {pid}) did not stop."))
}

fn check_startable(profile: &Path) -> Result {
    if let Some(pid) = running() {
        return Err(format!(
            "KChess already runs (pid {pid}); `cargo xtask app --stop` first."
        ));
    }
    if is_real_profile(profile, &real_profiles()) {
        return Err(format!(
            "{} is a real profile; development runs use a scratch one.",
            profile.display()
        ));
    }
    std::fs::create_dir_all(profile).map_err(|cause| cause.to_string())
}

/// Starts `binary`, with a fresh log and its process ID in the PID file.
fn launch(
    binary: &Path,
    profile: &Path,
    engine: &Path,
    args: &[String],
    keep_page: bool,
) -> Result<Child> {
    stop_outdated_service(profile, binary)?;
    let log = std::fs::File::create(log_file()).map_err(|cause| cause.to_string())?;
    let mut command = if cfg!(target_os = "macos") {
        // Signs the build with a stable identity, so the keychain remembers it, then execs it.
        let mut command = Command::new(root().join("tooling/macos-dev-runner.sh"));
        command.arg(binary);
        command
    } else {
        Command::new(binary)
    };
    if keep_page {
        command.env("KCHESS_PAGE_FILE", page_file());
    }
    let child = command
        .args(args)
        .env("KCHESS_USER_DATA_DIR", profile)
        .env("KCHESS_ENGINE_PATH", engine)
        .current_dir(root())
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|cause| cause.to_string())?)
        .stderr(log)
        .spawn()
        .map_err(|cause| format!("Could not start {}: {cause}", binary.display()))?;
    std::fs::write(pid_file(), child.id().to_string()).map_err(|cause| cause.to_string())?;
    Ok(child)
}

fn start(profile: &Path, args: &[String]) -> Result {
    check_startable(profile)?;
    let engine = engine()?;
    let binary = build()?;
    let child = launch(&binary, profile, &engine, args, false)?;
    say(&format!(
        "KChess started: pid {}, profile {}, output in .data/gpui.log. Stop it with `cargo xtask app --stop`.",
        child.id(),
        profile.display()
    ));
    println!("{}", child.id());
    Ok(())
}

/// What a build is made from, and whether it changed: a file's time and size.
type Sources = BTreeMap<PathBuf, (SystemTime, u64)>;

fn add_sources(path: &Path, sources: &mut Sources) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.is_file() {
        if let Ok(modified) = metadata.modified() {
            sources.insert(path.to_path_buf(), (modified, metadata.len()));
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !is_output(&name.to_string_lossy()) {
            add_sources(&entry.path(), sources);
        }
    }
}

/// A folder or file under `apps` or `crates` that no build reads.
fn is_output(name: &str) -> bool {
    name.starts_with('.') || name == "target" || name == "dist"
}

fn sources() -> Sources {
    let mut sources = Sources::new();
    for path in ["apps", "crates", "Cargo.toml", "Cargo.lock"] {
        add_sources(&root().join(path), &mut sources);
    }
    sources
}

/// The sources once a save has settled, when they differ from `known`.
fn changed_sources(known: &Sources) -> Option<Sources> {
    let mut found = sources();
    if found == *known {
        return None;
    }
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let settled = sources();
        if settled == found {
            return Some(found);
        }
        found = settled;
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|file| file.modified())
        .ok()
}

/// Stops the app this watch started and waits for it, which also frees its window's place.
fn stop_child(app: &mut Option<Child>) -> Result {
    if let Some(mut child) = app.take() {
        terminate(child.id())?;
        child.wait().map_err(|cause| cause.to_string())?;
    }
    Ok(())
}

/// Starts the app, then rebuilds and restarts it whenever a source changes, until it is quit or
/// stopped. A build that fails leaves the running app alone, and the next save tries again.
fn watch(profile: &Path, args: &[String]) -> Result {
    check_startable(profile)?;
    let engine = engine()?;
    let mut known = sources();
    let mut binary = build()?;
    let mut app = Some(launch(&binary, profile, &engine, args, true)?);
    say("Watching apps/ and crates/. Stop with Ctrl-C or `cargo xtask app --stop`.");
    loop {
        std::thread::sleep(Duration::from_millis(250));
        if let Some(child) = app.as_mut()
            && let Some(status) = child.try_wait().map_err(|cause| cause.to_string())?
        {
            app = None;
            if status.success() || !pid_file().exists() {
                let _ = std::fs::remove_file(pid_file());
                say("KChess quit; no longer watching.");
                return Ok(());
            }
            say(&format!(
                "KChess stopped ({status}); see .data/gpui.log. Save to rebuild."
            ));
        }
        let Some(changed) = changed_sources(&known) else {
            continue;
        };
        known = changed;
        // Windows cannot replace a running executable.
        if cfg!(windows) {
            stop_child(&mut app)?;
        }
        let before = modified(&binary);
        let began = std::time::Instant::now();
        say("Rebuilding…");
        match build() {
            Ok(built) => binary = built,
            Err(cause) => {
                say(&format!("{cause} Save to try again."));
                continue;
            }
        }
        // A test or a doc changes no executable: the running app is already current.
        if app.is_some() && modified(&binary) == before {
            say("Nothing in the app changed.");
            continue;
        }
        stop_child(&mut app)?;
        app = Some(launch(&binary, profile, &engine, args, true)?);
        say(&format!(
            "Restarted in {:.1}s.",
            began.elapsed().as_secs_f32()
        ));
    }
}

fn stop() -> Result {
    let Some(pid) = running() else {
        say("KChess is not running from `cargo xtask app`.");
        let _ = std::fs::remove_file(pid_file());
        return Ok(());
    };
    // First, so a watch sees this was asked for and ends with the app.
    let _ = std::fs::remove_file(pid_file());
    terminate(pid)?;
    say(&format!("Stopped KChess (pid {pid})."));
    Ok(())
}

/// `app [--profile <dir>] [--watch] [--stop] [-- <app arguments>]`.
pub fn run(args: &[String]) -> Result {
    let (ours, theirs) = match args.iter().position(|arg| arg == "--") {
        Some(at) => (&args[..at], &args[at + 1..]),
        None => (args, &[][..]),
    };
    if ours.iter().any(|arg| arg == "--stop") {
        return stop();
    }
    let profile = match ours.iter().position(|arg| arg == "--profile") {
        Some(at) => PathBuf::from(
            ours.get(at + 1)
                .ok_or("--profile needs the profile folder.")?,
        ),
        None => root().join(PROFILE),
    };
    if ours.iter().any(|arg| arg == "--watch") {
        return watch(&profile, theirs);
    }
    start(&profile, theirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_app_among_the_build_messages() {
        let messages = r#"{"reason":"compiler-artifact","target":{"name":"kchess_core"},"executable":null}
{"reason":"compiler-artifact","target":{"name":"KChess"},"executable":"/t/debug/KChess"}
{"reason":"build-finished","success":true}"#;
        assert_eq!(executable(messages), Some(PathBuf::from("/t/debug/KChess")));
        assert_eq!(executable("{\"reason\":\"build-finished\"}"), None);
    }

    #[test]
    fn a_service_older_than_its_build_is_outdated() {
        let built = UNIX_EPOCH + Duration::from_millis(2_000);
        assert_eq!(
            outdated_service(r#"{"pid":7,"startedAt":1999}"#, built),
            Some(7)
        );
        // Started from this build, or after it (signing touches the file before the start).
        assert_eq!(
            outdated_service(r#"{"pid":7,"startedAt":2000}"#, built),
            None
        );
        assert_eq!(outdated_service("not json", built), None);
    }

    #[test]
    fn sources_change_with_an_edit_and_skip_build_output() {
        let dir = tempfile::tempdir().expect("folder");
        let page = dir.path().join("page.rs");
        std::fs::write(&page, "one").expect("source");
        std::fs::create_dir(dir.path().join("dist")).expect("output");
        std::fs::write(dir.path().join("dist/KChess"), "built").expect("output");
        let mut before = Sources::new();
        add_sources(dir.path(), &mut before);
        assert_eq!(before.keys().collect::<Vec<_>>(), [&page]);
        std::fs::write(&page, "edited").expect("source");
        let mut after = Sources::new();
        add_sources(dir.path(), &mut after);
        assert_ne!(before, after);
    }

    #[test]
    fn refuses_a_real_profile() {
        let real = [PathBuf::from(
            "/Users/me/Library/Application Support/KChess",
        )];
        assert!(is_real_profile(
            Path::new("/Users/me/Library/Application Support/KChess"),
            &real
        ));
        assert!(!is_real_profile(
            Path::new("/repo/.data/gpui-profile"),
            &real
        ));
    }
}
