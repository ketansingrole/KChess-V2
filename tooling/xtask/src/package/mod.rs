//! `cargo xtask package [--skip-build] [--snap]`: the app for this machine, with its own native
//! Stockfish, the core service, the command line and the voice runtime beside it, in the formats
//! KChess has always shipped: macOS a .dmg and a .zip (Apple Silicon); Windows the per-user NSIS
//! installer (.exe); Linux an .AppImage and a .deb. The updater installs each kind
//! (`apps/gpui/src/updates.rs`). Every file gets a line in SHA256SUMS.txt. Output:
//! `apps/gpui/dist/`.
//!
//! Every package carries the identity every KChess installation runs (`com.kchess.app`, "KChess",
//! the Electron app's NSIS GUID and .deb name), with the update manifests (`latest*.yml`) the
//! Electron app's updater reads, so it installs this over itself.
//!
//! macOS signing: without `KCHESS_SIGN_IDENTITY` the bundle is signed ad hoc. With a Developer ID
//! identity it is signed with the hardened runtime, then notarized and stapled (`APPLE_ID`,
//! `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`).

// The scripts and archives are written the same everywhere; only the platform tools differ.
#[cfg(unix)]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
mod manifest;
mod windows;

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{Result, hex, root, run_with, say};

// Each platform's packages use some of these; the others are for the other platforms.
#[allow(dead_code)]
pub const AUTHOR: &str = "Ketan Singrole";
pub const COPYRIGHT: &str = "Copyright © 2026 Ketan Singrole. Licensed under the GPL-3.0-or-later.";
#[allow(dead_code)]
pub const HOMEPAGE: &str = "https://github.com/ketansingrole/KChess-V2#readme";
#[allow(dead_code)]
pub const SYNOPSIS: &str = "Desktop chess: Stockfish, Lichess, puzzles and training";
#[allow(dead_code)]
pub const DESCRIPTION: &str =
    "A desktop chess app: play Stockfish, play on Lichess, follow friends, review games.";

/// Who the package installs as (each platform reads its own fields).
#[allow(dead_code)]
pub struct Identity {
    /// The Mac app, the Windows product and install folder.
    pub name: &'static str,
    pub app_id: &'static str,
    /// The Windows executable's and the Linux launcher's name.
    pub windows_exe: &'static str,
    pub linux_exe: &'static str,
    /// The .deb package (and the Windows uninstall key's display name base).
    pub package: &'static str,
    /// The NSIS installation's GUID: the Electron app's, so its installers upgrade it in place.
    pub guid: &'static str,
    pub version: String,
}

impl Identity {
    pub fn new(version: String) -> Identity {
        Identity {
            name: "KChess",
            app_id: "com.kchess.app",
            windows_exe: "KChess",
            linux_exe: "kchess",
            package: "kchess",
            guid: "6bb66f38-a1a5-5ec8-b24f-c899466885ca",
            version,
        }
    }

    /// The packages' file name prefix (`KChess-2026.10.16-mac-arm64.zip`).
    pub fn prefix(&self) -> String {
        self.name.replace(' ', "-")
    }
}

/// What is built and staged before the platform's packages are made.
pub struct Built {
    pub app: PathBuf,
    pub service: PathBuf,
    pub cli: PathBuf,
    pub stockfish: PathBuf,
    pub voice: PathBuf,
    pub licenses: PathBuf,
    /// The Electron bridge (macOS only).
    #[allow(dead_code)]
    pub bridge: PathBuf,
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Copy `from` to `to`, executable on Unix.
pub fn copy_program(from: &Path, to: &Path) -> Result {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|cause| cause.to_string())?;
    }
    std::fs::copy(from, to).map_err(|cause| format!("Cannot copy {}: {cause}", from.display()))?;
    executable(to)
}

pub fn executable(path: &Path) -> Result {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|cause| cause.to_string())?;
    }
    let _ = path;
    Ok(())
}

/// Copy the voice runtime into `dir/voice`; the paths of its programs and libraries.
pub fn copy_voice(built: &Built, dir: &Path) -> Result<Vec<PathBuf>> {
    let target = dir.join("voice");
    std::fs::create_dir_all(&target).map_err(|cause| cause.to_string())?;
    let mut code = Vec::new();
    for entry in std::fs::read_dir(&built.voice)
        .map_err(|cause| cause.to_string())?
        .flatten()
    {
        let to = target.join(entry.file_name());
        std::fs::copy(entry.path(), &to).map_err(|cause| cause.to_string())?;
        if !entry.file_name().to_string_lossy().ends_with(".txt") {
            executable(&to)?;
            code.push(to);
        }
    }
    Ok(code)
}

/// Names only the development builds' sidebar badge carries (`shell.rs`).
const DEV_BUILD_MARKERS: [&str; 2] = ["data-dev-build", "kchess-dev-build-badge"];

fn verify_no_dev_build(path: &Path) -> Result {
    let bytes = std::fs::read(path).map_err(|cause| cause.to_string())?;
    for marker in DEV_BUILD_MARKERS {
        if bytes
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
        {
            return Err(format!(
                "{} contains the development build badge ({marker}).",
                path.display()
            ));
        }
    }
    Ok(())
}

fn build(skip: bool) -> Result<Built> {
    let target = root().join("target/release");
    if !skip {
        say("Building the app, core service, command line and voice helper (release)…");
        let mut cargo = Command::new("cargo");
        cargo.args([
            "build",
            "--release",
            "--locked",
            "-p",
            "kchess-gpui",
            "-p",
            "kchess-service",
            "-p",
            "kchess-cli",
            "-p",
            "kchess-voice",
        ]);
        // What a Mac Electron app from before 2026.10.12 updates to (`mac::bridge`).
        if cfg!(target_os = "macos") {
            cargo.args(["-p", "kchess-electron-bridge"]);
        }
        run_with(&mut cargo)?;
    }
    let voice = crate::voice::stage("release")?;
    say("Fetching Stockfish…");
    let fetched = Command::new("cargo")
        .args([
            "run",
            "--release",
            "--locked",
            "-q",
            "-p",
            "kchess-core",
            "--example",
            "fetch_stockfish",
            "--",
        ])
        .arg(root().join("target/package/stockfish"))
        .current_dir(root())
        .output()
        .map_err(|cause| cause.to_string())?;
    if !fetched.status.success() {
        return Err(format!(
            "Fetching Stockfish failed: {}",
            String::from_utf8_lossy(&fetched.stderr).trim()
        ));
    }
    let stockfish = PathBuf::from(String::from_utf8_lossy(&fetched.stdout).trim());
    let licenses = root().join("target/package/THIRD_PARTY_LICENSES.txt");
    std::fs::create_dir_all(root().join("target/package")).map_err(|cause| cause.to_string())?;
    crate::licenses::write(&licenses)?;
    let built = Built {
        app: target.join(exe("KChess")),
        service: target.join(exe("kchess-service")),
        cli: target.join(exe("kchess-cli")),
        stockfish,
        voice,
        licenses,
        bridge: target.join("kchess-electron-bridge"),
    };
    // Debug builds draw a development badge in the sidebar; a release binary must not contain it.
    verify_no_dev_build(&built.app)?;
    Ok(built)
}

fn sha256(path: &Path) -> Result<String> {
    use sha2::Digest;
    let bytes = std::fs::read(path).map_err(|cause| cause.to_string())?;
    Ok(hex(&sha2::Sha256::digest(&bytes)))
}

pub fn run(args: &[String]) -> Result {
    let skip = args.iter().any(|arg| arg == "--skip-build");
    let snap = args.iter().any(|arg| arg == "--snap");
    // KCHESS_PACKAGE_VERSION builds a test installer above every release (windows-upgrade.yml);
    // the app reads the same variable (apps/gpui/build.rs).
    let version = std::env::var("KCHESS_PACKAGE_VERSION")
        .ok()
        .filter(|version| !version.is_empty())
        .map_or_else(crate::util::version, Ok)?;
    let identity = Identity::new(version);
    let built = build(skip)?;
    let out = root().join("apps/gpui/dist");
    let stage = root().join("target/package/stage");
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&out).map_err(|cause| cause.to_string())?;
    std::fs::create_dir_all(&stage).map_err(|cause| cause.to_string())?;
    #[cfg(target_os = "macos")]
    let packages = mac::package(&identity, &built, &stage, &out)?;
    #[cfg(windows)]
    let packages = windows::package(&identity, &built, &stage, &out)?;
    #[cfg(all(unix, not(target_os = "macos")))]
    let packages = if snap {
        linux::package_snap(&identity, &built, &stage, &out)?
    } else {
        linux::package(&identity, &built, &stage, &out)?
    };
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    if snap {
        return Err("The Snap is built on Linux.".into());
    }
    // The Snap Store updates the Snap; the manifests are for the other packages.
    if !snap {
        manifest::write(&identity, &packages, &out)?;
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&out)
        .map_err(|cause| cause.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    let mut sums = String::new();
    for path in &files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        sums.push_str(&format!("{}  {name}\n", sha256(path)?));
    }
    std::fs::write(out.join("SHA256SUMS.txt"), &sums).map_err(|cause| cause.to_string())?;
    say(&format!("Done: {}", out.display()));
    eprint!("{sums}");
    Ok(())
}
