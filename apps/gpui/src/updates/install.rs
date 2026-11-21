//! Where the app is installed and how an update replaces it: the bundle or AppImage swapped in,
//! the installer or the package manager run, on restart or once KChess quits, and what is left
//! tidied the next time it starts.

use super::*;

/// How the running app was installed, if it was.
pub(super) fn installed() -> Option<Install> {
    let appimage = kchess_host::env::var_os("APPIMAGE").map(PathBuf::from);
    detect(
        std::env::consts::OS,
        &std::env::current_exe().ok()?,
        appimage,
    )
}

/// Where 2026.10.18's Electron bridge left KChess: ShipIt names the folder it installs after the
/// update's executable, which was the bridge's, so Spotlight found no KChess.
pub(super) const BRIDGE_FOLDER: &str = "kchess-electron-bridge.app";

/// Move an installation in the bridge's folder to `KChess.app` and reopen it from there; whether
/// this process should exit before it starts anything.
pub fn leave_bridge_folder() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    let Some(Install {
        package: Package::MacZip,
        path,
    }) = installed()
    else {
        return false;
    };
    match rename_bridge_folder(&path) {
        Ok(Some(moved)) => {
            log(&format!("moved {} to {}", path.display(), moved.display()));
            reopen_after_exit(&moved);
            true
        }
        Ok(None) => false,
        Err(cause) => {
            log(&format!("{} stays where it is ({cause})", path.display()));
            false
        }
    }
}

/// The new place of `bundle` when it was the bridge's folder, never over a `KChess.app` beside it.
pub(super) fn rename_bridge_folder(bundle: &Path) -> std::io::Result<Option<PathBuf>> {
    if bundle.file_name() != Some(BRIDGE_FOLDER.as_ref()) {
        return Ok(None);
    }
    let target = bundle.with_file_name(kchess_delta::installed::MAC_BUNDLE);
    if target.exists() {
        return Ok(None);
    }
    std::fs::rename(bundle, &target)?;
    Ok(Some(target))
}

/// The installation `exe` runs from on `os`. Only a package counts: a `.app` bundle; a folder
/// the Windows installer made (its uninstaller beside the app); an AppImage (`$APPIMAGE`, set by
/// the AppImage itself); or the .deb's folder under `/opt`. Never a build's `target`.
pub(super) fn detect(os: &str, exe: &Path, appimage: Option<PathBuf>) -> Option<Install> {
    let built = exe.components().any(|part| part.as_os_str() == "target");
    match os {
        "macos" => exe
            .ancestors()
            .nth(3)
            .filter(|bundle| bundle.extension().is_some_and(|ext| ext == "app"))
            .map(|bundle| Install {
                package: Package::MacZip,
                path: bundle.to_path_buf(),
            }),
        "windows" => {
            let folder = exe.parent()?;
            let uninstaller = std::fs::read_dir(folder)
                .ok()?
                .filter_map(Result::ok)
                .any(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    name.starts_with("Uninstall ") && name.ends_with(".exe")
                });
            (!built && uninstaller).then(|| Install {
                package: Package::WindowsInstaller,
                path: folder.to_path_buf(),
            })
        }
        "linux" => match appimage.filter(|path| path.is_file()) {
            Some(path) => Some(Install {
                package: Package::AppImage,
                path,
            }),
            None => (!built && exe.starts_with("/opt")).then(|| Install {
                package: Package::Deb,
                path: exe.to_path_buf(),
            }),
        },
        _ => None,
    }
}

/// Put the staged update in place of `install`, which `lock` holds; with `reopen`, open the new
/// KChess once this one has quit.
#[allow(
    clippy::disallowed_methods,
    reason = "off the UI thread (`on_runtime`), or at quit once the windows are closed; allowed, not expected, since some platforms' builds have no such call"
)]
pub(super) fn apply(
    staged: &Path,
    install: &Install,
    lock: &InstallLock,
    reopen: bool,
) -> Result<(), String> {
    match install.package {
        Package::MacZip => {
            replace(staged, &install.path)?;
            if reopen {
                reopen_after_exit(&install.path);
            }
        }
        Package::WindowsInstaller => {
            // Silently over this installation, which it closes, so
            // only once this app has saved and closed its core (`apply_on_quit`).
            let mut installer = Command::new(staged);
            // It runs on after this app exits: the installation stays locked until it is done.
            installer.args(["--updated", "/S"]).stdin(lock.inherited()?);
            if reopen {
                installer.arg("--force-run");
            }
            installer
                .spawn()
                .map_err(|cause| format!("The installer could not start ({cause})"))?;
        }
        Package::AppImage => {
            swap_file(staged, &install.path)?;
            if reopen {
                reopen_after_exit(&install.path);
            }
        }
        Package::Deb => {
            let status = Command::new("pkexec")
                .args(["dpkg", "-i"])
                .arg(staged)
                .status()
                .map_err(|_| {
                    "KChess could not ask for permission to install. Install the .deb with your package manager.".to_string()
                })?;
            if !status.success() {
                return Err("The update was not installed".into());
            }
            if reopen {
                reopen_after_exit(&install.path);
            }
        }
    }
    Ok(())
}

/// Where earlier releases kept the old bundle while the new one went in place.
pub(super) fn aside(installed: &Path) -> PathBuf {
    installed.with_file_name(format!(
        ".{}-old",
        installed
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    ))
}

/// Where `replace` puts the new bundle, and `swap_file` the new AppImage, before it takes the old
/// one's name.
pub(super) fn incoming(installed: &Path) -> PathBuf {
    installed.with_file_name(format!(
        ".{}.new",
        installed
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    ))
}

/// Put `staged` where the running app is. The new bundle is first put whole beside the installed
/// one, on its disk; then the two trade places in one step, so an interruption at any point leaves
/// a complete app where KChess opens from. What is left beside it goes on the next start (`tidy`).
pub(super) fn replace(staged: &Path, installed: &Path) -> Result<(), String> {
    replace_by(staged, installed, exchange)
}

/// `replace`, trading places with `exchange`. Where the disk cannot exchange two paths in one
/// step, the installed app stays as it is: moving it aside first would leave nothing where KChess
/// opens from if the app stopped between the two moves.
pub(super) fn replace_by(
    staged: &Path,
    installed: &Path,
    exchange: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    let incoming = incoming(installed);
    let fail = |cause: std::io::Error| format!("The update could not be installed ({cause})");
    std::fs::remove_dir_all(&incoming)
        .ok_if_missing()
        .warn_on_err();
    if std::fs::rename(staged, &incoming).is_err() {
        copy_tree(staged, &incoming).map_err(|cause| {
            std::fs::remove_dir_all(&incoming)
                .ok_if_missing()
                .warn_on_err();
            fail(cause)
        })?;
    }
    let exchanged = exchange(&incoming, installed);
    // `incoming` now holds the old bundle, or (not exchanged) the new one: neither is needed.
    std::fs::remove_dir_all(&incoming)
        .ok_if_missing()
        .warn_on_err();
    exchanged.map_err(|cause| {
        format!("KChess could not replace itself ({cause}). Install the update by hand.")
    })
}

/// Swap the entries at `a` and `b` in one step (`renamex_np` with `RENAME_SWAP`).
#[cfg(target_os = "macos")]
pub(super) fn exchange(a: &Path, b: &Path) -> std::io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, a, CWD, b, RenameFlags::EXCHANGE).map_err(std::io::Error::from)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn exchange(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Replace the file at `installed` with `staged`, executable, in one rename on the same disk.
pub(super) fn swap_file(staged: &Path, installed: &Path) -> Result<(), String> {
    let incoming = incoming(installed);
    let fail = |cause: std::io::Error| format!("The update could not be installed ({cause})");
    std::fs::copy(staged, &incoming).map_err(fail)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&incoming, std::fs::Permissions::from_mode(0o755))
            .map_err(fail)?;
    }
    std::fs::rename(&incoming, installed).map_err(|cause| {
        std::fs::remove_file(&incoming)
            .ok_if_missing()
            .warn_on_err();
        fail(cause)
    })
}

pub(super) fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Open the app at `target` once this process has exited, so the two never run together.
pub(super) fn reopen_after_exit(target: &Path) {
    let open = if cfg!(target_os = "macos") {
        "open -n \"$0\""
    } else {
        "exec \"$0\""
    };
    let result = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; {open}"
        ))
        .arg(target)
        .arg(std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(cause) = result {
        log(&format!("relaunch failed: {cause}"));
    }
}

/// What a KChess process, or the helper installing after it quits, holds while an update goes in
/// place of an installation: one installer at a time for each installation, across every KChess
/// process, and no start tidies what an installer is still using. In the downloads root, beside
/// the packages a helper installs from (`INSTALLING`).
pub(super) struct InstallLock {
    pub(super) held: std::fs::File,
}

/// Names in the downloads root that belong to installations, not to running apps' folders.
pub(super) const INSTALLING: &str = ".install-";

impl InstallLock {
    /// The lock file for `installed`, by its canonical path.
    pub(super) fn path(root: &Path, installed: &Path) -> PathBuf {
        let installed = std::fs::canonicalize(installed).unwrap_or_else(|_| installed.into());
        let digest = Sha256::digest(installed.as_os_str().as_encoded_bytes());
        let name: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        root.join(format!("{INSTALLING}{name}.lock"))
    }

    /// Take the installation, or say another KChess is installing into it.
    pub(super) fn take(root: &Path, installed: &Path) -> Result<Self, String> {
        let file = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(Self::path(root, installed))
            .map_err(|cause| format!("The update could not be installed ({cause})"))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { held: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(
                "Another KChess is installing an update. Try again once it has finished.".into(),
            ),
            Err(std::fs::TryLockError::Error(cause)) => {
                Err(format!("The update could not be installed ({cause})."))
            }
        }
    }

    /// The lock for a process that finishes the installation after this app exits, as its
    /// standard input: the installation stays locked, with no moment free for another KChess to
    /// take it, until that process (and this app) has closed it (`File::lock`).
    pub(super) fn inherited(&self) -> Result<Stdio, String> {
        self.held
            .try_clone()
            .map(Stdio::from)
            .map_err(|cause| format!("The update could not be installed ({cause})"))
    }
}

/// Remove the folders of apps no longer running: their lock is free (or they left no lock).
/// Installations' locks and packages are their installers' (`InstallLock`).
pub(super) fn clear_stopped(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".lock" || name == KEPT || name.starts_with(INSTALLING) {
            continue;
        }
        if let Some(folder) = name.strip_suffix(".lock") {
            let Ok(lock) = std::fs::File::options().write(true).open(&path) else {
                continue;
            };
            if lock.try_lock().is_ok() {
                std::fs::remove_dir_all(root.join(folder))
                    .ok_if_missing()
                    .warn_on_err();
                drop(lock);
                std::fs::remove_file(&path).ok_if_missing().warn_on_err();
            }
        } else if !root.join(format!("{name}.lock")).exists() {
            std::fs::remove_dir_all(&path).ok_if_missing().warn_on_err();
        }
    }
}

/// What an interrupted install left beside the app: the old bundle, or a new one not yet in place.
/// Only while no KChess is installing into it (`InstallLock`): a running installer's new copy stays.
pub(super) fn tidy(install: Option<&Install>, root: Option<&Path>) {
    let _installing = match (install, root) {
        (Some(install), Some(root)) => match InstallLock::take(root, &install.path) {
            Ok(lock) => Some(lock),
            Err(_) => return,
        },
        _ => None,
    };
    match install {
        Some(Install {
            package: Package::MacZip,
            path,
        }) => {
            std::fs::remove_dir_all(aside(path))
                .ok_if_missing()
                .warn_on_err();
            std::fs::remove_dir_all(incoming(path))
                .ok_if_missing()
                .warn_on_err();
        }
        Some(Install {
            package: Package::AppImage,
            path,
        }) => {
            std::fs::remove_file(incoming(path))
                .ok_if_missing()
                .warn_on_err();
        }
        Some(Install {
            package: Package::Deb,
            path,
        }) => {
            // What a helper stopped before it finished left: its package, a half-written outcome.
            if let Some(root) = root {
                let held = InstallLock::path(root, path);
                for left in ["deb", "outcome.log", "outcome.new"] {
                    std::fs::remove_file(held.with_extension(left))
                        .ok_if_missing()
                        .warn_on_err();
                }
            }
        }
        _ => {}
    }
}

/// Install once the saves are done and the core has closed: on quit, or (`reopen`) to restart into
/// the update. The installation is this app's (`InstallLock`) until it exits, and stays locked
/// by the Windows installer, or the helper running the package manager (which asks for a
/// password), until it is done.
pub(super) fn apply_on_quit(staged: &Path, install: &Install, reopen: bool) -> Result<(), String> {
    let root = downloads_root()?;
    let lock = InstallLock::take(&root, &install.path)?;
    if install.package != Package::Deb {
        apply(staged, install, &lock, reopen)?;
    } else {
        deb_helper(
            &root,
            staged,
            install,
            &lock,
            std::process::id(),
            "pkexec".as_ref(),
        )?;
    }
    // Held until this process exits; the installer or helper holds it from the start.
    std::mem::forget(lock);
    Ok(())
}

/// Hand the staged .deb and the installation's `lock` to a helper that installs it with
/// `elevate dpkg -i` once process `after` (this app) has exited, and records how that went for
/// the next start (`last_install_error`).
pub(super) fn deb_helper(
    root: &Path,
    staged: &Path,
    install: &Install,
    lock: &InstallLock,
    after: u32,
    elevate: &std::ffi::OsStr,
) -> Result<std::process::Child, String> {
    let held = InstallLock::path(root, &install.path);
    let package = held.with_extension("deb");
    // Out of this app's download folder, which the next start may clear: the helper's now.
    std::fs::rename(staged, &package)
        .map_err(|cause| format!("The update could not be installed ({cause})"))?;
    let helper = Command::new("sh")
        .arg("-c")
        .arg(DEB_HELPER)
        .arg("kchess-update")
        .arg(&package)
        .arg(after.to_string())
        .arg(held.with_extension(OUTCOME))
        .arg(elevate)
        .stdin(lock.inherited()?)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    helper.map_err(|cause| {
        std::fs::remove_file(&package).ok_if_missing().warn_on_err();
        format!("The update could not be installed ({cause}).")
    })
}

/// The extension of the file a .deb helper records its outcome in, beside the installation's lock.
pub(super) const OUTCOME: &str = "outcome";

/// Installs the .deb (`$1`) with `$4 dpkg -i` once KChess (`$2`) has exited, and records the exit
/// status and what the package manager said (`$3`) for the next start. Its standard input is the
/// installation's lock (`InstallLock::inherited`), kept on descriptor 9 until the package
/// manager is done, the outcome recorded and the package removed.
pub(super) const DEB_HELPER: &str = r#"exec 9<&0 0</dev/null
while kill -0 "$2" 2>/dev/null; do sleep 0.2; done
"$4" dpkg -i "$1" 9<&- >/dev/null 2>"$3.log"
echo $? > "$3.new"
tail -c 2000 "$3.log" >> "$3.new"
mv -f "$3.new" "$3"
rm -f "$1" "$3.log""#;

/// `pkexec`'s status when the person dismissed its password prompt.
pub(super) const DISMISSED: &str = "126";

/// What the helper installing a .deb on quit left for this start: the error to show when the
/// package manager failed. An update installed, or a password prompt dismissed, shows nothing.
/// A helper still running records its outcome for a later start.
pub(super) fn last_install_error(install: Option<&Install>, root: Option<&Path>) -> Option<String> {
    let (Some(install), Some(root)) = (install, root) else {
        return None;
    };
    let held = InstallLock::path(root, &install.path);
    // What the helpers of earlier versions left: a failure, without its cause.
    if std::fs::remove_file(held.with_extension("failed")).is_ok() {
        log("the last update was not installed");
        return Some("The last update was not installed".into());
    }
    let outcome = held.with_extension(OUTCOME);
    let recorded = std::fs::read_to_string(&outcome).ok()?;
    std::fs::remove_file(&outcome).ok_if_missing().warn_on_err();
    let (status, said) = recorded.split_once('\n').unwrap_or((&recorded, ""));
    match status.trim() {
        "0" => {
            log("the update was installed");
            None
        }
        DISMISSED => {
            log("the update was not installed: permission was not given");
            None
        }
        status => {
            log(&format!(
                "the update was not installed (status {status}): {}",
                said.trim()
            ));
            Some("The last update was not installed".into())
        }
    }
}
