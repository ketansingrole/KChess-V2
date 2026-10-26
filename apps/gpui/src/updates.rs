//! App updates (`appUpdates.ts`, `AppUpdateSettings.vue`): the GPUI app checks the release feed
//! the Electron app reads (GitHub releases of KChess), downloads its own package for this
//! platform (`tooling/gpui-package.mjs` names them), checks it against the release's
//! `SHA256SUMS.txt`, and replaces itself on restart or when it quits.
//!
//! Only an installed package updates: a development build reports updates as manual, unless
//! `KCHESS_UPDATE_FEED` points it at another feed (tests serve one locally).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;
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

#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

#[derive(Clone, Deserialize)]
struct Release {
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
    /// The unpacked new app, ready to take this one's place.
    staged: Option<PathBuf>,
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

pub fn current_version() -> String {
    serde_json::from_str::<serde_json::Value>(PACKAGE_JSON)
        .ok()
        .and_then(|package| package["version"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// This platform's package of a release (`gpui-package.mjs`'s names).
pub fn asset_name(version: &str) -> Option<String> {
    let (platform, extension) = match std::env::consts::OS {
        "macos" => ("mac", "zip"),
        "windows" => ("win", "zip"),
        "linux" => ("linux", "tar.gz"),
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        _ => return None,
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

/// What the running app is, as a package: the `.app` bundle on macOS, else its folder. Only a
/// package counts: a bundle, or a folder with its Stockfish beside it outside a build's `target`.
fn installed_root() -> Option<PathBuf> {
    package_root(&std::env::current_exe().ok()?)
}

fn package_root(exe: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return exe
            .ancestors()
            .nth(3)
            .filter(|bundle| bundle.extension().is_some_and(|ext| ext == "app"))
            .map(Path::to_path_buf);
    }
    let folder = exe.parent()?;
    let built = folder.components().any(|part| part.as_os_str() == "target");
    let engine = folder.join(kchess_core::engine::managed::executable_name());
    (!built && engine.is_file()).then(|| folder.to_path_buf())
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

/// Download `asset`, check it against the release's digest and unpack it into `folder`; the
/// unpacked app. `progress` hears the bytes received so far.
async fn fetch(
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
    let archive = folder.join(&asset.name);
    let mut file = tokio::fs::File::create(&archive)
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
    let unpacked = folder.join("app");
    std::fs::create_dir_all(&unpacked).map_err(|cause| cause.to_string())?;
    let status = if cfg!(target_os = "macos") {
        Command::new("ditto")
            .args(["-x", "-k"])
            .arg(&archive)
            .arg(&unpacked)
            .status()
    } else {
        // bsdtar on Windows reads zips; GNU tar on Linux reads the .tar.gz.
        Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&unpacked)
            .status()
    };
    if !status.is_ok_and(|status| status.success()) {
        return Err("The update could not be unpacked.".into());
    }
    let _ = std::fs::remove_file(&archive);
    // The package holds one folder (or one .app): that is the new app.
    let mut entries = std::fs::read_dir(&unpacked)
        .map_err(|cause| cause.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir());
    let app = entries.next().ok_or("The update holds no app.")?;
    Ok(app)
}

/// Put `staged` where the running app is, keeping the old one aside until it is done.
fn replace(staged: &Path, installed: &Path) -> Result<(), String> {
    let aside = installed.with_file_name(format!(
        ".{}-old",
        installed
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    let _ = std::fs::remove_dir_all(&aside);
    std::fs::rename(installed, &aside).map_err(|cause| {
        format!("KChess could not replace itself ({cause}). Install the update by hand.")
    })?;
    if let Err(cause) = std::fs::rename(staged, installed).or_else(|_| copy_tree(staged, installed))
    {
        let _ = std::fs::rename(&aside, installed);
        return Err(format!("The update could not be installed ({cause})."));
    }
    // A running Windows executable cannot be deleted; the next update clears it.
    let _ = std::fs::remove_dir_all(&aside);
    Ok(())
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

/// Open the app at `root` once this one has gone.
fn relaunch(root: &Path) {
    let result = if cfg!(target_os = "macos") {
        Command::new("sh")
            .arg("-c")
            .arg("sleep 1; open -n \"$0\"")
            .arg(root)
            .spawn()
    } else {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.file_name().map(|name| root.join(name)))
            .unwrap_or_else(|| root.join("kchess"));
        Command::new(exe).spawn()
    };
    if let Err(cause) = result {
        log(&format!("relaunch failed: {cause}"));
    }
}

pub fn init(cx: &mut App) {
    let feed = std::env::var("KCHESS_UPDATE_FEED")
        .ok()
        .filter(|feed| !feed.is_empty());
    let installed = !cfg!(debug_assertions) && installed_root().is_some();
    let feed = feed.or_else(|| installed.then(|| FEED.to_string()));
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
        let staged = quitting.read(cx).staged.clone();
        if let (true, Some(staged), Some(root)) = (on_quit, staged, installed_root())
            && let Err(cause) = replace(&staged, &root)
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
        let newer = parse_version(&release.tag_name)
            .zip(parse_version(&self.current))
            .is_some_and(|(latest, current)| latest > current);
        let has_package = asset_name(release.tag_name.trim_start_matches('v'))
            .is_some_and(|name| release.assets.iter().any(|asset| asset.name == name));
        if !newer || !has_package {
            self.phase = Phase::UpToDate;
            self.version = None;
            self.release = None;
            return;
        }
        self.phase = Phase::Available;
        self.version = Some(release.tag_name.trim_start_matches('v').to_string());
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

    /// `download`: fetch, check and unpack the package for this platform.
    pub fn download(&mut self, cx: &mut Context<Self>) {
        let Some(release) = self.release.clone() else {
            return;
        };
        if !matches!(self.phase, Phase::Available | Phase::Error) {
            return;
        }
        let version = release.tag_name.trim_start_matches('v').to_string();
        let Some(asset) = asset_name(&version).and_then(|name| {
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
        let folder = std::env::temp_dir().join("kchess-update").join(&version);
        let (sender, mut heard) = futures::channel::mpsc::unbounded();
        let done = on_runtime(fetch(release, asset, folder, sender));
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

    /// `install`: put the downloaded app in place of this one and reopen it.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Some(staged) = self.staged.clone() else {
            return;
        };
        let Some(root) = installed_root() else {
            self.phase = Phase::Error;
            self.error = Some(
                "This copy of KChess is not an installed package; install the update by hand."
                    .into(),
            );
            cx.notify();
            return;
        };
        match replace(&staged, &root) {
            Ok(()) => {
                self.staged = None;
                relaunch(&root);
                cx.quit();
            }
            Err(cause) => {
                self.phase = Phase::Error;
                self.error = Some(cause);
                cx.notify();
            }
        }
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
    use super::{asset_name, digest_for, package_root, parse_version, replace};
    use std::path::Path;

    #[test]
    fn only_a_package_replaces_itself() {
        let scratch = std::env::temp_dir().join(format!("kchess-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        if cfg!(target_os = "macos") {
            let exe = scratch.join("KChess.app/Contents/MacOS/kchess");
            assert_eq!(
                package_root(&exe).as_deref(),
                Some(scratch.join("KChess.app").as_path())
            );
            assert_eq!(package_root(Path::new("/repo/target/debug/KChess")), None);
        } else {
            let folder = scratch.join("KChess");
            std::fs::create_dir_all(&folder).unwrap();
            let exe = folder.join("kchess");
            assert_eq!(package_root(&exe), None, "no Stockfish beside it");
            std::fs::write(
                folder.join(kchess_core::engine::managed::executable_name()),
                "",
            )
            .unwrap();
            assert_eq!(package_root(&exe).as_deref(), Some(folder.as_path()));
            let built = scratch.join("target/release");
            std::fs::create_dir_all(&built).unwrap();
            std::fs::write(
                built.join(kchess_core::engine::managed::executable_name()),
                "",
            )
            .unwrap();
            assert_eq!(package_root(&built.join("KChess")), None);
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_new_app_takes_the_old_ones_place() {
        let scratch = std::env::temp_dir().join(format!("kchess-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let (installed, staged) = (scratch.join("KChess"), scratch.join("new/KChess"));
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(installed.join("kchess"), "old").unwrap();
        std::fs::write(staged.join("kchess"), "new").unwrap();
        replace(&staged, &installed).unwrap();
        assert_eq!(
            std::fs::read_to_string(installed.join("kchess")).unwrap(),
            "new"
        );
        assert!(!scratch.join(".KChess-old").exists());
        let _ = std::fs::remove_dir_all(&scratch);
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
    fn the_package_and_its_checksum_are_found() {
        let name = asset_name("2026.11.0").unwrap();
        assert!(name.starts_with("KChess-Preview-2026.11.0-"));
        let sums = format!("ABC123  {name}\nfff  other.zip\n");
        assert_eq!(digest_for(&sums, &name).as_deref(), Some("abc123"));
        assert_eq!(digest_for(&sums, "missing.zip"), None);
    }
}
