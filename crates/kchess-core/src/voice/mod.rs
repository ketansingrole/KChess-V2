//! The offline voice model that voice input loads (formerly `core/src/services/voiceModel.ts`
//! and `voiceModelWorker.ts`). The model is downloaded once, verified against its published
//! SHA-256, unpacked and repacked as `model.tar.gz` in the profile's voice folder, and recorded
//! with a marker (`model.tar.gz.json`). A cache is trusted only while the marker and the file
//! still match; a damaged cache is replaced by the next `ensure`.
//!
//! Concurrent `ensure` calls share one preparation, which runs on its own task, so dropping a
//! caller does not abandon the download. Preparation happens in a `.prepare-*` folder inside the
//! voice folder that is removed on every outcome; only a fully prepared archive is published.

use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::archive::{self, Member};
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(120);
/// Larger downloads are refused, both from the response header and while streaming.
pub const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// Total uncompressed size the archive may expand to.
pub const MAX_EXTRACTED_BYTES: u64 = 200 * 1024 * 1024;
const MODEL_FILE: &str = "model.tar.gz";
const USER_AGENT: &str = "KChess";
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
    /// The Vosk English model (`VOICE_MODEL` in the TypeScript core).
    pub fn official() -> VoiceModel {
        VoiceModel {
            name: "vosk-model-small-en-us-0.15".into(),
            url: "https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip".into(),
            sha256: "30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498".into(),
        }
    }
}

/// Where the zip comes from: the model's URL, or a local file (the e2e fixture, which
/// `tooling/prepare-voice-model.mjs` supplies so the tests need no network).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Archive {
    Url(String),
    File(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Checking,
    Downloading,
    Preparing,
}

/// What the preparation is doing (`VoiceModelProgress`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    pub received: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// What the directory holds (`VoiceModelStatus`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    pub bytes: u64,
    pub busy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
}

/// Receives each progress report. It must not block: the preparation calls it on its own task.
pub type Listener = Arc<dyn Fn(&Progress) + Send + Sync>;

/// The marker written beside a published model (`model.tar.gz.json`).
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Marker {
    /// The SHA-256 of the downloaded zip the model was prepared from.
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
    directory: PathBuf,
    archive: Archive,
    model: VoiceModel,
    client: reqwest::Client,
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
        directory: PathBuf,
        archive: Archive,
        model: VoiceModel,
    ) -> VoiceModelCache {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        VoiceModelCache {
            inner: Arc::new(Inner {
                host,
                directory,
                archive,
                model,
                client,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// The published archive (`model.tar.gz` in the voice folder).
    pub fn path(&self) -> PathBuf {
        self.inner.directory.join(MODEL_FILE)
    }

    /// Whether a verified model is installed, and whether a preparation is running.
    pub async fn status(&self) -> Status {
        if let Some(progress) = self.busy() {
            return busy(progress);
        }
        let installed = self.cached().await;
        // Preparation may start while the cache checksum is being checked.
        if let Some(progress) = self.busy() {
            return busy(progress);
        }
        if !installed {
            return Status {
                installed: false,
                bytes: 0,
                busy: false,
                progress: None,
            };
        }
        let bytes = match tokio::fs::metadata(self.path()).await {
            Ok(metadata) => metadata.len(),
            Err(cause) => {
                self.debug(&format!("Voice model file is missing: {cause}"));
                0
            }
        };
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
        if let (Some(listener), Some(progress)) = (&listener, replay) {
            listener(&progress);
        }
        let outcome = match pending {
            Some(pending) => pending.await,
            None => Err(STOPPED_MESSAGE.to_string()),
        };
        self.lock()
            .listeners
            .retain(|(listener, _)| *listener != id);
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
        match self.inner.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
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

    /// Whether the published archive still matches its marker and checksum. A failure to read
    /// them is logged and counts as not installed; a mismatch is simply not installed.
    async fn cached(&self) -> bool {
        match self.check().await {
            Ok(installed) => installed,
            Err(cause) => {
                self.debug(&format!(
                    "Voice model cache check failed: {}",
                    cause.message
                ));
                false
            }
        }
    }

    async fn check(&self) -> Result<bool> {
        let path = self.path();
        let text = tokio::fs::read_to_string(marker_path(&path))
            .await
            .map_err(|cause| CoreError::new(cause.to_string()))?;
        let marker: Marker =
            serde_json::from_str(&text).map_err(|cause| CoreError::new(cause.to_string()))?;
        if marker.source != self.inner.model.sha256 || marker.size == 0 {
            return Ok(false);
        }
        let length = tokio::fs::metadata(&path)
            .await
            .map_err(|cause| CoreError::new(cause.to_string()))?
            .len();
        if length != marker.size {
            return Ok(false);
        }
        Ok(blocking_sha256(path).await? == marker.sha256)
    }

    async fn prepare(&self) -> Result<PathBuf> {
        self.report(Progress {
            phase: Phase::Checking,
            received: 0,
            total: None,
        });
        if self.cached().await {
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
            Err(cause) => Err(CoreError::new(format!(
                "Couldn’t prepare voice input. {} Connect to the internet and try again.",
                cause.message
            ))),
        }
    }

    /// Obtain, verify, prepare and publish the model through `staging`. Only a fully prepared
    /// archive reaches the voice folder; a missing marker makes an interrupted install retry.
    async fn stage(&self, staging: &Path) -> Result<()> {
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
        let marker = self.prepare_archive(&zip, staging).await?;
        let prepared = staging.join(MODEL_FILE);
        let path = self.path();
        let marker_file = marker_path(&path);
        remove_if_present(&marker_file).await?;
        remove_if_present(&path).await?;
        tokio::fs::rename(&prepared, &path)
            .await
            .map_err(|cause| io_error("Could not publish the voice model", cause))?;
        let ready = staging.join("ready.json");
        let text = serde_json::to_vec(&marker).map_err(|cause| {
            CoreError::new(format!("Could not record the voice model: {cause}"))
        })?;
        tokio::fs::write(&ready, text)
            .await
            .map_err(|cause| io_error("Could not record the voice model", cause))?;
        tokio::fs::rename(&ready, &marker_file)
            .await
            .map_err(|cause| io_error("Could not record the voice model", cause))
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
                    .map_err(|cause| io_error("Could not stage the voice model", cause))?;
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

    async fn download(&self, url: &str, zip: &Path) -> Result<u64> {
        let mut response = self
            .inner
            .client
            .get(url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|cause| CoreError::new(error_text(&cause)))?;
        if !response.status().is_success() {
            return Err(CoreError::new(format!(
                "Voice model download failed: HTTP {}.",
                response.status().as_u16()
            )));
        }
        let total = response.content_length().filter(|total| *total > 0);
        if total.is_some_and(|total| total > MAX_DOWNLOAD_BYTES) {
            return Err(CoreError::new("Voice model download is too large."));
        }
        let mut file = tokio::fs::File::create(zip)
            .await
            .map_err(|cause| io_error("Could not stage the voice model", cause))?;
        let mut received = 0_u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|cause| CoreError::new(error_text(&cause)))?
        {
            received += chunk.len() as u64;
            if received > MAX_DOWNLOAD_BYTES {
                return Err(CoreError::new("Voice model download is too large."));
            }
            file.write_all(&chunk)
                .await
                .map_err(|cause| io_error("Could not stage the voice model", cause))?;
            self.report(Progress {
                phase: Phase::Downloading,
                received,
                total,
            });
        }
        file.flush()
            .await
            .map_err(|cause| io_error("Could not stage the voice model", cause))?;
        Ok(received)
    }

    /// Verify and repack the zip on a blocking thread. On timeout the work is told to stop at its
    /// next member and awaited, so the staging folder is only removed once nothing writes to it.
    async fn prepare_archive(&self, zip: &Path, staging: &Path) -> Result<Marker> {
        let cancel = Arc::new(AtomicBool::new(false));
        let (zip_path, staging_path) = (zip.to_path_buf(), staging.to_path_buf());
        let model = self.inner.model.clone();
        let flag = Arc::clone(&cancel);
        let mut work =
            tokio::task::spawn_blocking(move || build(&zip_path, &staging_path, &model, &flag));
        let outcome = match tokio::time::timeout(PREPARE_TIMEOUT, &mut work).await {
            Ok(joined) => joined.map_err(|_| CoreError::new(STOPPED_MESSAGE))?,
            Err(_) => {
                cancel.store(true, Ordering::SeqCst);
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

fn marker_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".json");
    PathBuf::from(name)
}

async fn remove_if_present(path: &Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(io_error("Could not replace the voice model", cause)),
    }
}

async fn blocking_sha256(path: PathBuf) -> Result<String> {
    tokio::task::spawn_blocking(move || archive::sha256_file(&path))
        .await
        .map_err(|_| CoreError::new(STOPPED_MESSAGE))?
        .map_err(|cause| io_error("Could not check the voice model", cause))
}

fn io_error(context: &str, cause: io::Error) -> CoreError {
    CoreError::new(format!("{context}: {cause}"))
}

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
/// name, repack it as `model.tar.gz` in `staging`, and describe the result.
fn build(zip: &Path, staging: &Path, model: &VoiceModel, cancel: &AtomicBool) -> Result<Marker> {
    let digest = archive::sha256_file(zip)
        .map_err(|cause| io_error("Could not read the voice model archive", cause))?;
    if digest != model.sha256 {
        return Err(CoreError::new(CHECKSUM_MESSAGE));
    }
    let extracted = staging.join("unpacked");
    extract(zip, &extracted, &model.name, cancel)?;
    let prepared = staging.join(MODEL_FILE);
    pack(&extracted, &model.name, &prepared)?;
    let size = fs::metadata(&prepared)
        .map_err(|cause| io_error("Could not record the voice model", cause))?
        .len();
    let sha256 = archive::sha256_file(&prepared)
        .map_err(|cause| io_error("Could not record the voice model", cause))?;
    Ok(Marker {
        source: model.sha256.clone(),
        sha256,
        size,
    })
}

/// Unpack the files under `name/` into `root`. Every member must be inside that folder, and links,
/// unsafe paths and archives that expand beyond the budget fail with the invalid-archive message.
/// Directory entries are created implicitly from the files beneath them.
fn extract(zip: &Path, root: &Path, name: &str, cancel: &AtomicBool) -> Result<()> {
    fs::create_dir_all(root)
        .map_err(|cause| io_error("Could not unpack the voice model", cause))?;
    let file = File::open(zip)
        .map_err(|cause| io_error("Could not read the voice model archive", cause))?;
    let mut archive =
        zip::ZipArchive::new(BufReader::new(file)).map_err(|_| CoreError::new(INVALID_MESSAGE))?;
    let folder = format!("{name}/");
    let mut budget = MAX_EXTRACTED_BYTES;
    let mut files = 0_usize;
    for index in 0..archive.len() {
        if cancel.load(Ordering::SeqCst) {
            return Err(CoreError::aborted("Voice model preparation was cancelled."));
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|_| CoreError::new(INVALID_MESSAGE))?;
        if entry.unix_mode().is_some_and(archive::is_symlink_mode) {
            return Err(CoreError::new(INVALID_MESSAGE));
        }
        let member = entry.name().to_string();
        if !member.starts_with(&folder) {
            return Err(CoreError::new(INVALID_MESSAGE));
        }
        if entry.is_dir() {
            continue;
        }
        let relative =
            archive::safe_relative(&member).ok_or_else(|| CoreError::new(INVALID_MESSAGE))?;
        match archive::write_member(root, &relative, &mut entry, &mut budget)
            .map_err(|cause| io_error("Could not unpack the voice model", cause))?
        {
            Member::Written => files += 1,
            Member::OverBudget => return Err(CoreError::new(INVALID_MESSAGE)),
        }
    }
    if files == 0 {
        return Err(CoreError::new("Voice model archive is empty."));
    }
    Ok(())
}

/// Write `extracted/name` as a gzip-compressed tar with deterministic headers, so the same files
/// always repack to the same bytes.
fn pack(extracted: &Path, name: &str, prepared: &Path) -> Result<()> {
    let file = File::create(prepared)
        .map_err(|cause| io_error("Could not record the voice model", cause))?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);
    builder.follow_symlinks(false);
    builder
        .append_dir_all(name, extracted.join(name))
        .map_err(|cause| io_error("Could not record the voice model", cause))?;
    let encoder = builder
        .into_inner()
        .map_err(|cause| io_error("Could not record the voice model", cause))?;
    encoder
        .finish()
        .map(drop)
        .map_err(|cause| io_error("Could not record the voice model", cause))
}
