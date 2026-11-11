//! The offline voice model that voice input loads. The model is downloaded once, verified against its published
//! SHA-256, unpacked as the `model` folder in the profile's voice folder, and recorded with a
//! marker (`model.json`) listing every file's size and SHA-256. A cache is trusted only while the
//! marker and the files still match; a damaged cache is replaced by the next `ensure`. The
//! recognizer (`recognizer.rs`) reads the folder. A verified `model.tar.gz` from an earlier
//! release is unpacked into the folder instead of downloading the model again.
//!
//! Concurrent `ensure` calls share one preparation, which runs on its own task, so dropping a
//! caller does not abandon the download. Preparation happens in a `.prepare-*` folder inside the
//! voice folder that is removed on every outcome; only a fully prepared archive is published.

use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use kchess_contracts::types::UsageKind;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

pub mod recognizer;

use crate::archive::{self, Refusal};
use crate::download;
use crate::error::{CoreError, ErrorKind, Result};
use crate::host::{Host, Level};
use crate::usage::Meter;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(120);
/// Larger downloads are refused, both from the response header and while streaming.
pub const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// Total uncompressed size the archive may expand to.
pub const MAX_EXTRACTED_BYTES: u64 = 200 * 1024 * 1024;
const MODEL_FOLDER: &str = "model";
const MARKER_FILE: &str = "model.json";
/// What earlier releases published (a repacked archive for the browser recognizer), removed by
/// the next preparation.
const LEGACY_FILES: [&str; 2] = ["model.tar.gz", "model.tar.gz.json"];
const LEGACY_ARCHIVE: &str = LEGACY_FILES[0];
const LEGACY_MARKER: &str = LEGACY_FILES[1];
const CHECKSUM_MESSAGE: &str = "Voice model checksum mismatch. Try downloading it again.";
const INVALID_MESSAGE: &str = "Invalid voice model archive.";
const STOPPED_MESSAGE: &str = "Voice model preparation stopped unexpectedly.";

/// The model KChess ships a download for: its archive's folder name and the SHA-256 of the zip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceModel {
    pub name: String,
    pub url: String,
    pub sha256: String,
}

impl VoiceModel {
    /// The Vosk English model.
    pub fn official() -> VoiceModel {
        VoiceModel {
            name: "vosk-model-small-en-us-0.15".into(),
            url: "https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip".into(),
            sha256: "30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498".into(),
        }
    }
}

/// The archive and model this core prepares: the official download, or the test switches' model.
pub fn active_model(test: Option<&crate::host::TestVoiceModel>) -> (Archive, VoiceModel) {
    if let Some(test) = test {
        let model = VoiceModel {
            name: test.name.clone(),
            url: test.archive.to_string_lossy().into_owned(),
            sha256: test.sha256.clone(),
        };
        return (Archive::File(test.archive.clone()), model);
    }
    let model = VoiceModel::official();
    (Archive::Url(model.url.clone()), model)
}

/// Where the zip comes from: the model's URL, or a local file (a test fixture, so the tests need
/// no network).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Archive {
    Url(String),
    File(PathBuf),
}

pub use kchess_contracts::voice::{Phase, Progress, Status};

/// Receives each progress report. It must not block: the preparation calls it on its own task.
pub type Listener = Arc<dyn Fn(&Progress) + Send + Sync>;

/// The voice folder of the profile whose data directory is `data_dir`, which the model cache
/// prepares the model in.
pub fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("voice")
}

/// The model folder of the profile whose data directory is `data_dir`.
pub fn model_dir(data_dir: &Path) -> PathBuf {
    cache_dir(data_dir).join(MODEL_FOLDER)
}

/// One file of the published folder.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct MarkerFile {
    /// Its path inside the folder, `/`-separated.
    path: String,
    size: u64,
    sha256: String,
}

/// The marker written beside a published model (`model.json`).
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Marker {
    /// The SHA-256 of the downloaded zip the model was prepared from.
    source: String,
    files: Vec<MarkerFile>,
}

impl Marker {
    fn bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }
}

/// The marker an earlier release wrote beside its repacked `model.tar.gz`.
#[derive(Deserialize)]
struct LegacyMarker {
    /// The SHA-256 of the downloaded zip the archive was prepared from.
    source: String,
    /// The SHA-256 of `model.tar.gz`.
    sha256: String,
    size: u64,
}

type Pending = Shared<BoxFuture<'static, std::result::Result<PathBuf, String>>>;

#[derive(Default)]
struct State {
    pending: Option<Pending>,
    progress: Option<Progress>,
    listeners: Vec<(u64, Listener)>,
    next_listener: u64,
}

struct Inner {
    host: Arc<dyn Host>,
    meter: Arc<dyn Meter>,
    directory: PathBuf,
    archive: Archive,
    model: VoiceModel,
    state: Mutex<State>,
}

/// One verified, persistent model per profile. Concurrent `ensure` calls share preparation.
#[derive(Clone)]
pub struct VoiceModelCache {
    inner: Arc<Inner>,
}

impl VoiceModelCache {
    pub fn new(
        host: Arc<dyn Host>,
        meter: Arc<dyn Meter>,
        directory: PathBuf,
        archive: Archive,
        model: VoiceModel,
    ) -> VoiceModelCache {
        VoiceModelCache {
            inner: Arc::new(Inner {
                host,
                meter,
                directory,
                archive,
                model,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// The published model folder (`model` in the voice folder).
    pub fn path(&self) -> PathBuf {
        self.inner.directory.join(MODEL_FOLDER)
    }

    fn marker_path(&self) -> PathBuf {
        self.inner.directory.join(MARKER_FILE)
    }

    /// Whether a verified model is installed, and whether a preparation is running.
    pub async fn status(&self) -> Status {
        if let Some(progress) = self.busy() {
            return busy(progress);
        }
        let bytes = match self.cached().await {
            Some(bytes) => bytes,
            // An earlier release's archive is unpacked by the next `ensure` without a download.
            None => self.legacy().await.map_or(0, |(_, size)| size),
        };
        // Preparation may start while the cache checksums are being checked.
        if let Some(progress) = self.busy() {
            return busy(progress);
        }
        Status {
            installed: bytes > 0,
            bytes,
            busy: false,
            progress: None,
        }
    }

    /// The path of the verified model, preparing it first when needed. Concurrent callers share
    /// one preparation; `listener` receives its progress until this call settles.
    pub async fn ensure(&self, listener: Option<Listener>) -> Result<PathBuf> {
        let (id, pending, replay) = {
            let mut state = self.lock();
            let id = state.next_listener;
            state.next_listener += 1;
            let replay = state.progress.clone();
            if let Some(listener) = &listener {
                state.listeners.push((id, Arc::clone(listener)));
            }
            if state.pending.is_none() {
                state.pending = Some(self.spawn_preparation());
            }
            (id, state.pending.clone(), replay)
        };
        // Also when the call is cancelled or abandoned mid-preparation: its task is dropped at the
        // await, and a listener left behind would repeat every later progress to every window.
        let _forget = crate::tasks::on_drop(|| {
            self.lock()
                .listeners
                .retain(|(listener, _)| *listener != id);
        });
        if let (Some(listener), Some(progress)) = (&listener, replay) {
            listener(&progress);
        }
        let outcome = match pending {
            Some(pending) => pending.await,
            None => Err(STOPPED_MESSAGE.to_string()),
        };
        outcome.map_err(CoreError::new)
    }

    fn spawn_preparation(&self) -> Pending {
        let cache = self.clone();
        let task = tokio::spawn(async move {
            let outcome = cache.prepare().await;
            cache.finish();
            outcome
        });
        async move {
            match task.await {
                Ok(Ok(path)) => Ok(path),
                Ok(Err(error)) => Err(error.message),
                Err(_) => Err(STOPPED_MESSAGE.to_string()),
            }
        }
        .boxed()
        .shared()
    }

    /// Forget the finished preparation so the next `status` or `ensure` starts from scratch.
    fn finish(&self) {
        let mut state = self.lock();
        state.pending = None;
        state.progress = None;
    }

    /// `Some(progress)` while a preparation runs (the progress may not have been reported yet).
    fn busy(&self) -> Option<Option<Progress>> {
        let state = self.lock();
        state.pending.is_some().then(|| state.progress.clone())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        crate::tasks::lock(&self.inner.state)
    }

    fn report(&self, progress: Progress) {
        let listeners: Vec<Listener> = {
            let mut state = self.lock();
            state.progress = Some(progress.clone());
            state
                .listeners
                .iter()
                .map(|(_, listener)| Arc::clone(listener))
                .collect()
        };
        for listener in listeners {
            listener(&progress);
        }
    }

    fn debug(&self, message: &str) {
        self.inner.host.log(Level::Debug, "voice", message);
    }

    /// The installed model's size while the published folder still matches its marker: every
    /// listed file present with its size and checksum. A failure to read them is logged and counts
    /// as not installed; a mismatch is simply not installed.
    async fn cached(&self) -> Option<u64> {
        match self.check().await {
            Ok(bytes) => bytes,
            Err(cause) => {
                self.debug(&format!(
                    "Voice model cache check failed: {}",
                    cause.message
                ));
                None
            }
        }
    }

    async fn check(&self) -> Result<Option<u64>> {
        let text = tokio::fs::read_to_string(self.marker_path())
            .await
            .map_err(|cause| CoreError::wrap("voice model marker read", cause))?;
        let marker: Marker = serde_json::from_str(&text)
            .map_err(|cause| CoreError::wrap("voice model marker parse", cause))?;
        if marker.source != self.inner.model.sha256 || marker.files.is_empty() {
            return Ok(None);
        }
        let folder = self.path();
        let verified = tokio::task::spawn_blocking(move || verify(&folder, &marker))
            .await
            .map_err(|_| CoreError::new(STOPPED_MESSAGE))??;
        Ok(verified)
    }

    /// The path and size of an earlier release's `model.tar.gz` while it still matches its marker
    /// and was prepared from the current model. Anything else is not a usable archive.
    async fn legacy(&self) -> Option<(PathBuf, u64)> {
        let legacy = self.inner.directory.join(LEGACY_ARCHIVE);
        let text = tokio::fs::read_to_string(self.inner.directory.join(LEGACY_MARKER))
            .await
            .ok()?;
        let marker: LegacyMarker = serde_json::from_str(&text).ok()?;
        if marker.source != self.inner.model.sha256 || marker.size == 0 {
            return None;
        }
        let length = tokio::fs::metadata(&legacy).await.ok()?.len();
        if length != marker.size {
            return None;
        }
        let path = legacy.clone();
        let digest = tokio::task::spawn_blocking(move || kchess_delta::file_digest(&path))
            .await
            .ok()?
            .ok()?;
        (digest == marker.sha256).then_some((legacy, marker.size))
    }

    async fn prepare(&self) -> Result<PathBuf> {
        self.report(Progress {
            phase: Phase::Checking,
            received: 0,
            total: None,
        });
        if self.cached().await.is_some() {
            return Ok(self.path());
        }
        let directory = self.inner.directory.clone();
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|cause| {
                CoreError::new(format!("Could not create the voice folder: {cause}"))
            })?;
        let staging = tempfile::Builder::new()
            .prefix(".prepare-")
            .tempdir_in(&directory)
            .map_err(|cause| {
                CoreError::new(format!("Could not create a staging folder: {cause}"))
            })?;
        match self.stage(staging.path()).await {
            Ok(()) => Ok(self.path()),
            Err(cause) => {
                // Connection guidance only when the connection is what failed.
                let advice = match cause.kind {
                    Some(ErrorKind::Offline | ErrorKind::TimedOut) => {
                        " Connect to the internet and try again."
                    }
                    _ => "",
                };
                Err(CoreError::new(format!(
                    "Couldn’t prepare voice input. {}{advice}",
                    cause.message
                )))
            }
        }
    }

    /// Obtain, verify, prepare and publish the model through `staging`. Only a fully prepared
    /// folder reaches the voice folder; a missing marker makes an interrupted install retry.
    async fn stage(&self, staging: &Path) -> Result<()> {
        let marker = match self.migrate(staging).await {
            Some(marker) => marker,
            None => {
                self.report(Progress {
                    phase: Phase::Downloading,
                    received: 0,
                    total: None,
                });
                let zip = staging.join("model.zip");
                let received = self.fetch(&zip).await?;
                self.report(Progress {
                    phase: Phase::Preparing,
                    received,
                    total: None,
                });
                self.prepare_archive(&zip, staging).await?
            }
        };
        let prepared = staging.join("unpacked").join(&self.inner.model.name);
        let path = self.path();
        let marker_file = self.marker_path();
        remove_if_present(&marker_file).await?;
        for legacy in LEGACY_FILES {
            remove_if_present(&self.inner.directory.join(legacy)).await?;
        }
        remove_dir_if_present(&path).await?;
        tokio::fs::rename(&prepared, &path)
            .await
            .map_err(|cause| CoreError::wrap("Could not publish the voice model", cause))?;
        let ready = staging.join("ready.json");
        let text = serde_json::to_vec(&marker).map_err(|cause| {
            CoreError::new(format!("Could not record the voice model: {cause}"))
        })?;
        tokio::fs::write(&ready, text)
            .await
            .map_err(|cause| CoreError::wrap("Could not record the voice model", cause))?;
        tokio::fs::rename(&ready, &marker_file)
            .await
            .map_err(|cause| CoreError::wrap("Could not record the voice model", cause))
    }

    /// Unpack an earlier release's verified `model.tar.gz` into `staging/unpacked`, so upgrading
    /// keeps the model without a download. `None` when there is none or it cannot be unpacked;
    /// the caller then downloads the model, and publishing removes the legacy files.
    async fn migrate(&self, staging: &Path) -> Option<Marker> {
        let (legacy, size) = self.legacy().await?;
        self.report(Progress {
            phase: Phase::Preparing,
            received: size,
            total: None,
        });
        let staging = staging.to_path_buf();
        let model = self.inner.model.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let built = unpack_legacy(&legacy, &staging, &model);
            if built.is_err() {
                let _ = fs::remove_dir_all(staging.join("unpacked"));
            }
            built
        })
        .await;
        match outcome {
            Ok(Ok(marker)) => Some(marker),
            Ok(Err(error)) => {
                self.inner.host.log(
                    Level::Warn,
                    "voice",
                    &format!("Earlier voice model could not be reused: {}", error.message),
                );
                None
            }
            Err(_) => None,
        }
    }

    /// Put the zip into `zip`, returning the bytes kept.
    async fn fetch(&self, zip: &Path) -> Result<u64> {
        match &self.inner.archive {
            Archive::File(source) => {
                let length = tokio::fs::metadata(source)
                    .await
                    .map_err(|cause| {
                        CoreError::new(format!("Voice model is unavailable: {cause}"))
                    })?
                    .len();
                if length > MAX_DOWNLOAD_BYTES {
                    return Err(CoreError::new("Voice model download is too large."));
                }
                tokio::fs::copy(source, zip)
                    .await
                    .map_err(|cause| CoreError::wrap("Could not stage the voice model", cause))?;
                self.report(Progress {
                    phase: Phase::Downloading,
                    received: length,
                    total: Some(length),
                });
                Ok(length)
            }
            Archive::Url(url) => self.download(url, zip).await,
        }
    }

    /// Downloads the zip. The preparation has no caller to cancel it (callers share it and may
    /// leave), so only its deadline ends it.
    async fn download(&self, url: &str, zip: &Path) -> Result<u64> {
        let client = download::client(None)?;
        let limits = download::Limits {
            max_bytes: MAX_DOWNLOAD_BYTES,
            deadline: Some(std::time::Instant::now() + DOWNLOAD_TIMEOUT),
        };
        let never = CancellationToken::new();
        let usage = download::Usage {
            meter: &*self.inner.meter,
            kind: UsageKind::Other,
        };
        let mut body =
            download::open(&client, || client.get(url), limits, &never, usage, &WORDING).await?;
        let mut file = tokio::fs::File::create(zip)
            .await
            .map_err(|cause| CoreError::wrap("Could not stage the voice model", cause))?;
        while let Some(chunk) = body.next().await? {
            file.write_all(&chunk)
                .await
                .map_err(|cause| CoreError::wrap("Could not stage the voice model", cause))?;
            self.report(Progress {
                phase: Phase::Downloading,
                received: body.received(),
                total: body.total(),
            });
        }
        file.flush()
            .await
            .map_err(|cause| CoreError::wrap("Could not stage the voice model", cause))?;
        Ok(body.received())
    }

    /// Verify and unpack the zip on a blocking thread. On timeout the work is told to stop at its
    /// next member and awaited, so the staging folder is only removed once nothing writes to it.
    async fn prepare_archive(&self, zip: &Path, staging: &Path) -> Result<Marker> {
        let cancel = CancellationToken::new();
        let (zip_path, staging_path) = (zip.to_path_buf(), staging.to_path_buf());
        let model = self.inner.model.clone();
        let flag = cancel.clone();
        let mut work =
            tokio::task::spawn_blocking(move || build(&zip_path, &staging_path, &model, &flag));
        let outcome = match tokio::time::timeout(PREPARE_TIMEOUT, &mut work).await {
            Ok(joined) => joined.map_err(|_| CoreError::new(STOPPED_MESSAGE))?,
            Err(_) => {
                cancel.cancel();
                let _ = work.await;
                Err(CoreError::new("Voice model preparation timed out."))
            }
        };
        if let Err(error) = &outcome {
            self.inner.host.log(
                Level::Warn,
                "voice",
                &format!("Voice model preparation failed: {}", error.message),
            );
        }
        outcome
    }
}

fn busy(progress: Option<Progress>) -> Status {
    Status {
        installed: false,
        bytes: 0,
        busy: true,
        progress,
    }
}

async fn remove_if_present(path: &Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(CoreError::wrap("Could not replace the voice model", cause)),
    }
}

async fn remove_dir_if_present(path: &Path) -> Result<()> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(CoreError::wrap("Could not replace the voice model", cause)),
    }
}

/// The total size of the files `marker` lists under `folder` when each still has its size and
/// checksum; `None` on the first that does not.
fn verify(folder: &Path, marker: &Marker) -> Result<Option<u64>> {
    for file in &marker.files {
        let Some(relative) = archive::safe_relative(&file.path) else {
            return Ok(None);
        };
        let path = folder.join(relative);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() && metadata.len() == file.size => {}
            Ok(_) => return Ok(None),
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(cause) => return Err(CoreError::wrap("Could not check the voice model", cause)),
        }
        let digest = kchess_delta::file_digest(&path)
            .map_err(|cause| CoreError::wrap("Could not check the voice model", cause))?;
        if digest != file.sha256 {
            return Ok(None);
        }
    }
    Ok(Some(marker.bytes()))
}

/// Every file under `folder` with its size and SHA-256, sorted by path.
fn describe(folder: &Path) -> io::Result<Vec<MarkerFile>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<MarkerFile>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(io::Error::other)?
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push(MarkerFile {
                    path: relative,
                    size: entry.metadata()?.len(),
                    sha256: kchess_delta::file_digest(&path)?,
                });
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(folder, folder, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// How the model download words its failures.
const WORDING: download::Wording = download::Wording {
    network: error_text,
    status: |status| format!("Voice model download failed: HTTP {status}."),
    too_large: "Voice model download is too large.",
};

/// The message of a failed request, with its causes beneath it. The server's own response text
/// is never included.
fn error_text(cause: &reqwest::Error) -> String {
    let mut text = if cause.is_timeout() {
        "Voice model download timed out.".to_string()
    } else {
        "Voice model download failed (network error).".to_string()
    };
    let mut source = std::error::Error::source(cause);
    while let Some(next) = source {
        text.push_str(": ");
        text.push_str(&next.to_string());
        source = next.source();
    }
    text
}

/// The blocking half of preparation: check the zip's digest, unpack it under the model's folder
/// name in `staging/unpacked`, and describe every file of the result.
fn build(
    zip: &Path,
    staging: &Path,
    model: &VoiceModel,
    cancel: &CancellationToken,
) -> Result<Marker> {
    let digest = kchess_delta::file_digest(zip)
        .map_err(|cause| CoreError::wrap("Could not read the voice model archive", cause))?;
    if digest != model.sha256 {
        return Err(CoreError::new(CHECKSUM_MESSAGE));
    }
    let extracted = staging.join("unpacked");
    extract(zip, &extracted, &model.name, cancel)?;
    let files = describe(&extracted.join(&model.name))
        .map_err(|cause| CoreError::wrap("Could not record the voice model", cause))?;
    Ok(Marker {
        source: model.sha256.clone(),
        files,
    })
}

/// The blocking half of a migration: unpack the files under `name/` of an earlier release's
/// `model.tar.gz` into `staging/unpacked` and describe them.
fn unpack_legacy(archive: &Path, staging: &Path, model: &VoiceModel) -> Result<Marker> {
    let extracted = staging.join("unpacked");
    let reader = |file| flate2::read::GzDecoder::new(BufReader::new(file));
    unpack(archive, &extracted, &model.name, None, |file, to| {
        archive::unpack_tar(reader(file), to)
    })?;
    let files = describe(&extracted.join(&model.name))
        .map_err(|cause| CoreError::wrap("Could not record the voice model", cause))?;
    Ok(Marker {
        source: model.sha256.clone(),
        files,
    })
}

/// Unpack the zip's files under `name/` into `root`.
fn extract(zip: &Path, root: &Path, name: &str, cancel: &CancellationToken) -> Result<()> {
    unpack(zip, root, name, Some(cancel), |file, to| {
        archive::unpack_zip(BufReader::new(file), to)
    })
}

/// Unpack `source` with `unpack` into `root`. Every file must be inside `name/`; links, unsafe
/// paths and archives that expand beyond the budget fail with the invalid-archive message.
fn unpack(
    source: &Path,
    root: &Path,
    name: &str,
    cancel: Option<&CancellationToken>,
    unpack: impl FnOnce(File, &archive::Unpack) -> std::result::Result<usize, Refusal>,
) -> Result<()> {
    fs::create_dir_all(root)
        .map_err(|cause| CoreError::wrap("Could not unpack the voice model", cause))?;
    let file = File::open(source)
        .map_err(|cause| CoreError::wrap("Could not read the voice model archive", cause))?;
    let folder = format!("{name}/");
    let to = archive::Unpack {
        root,
        budget: MAX_EXTRACTED_BYTES,
        prefix: Some(&folder),
        cancel,
    };
    match unpack(file, &to) {
        Ok(0) => Err(CoreError::new("Voice model archive is empty.")),
        Ok(_) => Ok(()),
        Err(Refusal::Cancelled) => {
            Err(CoreError::aborted("Voice model preparation was cancelled."))
        }
        Err(Refusal::Io(cause)) => Err(CoreError::wrap("Could not unpack the voice model", cause)),
        Err(_) => Err(CoreError::new(INVALID_MESSAGE)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_host::TestHost;

    /// A call cancelled while the model is being prepared stops listening: before, its listener
    /// stayed for good and every later progress went to every window once more for each one.
    #[tokio::test]
    async fn a_cancelled_install_leaves_no_listener_behind() {
        // A server that accepts the connection and never answers keeps the preparation running.
        let server = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let url = format!("http://{}/model.zip", server.local_addr().expect("address"));
        let directory = std::env::temp_dir().join(format!(
            "kchess-voice-cancel-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let cache = VoiceModelCache::new(
            TestHost::quiet(),
            Arc::new(crate::usage::Unmetered),
            directory.clone(),
            Archive::Url(url),
            VoiceModel::official(),
        );
        for _ in 0..3 {
            let listener: Listener = Arc::new(|_: &Progress| {});
            let mut install = Box::pin(cache.ensure(Some(listener)));
            tokio::select! {
                biased;
                _ = &mut install => panic!("the preparation cannot finish"),
                () = tokio::task::yield_now() => {}
            }
            assert_eq!(cache.lock().listeners.len(), 1, "listening while it runs");
            drop(install);
            assert!(cache.lock().listeners.is_empty(), "the listener is gone");
        }
        drop(server);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn the_test_switches_replace_the_model() {
        let (archive, model) = active_model(None);
        assert_eq!(model, VoiceModel::official());
        assert_eq!(archive, Archive::Url(VoiceModel::official().url));
        let test = crate::host::TestVoiceModel {
            archive: PathBuf::from("/profile/test-model.zip"),
            name: "test-model".into(),
            sha256: "abc123".into(),
        };
        let (archive, model) = active_model(Some(&test));
        assert_eq!(
            archive,
            Archive::File(PathBuf::from("/profile/test-model.zip"))
        );
        assert_eq!(model.name, "test-model");
        assert_eq!(model.sha256, "abc123");
    }
}
