//! What a Mac Electron KChess from before 2026.10.12 updates to. Those releases were
//! `com.kchess.electron`, and Squirrel.Mac only installs an update with the running app's bundle
//! identifier, so the Mac zip carries this bridge under that identifier (`legacy/KChess.app`,
//! `tooling/xtask/src/package/mac.rs`) beside KChess itself. Squirrel puts the bridge in the old
//! app's place and opens it; the bridge unpacks KChess from the same zip, checks that KChess's
//! developer signed it, takes its place and opens it. Until that succeeds the bridge stays where it
//! is, so opening it again retries.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// The bundle identifier of the app the bridge installs.
const APP_ID: &str = "com.kchess.app";
const REPOSITORY: &str = "ketansingrole/KChess-V2";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            alert(&message);
            ExitCode::FAILURE
        }
    }
}

/// The release this bridge shipped in, whose zip holds the KChess it installs.
fn version() -> &'static str {
    match option_env!("KCHESS_PACKAGE_VERSION") {
        Some(version) if !version.is_empty() => version,
        _ => env!("CARGO_PKG_VERSION"),
    }
}

fn archive_name() -> String {
    format!("KChess-{}-mac-arm64.zip", version())
}

fn run() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|cause| cause.to_string())?;
    let bundle = bundle_of(&exe).ok_or("KChess is not in an app bundle.")?;
    let parent = bundle.parent().ok_or("KChess is not in a folder.")?;
    // Beside the bundle, so the new app takes its place by a rename on the same volume.
    let work = parent.join(format!(".KChess-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)
        .map_err(|cause| format!("KChess could not write to {} ({cause}).", parent.display()))?;
    let installed = install(&bundle, &work);
    let _ = std::fs::remove_dir_all(&work);
    installed?;
    Command::new("/usr/bin/open")
        .arg(&bundle)
        .status()
        .map_err(|cause| cause.to_string())?;
    Ok(())
}

/// Put KChess in `bundle`'s place: from the zip Electron's updater kept, or else downloaded.
fn install(bundle: &Path, work: &Path) -> Result<(), String> {
    let team = team(bundle);
    let mut failure = String::new();
    for (index, archive) in [cached_archive(), Some(work.join(archive_name()))]
        .into_iter()
        .flatten()
        .enumerate()
    {
        if !archive.is_file()
            && let Err(cause) = download(&archive)
        {
            failure = cause;
            continue;
        }
        let unpacked = work.join(format!("unpacked-{index}"));
        let app = unpack(&archive, &unpacked).and_then(|app| {
            verify(&app, team.as_deref())?;
            Ok(app)
        });
        match app {
            Ok(app) => return swap(&app, bundle, &work.join("old.app")),
            Err(cause) => failure = cause,
        }
    }
    Err(failure)
}

/// The `.app` whose `Contents/MacOS` holds `exe`.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let bundle = exe.parent()?.parent()?.parent()?;
    bundle
        .extension()
        .is_some_and(|ext| ext == "app")
        .then(|| bundle.to_path_buf())
}

/// The zip electron-updater downloaded and checked, which it keeps after handing it to Squirrel.
fn cached_archive() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home)
        .join("Library/Caches/kchess-electron-updater/pending")
        .join(archive_name());
    path.is_file().then_some(path)
}

fn download(to: &Path) -> Result<(), String> {
    let url = format!(
        "https://github.com/{REPOSITORY}/releases/download/v{}/{}",
        version(),
        archive_name()
    );
    let status = Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--retry",
            "3",
        ])
        .arg("--output")
        .arg(to)
        .arg(&url)
        .status();
    if status.is_ok_and(|status| status.success()) {
        Ok(())
    } else {
        Err("KChess could not be downloaded. Check your internet connection.".into())
    }
}

/// Unpack `archive` into `into`; the KChess bundle at its top.
fn unpack(archive: &Path, into: &Path) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(into);
    let status = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(archive)
        .arg(into)
        .status();
    let app = into.join("KChess.app");
    if status.is_ok_and(|status| status.success()) && app.is_dir() {
        Ok(app)
    } else {
        Err("The update could not be unpacked.".into())
    }
}

/// The team that signed `bundle`; none when it is signed ad hoc.
fn team(bundle: &Path) -> Option<String> {
    let output = Command::new("/usr/bin/codesign")
        .args(["--display", "--verbose=2"])
        .arg(bundle)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|team| !team.is_empty() && *team != "not set")
        .map(str::to_string)
}

/// What `app` must satisfy: KChess, signed by the bridge's own team with a Developer ID.
fn requirement(team: Option<&str>) -> String {
    match team {
        Some(team) => format!(
            "=anchor apple generic and identifier \"{APP_ID}\" and certificate leaf[subject.OU] = \"{team}\""
        ),
        None => format!("=identifier \"{APP_ID}\""),
    }
}

fn verify(app: &Path, team: Option<&str>) -> Result<(), String> {
    let status = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "-R"])
        .arg(requirement(team))
        .arg(app)
        .stderr(Stdio::null())
        .status();
    if status.is_ok_and(|status| status.success()) {
        Ok(())
    } else {
        Err("The update is not signed by KChess's developer.".into())
    }
}

/// Move `bundle` to `aside` and `app` into its place; on failure, put `bundle` back.
fn swap(app: &Path, bundle: &Path, aside: &Path) -> Result<(), String> {
    let fail = |cause: std::io::Error| format!("KChess could not replace itself ({cause}).");
    std::fs::rename(bundle, aside).map_err(fail)?;
    if let Err(cause) = std::fs::rename(app, bundle) {
        let _ = std::fs::rename(aside, bundle);
        return Err(fail(cause));
    }
    Ok(())
}

fn alert(message: &str) {
    let _ = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display alert (item 1 of argv) message (item 2 of argv) as critical",
            "-e",
            "end run",
            "KChess could not finish updating",
        ])
        .arg(format!("{message} Open KChess again to retry."))
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_of_its_executable() {
        assert_eq!(
            bundle_of(Path::new("/Applications/KChess.app/Contents/MacOS/KChess")),
            Some(PathBuf::from("/Applications/KChess.app"))
        );
        assert_eq!(bundle_of(Path::new("/usr/local/bin/kchess")), None);
    }

    #[test]
    fn installs_the_release_it_shipped_in() {
        assert_eq!(
            archive_name(),
            format!("KChess-{}-mac-arm64.zip", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            requirement(Some("4NBN7U2PZL")),
            "=anchor apple generic and identifier \"com.kchess.app\" and certificate leaf[subject.OU] = \"4NBN7U2PZL\""
        );
    }

    #[test]
    fn swap_puts_the_app_in_place_and_keeps_the_bundle_when_it_cannot() {
        let dir = tempfile::tempdir().unwrap();
        let (app, bundle, aside) = (
            dir.path().join("new.app"),
            dir.path().join("KChess.app"),
            dir.path().join("old.app"),
        );
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("bridge"), "").unwrap();
        assert!(
            swap(&app, &bundle, &aside).is_err(),
            "nothing to put in place"
        );
        assert!(bundle.join("bridge").is_file());

        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("kchess"), "").unwrap();
        swap(&app, &bundle, &aside).unwrap();
        assert!(bundle.join("kchess").is_file());
        assert!(aside.join("bridge").is_file());
    }

    /// A bundle signed ad hoc as `identifier`.
    #[cfg(target_os = "macos")]
    fn signed_bundle(dir: &Path, identifier: &str) -> PathBuf {
        let app = dir.join(format!("{identifier}.app"));
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::copy("/usr/bin/true", app.join("Contents/MacOS/kchess")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{identifier}</string><key>CFBundleExecutable</key><string>kchess</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>\n"
            ),
        )
        .unwrap();
        let status = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&app)
            .status()
            .unwrap();
        assert!(status.success());
        app
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn verifies_only_kchess_signed_by_the_bridges_team() {
        let dir = tempfile::tempdir().unwrap();
        let kchess = signed_bundle(dir.path(), APP_ID);
        let other = signed_bundle(dir.path(), "com.example.other");
        assert_eq!(team(&kchess), None);
        assert_eq!(verify(&kchess, None), Ok(()));
        assert!(verify(&other, None).is_err());
        assert!(
            verify(&kchess, Some("4NBN7U2PZL")).is_err(),
            "an ad hoc signature is not the team's"
        );
        std::fs::write(kchess.join("Contents/MacOS/kchess"), "changed").unwrap();
        assert!(verify(&kchess, None).is_err(), "a changed app");
    }
}
