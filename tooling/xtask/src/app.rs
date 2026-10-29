//! `app`: build and start the GPUI app on a scratch profile with a native Stockfish, in the
//! background, and stop it again by its process ID (never by the binary's path, which would also
//! stop a copy someone runs on their real profile).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Builds the app and answers its executable, as Cargo reports it.
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
    alive(pid).then_some(pid)
}

/// Whether `pid` is a running KChess (a reused process ID is someone else's).
fn alive(pid: u32) -> bool {
    if cfg!(windows) {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("KChess"))
    } else {
        Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .is_ok_and(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .ends_with("KChess")
            })
    }
}

fn start(profile: &Path, args: &[String]) -> Result {
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
    std::fs::create_dir_all(profile).map_err(|cause| cause.to_string())?;
    let engine = engine()?;
    let binary = build()?;
    let log = std::fs::File::create(log_file()).map_err(|cause| cause.to_string())?;
    let mut command = if cfg!(target_os = "macos") {
        // Signs the build with a stable identity, so the keychain remembers it, then execs it.
        let mut command = Command::new(root().join("tooling/macos-dev-runner.sh"));
        command.arg(&binary);
        command
    } else {
        Command::new(&binary)
    };
    let child = command
        .args(args)
        .env("KCHESS_USER_DATA_DIR", profile)
        .env("KCHESS_ENGINE_PATH", &engine)
        .current_dir(root())
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|cause| cause.to_string())?)
        .stderr(log)
        .spawn()
        .map_err(|cause| format!("Could not start {}: {cause}", binary.display()))?;
    std::fs::write(pid_file(), child.id().to_string()).map_err(|cause| cause.to_string())?;
    say(&format!(
        "KChess started: pid {}, profile {}, output in .data/gpui.log. Stop it with `cargo xtask app --stop`.",
        child.id(),
        profile.display()
    ));
    println!("{}", child.id());
    Ok(())
}

fn stop() -> Result {
    let Some(pid) = running() else {
        say("KChess is not running from `cargo xtask app`.");
        let _ = std::fs::remove_file(pid_file());
        return Ok(());
    };
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
        .ok_or_else(|| format!("Could not stop pid {pid}."))?;
    let _ = std::fs::remove_file(pid_file());
    say(&format!("Stopped KChess (pid {pid})."));
    Ok(())
}

/// `app [--profile <dir>] [--stop] [-- <app arguments>]`.
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
