//! The Stockfish KChess downloads and owns: install state, the verified download of the latest official release,
//! and the atomic replacement of the working engine.
//!
//! Replacement goes through an `EngineHandoff`: the host stops the engine owners and waits until
//! none holds the executable, the new files are renamed into place, then the handoff resumes.
//! A failed download, digest, archive or probe leaves the working engine and its VERSION
//! untouched.

#[cfg(test)]
mod tests;

use kchess_contracts::CoreEvent;
use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use super::uci::{EngineHub, UciController};
use crate::archive::{self, Refusal};
use crate::download;
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};
use crate::ids::hex;
use crate::usage::{Meter, Unmetered};

/// The GitHub endpoint for the latest official Stockfish release.
pub const RELEASE_URL: &str =
    "https://api.github.com/repos/official-stockfish/Stockfish/releases/latest";
/// Where every official asset downloads from: `<tag>/<asset name>` follows.
const DOWNLOAD_BASE: &str = "https://github.com/official-stockfish/Stockfish/releases/download/";

// The pin has a file of its own: CI keys its engine cache on it, and `cargo xtask pins --check`
// reads it (`tooling/xtask/src/pinned.rs`).
mod shipped;
pub use shipped::{PinnedRelease, SHIPPED};

/// Whether `url` may be downloaded for `asset` of release `tag`, found at `release_url`: an
/// official release's asset comes from the official repository's downloads alone, whatever
/// address the release metadata named. A test endpoint serves its own.
fn download_allowed(release_url: &str, tag: &str, asset: &str, url: &str) -> bool {
    let official = release_url == RELEASE_URL
        || release_url.starts_with(
            "https://api.github.com/repos/official-stockfish/Stockfish/releases/tags/",
        );
    !official || url == format!("{DOWNLOAD_BASE}{tag}/{asset}")
}

/// The release endpoint of one tag.
pub fn tag_url(tag: &str) -> String {
    format!("https://api.github.com/repos/official-stockfish/Stockfish/releases/tags/{tag}")
}

/// Larger downloads are refused, both up front and while streaming.
pub const MAX_DOWNLOAD_BYTES: u64 = 300 * 1024 * 1024;
/// Total uncompressed size an archive may expand to (zip bombs).
pub const MAX_EXTRACTED_BYTES: u64 = 1024 * 1024 * 1024;
const RELEASE_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const VERSION_FILE: &str = "VERSION";

const COULD_NOT_START: &str =
    "The downloaded Stockfish could not start. The previous engine was retained.";

/// The executable name of the managed engine on this host.
pub fn executable_name() -> &'static str {
    if cfg!(windows) {
        "stockfish.exe"
    } else {
        "stockfish"
    }
}

/// The directory of the managed engine: the platform's override, else `engines/stockfish/current`
/// under the data directory.
pub fn default_dir(data_dir: &Path, managed_override: Option<&Path>) -> PathBuf {
    managed_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| data_dir.join("engines").join("stockfish").join("current"))
}

/// The Node names of the running platform (`process.platform` / `process.arch`), which the
/// asset choice is written in. Unsupported values map to "", which matches no asset.
pub fn node_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "win32",
        _ => "",
    }
}

pub fn node_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        _ => "",
    }
}

/// The platform an install selects its asset for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub platform: &'static str,
    pub arch: &'static str,
}

impl Target {
    pub fn current() -> Target {
        Target {
            platform: node_platform(),
            arch: node_arch(),
        }
    }
}

/// Whether a download is offered here: the platforms and architectures with official builds.
pub fn can_download(platform: &str, arch: &str) -> bool {
    ["darwin", "win32", "linux"].contains(&platform) && ["arm64", "x64"].contains(&arch)
}

/// Host-supplied values for one managed engine directory. The defaults are the GitHub endpoint,
/// the running platform, no handoff and the default exit deadline.
pub struct ManagedLocation {
    /// Directory holding the downloaded engine. Tests point this at a temporary folder.
    pub dir: PathBuf,
    /// The "latest release" endpoint. Tests point this at a local server.
    pub release_url: String,
    /// Sent with the release check only (never the download), so packaging on shared CI
    /// runners is not refused by GitHub's anonymous rate limit. The app sends none.
    pub github_token: Option<String>,
    pub target: Target,
    /// Suspends engine owners while the verified replacement is published.
    pub handoff: Option<Arc<dyn EngineHandoff>>,
    /// Cancels a running install.
    pub cancel: CancellationToken,
    /// How long the handoff may take to stop the owners before the install fails.
    pub exit_deadline: Duration,
    /// A release fixed in source (`SHIPPED`): only its tag and its digests are accepted.
    pub pinned: Option<&'static PinnedRelease>,
    /// Where the download's traffic is counted.
    pub meter: Arc<dyn Meter>,
}

impl ManagedLocation {
    pub fn new(dir: PathBuf) -> ManagedLocation {
        ManagedLocation {
            dir,
            release_url: RELEASE_URL.to_string(),
            github_token: None,
            target: Target::current(),
            handoff: None,
            cancel: CancellationToken::new(),
            exit_deadline: super::uci::EXIT_DEADLINE,
            pinned: None,
            meter: Arc::new(Unmetered),
        }
    }

    /// The release KChess ships (`SHIPPED`), fetched by its tag.
    pub fn shipped(dir: PathBuf) -> ManagedLocation {
        ManagedLocation {
            release_url: tag_url(SHIPPED.tag),
            pinned: Some(&SHIPPED),
            ..ManagedLocation::new(dir)
        }
    }
}

/// Keeps engine owners from starting. `suspend` resolves once none holds the executable; the
/// returned guard lets them start again when it is dropped.
pub trait EngineHandoff: Send + Sync {
    fn suspend(&self) -> Pin<Box<dyn Future<Output = Result<EngineGuard>> + Send + '_>>;
}

/// Held for the duration of one replacement; dropping it resumes the engine owners.
pub type EngineGuard = Box<dyn Send>;

pub use kchess_contracts::types::EngineManaged as ManagedEngine;

/// The result of `install_managed_engine`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub installed: bool,
    pub path: PathBuf,
    pub version: String,
    /// False when the installed engine already matched the latest release and nothing was downloaded.
    pub updated: bool,
}

/// One asset of a GitHub release. Fields the install checks are optional so that a malformed
/// entry is skipped rather than failing the whole release.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(default)]
    pub browser_download_url: Option<String>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub digest: Option<String>,
}

/// The Stockfish directory's install state. The version is the recorded release tag, if any.
pub async fn managed_engine(host: &dyn Host, dir: &Path) -> ManagedEngine {
    let path = dir.join(executable_name());
    match tokio::fs::metadata(&path).await {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return absent(&path),
        Err(cause) => {
            host.log(
                Level::Debug,
                "managed-engine",
                &format!("Managed engine check failed: {} {cause}", dir.display()),
            );
            return absent(&path);
        }
    }
    let version = tokio::fs::read_to_string(dir.join(VERSION_FILE))
        .await
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    ManagedEngine {
        installed: true,
        path: path.to_string_lossy().into_owned(),
        version,
    }
}

fn absent(path: &Path) -> ManagedEngine {
    ManagedEngine {
        installed: false,
        path: path.to_string_lossy().into_owned(),
        version: None,
    }
}

/// Physical engine directories may be shared by profiles; mutations of one directory are
/// serialized. The map keeps only weak references, so idle directories are forgotten.
///
/// One writer per directory per process; executables are not shared across profiles:
/// file mutation holds the per-directory lock, but engine suspension (`EngineHub`,
/// per-core) does not. A second core's warm engine can hold the executable open
/// while the first core replaces it (Windows rename failure). Do not point two
/// live profiles at the same `managed_engine_dir` with a warm engine running.
fn dir_lock(dir: &Path) -> Arc<AsyncMutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(Default::default);
    let mut locks = crate::tasks::lock(locks);
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(dir).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(dir.to_path_buf(), Arc::downgrade(&lock));
    lock
}

/// Remove the downloaded engine. Bundled and user-picked engines are never touched.
pub async fn delete_managed_engine(host: &dyn Host, location: &ManagedLocation) -> Result<()> {
    let lock = dir_lock(&location.dir);
    let _held = lock.lock().await;
    let dir = location.dir.clone();
    let result = with_handoff(location, || async move {
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(()),
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(cause) => Err(CoreError::new(format!(
                "Could not delete the Stockfish folder: {cause}"
            ))),
        }
    })
    .await;
    log_mutation_failure(host, &location.dir, &result);
    result
}

/// Install the latest official native build, verified against the digest GitHub publishes.
/// Checks the latest release first and downloads nothing when the installed engine is already
/// that version.
pub async fn install_managed_engine(
    host: &dyn Host,
    location: &ManagedLocation,
) -> Result<InstallResult> {
    let held = dir_lock(&location.dir).lock_owned().await;
    let result = install(host, location, held).await;
    log_mutation_failure(host, &location.dir, &result);
    result
}

fn log_mutation_failure<T>(host: &dyn Host, dir: &Path, result: &Result<T>) {
    if let Err(error) = result {
        host.log(
            Level::Debug,
            "managed-engine",
            &format!(
                "Engine mutation failed: {} {}",
                dir.display(),
                error.message
            ),
        );
    }
}

/// Run `commit` while the engine owners are suspended. The exit deadline bounds the suspension;
/// the owners resume after `commit` whether it succeeded or not.
async fn with_handoff<T, F, Fut>(location: &ManagedLocation, commit: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let Some(handoff) = &location.handoff else {
        return commit().await;
    };
    let guard = tokio::time::timeout(location.exit_deadline, handoff.suspend())
        .await
        .map_err(|_| CoreError::new(super::uci::EXIT_TIMEOUT_MESSAGE))??;
    let result = commit().await;
    drop(guard);
    result
}

async fn install(
    host: &dyn Host,
    location: &ManagedLocation,
    held: tokio::sync::OwnedMutexGuard<()>,
) -> Result<InstallResult> {
    if crate::app_store() {
        return Err(CoreError::new(
            "The App Store version uses the Stockfish it comes with.",
        ));
    }
    let dir = location.dir.clone();
    let cancel = &location.cancel;
    let client = download::client(None)
        .map_err(|_| CoreError::new("Could not start the Stockfish download."))?;

    let release = fetch_release(
        &*location.meter,
        &client,
        &location.release_url,
        location.github_token.as_deref(),
        cancel,
    )
    .await?;
    let current = managed_engine(host, &dir).await;
    if current.installed && current.version.as_deref() == Some(release.tag.as_str()) {
        return Ok(InstallResult {
            installed: true,
            path: current.path.into(),
            version: release.tag,
            updated: false,
        });
    }
    let pin = match location.pinned {
        Some(pinned) => {
            if release.tag != pinned.tag {
                return Err(CoreError::new(format!(
                    "GitHub answered Stockfish {} for the pinned {}.",
                    release.tag, pinned.tag
                )));
            }
            let entry = pinned
                .assets
                .iter()
                .find(|(platform, arch, _, _)| {
                    *platform == location.target.platform && *arch == location.target.arch
                })
                .ok_or_else(|| CoreError::new("No pinned Stockfish download for this platform."))?;
            Some((entry.2, entry.3))
        }
        None => None,
    };
    let asset = match pin {
        Some((name, _)) => release.assets.iter().find(|asset| asset.name == name),
        None => pick_stockfish_asset(
            &release.assets,
            location.target.platform,
            location.target.arch,
        ),
    }
    .ok_or_else(|| CoreError::new("No compatible official Stockfish download was found."))?
    .clone();
    let (Some(url), Some(size)) = (asset.browser_download_url.clone(), asset.size) else {
        return Err(CoreError::new("Invalid Stockfish download metadata."));
    };
    if !size.is_finite() || !is_plain_file_name(&asset.name) {
        return Err(CoreError::new("Invalid Stockfish download metadata."));
    }
    if !download_allowed(&location.release_url, &release.tag, &asset.name, &url) {
        return Err(CoreError::new(
            "The Stockfish download is not from the official release.",
        ));
    }
    let expected = match pin {
        Some((_, digest)) => digest.to_string(),
        None => expected_digest(asset.digest.as_deref()).ok_or_else(|| {
            CoreError::new("The Stockfish release has no SHA-256 digest, so it cannot be verified.")
        })?,
    };
    if size > MAX_DOWNLOAD_BYTES as f64 {
        return Err(CoreError::new("Stockfish download is unexpectedly large."));
    }

    let staging = tempfile::Builder::new()
        .prefix("kchess-stockfish-")
        .tempdir()
        .map_err(|cause| CoreError::new(format!("Could not create a staging folder: {cause}")))?;
    let archive = staging.path().join(&asset.name);
    let (digest, received) =
        download_archive(&*location.meter, &client, &url, &archive, cancel).await?;
    if received as f64 != size {
        return Err(CoreError::new("Stockfish download was incomplete."));
    }
    if digest != expected {
        return Err(CoreError::new(
            "Stockfish download failed its SHA-256 check.",
        ));
    }
    let kind = archive_kind(&asset.name)
        .ok_or_else(|| CoreError::new("Unsupported Stockfish archive."))?;

    let extracted = staging.path().join("extracted");
    let source = extract_engine(kind, archive, extracted).await?;

    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|cause| CoreError::wrap("Could not create the Stockfish folder", cause))?;
    let path = dir.join(executable_name());
    let pending = dir.join(format!("{}.pending", executable_name()));
    let pending_version = dir.join(format!("{VERSION_FILE}.pending"));
    let version_text = format!("{}\n", release.tag);
    let prepared = async {
        tokio::fs::copy(&source, &pending)
            .await
            .map_err(|cause| CoreError::wrap("Could not stage the Stockfish executable", cause))?;
        set_executable(&pending)?;
        // Verify UCI before replacing the working engine; this catches wrong architecture and
        // broken downloads.
        probe_engine(&pending).await?;
        tokio::fs::write(&pending_version, &version_text)
            .await
            .map_err(|cause| CoreError::wrap("Could not stage the Stockfish version", cause))
    }
    .await;
    if let Err(error) = prepared {
        discard(&[pending, pending_version]).await;
        return Err(error);
    }

    // The engine owners stay suspended, and the folder locked, until both renames are done: the
    // blocking task owns the guards, and finishes even when this call is dropped between them
    // (its connection closed), so the executable and its VERSION are replaced together.
    let suspended = match &location.handoff {
        Some(handoff) => {
            let suspended = tokio::time::timeout(location.exit_deadline, handoff.suspend())
                .await
                .map_err(|_| CoreError::new(super::uci::EXIT_TIMEOUT_MESSAGE))
                .and_then(|suspended| suspended);
            match suspended {
                Ok(suspended) => Some(suspended),
                Err(error) => {
                    discard(&[pending, pending_version]).await;
                    return Err(error);
                }
            }
        }
        None => None,
    };
    let renamed = {
        let (pending, path) = (pending.clone(), path.clone());
        let (pending_version, version_path) = (pending_version.clone(), dir.join(VERSION_FILE));
        tokio::task::spawn_blocking(move || {
            let _guards = (held, suspended);
            std::fs::rename(&pending, &path).map_err(|cause| {
                CoreError::wrap("Could not replace the Stockfish executable", cause)
            })?;
            std::fs::rename(&pending_version, &version_path)
                .map_err(|cause| CoreError::wrap("Could not record the Stockfish version", cause))
        })
    };
    let committed = renamed
        .await
        .map_err(|_| CoreError::new("Could not replace the Stockfish executable."))
        .and_then(|renamed| renamed);
    if let Err(error) = committed {
        discard(&[pending, pending_version]).await;
        return Err(error);
    }
    Ok(InstallResult {
        installed: true,
        path,
        version: release.tag,
        updated: true,
    })
}

/// The release tag and its assets, after the metadata checks.
struct Release {
    tag: String,
    assets: Vec<ReleaseAsset>,
}

/// How the release check words its failures.
const RELEASE_WORDING: download::Wording = download::Wording {
    network: |cause| {
        if cause.is_timeout() {
            "Could not check Stockfish releases (timed out).".into()
        } else {
            "Could not check Stockfish releases (network error).".into()
        }
    },
    status: |status| format!("Could not check Stockfish releases ({status})."),
    too_large: "Invalid Stockfish release metadata.",
};

/// How the download words its failures.
const DOWNLOAD_WORDING: download::Wording = download::Wording {
    network: |cause| {
        if cause.is_timeout() {
            "Stockfish download timed out.".into()
        } else {
            "Stockfish download failed (network error).".into()
        }
    },
    status: |status| format!("Stockfish download failed ({status})."),
    too_large: "Stockfish download is unexpectedly large.",
};

/// The most the release metadata may take.
const MAX_RELEASE_BYTES: u64 = 10 * 1024 * 1024;

fn usage(meter: &dyn Meter) -> download::Usage<'_> {
    download::Usage {
        meter,
        kind: kchess_contracts::types::UsageKind::Other,
    }
}

async fn fetch_release(
    meter: &dyn Meter,
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Release> {
    let request = || {
        let request = client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json");
        match token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    };
    let limits = download::Limits {
        max_bytes: MAX_RELEASE_BYTES,
        deadline: Some(std::time::Instant::now() + RELEASE_TIMEOUT),
    };
    let body = download::open(
        client,
        request,
        limits,
        cancel,
        usage(meter),
        &RELEASE_WORDING,
    )
    .await?
    .bytes()
    .await?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| CoreError::new("Invalid Stockfish release metadata."))?;
    let tag = value.get("tag_name").and_then(Value::as_str);
    let assets = value.get("assets").and_then(Value::as_array);
    let (Some(tag), Some(assets)) = (tag, assets) else {
        return Err(CoreError::new("Invalid Stockfish release metadata."));
    };
    let assets = assets
        .iter()
        .filter_map(|asset| serde_json::from_value::<ReleaseAsset>(asset.clone()).ok())
        .collect();
    Ok(Release {
        tag: tag.to_string(),
        assets,
    })
}

/// Streams the download into `archive`, hashing as it goes: its digest and the bytes kept.
async fn download_archive(
    meter: &dyn Meter,
    client: &reqwest::Client,
    url: &str,
    archive: &Path,
    cancel: &CancellationToken,
) -> Result<(String, u64)> {
    let limits = download::Limits {
        max_bytes: MAX_DOWNLOAD_BYTES,
        deadline: Some(std::time::Instant::now() + DOWNLOAD_TIMEOUT),
    };
    let mut body = download::open(
        client,
        || client.get(url),
        limits,
        cancel,
        usage(meter),
        &DOWNLOAD_WORDING,
    )
    .await?;
    let mut file = tokio::fs::File::create(archive)
        .await
        .map_err(|cause| CoreError::wrap("Could not stage the Stockfish download", cause))?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = body.next().await? {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|cause| CoreError::wrap("Could not stage the Stockfish download", cause))?;
    }
    file.flush()
        .await
        .map_err(|cause| CoreError::wrap("Could not stage the Stockfish download", cause))?;
    Ok((hex(&hasher.finalize()), body.received()))
}

/// The lowercase hex digest from GitHub's `sha256:<hex>` form, or `None` if absent or malformed.
fn expected_digest(digest: Option<&str>) -> Option<String> {
    let lower = digest?.to_ascii_lowercase();
    let hex = lower.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_string())
}

/// A release asset name that is one path component, so it can name a file in the staging folder.
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchiveKind {
    Zip,
    TarGz,
    Tar,
}

/// The archive formats the install extracts (`.zip`, `.tar.gz`, `.tgz`, `.tar`), case-insensitively.
fn archive_kind(name: &str) -> Option<ArchiveKind> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        Some(ArchiveKind::Zip)
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(ArchiveKind::TarGz)
    } else if lower.ends_with(".tar") {
        Some(ArchiveKind::Tar)
    } else {
        None
    }
}

/// Extract the archive and locate the engine executable inside it, off the async runtime. On
/// timeout the extraction is told to stop at its next member and awaited, so nothing writes to
/// the staging folder once this returns.
async fn extract_engine(
    kind: ArchiveKind,
    archive: PathBuf,
    extracted: PathBuf,
) -> Result<PathBuf> {
    let stop = CancellationToken::new();
    let flag = stop.clone();
    let mut work = tokio::task::spawn_blocking(move || -> Result<Option<PathBuf>> {
        extract(kind, &archive, &extracted, Some(&flag))?;
        find_binary(&extracted)
    });
    let failed = || CoreError::new("Stockfish archive could not be extracted.");
    let found = match tokio::time::timeout(EXTRACT_TIMEOUT, &mut work).await {
        Ok(joined) => joined.map_err(|_| failed())??,
        Err(_) => {
            stop.cancel();
            let _ = work.await;
            return Err(failed());
        }
    };
    found.ok_or_else(|| CoreError::new("Stockfish executable was missing from the release."))
}

/// Extract `archive` into `root`: regular files and folders only, each strictly inside `root`.
fn extract(
    kind: ArchiveKind,
    archive: &Path,
    root: &Path,
    cancel: Option<&CancellationToken>,
) -> Result<()> {
    fs::create_dir_all(root)
        .map_err(|cause| CoreError::wrap("Could not extract the Stockfish archive", cause))?;
    let file = fs::File::open(archive)
        .map_err(|cause| CoreError::wrap("Could not read the Stockfish archive", cause))?;
    let to = archive::Unpack {
        root,
        budget: MAX_EXTRACTED_BYTES,
        prefix: None,
        cancel,
    };
    let unpacked = match kind {
        ArchiveKind::Zip => archive::unpack_zip(BufReader::new(file), &to),
        ArchiveKind::TarGz => {
            archive::unpack_tar(flate2::read::GzDecoder::new(BufReader::new(file)), &to)
        }
        ArchiveKind::Tar => archive::unpack_tar(BufReader::new(file), &to),
    };
    unpacked.map(|_| ()).map_err(|refusal| match refusal {
        Refusal::Unreadable(cause) => {
            CoreError::new(format!("Could not extract the Stockfish archive: {cause}"))
        }
        Refusal::Link => CoreError::new("Stockfish archive contains a link."),
        Refusal::UnsafePath | Refusal::Outside => {
            CoreError::new("Stockfish archive has an unsafe path.")
        }
        Refusal::Unsupported => CoreError::new("Stockfish archive contains an unsupported entry."),
        Refusal::TooLarge => CoreError::new("Stockfish archive is unexpectedly large."),
        Refusal::Cancelled => CoreError::new("Stockfish archive could not be extracted."),
        Refusal::Io(cause) => CoreError::wrap("Could not extract the Stockfish archive", cause),
    })
}

/// The executable inside an extracted release: the first `stockfish*` file that is not
/// documentation. Links are not followed.
fn find_binary(root: &Path) -> Result<Option<PathBuf>> {
    let entries = fs::read_dir(root).map_err(|cause| {
        CoreError::wrap("Could not read the extracted Stockfish archive", cause)
    })?;
    for entry in entries {
        let entry = entry.map_err(|cause| {
            CoreError::wrap("Could not read the extracted Stockfish archive", cause)
        })?;
        let file_type = entry.file_type().map_err(|cause| {
            CoreError::wrap("Could not read the extracted Stockfish archive", cause)
        })?;
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(found) = find_binary(&path)? {
                return Ok(Some(found));
            }
        } else if file_type.is_file() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let documentation = [".txt", ".md", ".pdf"]
                .iter()
                .any(|ext| name.ends_with(ext));
            if name.starts_with("stockfish") && !documentation {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

/// Runs the candidate executable through the UCI handshake within the probe deadline. The probe
/// has a registry of its own: the core's refuses new engines while the executable is replaced.
async fn probe_engine(path: &Path) -> Result<()> {
    struct Quiet;
    impl Host for Quiet {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, _event: CoreEvent) {}
    }
    let hub = EngineHub::new(Arc::new(Quiet));
    let engine = UciController::start(&hub, tokio::process::Command::new(path), PROBE_TIMEOUT)
        .map_err(|_| CoreError::new(COULD_NOT_START))?;
    let ready = engine.ready().await;
    engine.close();
    ready.map_err(|_| CoreError::new(COULD_NOT_START))
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|cause| {
            CoreError::wrap("Could not make the Stockfish executable runnable", cause)
        })?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

async fn discard(paths: &[PathBuf]) {
    for path in paths {
        let _ = tokio::fs::remove_file(path).await;
    }
}

/// The asset of a release for `platform` (a Node `process.platform`) and `arch` (`process.arch`).
pub fn pick_stockfish_asset<'a>(
    assets: &'a [ReleaseAsset],
    platform: &str,
    arch: &str,
) -> Option<&'a ReleaseAsset> {
    if platform == "darwin" {
        return if arch == "arm64" || arch == "x64" {
            pick_mac_asset(assets, arch)
        } else {
            None
        };
    }
    let os = match platform {
        "win32" => "windows",
        "linux" => "linux",
        _ => "",
    };
    let cpu = match arch {
        "x64" => "x86-64",
        "arm64" => "arm64",
        _ => "",
    };
    if os.is_empty() || cpu.is_empty() {
        return None;
    }
    // Conservative universal builds dispatch at runtime to supported CPU instructions.
    let extension = if platform == "win32" { "zip" } else { "tar.gz" };
    let name = format!("stockfish-{os}-{cpu}-universal.{extension}");
    assets.iter().find(|asset| asset.name == name)
}

/// The macOS build for this CPU; the universal binary runs on both architectures. The first
/// matching asset, in preference order, wins.
pub fn pick_mac_asset<'a>(assets: &'a [ReleaseAsset], arch: &str) -> Option<&'a ReleaseAsset> {
    let preferred: &[(&str, &str)] = if arch == "arm64" {
        &[
            ("macos", "m1-apple-silicon"),
            ("macos", "apple-silicon"),
            ("macos", "arm64"),
        ]
    } else {
        &[("macos", "x86-64"), ("macos", "x64")]
    };
    preferred
        .iter()
        .chain(std::iter::once(&("macos", "universal")))
        .find_map(|(first, second)| {
            assets
                .iter()
                .find(|asset| matches_in_order(&asset.name, first, second))
        })
}

/// `/first.*second/i` on one name: `second` occurs somewhere after `first`, ignoring ASCII case.
fn matches_in_order(name: &str, first: &str, second: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower
        .find(first)
        .is_some_and(|index| lower[index + first.len()..].contains(second))
}
