//! `cargo xtask package [--skip-build] [--snap]`: the app for this machine, with its own native
//! Stockfish, the core service, the command line and the voice runtime beside it, in the formats
//! KChess has always shipped: macOS a .dmg and a .zip (Apple Silicon); Windows the per-user NSIS
//! installer (.exe); Linux an .AppImage and a .deb. The updater installs each kind
//! (`apps/gpui/src/updates.rs`). Every file gets a line in SHA256SUMS.txt. Output:
//! `apps/gpui/dist/`.
//!
//! Every package carries the identity every KChess installation runs (`com.kchess.app`, "KChess",
//! the Electron app's NSIS GUID and .deb name). Electron installations update to the frozen
//! stepping-stone release instead (`electron_step`, whose manifests the release adds).
//!
//! macOS signing: without `KCHESS_SIGN_IDENTITY` the bundle is signed ad hoc. With a Developer ID
//! identity it is signed with the hardened runtime, then notarized and stapled (`APPLE_ID`,
//! `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`).

// The scripts and archives are written the same everywhere; only the platform tools differ.
#[cfg(unix)]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod mas;
pub mod windows;

use std::path::{Path, PathBuf};
use std::process::Command;

use kchess_delta::Asset;

use crate::util::{Result, exe, root, run_with, say};

pub const AUTHOR: &str = "Ketan Singrole";
pub const COPYRIGHT: &str = "Copyright © 2026 Ketan Singrole. Licensed under the GPL-3.0-or-later.";
#[cfg(unix)]
pub const HOMEPAGE: &str = "https://ketansingrole.github.io/KChess-V2/";
#[cfg(unix)]
pub const SYNOPSIS: &str = "Desktop chess: Stockfish, Lichess, puzzles and training";
#[cfg(unix)]
pub const DESCRIPTION: &str =
    "A desktop chess app: play Stockfish, play on Lichess, follow friends, review games.";

/// The Mac app (`KChess.app`), the Windows product and its install folder, and the Linux app's
/// folder (`/opt/KChess`).
pub const NAME: &str = "KChess";
/// The NSIS installation's GUID: the Electron app's, so its installers upgrade it in place.
pub const GUID: &str = "6bb66f38-a1a5-5ec8-b24f-c899466885ca";

/// Who the package installs as: every KChess installation updates through these, so they never
/// change. Each platform has only the fields it reads.
pub struct Identity {
    pub name: &'static str,
    #[cfg(target_os = "macos")]
    pub app_id: &'static str,
    /// The program inside the Mac bundle (`Contents/MacOS`).
    #[cfg(target_os = "macos")]
    pub mac_exe: &'static str,
    /// The Windows executable, without `.exe`.
    pub windows_exe: &'static str,
    /// The Linux launcher, and its desktop entry's and icon's names.
    #[cfg(unix)]
    pub linux_exe: &'static str,
    /// The .deb package.
    #[cfg(unix)]
    pub package: &'static str,
    pub guid: &'static str,
    pub version: String,
}

impl Identity {
    pub fn new(version: String) -> Identity {
        Identity {
            name: NAME,
            #[cfg(target_os = "macos")]
            app_id: "com.kchess.app",
            #[cfg(target_os = "macos")]
            mac_exe: "kchess",
            windows_exe: "KChess",
            #[cfg(unix)]
            linux_exe: "kchess",
            #[cfg(unix)]
            package: "kchess",
            guid: GUID,
            version,
        }
    }

    /// This version's package of kind `asset`, as the release names it.
    pub fn asset(&self, asset: Asset) -> String {
        asset.released(&self.version)
    }
}

/// The crates whose programs every package ships: the app, the core service, the command line
/// and the speech helper. They are built for release, attributed (`licenses`), and checked for
/// `unwrap` (`check`).
pub const SHIPPED: [&str; 4] = [
    "kchess-gpui",
    "kchess-service",
    "kchess-cli",
    "kchess-voice",
];

/// The license texts of everything a package ships, beside its programs.
const LICENSES: &str = "THIRD_PARTY_LICENSES.txt";

/// What is built and staged before the platform's packages are made.
pub struct Built {
    pub app: PathBuf,
    pub service: PathBuf,
    pub cli: PathBuf,
    pub stockfish: PathBuf,
    /// Fairy-Stockfish, the variant engine (`crate::fairy`).
    pub fairy: PathBuf,
    pub voice: PathBuf,
    pub licenses: PathBuf,
}

impl Built {
    /// The programs a package installs side by side, each with the name it is installed as: the
    /// app as `app`, the engines, the core service and, with `cli`, the command line. The app
    /// finds them by these names beside itself (`apps/gpui/src/profile.rs`).
    pub fn programs(&self, app: &str, cli: bool) -> Vec<(&Path, String)> {
        let mut programs = vec![
            (self.app.as_path(), exe(app)),
            (self.stockfish.as_path(), exe("stockfish")),
            (self.fairy.as_path(), exe("fairy-stockfish")),
            (self.service.as_path(), exe("kchess-service")),
        ];
        if cli {
            programs.push((self.cli.as_path(), exe("kchess-cli")));
        }
        programs
    }
}

/// The installation's folder as Windows and Linux lay it out: the programs (the app as `app`),
/// the voice runtime in `voice/` and the license texts.
pub fn stage_folder(built: &Built, folder: &Path, app: &str) -> Result {
    for (from, name) in built.programs(app, true) {
        copy_program(from, &folder.join(name))?;
    }
    copy_voice(built, folder)?;
    copy_licenses(built, folder)
}

pub fn copy_licenses(built: &Built, dir: &Path) -> Result {
    std::fs::copy(&built.licenses, dir.join(LICENSES))
        .map(|_| ())
        .map_err(|cause| cause.to_string())
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

/// The element id only the development builds' sidebar badge carries (`apps/gpui/src/shell.rs`).
const DEV_BUILD_MARKER: &str = "kchess-dev-build-badge";

fn verify_no_dev_build(path: &Path) -> Result {
    let bytes = std::fs::read(path).map_err(|cause| cause.to_string())?;
    if bytes
        .windows(DEV_BUILD_MARKER.len())
        .any(|window| window == DEV_BUILD_MARKER.as_bytes())
    {
        return Err(format!(
            "{} contains the development build badge ({DEV_BUILD_MARKER}).",
            path.display()
        ));
    }
    Ok(())
}

fn build(skip: bool) -> Result<Built> {
    let target = root().join("target/release");
    if !skip {
        say("Building the app, core service, command line and voice helper (release)…");
        let mut cargo = Command::new("cargo");
        cargo.args(["build", "--release", "--locked"]);
        for package in SHIPPED {
            cargo.args(["-p", package]);
        }
        // A shipped build ignores the development overrides (`kchess_core::dev_overrides`).
        cargo.args(["--features", "kchess-core/packaged"]);
        run_with(&mut cargo)?;
    }
    let voice = crate::voice::stage("release")?;
    let fairy = crate::fairy::build()?;
    let stockfish = crate::engines::stockfish(&root().join("target/package/stockfish"), true)?;
    let licenses = root().join("target/package").join(LICENSES);
    std::fs::create_dir_all(root().join("target/package")).map_err(|cause| cause.to_string())?;
    crate::licenses::write(&licenses)?;
    let built = Built {
        app: target.join(exe("KChess")),
        service: target.join(exe("kchess-service")),
        cli: target.join(exe("kchess-cli")),
        stockfish,
        fairy,
        voice,
        licenses,
    };
    // Debug builds draw a development badge in the sidebar; a release binary must not contain it.
    verify_no_dev_build(&built.app)?;
    Ok(built)
}

/// The packages this machine releases: the Snap alone with `--snap`.
fn released(snap: bool) -> &'static [Asset] {
    if snap {
        &[Asset::Snap]
    } else if cfg!(target_os = "macos") {
        &[Asset::MacZip, Asset::MacDmg]
    } else if cfg!(windows) {
        &[Asset::WindowsInstaller]
    } else {
        &[Asset::AppImage, Asset::Deb]
    }
}

/// Every package of `kinds` is in `out` and holds something: a release never goes out without one.
fn check_released(out: &Path, identity: &Identity, kinds: &[Asset]) -> Result {
    for kind in kinds {
        let path = out.join(identity.asset(*kind));
        if std::fs::metadata(&path).map_or(true, |file| file.len() == 0) {
            return Err(format!("Packaging made no {}.", path.display()));
        }
    }
    Ok(())
}

pub fn run(args: &[String]) -> Result {
    let skip = args.iter().any(|arg| arg == "--skip-build");
    let snap = args.iter().any(|arg| arg == "--snap");
    let store = args.iter().any(|arg| arg == "--mas");
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
    // The App Store's package leaves the other packages in place.
    if !store {
        let _ = std::fs::remove_dir_all(&out);
    }
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&out).map_err(|cause| cause.to_string())?;
    std::fs::create_dir_all(&stage).map_err(|cause| cause.to_string())?;
    #[cfg(target_os = "macos")]
    if store {
        // The App Store's package goes to the store alone: no manifest, checksums or release asset.
        let _ = std::fs::remove_dir_all(out.join("mas"));
        for pkg in mas::package(&identity, &built, &stage, &out)? {
            say(&format!("Done: {}", pkg.display()));
        }
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    if store {
        return Err("The Mac App Store package is built on macOS.".into());
    }
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
    for package in &packages {
        say(&format!("Packaged {}", package.display()));
    }
    check_released(&out, &identity, released(snap))?;
    let sums = crate::sums::write(&out)?;
    say(&format!("Done: {}", out.display()));
    eprint!("{sums}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A platform's packaging fails without any package it releases, or with an empty one.
    #[test]
    fn packaging_fails_without_every_package_it_releases() {
        let out = tempfile::tempdir().unwrap();
        let identity = Identity::new("2026.11.9".into());
        let kinds = [Asset::AppImage, Asset::Deb];
        std::fs::write(
            out.path().join("KChess-2026.11.9-linux-x86_64.AppImage"),
            b"x",
        )
        .unwrap();
        let missing = check_released(out.path(), &identity, &kinds).unwrap_err();
        assert!(
            missing.contains("KChess-2026.11.9-linux-amd64.deb"),
            "{missing}"
        );
        std::fs::write(out.path().join("KChess-2026.11.9-linux-amd64.deb"), b"").unwrap();
        assert!(check_released(out.path(), &identity, &kinds).is_err());
        std::fs::write(out.path().join("KChess-2026.11.9-linux-amd64.deb"), b"x").unwrap();
        assert_eq!(check_released(out.path(), &identity, &kinds), Ok(()));
        assert_eq!(released(true), [Asset::Snap]);
    }
}
