//! App updates (`appUpdates.ts`, `AppUpdateSettings.vue`): the GPUI app checks the release feed
//! the Electron app reads (GitHub releases of KChess), downloads the package it was installed from
//! (`tooling/gpui-package.mjs` makes them in Electron's formats), checks it against the release's
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
/// `checkAutomatically`: on startup and every six hours.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The release's own version (`package.json`), the one releases are tagged with.
const PACKAGE_JSON: &str = include_str!("../../../package.json");
/// The name this identity's packages carry, which `gpui-package.mjs` builds in: the preview
/// beside Electron, or (`--final`) the app replacing it.
const PREFIX: &str = match option_env!("KCHESS_PACKAGE_PREFIX") {
    Some(prefix) => prefix,
    None => "KChess-Preview",
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Downloaded,
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
    /// How this copy was installed; none for a build run from where it was made.
    install: Option<Install>,
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

pub fn current_version() -> String {
    serde_json::from_str::<serde_json::Value>(PACKAGE_JSON)
        .ok()
        .and_then(|package| package["version"].as_str().map(str::to_string))
        .unwrap_or_default()
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

/// Put the staged update in place of `install`; with `reopen`, open the new KChess once this one
/// has quit.
fn apply(staged: &Path, install: &Install, reopen: bool) -> Result<(), String> {
    match install.package {
        Package::MacZip => {
            replace(staged, &install.path)?;
            if reopen {
                reopen_after_exit(&install.path);
            }
        }
        Package::WindowsInstaller => {
            // As Electron's updater runs it: silently over this installation, which it closes.
            let mut installer = Command::new(staged);
            installer.args(["--updated", "/S"]);
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

/// Where `replace` keeps the old bundle until the new one is in place.
fn aside(installed: &Path) -> PathBuf {
    installed.with_file_name(format!(
        ".{}-old",
        installed
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    ))
}

/// Where `swap_file` writes the new AppImage before it takes the old one's name.
fn incoming(installed: &Path) -> PathBuf {
    installed.with_file_name(format!(
        ".{}.new",
        installed
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    ))
}

/// Put `staged` where the running app is, keeping the old one aside until it is done.
fn replace(staged: &Path, installed: &Path) -> Result<(), String> {
    let aside = aside(installed);
    let _ = std::fs::remove_dir_all(&aside);
    std::fs::rename(installed, &aside).map_err(|cause| {
        format!("KChess could not replace itself ({cause}). Install the update by hand.")
    })?;
    if let Err(cause) = std::fs::rename(staged, installed).or_else(|_| copy_tree(staged, installed))
    {
        let _ = std::fs::remove_dir_all(installed);
        let _ = std::fs::rename(&aside, installed);
        return Err(format!("The update could not be installed ({cause})."));
    }
    let _ = std::fs::remove_dir_all(&aside);
    Ok(())
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

/// Where this identity's downloads go: one folder per version, cleared on startup.
fn downloads() -> PathBuf {
    std::env::temp_dir().join("kchess-update").join(PREFIX)
}

/// What an earlier run left behind: its downloads, and an old bundle or new AppImage that an
/// interrupted install did not remove.
fn tidy(install: Option<&Install>) {
    let _ = std::fs::remove_dir_all(downloads());
    match install {
        Some(Install {
            package: Package::MacZip,
            path,
        }) => {
            let _ = std::fs::remove_dir_all(aside(path));
        }
        Some(Install {
            package: Package::AppImage,
            path,
        }) => {
            let _ = std::fs::remove_file(incoming(path));
        }
        _ => {}
    }
}

pub fn init(cx: &mut App) {
    let install = (!cfg!(debug_assertions)).then(installed).flatten();
    tidy(install.as_ref());
    let feed = std::env::var("KCHESS_UPDATE_FEED")
        .ok()
        .filter(|feed| !feed.is_empty())
        .or_else(|| install.is_some().then(|| FEED.to_string()));
    let updates = cx.new(|cx: &mut Context<Updates>| {
        let settings = cx.global::<SettingsHandle>().0.clone();
        cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
            this.schedule(cx)
        })
        .detach();
        let mut updates = Updates {
            phase: if feed.is_some() {
                Phase::Idle
            } else {
                Phase::Disabled
            },
            current: current_version(),
            version: None,
            released: None,
            checked_at: None,
            progress: None,
            error: None,
            release: None,
            staged: None,
            install,
            feed,
            work: None,
            _timer: None,
        };
        updates.schedule(cx);
        updates
    });
    // An update downloaded and waiting goes in when KChess quits, if the settings say so.
    let quitting = updates.clone();
    crate::core::on_quit(cx, move |cx| {
        let on_quit = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .update_install_on_quit;
        let updates = quitting.read(cx);
        if let (true, Some(staged), Some(install)) =
            (on_quit, updates.staged.as_ref(), updates.install.as_ref())
            && let Err(cause) = apply(staged, install, false)
        {
            log(&cause);
        }
        None
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
            Phase::Checking | Phase::Downloading | Phase::Downloaded
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
        let folder = downloads().join(&version);
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

    /// `install`: put the downloaded update in place of this app and reopen it.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Some(staged) = self.staged.clone() else {
            return;
        };
        let Some(install) = self.install.clone() else {
            self.phase = Phase::Error;
            self.error = Some(
                "This copy of KChess is not an installed package; install the update by hand."
                    .into(),
            );
            cx.notify();
            return;
        };
        // Off the UI thread: the package manager may wait for a password.
        let done = on_runtime(async move { apply(&staged, &install, true) });
        self.work = Some(cx.spawn(async move |this, cx| {
            let done = done
                .await
                .unwrap_or_else(|_| Err("The update could not be installed.".into()));
            let _ = this.update(cx, |this, cx| match done {
                Ok(()) => {
                    // Installed: quitting must not install it again.
                    this.staged = None;
                    cx.quit();
                }
                Err(cause) => {
                    log(&format!("install failed: {cause}"));
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
        Install, Package, Release, Signer, asset_name, detect, digest_for, download, latest, offer,
        parse_version, replace, signed_alike, swap_file,
    };
    use sha2::{Digest, Sha256};
    use std::io::{Read as _, Write as _};
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("kchess-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
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

    #[test]
    fn the_new_bundle_takes_the_old_ones_place() {
        let folder = scratch("replace");
        let (installed, staged) = (folder.join("KChess.app"), folder.join("new/KChess.app"));
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(installed.join("kchess"), "old").unwrap();
        std::fs::write(staged.join("kchess"), "new").unwrap();
        replace(&staged, &installed).unwrap();
        assert_eq!(
            std::fs::read_to_string(installed.join("kchess")).unwrap(),
            "new"
        );
        assert!(!folder.join(".KChess.app-old").exists());
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
        // Unsigned (Linux, or Windows without a certificate): the checksum is the check.
        assert_eq!(signed_alike(&Signer::default(), &Signer::default()), Ok(()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn codesign_reads_who_signed_a_bundle() {
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
        assert!(name(Package::MacZip).starts_with("KChess-Preview-2026.11.0-mac-"));
        if x64 {
            assert_eq!(
                name(Package::WindowsInstaller),
                "KChess-Preview-2026.11.0-win-x64.exe"
            );
            assert_eq!(
                name(Package::AppImage),
                "KChess-Preview-2026.11.0-linux-x86_64.AppImage"
            );
            assert_eq!(
                name(Package::Deb),
                "KChess-Preview-2026.11.0-linux-amd64.deb"
            );
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
