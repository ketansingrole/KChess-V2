//! App updates: the GPUI app checks the release feed
//! (GitHub releases of KChess), downloads the package it was installed from, checks it against the release's
//! `SHA256SUMS.txt` (itself signed with the release key, `update-key.pub`) and against the
//! running app's signature, and installs it on restart or when
//! KChess quits: a new bundle on macOS, the installer run silently on Windows, a new AppImage, or the .deb through the package manager.
//!
//! Only an installed package updates: a development build reports updates as manual, unless
//! `KCHESS_UPDATE_FEED` points it at another feed (tests serve one locally).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use futures::StreamExt as _;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use kchess_contracts::Level;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::settings::{SettingsChanged, SettingsHandle};

mod patch;

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
/// The name KChess's packages carry.
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

/// The release key's public half (`update-key.pub`): its one line that is not a comment, the
/// base64 of a 32-byte Ed25519 key. `None` until a key is made (`cargo xtask update-key`).
pub fn update_key() -> Option<Vec<u8>> {
    parse_update_key(include_str!("../update-key.pub"))
}

pub fn parse_update_key(file: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let line = file
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;
    base64::engine::general_purpose::STANDARD
        .decode(line)
        .ok()
        .filter(|key| key.len() == 32)
}

/// Whether `signature` (base64, `SHA256SUMS.txt.sig`) is `key`'s Ed25519 signature of `sums`.
pub fn sums_signed_by(key: &[u8], sums: &[u8], signature: &str) -> Result<(), String> {
    use base64::Engine as _;
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .map_err(|_| "The release's checksum signature could not be read")?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(sums, &signature)
        .map_err(|_| "The release's checksums are not signed by KChess".to_string())
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

/// An error and every cause under it, as one line.
fn causes(error: &dyn std::error::Error) -> String {
    let mut line = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        line.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    line
}

/// `message` for the person; the request's own error for the log, which the message alone
/// hides (a download that failed one run in forty left nothing to go on).
fn failed(message: &'static str) -> impl FnOnce(reqwest::Error) -> String {
    move |error| {
        crate::log::write(
            Level::Warn,
            "updates",
            &format!("{message}: {}", causes(&error)),
        );
        message.to_string()
    }
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
        .user_agent(kchess_contracts::USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        // A server that stops sending ends the download instead of holding it forever.
        .read_timeout(IDLE)
        // Byte-exact: the installer's size and progress are of the file as published.
        .no_gzip()
        .build()
        .map_err(|cause| cause.to_string())
}

/// How many times a small request (the check, the checksums, their signature) is made before
/// it fails: a dropped connection or a busy server answers the next try, not six hours later.
const TRIES: u32 = 3;

/// One small request's answer, or why it failed: `Retry` for what a later try may not meet
/// (no connection, a timeout, a server error, too many requests), `Fail` for the rest.
enum Answer {
    Fail(String),
    Retry(String),
}

/// `request` until it answers, at most `TRIES` times, waiting as a resumed download does
/// (`backoff`) between tries.
async fn retried<T, F: Future<Output = Result<T, Answer>>>(
    what: &str,
    mut request: impl FnMut() -> F,
) -> Result<T, String> {
    let mut tries = 1;
    loop {
        match request().await {
            Ok(answer) => return Ok(answer),
            Err(Answer::Fail(cause)) => return Err(cause),
            Err(Answer::Retry(cause)) if tries < TRIES => {
                log(&format!("{what}: {cause}, trying again ({tries}/{TRIES})"));
                tokio::time::sleep(backoff(tries)).await;
                tries += 1;
            }
            Err(Answer::Retry(cause)) => return Err(cause),
        }
    }
}

/// The answer to a small request as `Answer`: `failure` when there was none, `refused` for a
/// failing status. A server error or `429` may pass; any other failing status will not.
fn answered(
    response: Result<reqwest::Response, reqwest::Error>,
    failure: &'static str,
    refused: impl FnOnce(reqwest::StatusCode) -> String,
) -> Result<reqwest::Response, Answer> {
    let response = response.map_err(failed(failure)).map_err(Answer::Retry)?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    log(&format!("{failure}: the server answered {status}"));
    let cause = refused(status);
    if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        Err(Answer::Retry(cause))
    } else {
        Err(Answer::Fail(cause))
    }
}

async fn latest(feed: String) -> Result<Release, String> {
    let url = format!("{feed}/repos/{REPOSITORY}/releases/latest");
    let client = http()?;
    retried("check", || async {
        let response = client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(30))
            .send()
            .await;
        answered(response, "Could not reach the update server", |status| {
            format!("The update server answered {status}.")
        })?
        .json::<Release>()
        .await
        .map_err(failed("The update server's answer could not be read"))
        .map_err(Answer::Retry)
    })
    .await
}

/// How many times a download that stops part way picks up again before it fails.
const RESUMES: u32 = 4;
/// The wait before the first resume; each later one waits twice as long.
const RESUME_AFTER: Duration = if cfg!(test) {
    Duration::from_millis(10)
} else {
    Duration::from_secs(2)
};

/// The wait before try (or resume) `attempt`, counting from 1: `RESUME_AFTER`, doubled each time.
fn backoff(attempt: u32) -> Duration {
    RESUME_AFTER * 2u32.pow(attempt.saturating_sub(1))
}

/// How often a download reports its progress: every this many bytes, and at its end. One report
/// for every chunk the server sent was thousands of redraws of the window for one update.
const PROGRESS_STEP: u64 = 1 << 20;

/// A download so far, in its `.part` file: the digest of what it holds, and how much of how much.
/// Written through a buffer on the download's own thread (`on_runtime`), not a thread hop for
/// each chunk.
struct Part {
    file: std::io::BufWriter<std::fs::File>,
    hasher: Sha256,
    received: u64,
    total: u64,
    reported: u64,
}

impl Part {
    /// The part at `path`, carrying on from what an earlier download left in it.
    fn open(path: &Path, total: u64) -> std::io::Result<Part> {
        use std::io::{Read as _, Seek as _};
        let mut file = std::fs::File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        let mut hasher = Sha256::new();
        let mut received = 0u64;
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            received += read as u64;
        }
        file.seek(std::io::SeekFrom::End(0))?;
        Ok(Part {
            file: std::io::BufWriter::with_capacity(1 << 20, file),
            hasher,
            received,
            total,
            reported: 0,
        })
    }

    /// Start the file again from nothing: the server sent the whole package, not the rest.
    fn restart(&mut self) -> std::io::Result<()> {
        use std::io::{Seek as _, Write as _};
        self.file.flush()?;
        self.file.get_mut().set_len(0)?;
        self.file.get_mut().seek(std::io::SeekFrom::Start(0))?;
        self.hasher = Sha256::new();
        self.received = 0;
        self.reported = 0;
        Ok(())
    }

    fn write(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        std::io::Write::write_all(&mut self.file, chunk)?;
        self.hasher.update(chunk);
        self.received += chunk.len() as u64;
        Ok(())
    }

    /// Tell `progress` how far it is, once a step further on or at the end.
    fn report(&mut self, progress: &futures::channel::mpsc::UnboundedSender<(u64, u64)>) {
        if self.received - self.reported >= PROGRESS_STEP || self.received >= self.total {
            self.reported = self.received;
            let _ = progress.unbounded_send((self.received, self.total));
        }
    }

    /// The SHA-256 of everything written, once it is on disk.
    fn finish(&mut self) -> std::io::Result<String> {
        std::io::Write::flush(&mut self.file)?;
        Ok(hex(&self.hasher.clone().finalize()))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The SHA-256 of the file at `path`, if it can be read.
fn file_digest(path: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        match file.read(&mut buffer).ok()? {
            0 => return Some(hex(&hasher.finalize())),
            read => hasher.update(&buffer[..read]),
        }
    }
}

/// Why a download attempt ended early: for good, or with the connection, which may resume.
enum Stop {
    Fail(String),
    Dropped(String),
}

/// `(first byte, whole length)` of a `Content-Range: bytes 100-199/1000` answer.
fn content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (range, total) = value.trim().strip_prefix("bytes ")?.split_once('/')?;
    let first = range.split_once('-')?.0.trim().parse().ok()?;
    Some((first, total.trim().parse().ok()))
}

/// One request for what `part` still lacks of `url`, appended to it.
async fn receive(
    client: &reqwest::Client,
    url: &str,
    part: &mut Part,
    progress: &futures::channel::mpsc::UnboundedSender<(u64, u64)>,
) -> Result<(), Stop> {
    let io = |cause: std::io::Error| Stop::Fail(cause.to_string());
    let mut request = client.get(url);
    if part.received > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={}-", part.received));
    }
    let response = request
        .send()
        .await
        .map_err(failed("Could not download the update"))
        .map_err(Stop::Dropped)?;
    let status = response.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        // What is here is not a beginning of this package: start it again.
        part.restart().map_err(io)?;
        return Err(Stop::Dropped("The download stopped".into()));
    }
    if status.is_server_error() {
        log(&format!("download answered {status}"));
        return Err(Stop::Dropped("Could not download the update".into()));
    }
    if !status.is_success() {
        log(&format!("download answered {status}"));
        return Err(Stop::Fail("Could not download the update".into()));
    }
    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT
        && response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(content_range)
            .is_some_and(|(first, total)| {
                if let Some(total) = total {
                    part.total = total;
                }
                first == part.received
            });
    if !resumed {
        if part.received > 0 {
            part.restart().map_err(io)?;
        }
        part.total = response.content_length().unwrap_or(part.total);
    }
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk
            .map_err(failed("The download stopped"))
            .map_err(Stop::Dropped)?;
        part.write(&chunk).map_err(io)?;
        part.report(progress);
    }
    if part.total > 0 && part.received < part.total {
        return Err(Stop::Dropped("The download stopped".into()));
    }
    Ok(())
}

/// Fetch the rest of `part` from `url`, resuming a dropped connection; the digest of the whole.
async fn receive_all(
    client: &reqwest::Client,
    url: &str,
    part: &mut Part,
    progress: &futures::channel::mpsc::UnboundedSender<(u64, u64)>,
) -> Result<String, String> {
    // A connection that drops part way carries on from the bytes already here, as long as the
    // server can send the rest (`Range`); the update is never started over for one dropped link.
    let mut resumes = 0;
    loop {
        match receive(client, url, part, progress).await {
            Ok(()) => break,
            Err(Stop::Fail(cause)) => return Err(cause),
            Err(Stop::Dropped(cause)) if resumes < RESUMES => {
                resumes += 1;
                log(&format!(
                    "{cause} at {} of {} bytes, resuming ({resumes}/{RESUMES})",
                    part.received, part.total
                ));
                tokio::time::sleep(backoff(resumes)).await;
            }
            Err(Stop::Dropped(cause)) => return Err(cause),
        }
    }
    part.finish().map_err(|cause| cause.to_string())
}

/// Downloads kept from one start to the next, a folder for each version in the downloads root:
/// a package already downloaded and checked is used again, and one cut short (KChess quit, the
/// computer slept) carries on from its `.part`. Before, each start cleared the last one's
/// download, so quitting before installing meant fetching the whole package again.
const KEPT: &str = ".kept";

/// Where `version`'s download is kept.
fn kept_folder(root: &Path, version: &str) -> PathBuf {
    root.join(KEPT).join(version)
}

/// Remove the kept downloads `stale` names (by version), except one another KChess is still
/// downloading into (its folder's lock is held).
fn prune_kept(root: &Path, stale: impl Fn(Option<[u64; 3]>) -> bool) {
    let Ok(entries) = std::fs::read_dir(root.join(KEPT)) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        if !stale(parse_version(&name)) {
            continue;
        }
        let path = entry.path();
        let busy = std::fs::File::options()
            .write(true)
            .open(path.join(".lock"))
            .is_ok_and(|lock| lock.try_lock().is_err());
        if !busy {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// A small release file's text (the checksums or their signature).
async fn small_file(
    client: &reqwest::Client,
    release: &Release,
    name: &str,
    missing: &'static str,
    failure: &'static str,
) -> Result<String, String> {
    let entry = release
        .assets
        .iter()
        .find(|entry| entry.name == name)
        .ok_or(missing)?;
    retried(name, || async {
        let response = client
            .get(&entry.browser_download_url)
            .timeout(Duration::from_secs(60))
            .send()
            .await;
        answered(response, failure, |_| failure.to_string())?
            .text()
            .await
            .map_err(failed(failure))
            .map_err(Answer::Retry)
    })
    .await
}

/// Download `asset` into `folder` (its version's kept folder) and check it against the
/// release's digest; the downloaded file. `progress` hears the bytes received so far. The
/// checksums must be signed by `key` when there is one (`update_key()`, the release key). The
/// checksums sit in the same release as the packages, so on their own they only catch a broken
/// download: the signature is what stops a replaced package.
async fn download_signed(
    release: Release,
    asset: Asset,
    folder: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
    key: Option<Vec<u8>>,
) -> Result<PathBuf, String> {
    let client = http()?;
    let sums = small_file(
        &client,
        &release,
        "SHA256SUMS.txt",
        "The release lists no checksums",
        "Could not download the checksums",
    )
    .await?;
    if let Some(key) = key {
        let signature = small_file(
            &client,
            &release,
            "SHA256SUMS.txt.sig",
            "The release's checksums are not signed",
            "Could not download the checksum signature",
        )
        .await?;
        sums_signed_by(&key, sums.as_bytes(), &signature)?;
    }
    let expected =
        digest_for(&sums, &asset.name).ok_or("The release lists no checksum for this package")?;
    let fail = |cause: std::io::Error| cause.to_string();
    std::fs::create_dir_all(&folder).map_err(fail)?;
    // One download of a version at a time: another KChess downloading it is waited for, and its
    // finished package is then used here too.
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join(".lock"))
        .map_err(fail)?;
    lock.lock().map_err(fail)?;
    let file_path = folder.join(&asset.name);
    if file_digest(&file_path).is_some_and(|digest| digest == expected) {
        log(&format!("{} was already downloaded", asset.name));
        let size = std::fs::metadata(&file_path).map_or(asset.size, |meta| meta.len());
        let _ = progress.unbounded_send((size, size));
        return Ok(file_path);
    }
    let _ = std::fs::remove_file(&file_path);
    let part_path = folder.join(format!("{}.part", asset.name));
    let mut part = Part::open(&part_path, asset.size).map_err(fail)?;
    let carried = part.received;
    if carried > 0 {
        log(&format!(
            "carrying on from {carried} bytes of {}",
            asset.name
        ));
    }
    let url = &asset.browser_download_url;
    let mut digest = receive_all(&client, url, &mut part, &progress).await;
    if carried > 0 && digest.as_ref().is_ok_and(|digest| *digest != expected) {
        // What an earlier start left was not this package's beginning: fetch it all again.
        log("the kept part did not match; downloading it again");
        part.restart().map_err(fail)?;
        digest = receive_all(&client, url, &mut part, &progress).await;
    }
    let digest = digest?;
    drop(part);
    if digest != expected {
        let _ = std::fs::remove_file(&part_path);
        return Err("The downloaded update did not match its checksum".into());
    }
    std::fs::rename(&part_path, &file_path).map_err(fail)?;
    Ok(file_path)
}

/// Unpack a macOS update's zip into `folder`; the `.app` it holds. The zip stays where it was
/// kept, so a later start can unpack it again.
fn unpack(archive: &Path, folder: &Path) -> Result<PathBuf, String> {
    let unpacked = folder.join("app");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(|cause| cause.to_string())?;
    let status = Command::new("ditto")
        .args(["-x", "-k"])
        .arg(archive)
        .arg(&unpacked)
        .status();
    if !status.is_ok_and(|status| status.success()) {
        return Err("The update could not be unpacked".into());
    }
    std::fs::read_dir(&unpacked)
        .map_err(|cause| cause.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .ok_or_else(|| "The update holds no app".into())
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
        return Err("The update's signature is not valid".into());
    }
    if running.identity.is_some() && update.identity != running.identity {
        return Err("The update is not signed by KChess's developer".into());
    }
    if running.app.is_some() && update.app != running.app {
        return Err("The update is not KChess".into());
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
            let mut powershell = Command::new("powershell");
            #[cfg(windows)]
            {
                // No console window flashes up beside the app while the signature is checked.
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                powershell.creation_flags(CREATE_NO_WINDOW);
            }
            let output = powershell
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

/// Download and check `asset` (of `package`) into its kept `folder`, and stage it: the new
/// bundle unpacked into `work` on macOS, else the package itself. With the installation
/// `install` known, the update must be signed as the running app is.
#[allow(clippy::too_many_arguments)]
async fn fetch(
    release: Release,
    asset: Asset,
    package: Package,
    install: Option<Install>,
    folder: PathBuf,
    work: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
    key: Option<Vec<u8>>,
) -> Result<PathBuf, String> {
    let file = download_signed(release, asset, folder, progress, key).await?;
    let staged = match package {
        Package::MacZip => unpack(&file, &work)?,
        _ => file,
    };
    if let Some(install) = install {
        signed_as_running(package, &install, &staged)?;
    }
    Ok(staged)
}

/// Whether `staged`, an update for `install` of `package`'s kind, is signed as the running app.
fn signed_as_running(package: Package, install: &Install, staged: &Path) -> Result<(), String> {
    let running = running_signer(install);
    signed_alike(&running, &signer(package, staged))?;
    if package == Package::MacZip
        && let (Some(team), Some(app)) = (&running.identity, &running.app)
        && !meets_requirement(staged, &developer_requirement(app, team))
    {
        return Err("The update is not signed by KChess's developer".into());
    }
    Ok(())
}

/// The update patch from this installation's version `current` to `release`, when the release
/// has one for the package this app updates from and it is the smaller download.
fn patch_asset(release: &Release, package: Package, current: &str) -> Option<Asset> {
    let version = release.tag_name.trim_start_matches('v');
    let name = asset_name(version, package)?;
    let whole = release.assets.iter().find(|asset| asset.name == name)?;
    let patch = patch::asset_name(&name, current)?;
    release
        .assets
        .iter()
        .find(|asset| asset.name == patch && (whole.size == 0 || asset.size < whole.size))
        .cloned()
}

/// The update to `release` staged for `install`: by its patch when the release has one for
/// `current`, by the whole package `asset` when it has none, or when this installation's files
/// are not the ones the patch starts from (a file deleted or changed since it was installed).
#[allow(clippy::too_many_arguments)]
async fn fetch_update(
    release: Release,
    asset: Asset,
    package: Package,
    install: Option<Install>,
    current: String,
    folder: PathBuf,
    work: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
    key: Option<Vec<u8>>,
) -> Result<PathBuf, String> {
    let patch = install
        .clone()
        .and_then(|install| Some((install, patch_asset(&release, package, &current)?)));
    if let Some((installed, delta)) = patch {
        let patched = fetch_patch(
            release.clone(),
            delta,
            package,
            installed,
            current,
            folder.clone(),
            work.clone(),
            progress.clone(),
            key.clone(),
        )
        .await;
        match patched {
            Ok(staged) => return Ok(staged),
            Err(cause) => log(&format!(
                "patch failed: {cause}; downloading the whole package"
            )),
        }
    }
    fetch(
        release, asset, package, install, folder, work, progress, key,
    )
    .await
}

/// `fetch` by the update patch `delta` from `current`: download it into the kept `folder`,
/// rebuild the new release from the installed files into `work`, and check what it made as a
/// whole package's download is checked.
#[allow(clippy::too_many_arguments)]
async fn fetch_patch(
    release: Release,
    delta: Asset,
    package: Package,
    install: Install,
    current: String,
    folder: PathBuf,
    work: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
    key: Option<Vec<u8>>,
) -> Result<PathBuf, String> {
    let version = release.tag_name.trim_start_matches('v').to_string();
    let deb = asset_name(&version, package).unwrap_or_default();
    let file = download_signed(release, delta, folder, progress, key).await?;
    let staged = patch::rebuild(
        &patch::Patch {
            delta: &file,
            package,
            install: &install.path,
            root: Path::new("/"),
            current: &current,
            version: &version,
            deb: &deb,
        },
        &work,
    )?;
    signed_as_running(package, &install, &staged)?;
    Ok(staged)
}

/// What a Developer ID build's update must satisfy: signed through Apple's own certificate
/// chain, as `app`, by `team`. The team and identifier `codesign --display` prints are the
/// bundle's own claims, which a self-signed certificate can make; this requirement is not.
fn developer_requirement(app: &str, team: &str) -> String {
    format!(
        "=anchor apple generic and identifier \"{app}\" and certificate leaf[subject.OU] = \"{team}\""
    )
}

/// Whether the bundle at `path` passes `codesign --verify` against `requirement`.
fn meets_requirement(path: &Path, requirement: &str) -> bool {
    Command::new("codesign")
        .args(["--verify", "--deep", "--strict", "-R", requirement])
        .arg(path)
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
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
    let fail = |cause: std::io::Error| format!("The update could not be installed ({cause})");
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
fn exchange(a: &Path, b: &Path) -> std::io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, a, CWD, b, RenameFlags::EXCHANGE).map_err(std::io::Error::from)
}

#[cfg(not(target_os = "macos"))]
fn exchange(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Replace the file at `installed` with `staged`, executable, in one rename on the same disk.
fn swap_file(staged: &Path, installed: &Path) -> Result<(), String> {
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
    let cache = dirs::cache_dir().ok_or("KChess has no folder to download updates to")?;
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
    let refused = || format!("The update folder {} is not private", dir.display());
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
    fn inherited(&self) -> Result<Stdio, String> {
        self.held
            .try_clone()
            .map(Stdio::from)
            .map_err(|cause| format!("The update could not be installed ({cause})"))
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
        if name == ".lock" || name == KEPT || name.starts_with(INSTALLING) {
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
            // What a helper stopped before it finished left: its package, a half-written outcome.
            if let Some(root) = root {
                let held = InstallLock::path(root, path);
                for left in ["deb", "outcome.log", "outcome.new"] {
                    let _ = std::fs::remove_file(held.with_extension(left));
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
fn apply_on_quit(staged: &Path, install: &Install, reopen: bool) -> Result<(), String> {
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
fn deb_helper(
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
        let _ = std::fs::remove_file(&package);
        format!("The update could not be installed ({cause}).")
    })
}

/// The extension of the file a .deb helper records its outcome in, beside the installation's lock.
const OUTCOME: &str = "outcome";

/// Installs the .deb (`$1`) with `$4 dpkg -i` once KChess (`$2`) has exited, and records the exit
/// status and what the package manager said (`$3`) for the next start. Its standard input is the
/// installation's lock (`InstallLock::inherited`), kept on descriptor 9 until the package
/// manager is done, the outcome recorded and the package removed.
const DEB_HELPER: &str = r#"exec 9<&0 0</dev/null
while kill -0 "$2" 2>/dev/null; do sleep 0.2; done
"$4" dpkg -i "$1" 9<&- >/dev/null 2>"$3.log"
echo $? > "$3.new"
tail -c 2000 "$3.log" >> "$3.new"
mv -f "$3.new" "$3"
rm -f "$1" "$3.log""#;

/// `pkexec`'s status when the person dismissed its password prompt.
const DISMISSED: &str = "126";

/// What the helper installing a .deb on quit left for this start: the error to show when the
/// package manager failed. An update installed, or a password prompt dismissed, shows nothing.
/// A helper still running records its outcome for a later start.
fn last_install_error(install: Option<&Install>, root: Option<&Path>) -> Option<String> {
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
    let _ = std::fs::remove_file(&outcome);
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

pub fn init(cx: &mut App) {
    let store = kchess_core::app_store();
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
    let feed = kchess_core::dev_override("KCHESS_UPDATE_FEED")
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
            this.schedule(cx)
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
                .unwrap_or_else(|_| Err("The check stopped".into()));
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
            // An update downloaded before (its install failed) is not installed on quit either.
            self.staged = None;
            return;
        };
        // An update whose install failed is still whole: the same version is ready again, and a
        // newer one replaces it, which used to install the old one on quit while offering the new.
        if self.staged.is_some() {
            if self.version.as_deref() == Some(version.as_str()) {
                self.phase = Phase::Downloaded;
                self.released = release.published_at.clone();
                self.release = Some(release);
                return;
            }
            self.staged = None;
        }
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
            return crate::core::quit_saved(cx);
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
