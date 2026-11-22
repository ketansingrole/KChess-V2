//! App updates: the GPUI app checks the release feed
//! (GitHub releases of KChess), downloads the package it was installed from, checks it against the release's
//! `SHA256SUMS.txt` (itself signed with the release key, `update-key.pub`) and against the
//! running app's signature, and installs it on restart or when
//! KChess quits: a new bundle on macOS, the installer run silently on Windows, a new AppImage, or the .deb through the package manager.
//!
//! Only an installed package updates: a development build reports updates as manual, unless
//! `KCHESS_UPDATE_FEED` points it at another feed (tests serve one locally).
//!
//! Here: what an update is and the `Updates` entity the Settings page drives. Its parts:
//! `download` (the feed and the package), `signing` (whose build it is), `install` (putting it
//! in place) and `patch` (a smaller download made from the installed version).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use futures::StreamExt as _;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use kchess_contracts::Level;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::log::{IoResultExt as _, ResultExt as _};
use crate::settings::{SettingsChanged, SettingsHandle};

mod download;
mod install;
mod patch;
mod signing;

use download::*;
pub use install::leave_bridge_folder;
use install::*;
use signing::*;

use kchess_delta::{PREFIX, REPOSITORY, Version, sums};

const FEED: &str = "https://api.github.com";
/// How long a download waits for the server's next bytes before it counts as stopped.
const IDLE: Duration = if cfg!(test) {
    Duration::from_millis(500)
} else {
    Duration::from_secs(30)
};
/// On startup and every six hours.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The waits before checking again after a check failed, one after each failure in a row: a
/// check at login often runs before the network is up, and used to wait six hours for the next.
/// After the last, the six-hourly check takes over.
const CHECK_AGAIN_AFTER: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
];

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
    /// The Mac App Store updates this installation.
    Store,
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
    /// The next check after a failed one (`CHECK_AGAIN_AFTER`), and how many failed in a row.
    _again: Option<Task<()>>,
    failed_checks: usize,
}

/// How views reach updates: `cx.global::<UpdatesHandle>().0`.
#[derive(Clone)]
pub struct UpdatesHandle(pub Entity<Updates>);

impl Global for UpdatesHandle {}

/// `vYYYY.M.C` or `YYYY.M.C` as a release, to compare releases.
pub fn parse_version(text: &str) -> Option<Version> {
    Version::of_tag(text)
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

/// This platform's `package` of a release, as the release names it
/// (`KChess-2026.10.12-linux-x86_64.AppImage`, `…-linux-amd64.deb`).
pub fn asset_name(version: &str, package: Package) -> Option<String> {
    use kchess_delta::Asset;
    let asset = match package {
        Package::MacZip => Asset::MacZip,
        Package::WindowsInstaller => Asset::WindowsInstaller,
        Package::AppImage => Asset::AppImage,
        Package::Deb => Asset::Deb,
    };
    Some(asset.name(version, kchess_delta::Arch::current()?))
}

/// The release key's public half (`update-key.pub`). `None` until a key is made
/// (`cargo xtask update-key`).
pub fn update_key() -> Option<Vec<u8>> {
    sums::parse_key(include_str!("../update-key.pub"))
}

/// Whether `signature` is `key`'s signature of the release's checksums `sums`.
pub fn sums_signed_by(key: &[u8], sums: &[u8], signature: &str) -> Result<(), String> {
    sums::verify(key, sums, signature).map_err(|unsigned| {
        match unsigned {
            sums::Unsigned::Unreadable => "The release's checksum signature could not be read",
            sums::Unsigned::NotTheKeys => "The release's checksums are not signed by KChess",
        }
        .to_string()
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

fn log(message: &str) {
    crate::log::write(Level::Info, "updates", message);
}

pub fn init(cx: &mut App) {
    let store = kchess_host::app_store();
    let install = (!cfg!(debug_assertions) && !store)
        .then(installed)
        .flatten();
    let root = install.as_ref().and_then(|_| downloads_root().ok());
    tidy(install.as_ref(), root.as_deref());
    if let (Some(install), Some(root)) = (&install, &root) {
        // Downloads of this version or older are done with, unless an installer is still running
        // from one.
        if let Ok(_installing) = InstallLock::take(root, &install.path) {
            let current = parse_version(&current_version());
            prune_kept(root, |kept| kept.is_none() || kept <= current);
        }
    }
    let failed = last_install_error(install.as_ref(), root.as_deref());
    let feed = kchess_host::dev_override("KCHESS_UPDATE_FEED")
        .map(|feed| feed.to_string_lossy().into_owned())
        .filter(|_| !store)
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
            this.schedule(cx);
            // Background downloads just turned on, with an update waiting for its download.
            if this.phase == Phase::Available && crate::settings::frontend(cx).update_auto_download
            {
                this.download(cx);
            }
        })
        .detach();
        let mut updates = Updates {
            phase: if failed.is_some() {
                Phase::Error
            } else if feed.is_some() {
                Phase::Idle
            } else if store {
                Phase::Store
            } else {
                Phase::Disabled
            },
            current: current_version(),
            version: None,
            released: None,
            checked_at: None,
            progress: None,
            error: failed,
            release: None,
            staged: None,
            restart: false,
            install,
            workspace,
            feed,
            work: None,
            _timer: None,
            _again: None,
            failed_checks: 0,
        };
        updates.schedule(cx);
        updates
    });
    // An update downloaded and waiting goes in when KChess quits, if the settings say so (or to
    // restart into it): only after every document is saved, since the installer closes the app.
    let quitting = updates.clone();
    crate::quit::after_quit(cx, move |cx| {
        let on_quit = crate::settings::frontend(cx).update_install_on_quit;
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
        let auto = crate::settings::frontend(cx).update_auto_check;
        if !auto || !self.can_check() {
            self._timer = None;
            self._again = None;
            return;
        }
        if self._timer.is_some() {
            return;
        }
        self.check(cx);
        self._timer = Some(crate::timer::every(CHECK_EVERY, cx, |this, cx| {
            this.check(cx);
            true
        }));
    }

    /// `check`: ask the feed for the latest release, and download a newer one when background
    /// downloads are on, whether the check was automatic or asked for.
    pub fn check(&mut self, cx: &mut Context<Self>) {
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
                .unwrap_or_else(|_| Err("The check stopped".into()));
            let _ = this.update(cx, |this, cx| {
                this.checked_at = Some(crate::dates::now_millis());
                match answer {
                    Ok(release) => {
                        this.failed_checks = 0;
                        this._again = None;
                        this.found(release, cx);
                    }
                    Err(cause) => {
                        this.phase = Phase::Error;
                        this.check_again(&cause, cx);
                        this.error = Some(cause);
                    }
                }
                cx.notify();
            });
        }));
    }

    /// After a failed check, check again soon while automatic checks are on (`CHECK_AGAIN_AFTER`).
    fn check_again(&mut self, cause: &str, cx: &mut Context<Self>) {
        let wait = CHECK_AGAIN_AFTER
            .get(self.failed_checks)
            .copied()
            .filter(|_| crate::settings::frontend(cx).update_auto_check);
        let Some(wait) = wait else {
            log(&format!("check failed: {cause}"));
            return;
        };
        self.failed_checks += 1;
        log(&format!(
            "check failed: {cause}; checking again in {} s",
            wait.as_secs()
        ));
        self._again = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| this.check(cx)).ok();
        }));
    }

    fn found(&mut self, release: Release, cx: &mut Context<Self>) {
        let Some(version) = offer(&release, &self.current, self.package()) else {
            log(&format!("up to date at {}", self.current));
            self.phase = Phase::UpToDate;
            self.version = None;
            self.release = None;
            // An update downloaded before (its install failed) is not installed on quit either.
            self.staged = None;
            return;
        };
        // An update whose install failed is still whole: the same version is ready again, and a
        // newer one replaces it, which used to install the old one on quit while offering the new.
        if self.staged.is_some() {
            if self.version.as_deref() == Some(version.as_str()) {
                log(&format!("found {version}, downloaded before"));
                self.phase = Phase::Downloaded;
                self.released = release.published_at.clone();
                self.release = Some(release);
                return;
            }
            self.staged = None;
        }
        self.phase = Phase::Available;
        let auto_download = crate::settings::frontend(cx).update_auto_download;
        log(&format!(
            "found {version}{}",
            if auto_download { ", downloading" } else { "" }
        ));
        // Kept in the notification centre once per version: found again on the next start,
        // it stays as it was, read or not.
        crate::notifications::centre::record(
            kchess_contracts::types::NotificationInput {
                title: format!("KChess {version} is available"),
                link: kchess_contracts::types::NotificationLink::Update {
                    version: version.clone(),
                },
                key: Some("update".into()),
                ..Default::default()
            },
            cx,
        );
        self.version = Some(version);
        self.released = release.published_at.clone();
        self.release = Some(release);
        if auto_download {
            self.download(cx);
        }
    }

    /// `download`: fetch, check and stage the package this app was installed from.
    pub fn download(&mut self, cx: &mut Context<Self>) {
        let (Some(release), Some(package)) = (self.release.clone(), self.package()) else {
            log("no download: no release or no package for this platform");
            return;
        };
        if !matches!(self.phase, Phase::Available | Phase::Error) {
            log(&format!("no download while {:?}", self.phase));
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
            log(&format!(
                "no download: {version} has no {package:?} package"
            ));
            return;
        };
        self.phase = Phase::Downloading;
        self.error = None;
        self.progress = None;
        cx.notify();
        let Some(workspace) = self.workspace.as_ref() else {
            self.phase = Phase::Error;
            self.error = Some("KChess has no private folder to download the update to".into());
            cx.notify();
            return;
        };
        let work = workspace.path.join(&version);
        let folder = match downloads_root() {
            Ok(root) => {
                // Only this version's download is worth keeping now.
                prune_kept(&root, |kept| kept != parse_version(&version));
                kept_folder(&root, &version)
            }
            Err(cause) => {
                log(&format!("no folder to keep downloads in: {cause}"));
                work.clone()
            }
        };
        let (sender, mut heard) = futures::channel::mpsc::unbounded();
        let done = on_runtime(fetch_update(
            release,
            asset,
            package,
            self.install.clone(),
            self.current.clone(),
            folder,
            work,
            sender,
            update_key(),
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
                .unwrap_or_else(|_| Err("The download stopped".into()));
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

    /// Whether retrying after an error installs the update already downloaded, whose install
    /// failed, rather than downloading it again.
    pub fn retries_install(&self) -> bool {
        self.phase == Phase::Error && self.staged.is_some()
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
                "This copy of KChess is not an installed package; install the update by hand"
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
        crate::quit::prepare_quit(
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
            return crate::quit::quit_saved(cx);
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
                Err(("The update could not be installed".into(), PathBuf::new()))
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
            // Only the repository's own pages open from here, whatever the feed answered.
            .filter(|url| url.starts_with(&format!("https://github.com/{REPOSITORY}/")))
            .unwrap_or_else(|| format!("https://github.com/{REPOSITORY}/releases"))
    }
}

#[cfg(test)]
mod tests;
