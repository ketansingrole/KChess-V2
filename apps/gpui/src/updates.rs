//! App updates: the GPUI app checks the release feed
//! the Electron app reads (GitHub releases of KChess), downloads the package it was installed from
//! (packaged in Electron's formats), checks it against the release's
//! `SHA256SUMS.txt` and against the running app's signature, and installs it on restart or when
//! KChess quits: a new bundle on macOS, the installer run silently on Windows (as Electron's
//! updater does), a new AppImage, or the .deb through the package manager.
//!
//! Only an installed package updates: a development build reports updates as manual, unless
//! `KCHESS_UPDATE_FEED` points it at another feed (tests serve one locally).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use futures::StreamExt as _;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use kchess_core::Level;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::settings::{SettingsChanged, SettingsHandle};

const REPOSITORY: &str = "ketansingrole/KChess-V2";
const FEED: &str = "https://api.github.com";
/// How long a download waits for the server's next bytes before it counts as stopped.
const IDLE: Duration = if cfg!(test) {
    Duration::from_millis(500)
} else {
    Duration::from_secs(30)
};
/// `checkAutomatically`: on startup and every six hours.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The name KChess's packages carry, as the Electron app's did.
const PREFIX: &str = "KChess";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Downloaded,
    /// Putting the update in place; then KChess quits.
    Installing,
    Error,
    /// Updates are not installed here (a development build).
    Disabled,
}

/// The kind of package KChess was installed from, which is the kind its updates come in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Package {
    /// The macOS .zip (the .dmg holds the same bundle): its `.app` takes the running one's place.
    MacZip,
    /// The Windows per-user installer, run silently over the installation.
    WindowsInstaller,
    /// The Linux AppImage: the new file replaces the running one.
    AppImage,
    /// The Linux .deb, installed by the package manager.
    Deb,
}

impl Package {
    /// What a build for this platform updates from when it is not installed (a test feed).
    fn for_platform() -> Option<Self> {
        match std::env::consts::OS {
            "macos" => Some(Self::MacZip),
            "windows" => Some(Self::WindowsInstaller),
            "linux" => Some(Self::AppImage),
            _ => None,
        }
    }
}

/// Where the running app is installed, and from which kind of package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub package: Package,
    /// The `.app` bundle, the Windows install folder, the AppImage file, or the .deb's executable.
    pub path: PathBuf,
}

#[derive(Clone, Deserialize)]
pub struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

#[derive(Clone, Deserialize)]
pub struct Release {
    tag_name: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

pub struct Updates {
    pub phase: Phase,
    pub current: String,
    /// The newer release, when one is known.
    pub version: Option<String>,
    pub released: Option<String>,
    pub checked_at: Option<i64>,
    /// Bytes received and expected while downloading.
    pub progress: Option<(u64, u64)>,
    pub error: Option<String>,
    release: Option<Release>,
    /// The checked update, ready to take this app's place: the new bundle on macOS, else the
    /// downloaded package.
    staged: Option<PathBuf>,
    /// Install the staged update as the app quits and reopen it (`restart`).
    restart: bool,
    /// How this copy was installed; none for a build run from where it was made.
    install: Option<Install>,
    /// This app's own download folder; none where updates are not checked.
    workspace: Option<Workspace>,
    feed: Option<String>,
    work: Option<Task<()>>,
    _timer: Option<Task<()>>,
}

/// How views reach updates: `cx.global::<UpdatesHandle>().0`.
#[derive(Clone)]
pub struct UpdatesHandle(pub Entity<Updates>);

impl Global for UpdatesHandle {}

/// `vYYYY.M.C` or `YYYY.M.C` as numbers, to compare releases.
pub fn parse_version(text: &str) -> Option<[u64; 3]> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut out = [0; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// This app's name in version labels, so a report says which KChess app it came from.
pub const CLIENT: &str = "GPUI";

/// `2026.10.3 · GPUI`: the running version as the Updates heading and palette show it.
pub fn client_version(version: &str) -> String {
    format!("{version} · {CLIENT}")
}

/// The release's own version (the workspace version, the one releases are tagged with; `build.rs`).
pub fn current_version() -> String {
    env!("KCHESS_VERSION").to_string()
}

/// This platform's `package` of a release, named as electron-builder names Electron's
/// (`KChess-2026.10.12-linux-x86_64.AppImage`, `…-linux-amd64.deb`).
pub fn asset_name(version: &str, package: Package) -> Option<String> {
    let arm = match std::env::consts::ARCH {
        "aarch64" => true,
        "x86_64" => false,
        _ => return None,
    };
    let (platform, arch, extension) = match package {
        Package::MacZip => ("mac", if arm { "arm64" } else { "x64" }, "zip"),
        Package::WindowsInstaller => ("win", if arm { "arm64" } else { "x64" }, "exe"),
        Package::AppImage => ("linux", if arm { "arm64" } else { "x86_64" }, "AppImage"),
        Package::Deb => ("linux", if arm { "arm64" } else { "amd64" }, "deb"),
    };
    Some(format!("{PREFIX}-{version}-{platform}-{arch}.{extension}"))
}

/// The SHA-256 a `SHA256SUMS.txt` lists for `name`.
pub fn digest_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (digest, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| digest.to_ascii_lowercase())
    })
}

/// The release to offer, with its version: one newer than `current` that has `package`.
pub fn offer(release: &Release, current: &str, package: Option<Package>) -> Option<String> {
    let version = release.tag_name.trim_start_matches('v');
    let newer = parse_version(version)
        .zip(parse_version(current))
        .is_some_and(|(latest, current)| latest > current);
    let name = asset_name(version, package?)?;
    (newer && release.assets.iter().any(|asset| asset.name == name)).then(|| version.to_string())
}

/// How the running app was installed, if it was.
fn installed() -> Option<Install> {
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    detect(
        std::env::consts::OS,
        &std::env::current_exe().ok()?,
        appimage,
    )
}

/// Where 2026.10.18's Electron bridge left KChess: ShipIt names the folder it installs after the
/// update's executable, which was the bridge's, so Spotlight found no KChess.
const BRIDGE_FOLDER: &str = "kchess-electron-bridge.app";

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
fn rename_bridge_folder(bundle: &Path) -> std::io::Result<Option<PathBuf>> {
    if bundle.file_name() != Some(BRIDGE_FOLDER.as_ref()) {
        return Ok(None);
    }
    let target = bundle.with_file_name("KChess.app");
    if target.exists() {
        return Ok(None);
    }
    std::fs::rename(bundle, &target)?;
    Ok(Some(target))
}

/// The installation `exe` runs from on `os`. Only a package counts: a `.app` bundle; a folder
/// the Windows installer made (its uninstaller beside the app); an AppImage (`$APPIMAGE`, set by
/// the AppImage itself); or the .deb's folder under `/opt`. Never a build's `target`.
fn detect(os: &str, exe: &Path, appimage: Option<PathBuf>) -> Option<Install> {
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

fn log(message: &str) {
    crate::log::write(Level::Info, "updates", message);
}

/// Run `work` on a Tokio runtime of its own, off the UI thread.
fn on_runtime<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> futures::channel::oneshot::Receiver<T> {
    let (done, result) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        if let Ok(runtime) = runtime {
            let _ = done.send(runtime.block_on(work));
        }
    });
    result
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(kchess_core::USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        // A server that stops sending ends the download instead of holding it forever.
        .read_timeout(IDLE)
        .build()
        .map_err(|cause| cause.to_string())
}

async fn latest(feed: String) -> Result<Release, String> {
    let url = format!("{feed}/repos/{REPOSITORY}/releases/latest");
    let response = http()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "Could not reach the update server.".to_string())?;
    if !response.status().is_success() {
        return Err(format!("The update server answered {}.", response.status()));
    }
    response
        .json::<Release>()
        .await
        .map_err(|_| "The update server's answer could not be read.".to_string())
}

/// Download `asset` into `folder` and check it against the release's digest; the downloaded
/// file. `progress` hears the bytes received so far.
async fn download(
    release: Release,
    asset: Asset,
    folder: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
) -> Result<PathBuf, String> {
    let client = http()?;
    let sums = release
        .assets
        .iter()
        .find(|entry| entry.name == "SHA256SUMS.txt")
        .ok_or("The release lists no checksums.")?;
    let sums = client
        .get(&sums.browser_download_url)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| "Could not download the checksums.".to_string())?
        .text()
        .await
        .map_err(|_| "Could not download the checksums.".to_string())?;
    let expected =
        digest_for(&sums, &asset.name).ok_or("The release lists no checksum for this package.")?;
    let response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| "Could not download the update.".to_string())?;
    let total = response.content_length().unwrap_or(asset.size);
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).map_err(|cause| cause.to_string())?;
    let file_path = folder.join(&asset.name);
    let mut file = tokio::fs::File::create(&file_path)
        .await
        .map_err(|cause| cause.to_string())?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|_| "The download stopped.".to_string())?;
        hasher.update(&chunk);
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|cause| cause.to_string())?;
        received += chunk.len() as u64;
        let _ = progress.unbounded_send((received, total));
    }
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|cause| cause.to_string())?;
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != expected {
        let _ = std::fs::remove_dir_all(&folder);
        return Err("The downloaded update did not match its checksum.".into());
    }
    Ok(file_path)
}

/// Unpack a macOS update's zip beside it; the `.app` it holds.
fn unpack(archive: &Path) -> Result<PathBuf, String> {
    let unpacked = archive.with_file_name("app");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(|cause| cause.to_string())?;
    let status = Command::new("ditto")
        .args(["-x", "-k"])
        .arg(archive)
        .arg(&unpacked)
        .status();
    if !status.is_ok_and(|status| status.success()) {
        return Err("The update could not be unpacked.".into());
    }
    let _ = std::fs::remove_file(archive);
    std::fs::read_dir(&unpacked)
        .map_err(|cause| cause.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .ok_or_else(|| "The update holds no app.".into())
}

/// Who signed a program: whether its signature holds, the signer (a macOS team or a Windows
/// certificate's subject; none when ad hoc or unsigned) and, on macOS, the app's identifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Signer {
    valid: bool,
    identity: Option<String>,
    app: Option<String>,
}

/// An update must be signed as well as the app it replaces, by the same signer, as the same app.
/// An unsigned app (a Linux package, or a build without credentials) relies on the checksum.
fn signed_alike(running: &Signer, update: &Signer) -> Result<(), String> {
    if running.valid && !update.valid {
        return Err("The update's signature is not valid.".into());
    }
    if running.identity.is_some() && update.identity != running.identity {
        return Err("The update is not signed by KChess's developer.".into());
    }
    if running.app.is_some() && update.app != running.app {
        return Err("The update is not KChess.".into());
    }
    Ok(())
}

/// The signature of `path`, a bundle or program of `package`'s kind.
fn signer(package: Package, path: &Path) -> Signer {
    match package {
        Package::MacZip => {
            let valid = Command::new("codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(path)
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            let details = Command::new("codesign")
                .args(["--display", "--verbose=2"])
                .arg(path)
                .output()
                .map(|output| String::from_utf8_lossy(&output.stderr).to_string())
                .unwrap_or_default();
            let field = |key: &str| {
                details
                    .lines()
                    .find_map(|line| line.strip_prefix(key))
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && *value != "not set")
                    .map(str::to_string)
            };
            Signer {
                valid,
                identity: field("TeamIdentifier="),
                app: field("Identifier="),
            }
        }
        Package::WindowsInstaller => {
            // The path goes by the environment, never into the script's text.
            let output = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$s = Get-AuthenticodeSignature -LiteralPath $env:KCHESS_SIGNED; \
                     \"$($s.Status)`n$($s.SignerCertificate.Subject)\"",
                ])
                .env("KCHESS_SIGNED", path)
                .output()
                .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                .unwrap_or_default();
            let mut lines = output.lines().map(str::trim);
            let valid = lines.next() == Some("Valid");
            Signer {
                valid,
                identity: lines
                    .next()
                    .filter(|subject| valid && !subject.is_empty())
                    .map(str::to_string),
                app: None,
            }
        }
        Package::AppImage | Package::Deb => Signer::default(),
    }
}

/// The running app's signature, which an update for `install` must share.
fn running_signer(install: &Install) -> Signer {
    match install.package {
        Package::WindowsInstaller => std::env::current_exe()
            .map(|exe| signer(install.package, &exe))
            .unwrap_or_default(),
        package => signer(package, &install.path),
    }
}

/// Download, check and stage `asset` (of `package`) in `folder`: the new bundle on macOS, else
/// the package itself. With `install` known, the update must be signed as the running app is.
async fn fetch(
    release: Release,
    asset: Asset,
    package: Package,
    install: Option<Install>,
    folder: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
) -> Result<PathBuf, String> {
    let file = download(release, asset, folder, progress).await?;
    let staged = match package {
        Package::MacZip => unpack(&file)?,
        _ => file,
    };
    if let Some(install) = install {
        signed_alike(&running_signer(&install), &signer(package, &staged))?;
    }
    Ok(staged)
}

/// Put the staged update in place of `install`, which `lock` holds; with `reopen`, open the new
/// KChess once this one has quit.
fn apply(staged: &Path, install: &Install, lock: &InstallLock, reopen: bool) -> Result<(), String> {
    match install.package {
        Package::MacZip => {
            replace(staged, &install.path)?;
            if reopen {
                reopen_after_exit(&install.path);
            }
        }
        Package::WindowsInstaller => {
            // As Electron's updater runs it: silently over this installation, which it closes, so
            // only once this app has saved and closed its core (`apply_on_quit`).
            let mut installer = Command::new(staged);
            // It runs on after this app exits: the installation stays locked until it is done.
            installer.args(["--updated", "/S"]).stdin(lock.inherited()?);
            if reopen {
                installer.arg("--force-run");
            }
            installer
                .spawn()
                .map_err(|cause| format!("The installer could not start ({cause})."))?;
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
                return Err("The update was not installed.".into());
            }
            if reopen {
                reopen_after_exit(&install.path);
            }
        }
    }
    Ok(())
}

/// Where earlier releases kept the old bundle while the new one went in place.
fn aside(installed: &Path) -> PathBuf {
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
fn incoming(installed: &Path) -> PathBuf {
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
fn replace(staged: &Path, installed: &Path) -> Result<(), String> {
    replace_by(staged, installed, exchange)
}

/// `replace`, trading places with `exchange`. Where the disk cannot exchange two paths in one
/// step, the installed app stays as it is: moving it aside first would leave nothing where KChess
/// opens from if the app stopped between the two moves.
fn replace_by(
    staged: &Path,
    installed: &Path,
    exchange: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    let incoming = incoming(installed);
    let fail = |cause: std::io::Error| format!("The update could not be installed ({cause}).");
    let _ = std::fs::remove_dir_all(&incoming);
    if std::fs::rename(staged, &incoming).is_err() {
        copy_tree(staged, &incoming).map_err(|cause| {
            let _ = std::fs::remove_dir_all(&incoming);
            fail(cause)
        })?;
    }
    let exchanged = exchange(&incoming, installed);
    // `incoming` now holds the old bundle, or (not exchanged) the new one: neither is needed.
    let _ = std::fs::remove_dir_all(&incoming);
    exchanged.map_err(|cause| {
        format!("KChess could not replace itself ({cause}). Install the update by hand.")
    })
}

/// Swap the entries at `a` and `b` in one step (`renamex_np` with `RENAME_SWAP`).
#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "renamex_np is the only atomic exchange of two paths on macOS"
)]
fn exchange(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    let path = |path: &Path| {
        std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
    };
    let (a, b) = (path(a)?, path(b)?);
    // SAFETY: both are valid NUL-terminated paths that outlive the call.
    match unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

#[cfg(not(target_os = "macos"))]
fn exchange(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Replace the file at `installed` with `staged`, executable, in one rename on the same disk.
fn swap_file(staged: &Path, installed: &Path) -> Result<(), String> {
    let incoming = incoming(installed);
    let fail = |cause: std::io::Error| format!("The update could not be installed ({cause}).");
    std::fs::copy(staged, &incoming).map_err(fail)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&incoming, std::fs::Permissions::from_mode(0o755))
            .map_err(fail)?;
    }
    std::fs::rename(&incoming, installed).map_err(|cause| {
        let _ = std::fs::remove_file(&incoming);
        fail(cause)
    })
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
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
fn reopen_after_exit(target: &Path) {
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

/// Where this identity's downloads go: a folder of this user's, never the shared temporary folder,
/// where another user could replace a checked package before it is installed.
fn downloads_root() -> Result<PathBuf, String> {
    let cache = dirs::cache_dir().ok_or("KChess has no folder to download updates to.")?;
    downloads_in(&cache)
}

/// `kchess-update/PREFIX` in `cache`, each folder private to this user.
fn downloads_in(cache: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(cache).map_err(|cause| cause.to_string())?;
    let parent = cache.join("kchess-update");
    private_dir(&parent, cache)?;
    let root = parent.join(PREFIX);
    private_dir(&root, cache)?;
    Ok(root)
}

/// Make `dir` a folder only its owner can use, refusing a link or another user's folder (its
/// owner must be `owner`'s).
fn private_dir(dir: &Path, owner: &Path) -> Result<(), String> {
    let refused = || format!("The update folder {} is not private.", dir.display());
    match std::fs::create_dir(dir) {
        Ok(()) => {}
        Err(cause) if cause.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(cause) => return Err(cause.to_string()),
    }
    let found = std::fs::symlink_metadata(dir).map_err(|cause| cause.to_string())?;
    if !found.is_dir() {
        return Err(refused());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let user = std::fs::metadata(owner)
            .map_err(|cause| cause.to_string())?
            .uid();
        if found.uid() != user {
            return Err(refused());
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|cause| cause.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = owner;
    Ok(())
}

/// This app's download folder in the root: kept from every other KChess (another profile, a
/// development build) by a lock held while this app runs, and removed once it is not.
struct Workspace {
    path: PathBuf,
    _held: std::fs::File,
}

impl Workspace {
    fn open(root: &Path) -> Result<Self, String> {
        let fail = |cause: std::io::Error| cause.to_string();
        // One app at a time clears stopped apps' folders and claims its own.
        let guard = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(".lock"))
            .map_err(fail)?;
        guard.lock().map_err(fail)?;
        clear_stopped(root);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let name = format!("{}-{nanos}", std::process::id());
        let held = std::fs::File::create_new(root.join(format!("{name}.lock"))).map_err(fail)?;
        held.try_lock().map_err(|cause| cause.to_string())?;
        let path = root.join(name);
        std::fs::create_dir(&path).map_err(fail)?;
        Ok(Self { path, _held: held })
    }
}

/// What a KChess process, or the helper installing after it quits, holds while an update goes in
/// place of an installation: one installer at a time for each installation, across every KChess
/// process, and no start tidies what an installer is still using. In the downloads root, beside
/// the packages a helper installs from (`INSTALLING`).
struct InstallLock {
    held: std::fs::File,
}

/// Names in the downloads root that belong to installations, not to running apps' folders.
const INSTALLING: &str = ".install-";

impl InstallLock {
    /// The lock file for `installed`, by its canonical path.
    fn path(root: &Path, installed: &Path) -> PathBuf {
        let installed = std::fs::canonicalize(installed).unwrap_or_else(|_| installed.into());
        let digest = Sha256::digest(installed.as_os_str().as_encoded_bytes());
        let name: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        root.join(format!("{INSTALLING}{name}.lock"))
    }

    /// Take the installation, or say another KChess is installing into it.
    fn take(root: &Path, installed: &Path) -> Result<Self, String> {
        let file = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(Self::path(root, installed))
            .map_err(|cause| format!("The update could not be installed ({cause})."))?;
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
    fn inherited(&self) -> Result<Stdio, String> {
        self.held
            .try_clone()
            .map(Stdio::from)
            .map_err(|cause| format!("The update could not be installed ({cause})."))
    }
}

/// Remove the folders of apps no longer running: their lock is free (or they left no lock).
/// Installations' locks and packages are their installers' (`InstallLock`).
fn clear_stopped(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".lock" || name.starts_with(INSTALLING) {
            continue;
        }
        if let Some(folder) = name.strip_suffix(".lock") {
            let Ok(lock) = std::fs::File::options().write(true).open(&path) else {
                continue;
            };
            if lock.try_lock().is_ok() {
                let _ = std::fs::remove_dir_all(root.join(folder));
                drop(lock);
                let _ = std::fs::remove_file(&path);
            }
        } else if !root.join(format!("{name}.lock")).exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// What an interrupted install left beside the app: the old bundle, or a new one not yet in place.
/// Only while no KChess is installing into it (`InstallLock`): a running installer's new copy stays.
fn tidy(install: Option<&Install>, root: Option<&Path>) {
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
            let _ = std::fs::remove_dir_all(aside(path));
            let _ = std::fs::remove_dir_all(incoming(path));
        }
        Some(Install {
            package: Package::AppImage,
            path,
        }) => {
            let _ = std::fs::remove_file(incoming(path));
        }
        Some(Install {
            package: Package::Deb,
            path,
        }) => {
            // A helper that stopped before installing it.
            if let Some(root) = root {
                let _ = std::fs::remove_file(InstallLock::path(root, path).with_extension("deb"));
            }
        }
        _ => {}
    }
}

/// Install once the saves are done and the core has closed: on quit, or (`reopen`) to restart into
/// the update. The installation is this app's (`InstallLock`) until it exits, and stays locked
/// by the Windows installer, or the helper running the package manager (which asks for a
/// password), until it is done.
fn apply_on_quit(staged: &Path, install: &Install, reopen: bool) -> Result<(), String> {
    let root = downloads_root()?;
    let lock = InstallLock::take(&root, &install.path)?;
    if install.package != Package::Deb {
        apply(staged, install, &lock, reopen)?;
        std::mem::forget(lock);
        return Ok(());
    }
    let held = InstallLock::path(&root, &install.path);
    let (package, failed) = (held.with_extension("deb"), held.with_extension("failed"));
    // Out of this app's download folder, which the next start may clear: the helper's now.
    std::fs::rename(staged, &package)
        .map_err(|cause| format!("The update could not be installed ({cause})."))?;
    let helper = Command::new("sh")
        .arg("-c")
        .arg(DEB_HELPER)
        .arg("kchess-update")
        .arg(&package)
        .arg(std::process::id().to_string())
        .arg(&failed)
        .stdin(lock.inherited()?)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(cause) = helper {
        let _ = std::fs::remove_file(&package);
        return Err(format!("The update could not be installed ({cause})."));
    }
    // Held until this process exits; the helper holds it from the start.
    std::mem::forget(lock);
    Ok(())
}

/// Installs the .deb (`$1`) once KChess (`$2`) has exited, and notes a failure (`$3`) for the next
/// start. Its standard input is the installation's lock (`InstallLock::inherited`), kept until
/// the package manager is done and the package removed.
const DEB_HELPER: &str = r#"exec 9<&0 0</dev/null
while kill -0 "$2" 2>/dev/null; do sleep 0.2; done
if pkexec dpkg -i "$1" 9<&-; then rm -f "$3"; else echo failed > "$3"; fi
rm -f "$1""#;

/// What the helper installing a .deb left for this start: whether it failed.
fn last_install_failed(install: Option<&Install>, root: Option<&Path>) -> bool {
    let (Some(install), Some(root)) = (install, root) else {
        return false;
    };
    let failed = InstallLock::path(root, &install.path).with_extension("failed");
    std::fs::remove_file(failed).is_ok()
}

pub fn init(cx: &mut App) {
    let install = (!cfg!(debug_assertions)).then(installed).flatten();
    let root = install.as_ref().and_then(|_| downloads_root().ok());
    tidy(install.as_ref(), root.as_deref());
    let failed = last_install_failed(install.as_ref(), root.as_deref());
    let feed = std::env::var("KCHESS_UPDATE_FEED")
        .ok()
        .filter(|feed| !feed.is_empty())
        .or_else(|| install.is_some().then(|| FEED.to_string()));
    let workspace = feed.as_ref().and_then(|_| {
        downloads_root()
            .and_then(|root| Workspace::open(&root))
            .inspect_err(|cause| log(&format!("no download folder: {cause}")))
            .ok()
    });
    let updates = cx.new(|cx: &mut Context<Updates>| {
        let settings = cx.global::<SettingsHandle>().0.clone();
        cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
            this.schedule(cx)
        })
        .detach();
        let mut updates = Updates {
            phase: if failed {
                Phase::Error
            } else if feed.is_some() {
                Phase::Idle
            } else {
                Phase::Disabled
            },
            current: current_version(),
            version: None,
            released: None,
            checked_at: None,
            progress: None,
            error: failed.then(|| "The last update was not installed.".to_string()),
            release: None,
            staged: None,
            restart: false,
            install,
            workspace,
            feed,
            work: None,
            _timer: None,
        };
        updates.schedule(cx);
        updates
    });
    // An update downloaded and waiting goes in when KChess quits, if the settings say so (or to
    // restart into it): only after every document is saved, since the installer closes the app.
    let quitting = updates.clone();
    crate::core::after_quit(cx, move |cx| {
        let on_quit = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .update_install_on_quit;
        let updates = quitting.read(cx);
        let reopen = updates.restart;
        let (true, Some(staged), Some(install)) = (
            on_quit || reopen,
            updates.staged.clone(),
            updates.install.clone(),
        ) else {
            return None;
        };
        Some(Box::new(move || {
            if let Err(cause) = apply_on_quit(&staged, &install, reopen) {
                log(&cause);
            }
        }))
    });
    cx.set_global(UpdatesHandle(updates));
}

impl Updates {
    pub fn can_check(&self) -> bool {
        self.feed.is_some()
    }

    /// The kind of package this app updates from.
    fn package(&self) -> Option<Package> {
        self.install
            .as_ref()
            .map(|install| install.package)
            .or_else(Package::for_platform)
    }

    /// Check on startup and every six hours while automatic checks are on.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        let auto = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .update_auto_check;
        if !auto || !self.can_check() {
            self._timer = None;
            return;
        }
        if self._timer.is_some() {
            return;
        }
        self._timer = Some(cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |this, cx| this.check(true, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(CHECK_EVERY).await;
            }
        }));
    }

    /// `check`: ask the feed for the latest release.
    pub fn check(&mut self, automatic: bool, cx: &mut Context<Self>) {
        let Some(feed) = self.feed.clone() else {
            return;
        };
        if matches!(
            self.phase,
            Phase::Checking | Phase::Downloading | Phase::Downloaded | Phase::Installing
        ) {
            return;
        }
        self.phase = Phase::Checking;
        self.error = None;
        cx.notify();
        let answer = on_runtime(latest(feed));
        self.work = Some(cx.spawn(async move |this, cx| {
            let answer = answer
                .await
                .unwrap_or_else(|_| Err("The check stopped.".into()));
            let _ = this.update(cx, |this, cx| {
                this.checked_at = Some(chrono::Utc::now().timestamp_millis());
                match answer {
                    Ok(release) => this.found(release, automatic, cx),
                    Err(cause) => {
                        log(&format!("check failed: {cause}"));
                        this.phase = Phase::Error;
                        this.error = Some(cause);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn found(&mut self, release: Release, automatic: bool, cx: &mut Context<Self>) {
        let Some(version) = offer(&release, &self.current, self.package()) else {
            self.phase = Phase::UpToDate;
            self.version = None;
            self.release = None;
            return;
        };
        self.phase = Phase::Available;
        self.version = Some(version);
        self.released = release.published_at.clone();
        self.release = Some(release);
        let auto_download = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .update_auto_download;
        if automatic && auto_download {
            self.download(cx);
        }
    }

    /// `download`: fetch, check and stage the package this app was installed from.
    pub fn download(&mut self, cx: &mut Context<Self>) {
        let (Some(release), Some(package)) = (self.release.clone(), self.package()) else {
            return;
        };
        if !matches!(self.phase, Phase::Available | Phase::Error) {
            return;
        }
        let version = release.tag_name.trim_start_matches('v').to_string();
        let Some(asset) = asset_name(&version, package).and_then(|name| {
            release
                .assets
                .iter()
                .find(|asset| asset.name == name)
                .cloned()
        }) else {
            return;
        };
        self.phase = Phase::Downloading;
        self.error = None;
        self.progress = None;
        cx.notify();
        let Some(workspace) = self.workspace.as_ref() else {
            self.phase = Phase::Error;
            self.error = Some("KChess has no private folder to download the update to.".into());
            cx.notify();
            return;
        };
        let folder = workspace.path.join(&version);
        let (sender, mut heard) = futures::channel::mpsc::unbounded();
        let done = on_runtime(fetch(
            release,
            asset,
            package,
            self.install.clone(),
            folder,
            sender,
        ));
        self.work = Some(cx.spawn(async move |this, cx| {
            while let Some(progress) = heard.next().await {
                let _ = this.update(cx, |this, cx| {
                    this.progress = Some(progress);
                    cx.notify();
                });
            }
            let done = done
                .await
                .unwrap_or_else(|_| Err("The download stopped.".into()));
            let _ = this.update(cx, |this, cx| {
                match done {
                    Ok(staged) => {
                        log(&format!("staged {}", staged.display()));
                        this.staged = Some(staged);
                        this.phase = Phase::Downloaded;
                    }
                    Err(cause) => {
                        log(&format!("download failed: {cause}"));
                        this.phase = Phase::Error;
                        this.error = Some(cause);
                    }
                }
                cx.notify();
            });
        }));
    }

    /// `install`: save everything, then put the downloaded update in place of this app and reopen
    /// it. One install at a time: it takes the staged update, so neither a second request nor
    /// quitting installs it again while this one runs.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        if self.phase == Phase::Installing || self.staged.is_none() {
            return;
        }
        if self.install.is_none() {
            self.phase = Phase::Error;
            self.error = Some(
                "This copy of KChess is not an installed package; install the update by hand."
                    .into(),
            );
            cx.notify();
            return;
        }
        self.phase = Phase::Installing;
        self.error = None;
        cx.notify();
        let (saved, cancelled) = (cx.entity().downgrade(), cx.entity().downgrade());
        // Every document reaches the profile first, as on any quit: the installer may close
        // the app.
        crate::core::prepare_quit(
            cx,
            "Restart without saving",
            move |cx| {
                let _ = saved.update(cx, |this, cx| this.install_saved(cx));
            },
            move |cx| {
                let _ = cancelled.update(cx, |this, cx| {
                    this.phase = Phase::Downloaded;
                    cx.notify();
                });
            },
        );
    }

    /// `install`, once everything is saved.
    fn install_saved(&mut self, cx: &mut Context<Self>) {
        let (Some(install), Some(staged)) = (self.install.clone(), self.staged.clone()) else {
            return;
        };
        if install.package == Package::WindowsInstaller {
            // The installer closes this app: it starts as the app quits, after the core closes.
            self.restart = true;
            return cx.quit();
        }
        self.staged = None;
        // Off the UI thread: the package manager may wait for a password.
        let done = on_runtime(async move {
            let applied = downloads_root()
                .and_then(|root| InstallLock::take(&root, &install.path))
                .and_then(|installing| apply(&staged, &install, &installing, true));
            applied.map_err(|cause| (cause, staged))
        });
        self.work = Some(cx.spawn(async move |this, cx| {
            let done = done.await.unwrap_or_else(|_| {
                Err(("The update could not be installed.".into(), PathBuf::new()))
            });
            let _ = this.update(cx, |this, cx| match done {
                Ok(()) => cx.quit(),
                Err((cause, staged)) => {
                    log(&format!("install failed: {cause}"));
                    // Still whole where it was: it can be installed again.
                    if staged.exists() {
                        this.staged = Some(staged);
                    }
                    this.phase = Phase::Error;
                    this.error = Some(cause);
                    cx.notify();
                }
            });
        }));
    }

    /// The release's page, for its notes or a manual download.
    pub fn release_page(&self) -> String {
        self.release
            .as_ref()
            .and_then(|release| release.html_url.clone())
            .unwrap_or_else(|| format!("https://github.com/{REPOSITORY}/releases"))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BRIDGE_FOLDER, INSTALLING, Install, InstallLock, Package, Release, Signer, Workspace,
        aside, asset_name, detect, digest_for, download, downloads_in, incoming,
        last_install_failed, latest, offer, parse_version, rename_bridge_folder, replace_by,
        signed_alike, swap_file, tidy,
    };
    use sha2::{Digest, Sha256};
    use std::io::{Read as _, Write as _};
    use std::path::{Path, PathBuf};

    /// Held by tests that start processes and by those that check a lock is free once dropped: a
    /// process started meanwhile can briefly share another test's lock.
    static PROCESSES: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn processes() -> std::sync::MutexGuard<'static, ()> {
        PROCESSES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("kchess-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn the_bridge_folder_becomes_kchess_app() {
        let folder = scratch("bridge-folder");
        let bridge = folder.join(BRIDGE_FOLDER);
        std::fs::create_dir_all(bridge.join("Contents")).unwrap();
        assert_eq!(
            rename_bridge_folder(&bridge).unwrap(),
            Some(folder.join("KChess.app"))
        );
        assert!(folder.join("KChess.app/Contents").is_dir() && !bridge.exists());
        assert_eq!(
            rename_bridge_folder(&folder.join("KChess.app")).unwrap(),
            None
        );

        std::fs::create_dir_all(&bridge).unwrap();
        assert_eq!(
            rename_bridge_folder(&bridge).unwrap(),
            None,
            "never over KChess.app"
        );
        assert!(bridge.is_dir());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn only_a_package_updates_itself() {
        let bundle = Path::new("/Applications/KChess.app");
        assert_eq!(
            detect("macos", &bundle.join("Contents/MacOS/kchess"), None),
            Some(Install {
                package: Package::MacZip,
                path: bundle.to_path_buf()
            })
        );
        assert_eq!(
            detect("macos", Path::new("/repo/target/release/KChess"), None),
            None
        );

        let folder = scratch("detect");
        let installed = folder.join("Programs/kchess");
        std::fs::create_dir_all(&installed).unwrap();
        let exe = installed.join("KChess.exe");
        assert_eq!(
            detect("windows", &exe, None),
            None,
            "no uninstaller beside it"
        );
        std::fs::write(installed.join("Uninstall KChess.exe"), "").unwrap();
        assert_eq!(
            detect("windows", &exe, None),
            Some(Install {
                package: Package::WindowsInstaller,
                path: installed.clone()
            })
        );

        let appimage = folder.join("KChess.AppImage");
        std::fs::write(&appimage, "").unwrap();
        let mounted = Path::new("/tmp/.mount_KChess/kchess");
        assert_eq!(
            detect("linux", mounted, Some(appimage.clone())),
            Some(Install {
                package: Package::AppImage,
                path: appimage
            })
        );
        assert_eq!(
            detect("linux", Path::new("/opt/KChess/kchess"), None),
            Some(Install {
                package: Package::Deb,
                path: PathBuf::from("/opt/KChess/kchess")
            })
        );
        assert_eq!(
            detect("linux", Path::new("/home/me/kchess/kchess"), None),
            None
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Only macOS exchanges two paths in one step; elsewhere a bundle is never replaced.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_new_bundle_takes_the_old_ones_place() {
        let folder = scratch("replace");
        let (installed, staged) = (folder.join("KChess.app"), folder.join("new/KChess.app"));
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(installed.join("kchess"), "old").unwrap();
        std::fs::write(staged.join("kchess"), "new").unwrap();
        super::replace(&staged, &installed).unwrap();
        assert_eq!(
            std::fs::read_to_string(installed.join("kchess")).unwrap(),
            "new"
        );
        assert!(!folder.join(".KChess.app-old").exists());
        assert!(!incoming(&installed).exists());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn an_interrupted_replacement_leaves_the_installed_app_whole() {
        let folder = scratch("interrupted");
        let installed = folder.join("KChess.app");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("kchess"), "old").unwrap();
        // Stopped halfway through copying the new bundle beside the installed one.
        let partial = incoming(&installed);
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(partial.join("half"), "").unwrap();
        let install = Install {
            package: Package::MacZip,
            path: installed.clone(),
        };
        tidy(Some(&install), None);
        assert!(!partial.exists());
        assert_eq!(
            std::fs::read_to_string(installed.join("kchess")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Where the disk cannot exchange the two bundles, nothing moves the installed app: a crash
    /// between moving it aside and moving the new one in would leave no app to open.
    #[test]
    fn without_an_exchange_the_installed_app_stays_in_place() {
        let folder = scratch("no-exchange");
        let (installed, staged) = (folder.join("KChess.app"), folder.join("new/KChess.app"));
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(installed.join("kchess"), "old").unwrap();
        std::fs::write(staged.join("kchess"), "new").unwrap();
        let unsupported = |_: &Path, _: &Path| Err(std::io::ErrorKind::Unsupported.into());
        let failed = replace_by(&staged, &installed, unsupported).unwrap_err();
        assert!(failed.contains("Install the update by hand"), "{failed}");
        assert_eq!(
            std::fs::read_to_string(installed.join("kchess")).unwrap(),
            "old"
        );
        assert!(!aside(&installed).exists());
        assert!(!incoming(&installed).exists());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Two KChess processes from one installation: only one installs at a time, and a start
    /// meanwhile leaves the installer's new copy alone.
    #[test]
    fn one_installer_at_a_time_for_each_installation() {
        let _processes = processes();
        let folder = scratch("install-lock");
        let root = downloads_in(&folder.join("cache")).unwrap();
        let installed = folder.join("KChess.AppImage");
        std::fs::write(&installed, "old").unwrap();
        let install = Install {
            package: Package::AppImage,
            path: installed.clone(),
        };
        let first = InstallLock::take(&root, &installed).unwrap();
        let second = InstallLock::take(&root, &installed).err().unwrap();
        assert!(second.contains("Another KChess is installing"), "{second}");
        // Paused after putting the new copy beside the installed one.
        std::fs::write(incoming(&installed), "new").unwrap();
        tidy(Some(&install), Some(&root));
        assert!(incoming(&installed).exists(), "the installer's copy stays");
        // An app starting clears stopped apps' folders, never an installer's files.
        let package = InstallLock::path(&root, &installed).with_extension("deb");
        std::fs::write(&package, "deb").unwrap();
        let _workspace = Workspace::open(&root).unwrap();
        assert!(InstallLock::path(&root, &installed).exists());
        assert!(package.exists());
        drop(first);
        tidy(Some(&install), Some(&root));
        assert!(
            !incoming(&installed).exists(),
            "an interrupted install is tidied"
        );
        assert!(InstallLock::take(&root, &installed).is_ok());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A .deb whose helper stopped before installing it goes on the next start, once no helper
    /// holds the installation; one that failed is reported once.
    #[test]
    fn a_deb_helpers_package_and_failure_outlive_the_app() {
        let _processes = processes();
        let folder = scratch("deb-helper");
        let root = downloads_in(&folder.join("cache")).unwrap();
        let install = Install {
            package: Package::Deb,
            path: folder.join("kchess"),
        };
        let held = InstallLock::path(&root, &install.path);
        assert!(
            held.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(INSTALLING)
        );
        let package = held.with_extension("deb");
        std::fs::write(&package, "deb").unwrap();
        // The helper still waits for a password: its package stays.
        let helper = InstallLock::take(&root, &install.path).unwrap();
        tidy(Some(&install), Some(&root));
        assert!(package.exists());
        drop(helper);
        tidy(Some(&install), Some(&root));
        assert!(!package.exists());
        std::fs::write(held.with_extension("failed"), "failed").unwrap();
        assert!(last_install_failed(Some(&install), Some(&root)));
        assert!(!last_install_failed(Some(&install), Some(&root)));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The process finishing an installation after the app exits holds it from the moment it
    /// starts: a KChess starting meanwhile neither installs into it nor tidies its package.
    #[cfg(unix)]
    #[test]
    fn an_installer_started_on_quit_keeps_the_installation_until_it_exits() {
        let _processes = processes();
        let folder = scratch("install-handover");
        let root = downloads_in(&folder.join("cache")).unwrap();
        let install = Install {
            package: Package::Deb,
            path: folder.join("kchess"),
        };
        let package = InstallLock::path(&root, &install.path).with_extension("deb");
        std::fs::write(&package, "deb").unwrap();
        let release = folder.join("release");
        let lock = InstallLock::take(&root, &install.path).unwrap();
        // An installer still waiting (for a password, or to finish): moves the lock off its
        // standard input as the helper does, and runs until told to stop.
        let mut installer = std::process::Command::new("sh")
            .arg("-c")
            .arg(r#"exec 9<&0 0</dev/null; while [ ! -e "$1" ]; do sleep 0.02; done"#)
            .arg("installer")
            .arg(&release)
            .stdin(lock.inherited().unwrap())
            .spawn()
            .unwrap();
        // The app has exited.
        drop(lock);
        assert!(InstallLock::take(&root, &install.path).is_err());
        tidy(Some(&install), Some(&root));
        assert!(package.exists(), "the installer's package stays");
        std::fs::write(&release, "").unwrap();
        assert!(installer.wait().unwrap().success());
        assert!(InstallLock::take(&root, &install.path).is_ok());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn each_running_app_keeps_its_own_downloads() {
        let _processes = processes();
        let folder = scratch("workspaces");
        let root = downloads_in(&folder).unwrap();
        let first = Workspace::open(&root).unwrap();
        let staged = first.path.join("2026.11.0/update");
        std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
        std::fs::write(&staged, "checked").unwrap();
        // Another app starting (a development build, another profile) leaves it alone.
        let second = Workspace::open(&root).unwrap();
        assert!(staged.exists());
        // Once the first app has stopped, the next one to start clears its folder.
        let stopped = first.path.clone();
        drop(first);
        let _third = Workspace::open(&root).unwrap();
        assert!(!stopped.exists());
        assert!(second.path.exists());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[cfg(unix)]
    #[test]
    fn downloads_never_go_through_a_folder_that_is_not_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let folder = scratch("private");
        let elsewhere = folder.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let cache = folder.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::os::unix::fs::symlink(&elsewhere, cache.join("kchess-update")).unwrap();
        assert!(downloads_in(&cache).is_err(), "a link is refused");
        let fresh = folder.join("fresh");
        let root = downloads_in(&fresh).unwrap();
        for dir in [root.parent().unwrap(), root.as_path()] {
            let mode = std::fs::metadata(dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "only its owner can use {}", dir.display());
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_new_appimage_replaces_the_running_one() {
        let folder = scratch("appimage");
        let (installed, staged) = (folder.join("KChess.AppImage"), folder.join("download"));
        std::fs::write(&installed, "old").unwrap();
        std::fs::write(&staged, "new").unwrap();
        swap_file(&staged, &installed).unwrap();
        assert_eq!(std::fs::read_to_string(&installed).unwrap(), "new");
        assert!(!folder.join(".KChess.AppImage.new").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&installed).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "executable");
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn an_update_is_signed_as_the_app_it_replaces() {
        let developer = Signer {
            valid: true,
            identity: Some("TEAM123".into()),
            app: Some("com.kchess.app".into()),
        };
        assert_eq!(signed_alike(&developer, &developer.clone()), Ok(()));
        let other_team = Signer {
            identity: Some("OTHER".into()),
            ..developer.clone()
        };
        assert!(signed_alike(&developer, &other_team).is_err());
        let ad_hoc = Signer {
            valid: true,
            identity: None,
            app: Some("com.kchess.app".into()),
        };
        assert!(
            signed_alike(&developer, &ad_hoc).is_err(),
            "lost its signer"
        );
        // An ad hoc build accepts another ad hoc build of the same app, never a broken one.
        assert_eq!(signed_alike(&ad_hoc, &ad_hoc.clone()), Ok(()));
        let broken = Signer {
            valid: false,
            ..ad_hoc.clone()
        };
        assert!(signed_alike(&ad_hoc, &broken).is_err());
        let other_app = Signer {
            app: Some("com.example.other".into()),
            ..ad_hoc.clone()
        };
        assert!(signed_alike(&ad_hoc, &other_app).is_err());
        // The way off ad hoc signing: a Developer ID build of the same app.
        let ad_hoc_build = Signer {
            app: developer.app.clone(),
            ..ad_hoc.clone()
        };
        assert_eq!(signed_alike(&ad_hoc_build, &developer), Ok(()));
        // Unsigned (Linux, or Windows without a certificate): the checksum is the check.
        assert_eq!(signed_alike(&Signer::default(), &Signer::default()), Ok(()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn codesign_reads_who_signed_a_bundle() {
        let _processes = processes();
        use super::signer;
        let folder = scratch("codesign");
        let bundle = |id: &str| {
            let app = folder.join(format!("{id}.app"));
            std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            std::fs::copy("/usr/bin/true", app.join("Contents/MacOS/kchess")).unwrap();
            std::fs::write(
                app.join("Contents/Info.plist"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
                     <key>CFBundleIdentifier</key><string>{id}</string>\
                     <key>CFBundleExecutable</key><string>kchess</string></dict></plist>"
                ),
            )
            .unwrap();
            let signed = std::process::Command::new("codesign")
                .args(["--force", "--sign", "-"])
                .arg(&app)
                .output()
                .unwrap();
            assert!(signed.status.success());
            app
        };
        let running = signer(Package::MacZip, &bundle("com.kchess.app"));
        assert!(running.valid);
        assert_eq!(running.identity, None, "ad hoc");
        assert_eq!(running.app.as_deref(), Some("com.kchess.app"));
        let other = signer(Package::MacZip, &bundle("com.example.other"));
        assert!(signed_alike(&running, &other).is_err());
        // A file changed after signing breaks the seal.
        let tampered = bundle("com.kchess.app.tampered");
        std::fs::write(tampered.join("Contents/MacOS/kchess"), "changed").unwrap();
        assert!(!signer(Package::MacZip, &tampered).valid);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn release_versions_compare_as_numbers() {
        assert_eq!(parse_version("v2026.10.12"), Some([2026, 10, 12]));
        assert!(parse_version("v2026.10.12") > parse_version("2026.9.30"));
        assert!(parse_version("v2026.11.0") > parse_version("2026.10.12"));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("2026.10.1-beta"), None);
    }

    #[test]
    fn packages_are_named_as_electrons_are() {
        let x64 = std::env::consts::ARCH == "x86_64";
        let name = |package| asset_name("2026.11.0", package).unwrap();
        assert!(name(Package::MacZip).starts_with("KChess-2026.11.0-mac-"));
        if x64 {
            assert_eq!(
                name(Package::WindowsInstaller),
                "KChess-2026.11.0-win-x64.exe"
            );
            assert_eq!(
                name(Package::AppImage),
                "KChess-2026.11.0-linux-x86_64.AppImage"
            );
            assert_eq!(name(Package::Deb), "KChess-2026.11.0-linux-amd64.deb");
        }
        let sums = format!("ABC123  {}\nfff  other.zip\n", name(Package::Deb));
        assert_eq!(
            digest_for(&sums, &name(Package::Deb)).as_deref(),
            Some("abc123")
        );
        assert_eq!(digest_for(&sums, "missing.zip"), None);
    }

    /// Serve `files` (path → body) over HTTP on a local port until the test ends; the base URL.
    fn serve(files: Vec<(String, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("");
                let found = files.iter().find(|(file, _)| file == path);
                let (status, body) = match found {
                    Some((_, body)) => ("200 OK", body.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            }
        });
        base
    }

    /// A feed whose latest release is `version`, with `package` and `sums` (given the package's
    /// real digest); the feed's URL.
    fn feed(version: &str, package: &[u8], sums: impl Fn(&str, &str) -> String) -> String {
        let name = asset_name(version, Package::AppImage).unwrap();
        let digest: String = Sha256::digest(package)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        // The release names its assets' URLs, so they are served first.
        let assets = serve(vec![
            (format!("/{name}"), package.to_vec()),
            ("/SHA256SUMS.txt".into(), sums(&digest, &name).into_bytes()),
        ]);
        let release = serde_json::json!({
            "tag_name": format!("v{version}"),
            "html_url": "https://example.invalid/release",
            "assets": [
                { "name": name, "browser_download_url": format!("{assets}/{name}"), "size": package.len() },
                { "name": "SHA256SUMS.txt", "browser_download_url": format!("{assets}/SHA256SUMS.txt") },
            ],
        });
        serve(vec![(
            "/repos/ketansingrole/KChess-V2/releases/latest".into(),
            release.to_string().into_bytes(),
        )])
    }

    /// A server that accepts each connection, answers with `head` (nothing when empty), and then
    /// sends nothing more; its base URL.
    fn stall(head: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut open = Vec::new();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let _ = stream.write_all(head.as_bytes());
                open.push(stream);
            }
        });
        base
    }

    #[test]
    fn a_stalled_server_ends_the_download() {
        let folder = scratch("stalled");
        let name = asset_name("2026.11.0", Package::AppImage).unwrap();
        let attempt = |sums: String, package: String| {
            let release: Release = serde_json::from_value(serde_json::json!({
                "tag_name": "v2026.11.0",
                "assets": [
                    { "name": name, "browser_download_url": package },
                    { "name": "SHA256SUMS.txt", "browser_download_url": sums },
                ],
            }))
            .unwrap();
            let asset = release.assets[0].clone();
            let (progress, _heard) = futures::channel::mpsc::unbounded();
            let target = folder.join("download");
            let started = std::time::Instant::now();
            let result = block_on(download(release, asset, target, progress));
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            result
        };
        // No answer at all to the checksums.
        let silent = stall("");
        assert_eq!(
            attempt(format!("{silent}/sums"), format!("{silent}/pkg")),
            Err("Could not download the checksums.".into())
        );
        // The package stops halfway through its body.
        let sums = serve(vec![(
            "/SHA256SUMS.txt".into(),
            format!("{}  {name}\n", "0".repeat(64)).into_bytes(),
        )]);
        let halfway = stall("HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nhalf");
        assert_eq!(
            attempt(format!("{sums}/SHA256SUMS.txt"), format!("{halfway}/pkg")),
            Err("The download stopped.".into())
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    fn block_on<T: Send + 'static>(
        work: impl std::future::Future<Output = T> + Send + 'static,
    ) -> T {
        futures::executor::block_on(super::on_runtime(work)).unwrap()
    }

    async fn check_and_download(feed: String, folder: PathBuf) -> Result<PathBuf, String> {
        let release: Release = latest(feed).await?;
        let version =
            offer(&release, "2026.10.12", Some(Package::AppImage)).ok_or("no update offered")?;
        let name = asset_name(&version, Package::AppImage).unwrap();
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .cloned()
            .unwrap();
        let (progress, _heard) = futures::channel::mpsc::unbounded();
        download(release, asset, folder, progress).await
    }

    #[test]
    fn an_update_is_found_downloaded_and_checked_from_a_feed() {
        let folder = scratch("feed");
        let package = b"the new KChess".to_vec();
        let url = feed("2026.11.0", &package, |digest, name| {
            format!("{digest}  {name}\n")
        });
        let file = block_on(check_and_download(url, folder.join("ok"))).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), package);

        // A package that does not match its checksum is deleted, never staged.
        let url = feed("2026.11.0", &package, |_, name| {
            format!("{}  {name}\n", "0".repeat(64))
        });
        let refused = block_on(check_and_download(url, folder.join("bad")));
        assert_eq!(
            refused,
            Err("The downloaded update did not match its checksum.".into())
        );
        assert!(!folder.join("bad").exists());

        let url = feed("2026.11.0", &package, |_, _| "abc  other.zip\n".into());
        assert_eq!(
            block_on(check_and_download(url, folder.join("none"))),
            Err("The release lists no checksum for this package.".into())
        );

        // The running version, or one without this platform's package, is not offered.
        let url = feed("2026.10.12", &package, |digest, name| {
            format!("{digest}  {name}\n")
        });
        assert_eq!(
            block_on(check_and_download(url, folder.join("same"))),
            Err("no update offered".into())
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
