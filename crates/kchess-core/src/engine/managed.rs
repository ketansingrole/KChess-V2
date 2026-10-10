//! The Stockfish KChess downloads and owns (formerly `core/src/services/managedEngine.ts` and
//! `stockfishAsset.ts`): install state, the verified download of the latest official release,
//! and the atomic replacement of the working engine.
//!
//! Replacement goes through an `EngineHandoff`: the host stops the engine owners and waits until
//! none holds the executable, the new files are renamed into place, then the handoff resumes.
//! A failed download, digest, archive or probe leaves the working engine and its VERSION
//! untouched.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::io::{self, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex as AsyncMutex, watch};

use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

/// The GitHub endpoint for the latest official Stockfish release.
pub const RELEASE_URL: &str =
    "https://api.github.com/repos/official-stockfish/Stockfish/releases/latest";
/// Larger downloads are refused, both up front and while streaming.
pub const MAX_DOWNLOAD_BYTES: u64 = 300 * 1024 * 1024;
/// Total uncompressed size an archive may expand to (zip bombs).
pub const MAX_EXTRACTED_BYTES: u64 = 1024 * 1024 * 1024;
const RELEASE_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the handoff may take to stop the engine owners before the install gives up.
pub const DEFAULT_EXIT_DEADLINE: Duration = Duration::from_secs(4);
const PROBE_OUTPUT_LIMIT: usize = 1_000_000;
const VERSION_FILE: &str = "VERSION";
const USER_AGENT: &str = "KChess";

const EXIT_TIMEOUT_MESSAGE: &str = "Stockfish did not exit. The previous engine was retained.";
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
    pub target: Target,
    /// Suspends engine owners while the verified replacement is published.
    pub handoff: Option<Arc<dyn EngineHandoff>>,
    /// Cancels a running install or delete (`true` cancels).
    pub cancel: Option<watch::Receiver<bool>>,
    /// How long the handoff may take to stop the owners before the install fails.
    pub exit_deadline: Duration,
}

impl ManagedLocation {
    pub fn new(dir: PathBuf) -> ManagedLocation {
        ManagedLocation {
            dir,
            release_url: RELEASE_URL.to_string(),
            target: Target::current(),
            handoff: None,
            cancel: None,
            exit_deadline: DEFAULT_EXIT_DEADLINE,
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

/// What the managed directory holds (`ManagedEngine` in the TypeScript contracts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedEngine {
    pub installed: bool,
    pub path: PathBuf,
    /// Release tag recorded when it was downloaded, e.g. `sf_17.1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

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
        Ok(_) => return ManagedEngine::absent(path),
        Err(cause) => {
            host.log(
                Level::Debug,
                "managed-engine",
                &format!("Managed engine check failed: {} {cause}", dir.display()),
            );
            return ManagedEngine::absent(path);
        }
    }
    let version = tokio::fs::read_to_string(dir.join(VERSION_FILE))
        .await
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    ManagedEngine {
        installed: true,
        path,
        version,
    }
}

impl ManagedEngine {
    fn absent(path: PathBuf) -> ManagedEngine {
        ManagedEngine {
            installed: false,
            path,
            version: None,
        }
    }
}

/// Physical engine directories may be shared by profiles; mutations of one directory are
/// serialized. The map keeps only weak references, so idle directories are forgotten.
fn dir_lock(dir: &Path) -> Arc<AsyncMutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(Default::default);
    let mut locks = match locks.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
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
    let lock = dir_lock(&location.dir);
    let _held = lock.lock().await;
    let result = install(host, location).await;
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
        .map_err(|_| CoreError::new(EXIT_TIMEOUT_MESSAGE))??;
    let result = commit().await;
    drop(guard);
    result
}

async fn install(host: &dyn Host, location: &ManagedLocation) -> Result<InstallResult> {
    let dir = location.dir.clone();
    let mut cancel = location.cancel.clone();
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| CoreError::new("Could not start the Stockfish download."))?;

    let release = fetch_release(host, &client, &location.release_url, &mut cancel).await?;
    let current = managed_engine(host, &dir).await;
    if current.installed && current.version.as_deref() == Some(release.tag.as_str()) {
        return Ok(InstallResult {
            installed: true,
            path: current.path,
            version: release.tag,
            updated: false,
        });
    }
    let asset = pick_stockfish_asset(
        &release.assets,
        location.target.platform,
        location.target.arch,
    )
    .ok_or_else(|| CoreError::new("No compatible official Stockfish download was found."))?
    .clone();
    let (Some(url), Some(size)) = (asset.browser_download_url.clone(), asset.size) else {
        return Err(CoreError::new("Invalid Stockfish download metadata."));
    };
    if !size.is_finite() || !is_plain_file_name(&asset.name) {
        return Err(CoreError::new("Invalid Stockfish download metadata."));
    }
    let expected = expected_digest(asset.digest.as_deref()).ok_or_else(|| {
        CoreError::new("The Stockfish release has no SHA-256 digest, so it cannot be verified.")
    })?;
    if size > MAX_DOWNLOAD_BYTES as f64 {
        return Err(CoreError::new("Stockfish download is unexpectedly large."));
    }

    let staging = tempfile::Builder::new()
        .prefix("kchess-stockfish-")
        .tempdir()
        .map_err(|cause| CoreError::new(format!("Could not create a staging folder: {cause}")))?;
    let archive = staging.path().join(&asset.name);
    let mut received = 0_u64;
    let downloaded = download_archive(&client, &url, &archive, &mut cancel, &mut received).await;
    usage(host, 0, received);
    let digest = downloaded?;
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
        .map_err(|cause| io_error("Could not create the Stockfish folder", cause))?;
    let path = dir.join(executable_name());
    let pending = dir.join(format!("{}.pending", executable_name()));
    let pending_version = dir.join(format!("{VERSION_FILE}.pending"));
    let version_text = format!("{}\n", release.tag);
    let prepared = async {
        tokio::fs::copy(&source, &pending)
            .await
            .map_err(|cause| io_error("Could not stage the Stockfish executable", cause))?;
        set_executable(&pending)?;
        // Verify UCI before replacing the working engine; this catches wrong architecture and
        // broken downloads.
        probe_engine(&pending).await?;
        tokio::fs::write(&pending_version, &version_text)
            .await
            .map_err(|cause| io_error("Could not stage the Stockfish version", cause))
    }
    .await;
    if let Err(error) = prepared {
        discard(&[pending, pending_version]).await;
        return Err(error);
    }

    let commit = {
        let (pending, path) = (pending.clone(), path.clone());
        let (pending_version, version_path) = (pending_version.clone(), dir.join(VERSION_FILE));
        move || async move {
            tokio::fs::rename(&pending, &path)
                .await
                .map_err(|cause| io_error("Could not replace the Stockfish executable", cause))?;
            tokio::fs::rename(&pending_version, &version_path)
                .await
                .map_err(|cause| io_error("Could not record the Stockfish version", cause))
        }
    };
    if let Err(error) = with_handoff(location, commit).await {
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

async fn fetch_release(
    host: &dyn Host,
    client: &reqwest::Client,
    url: &str,
    cancel: &mut Option<watch::Receiver<bool>>,
) -> Result<Release> {
    let response = until_cancelled(cancel, async {
        client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .timeout(RELEASE_TIMEOUT)
            .send()
            .await
            .map_err(|cause| {
                CoreError::new(if cause.is_timeout() {
                    "Could not check Stockfish releases (timed out)."
                } else {
                    "Could not check Stockfish releases (network error)."
                })
            })
    })
    .await?;
    usage(host, 1, 0);
    if !response.status().is_success() {
        return Err(CoreError::new(format!(
            "Could not check Stockfish releases ({}).",
            response.status().as_u16()
        )));
    }
    let value: Value = response
        .json()
        .await
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

/// Stream the download into `archive`, hashing as it goes. `received` counts the bytes kept.
async fn download_archive(
    client: &reqwest::Client,
    url: &str,
    archive: &Path,
    cancel: &mut Option<watch::Receiver<bool>>,
    received: &mut u64,
) -> Result<String> {
    let mut response = until_cancelled(cancel, async {
        client
            .get(url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|cause| {
                CoreError::new(if cause.is_timeout() {
                    "Stockfish download timed out."
                } else {
                    "Stockfish download failed (network error)."
                })
            })
    })
    .await?;
    if !response.status().is_success() {
        return Err(CoreError::new(format!(
            "Stockfish download failed ({}).",
            response.status().as_u16()
        )));
    }
    let mut file = tokio::fs::File::create(archive)
        .await
        .map_err(|cause| io_error("Could not stage the Stockfish download", cause))?;
    let mut hasher = Sha256::new();
    loop {
        let chunk = until_cancelled(cancel, async {
            response.chunk().await.map_err(|cause| {
                CoreError::new(if cause.is_timeout() {
                    "Stockfish download timed out."
                } else {
                    "Stockfish download failed (network error)."
                })
            })
        })
        .await?;
        let Some(chunk) = chunk else { break };
        *received += chunk.len() as u64;
        if *received > MAX_DOWNLOAD_BYTES {
            return Err(CoreError::new("Stockfish download is unexpectedly large."));
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|cause| io_error("Could not stage the Stockfish download", cause))?;
    }
    file.flush()
        .await
        .map_err(|cause| io_error("Could not stage the Stockfish download", cause))?;
    Ok(hex(&hasher.finalize()))
}

fn usage(host: &dyn Host, requests: u32, bytes: u64) {
    host.emit(
        "host:usage",
        json!({ "account": "", "category": "other", "requests": requests, "bytes": bytes }),
    );
}

/// Run `work` unless the install is cancelled first; a cancelled install fails as an abort.
async fn until_cancelled<T>(
    cancel: &mut Option<watch::Receiver<bool>>,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancelled(cancel) => Err(CoreError::aborted("This operation was aborted")),
        result = work => result,
    }
}

async fn cancelled(cancel: &mut Option<watch::Receiver<bool>>) {
    if let Some(receiver) = cancel {
        loop {
            if *receiver.borrow() {
                return;
            }
            if receiver.changed().await.is_err() {
                break;
            }
        }
    }
    std::future::pending::<()>().await
}

/// The lowercase hex digest from GitHub's `sha256:<hex>` form, or `None` if absent or malformed.
fn expected_digest(digest: Option<&str>) -> Option<String> {
    let lower = digest?.to_ascii_lowercase();
    let hex = lower.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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

/// Extract the archive and locate the engine executable inside it, off the async runtime.
async fn extract_engine(
    kind: ArchiveKind,
    archive: PathBuf,
    extracted: PathBuf,
) -> Result<PathBuf> {
    let work = tokio::task::spawn_blocking(move || -> Result<Option<PathBuf>> {
        extract(kind, &archive, &extracted)?;
        find_binary(&extracted)
    });
    let found = tokio::time::timeout(EXTRACT_TIMEOUT, work)
        .await
        .map_err(|_| CoreError::new("Stockfish archive could not be extracted."))?
        .map_err(|_| CoreError::new("Stockfish archive could not be extracted."))??;
    found.ok_or_else(|| CoreError::new("Stockfish executable was missing from the release."))
}

/// Extract `archive` into `root`. Only regular files and directories are written, with paths
/// kept strictly inside `root`; links, devices and absolute or `..` paths fail the extraction.
fn extract(kind: ArchiveKind, archive: &Path, root: &Path) -> Result<()> {
    fs::create_dir_all(root)
        .map_err(|cause| io_error("Could not extract the Stockfish archive", cause))?;
    let file = fs::File::open(archive)
        .map_err(|cause| io_error("Could not read the Stockfish archive", cause))?;
    let mut budget = MAX_EXTRACTED_BYTES;
    match kind {
        ArchiveKind::Zip => unpack_zip(BufReader::new(file), root, &mut budget),
        ArchiveKind::TarGz => unpack_tar(
            flate2::read::GzDecoder::new(BufReader::new(file)),
            root,
            &mut budget,
        ),
        ArchiveKind::Tar => unpack_tar(BufReader::new(file), root, &mut budget),
    }
}

fn unpack_tar<R: Read>(reader: R, root: &Path, budget: &mut u64) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(false);
    archive.set_preserve_mtime(false);
    let entries = archive.entries().map_err(|cause| extract_error(&cause))?;
    for entry in entries {
        let mut entry = entry.map_err(|cause| extract_error(&cause))?;
        let name = entry
            .path()
            .map_err(|cause| extract_error(&cause))?
            .to_string_lossy()
            .into_owned();
        let relative = safe_relative(&name)?;
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(CoreError::new("Stockfish archive contains a link."));
        }
        if kind.is_dir() {
            create_dir(root, &relative)?;
        } else if kind.is_file() {
            write_file(root, &relative, &mut entry, budget)?;
        } else {
            return Err(CoreError::new(
                "Stockfish archive contains an unsupported entry.",
            ));
        }
    }
    Ok(())
}

fn unpack_zip<R: Read + io::Seek>(reader: R, root: &Path, budget: &mut u64) -> Result<()> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|cause| extract_error(&cause))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|cause| extract_error(&cause))?;
        // S_IFLNK in the Unix mode bits marks a symbolic link.
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(CoreError::new("Stockfish archive contains a link."));
        }
        let relative = safe_relative(entry.name())?;
        if entry.is_dir() {
            create_dir(root, &relative)?;
        } else {
            write_file(root, &relative, &mut entry, budget)?;
        }
    }
    Ok(())
}

/// A member name as a path inside the archive root. Absolute paths, `..`, drive prefixes and
/// backslashes (a separator on Windows) are refused; `.` components are dropped.
fn safe_relative(name: &str) -> Result<PathBuf> {
    let unsafe_path = || CoreError::new("Stockfish archive has an unsafe path.");
    if name.contains('\\') {
        return Err(unsafe_path());
    }
    let mut relative = PathBuf::new();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => return Err(unsafe_path()),
        }
    }
    Ok(relative)
}

fn create_dir(root: &Path, relative: &Path) -> Result<()> {
    fs::create_dir_all(root.join(relative))
        .map_err(|cause| io_error("Could not extract the Stockfish archive", cause))
}

fn write_file(
    root: &Path,
    relative: &Path,
    reader: &mut impl Read,
    budget: &mut u64,
) -> Result<()> {
    if relative.as_os_str().is_empty() {
        return Err(CoreError::new("Stockfish archive has an unsafe path."));
    }
    let target = root.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|cause| io_error("Could not extract the Stockfish archive", cause))?;
    }
    let mut file = fs::File::create(&target)
        .map_err(|cause| io_error("Could not extract the Stockfish archive", cause))?;
    let copied = io::copy(&mut reader.by_ref().take(*budget + 1), &mut file)
        .map_err(|cause| io_error("Could not extract the Stockfish archive", cause))?;
    if copied > *budget {
        return Err(CoreError::new("Stockfish archive is unexpectedly large."));
    }
    *budget -= copied;
    Ok(())
}

fn extract_error(cause: &dyn std::fmt::Display) -> CoreError {
    CoreError::new(format!("Could not extract the Stockfish archive: {cause}"))
}

/// The executable inside an extracted release: the first `stockfish*` file that is not
/// documentation. Links are not followed.
fn find_binary(root: &Path) -> Result<Option<PathBuf>> {
    let entries = fs::read_dir(root)
        .map_err(|cause| io_error("Could not read the extracted Stockfish archive", cause))?;
    for entry in entries {
        let entry = entry
            .map_err(|cause| io_error("Could not read the extracted Stockfish archive", cause))?;
        let file_type = entry
            .file_type()
            .map_err(|cause| io_error("Could not read the extracted Stockfish archive", cause))?;
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

/// Run the candidate executable and require a `uciok` line within the probe deadline.
async fn probe_engine(path: &Path) -> Result<()> {
    match tokio::time::timeout(PROBE_TIMEOUT, run_probe(path)).await {
        Ok(result) => result,
        Err(_) => Err(CoreError::new(COULD_NOT_START)),
    }
}

async fn run_probe(path: &Path) -> Result<()> {
    let mut child = tokio::process::Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| CoreError::new(COULD_NOT_START))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A broken build can exit before reading: the write then fails, and the exit below
        // reports the probe as failed.
        let _ = stdin.write_all(b"uci\n").await;
    }
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| CoreError::new(COULD_NOT_START))?;
    let mut output = String::new();
    let mut scanned = 0_usize;
    let mut verified = false;
    let mut buffer = [0_u8; 8192];
    let mut outcome = Ok(());
    while let Ok(read) = stdout.read(&mut buffer).await {
        if read == 0 {
            // The end of output is the end of a last line without a newline.
            verified = is_uciok(&output[scanned..]);
            break;
        }
        output.push_str(&String::from_utf8_lossy(&buffer[..read]));
        let mut start = scanned;
        while let Some(offset) = output[start..].find('\n') {
            if is_uciok(&output[start..start + offset]) {
                verified = true;
                break;
            }
            start += offset + 1;
        }
        scanned = start;
        if verified || is_uciok(&output[scanned..]) {
            verified = true;
            break;
        }
        if output.len() > PROBE_OUTPUT_LIMIT {
            outcome = Err(CoreError::new("Invalid Stockfish output."));
            break;
        }
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
    if verified {
        return Ok(());
    }
    outcome.and(Err(CoreError::new(COULD_NOT_START)))
}

/// `/^uciok\s*$/` on one line.
fn is_uciok(line: &str) -> bool {
    line.strip_prefix("uciok")
        .is_some_and(|rest| rest.trim().is_empty())
}

fn io_error(context: &str, cause: io::Error) -> CoreError {
    CoreError::new(format!("{context}: {cause}"))
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|cause| io_error("Could not make the Stockfish executable runnable", cause))?;
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
