//! Fetching an update: the release feed, the package (resumed where it stopped, its digest
//! taken as it arrives), its checksums and their signature, each small request tried again a few
//! times, into a private folder the downloads share.

use super::*;

/// An error and every cause under it, as one line.
pub(super) fn causes(error: &dyn std::error::Error) -> String {
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
pub(super) fn failed(message: &'static str) -> impl FnOnce(reqwest::Error) -> String {
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
pub(super) fn on_runtime<T: Send + 'static>(
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

pub(super) fn http() -> Result<reqwest::Client, String> {
    kchess_host::http::client()
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
pub(super) const TRIES: u32 = 3;

/// One small request's answer, or why it failed: `Retry` for what a later try may not meet
/// (no connection, a timeout, a server error, too many requests), `Fail` for the rest.
pub(super) enum Answer {
    Fail(String),
    Retry(String),
}

/// `request` until it answers, at most `TRIES` times, waiting as a resumed download does
/// (`backoff`) between tries.
pub(super) async fn retried<T, F: Future<Output = Result<T, Answer>>>(
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
pub(super) fn answered(
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

pub(super) async fn latest(feed: String) -> Result<Release, String> {
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
pub(super) const RESUMES: u32 = 4;
/// The wait before the first resume; each later one waits twice as long.
pub(super) const RESUME_AFTER: Duration = if cfg!(test) {
    Duration::from_millis(10)
} else {
    Duration::from_secs(2)
};

/// The wait before try (or resume) `attempt`, counting from 1: `RESUME_AFTER`, doubled each time.
pub(super) fn backoff(attempt: u32) -> Duration {
    RESUME_AFTER * 2u32.pow(attempt.saturating_sub(1))
}

/// How often a download reports its progress: every this many bytes, and at its end. One report
/// for every chunk the server sent was thousands of redraws of the window for one update.
pub(super) const PROGRESS_STEP: u64 = 1 << 20;

/// A download so far, in its `.part` file: the digest of what it holds, and how much of how much.
/// Written through a buffer on the download's own thread (`on_runtime`), not a thread hop for
/// each chunk.
pub(super) struct Part {
    pub(super) file: std::io::BufWriter<std::fs::File>,
    pub(super) hasher: Sha256,
    pub(super) received: u64,
    pub(super) total: u64,
    pub(super) reported: u64,
}

impl Part {
    /// The part at `path`, carrying on from what an earlier download left in it.
    pub(super) fn open(path: &Path, total: u64) -> std::io::Result<Part> {
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
    pub(super) fn restart(&mut self) -> std::io::Result<()> {
        use std::io::{Seek as _, Write as _};
        self.file.flush()?;
        self.file.get_mut().set_len(0)?;
        self.file.get_mut().seek(std::io::SeekFrom::Start(0))?;
        self.hasher = Sha256::new();
        self.received = 0;
        self.reported = 0;
        Ok(())
    }

    pub(super) fn write(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        std::io::Write::write_all(&mut self.file, chunk)?;
        self.hasher.update(chunk);
        self.received += chunk.len() as u64;
        Ok(())
    }

    /// Tell `progress` how far it is, once a step further on or at the end.
    pub(super) fn report(
        &mut self,
        progress: &futures::channel::mpsc::UnboundedSender<(u64, u64)>,
    ) {
        if self.received - self.reported >= PROGRESS_STEP || self.received >= self.total {
            self.reported = self.received;
            let _ = progress.unbounded_send((self.received, self.total));
        }
    }

    /// The SHA-256 of everything written, once it is on disk.
    pub(super) fn finish(&mut self) -> std::io::Result<String> {
        std::io::Write::flush(&mut self.file)?;
        Ok(kchess_delta::hex(&self.hasher.clone().finalize()))
    }
}

/// Why a download attempt ended early: for good, or with the connection, which may resume.
pub(super) enum Stop {
    Fail(String),
    Dropped(String),
}

/// `(first byte, whole length)` of a `Content-Range: bytes 100-199/1000` answer.
pub(super) fn content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (range, total) = value.trim().strip_prefix("bytes ")?.split_once('/')?;
    let first = range.split_once('-')?.0.trim().parse().ok()?;
    Some((first, total.trim().parse().ok()))
}

/// One request for what `part` still lacks of `url`, appended to it.
pub(super) async fn receive(
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
pub(super) async fn receive_all(
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
pub(super) const KEPT: &str = ".kept";

/// Where `version`'s download is kept.
pub(super) fn kept_folder(root: &Path, version: &str) -> PathBuf {
    root.join(KEPT).join(version)
}

/// Held (shared) by each KChess whose staged update is the package in that kept folder.
const STAGED: &str = ".staged";

/// A checked download, and the hold on its folder (`hold_kept`) while it is offered.
pub(super) struct Downloaded {
    pub path: PathBuf,
    pub hold: Option<std::fs::File>,
}

/// Holds the kept download `folder` for as long as the answer lives, so another KChess's
/// `prune_kept` leaves it: a newer download elsewhere removed a package this app still offered,
/// and installing it failed. Taken before the download's own lock is let go, so no cleanup comes
/// between the two.
pub(super) fn hold_kept(folder: &Path) -> Option<std::fs::File> {
    let hold = std::fs::File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(folder.join(STAGED))
        .warn_on_err()?;
    hold.lock_shared().warn_on_err()?;
    Some(hold)
}

/// Remove the kept downloads `stale` names (by version), except one another KChess is still
/// downloading into (its folder's lock is held) or offers as ready (`hold_kept`).
pub(super) fn prune_kept(root: &Path, stale: impl Fn(Option<Version>) -> bool) {
    let Ok(entries) = std::fs::read_dir(root.join(KEPT)) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        if !stale(parse_version(&name)) {
            continue;
        }
        let path = entry.path();
        let busy = [".lock", STAGED].iter().any(|name| {
            std::fs::File::options()
                .write(true)
                .open(path.join(name))
                .is_ok_and(|lock| lock.try_lock().is_err())
        });
        if !busy {
            std::fs::remove_dir_all(&path).ok_if_missing().warn_on_err();
        }
    }
}

/// A small release file's text (the checksums or their signature).
pub(super) async fn small_file(
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
pub(super) async fn download_signed(
    release: Release,
    asset: Asset,
    folder: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
    key: Option<Vec<u8>>,
) -> Result<Downloaded, String> {
    let client = http()?;
    let sums = small_file(
        &client,
        &release,
        sums::FILE,
        "The release lists no checksums",
        "Could not download the checksums",
    )
    .await?;
    if let Some(key) = key {
        let signature = small_file(
            &client,
            &release,
            sums::SIGNATURE,
            "The release's checksums are not signed",
            "Could not download the checksum signature",
        )
        .await?;
        sums_signed_by(&key, sums.as_bytes(), &signature)?;
    }
    let expected = sums::digest_of(&sums, &asset.name)
        .ok_or("The release lists no checksum for this package")?;
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
    if kchess_delta::file_digest(&file_path).is_ok_and(|digest| digest == expected) {
        log(&format!("{} was already downloaded", asset.name));
        let size = std::fs::metadata(&file_path).map_or(asset.size, |meta| meta.len());
        let _ = progress.unbounded_send((size, size));
        let hold = hold_kept(&folder);
        return Ok(Downloaded {
            path: file_path,
            hold,
        });
    }
    std::fs::remove_file(&file_path)
        .ok_if_missing()
        .warn_on_err();
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
        std::fs::remove_file(&part_path)
            .ok_if_missing()
            .warn_on_err();
        return Err("The downloaded update did not match its checksum".into());
    }
    std::fs::rename(&part_path, &file_path).map_err(fail)?;
    let hold = hold_kept(&folder);
    drop(lock);
    Ok(Downloaded {
        path: file_path,
        hold,
    })
}

/// Unpack a macOS update's zip into `folder`; the `.app` it holds. The zip stays where it was
/// kept, so a later start can unpack it again.
#[allow(
    clippy::disallowed_methods,
    reason = "the updater's own thread runs it (`on_runtime`); allowed, not expected, since some platforms' builds have no such call"
)]
pub(super) fn unpack(archive: &Path, folder: &Path) -> Result<PathBuf, String> {
    let unpacked = folder.join("app");
    std::fs::remove_dir_all(&unpacked)
        .ok_if_missing()
        .warn_on_err();
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

/// Download and check `asset` (of `package`) into its kept `folder`, and stage it: the new
/// bundle unpacked into `work` on macOS, else the package itself. With the installation
/// `install` known, the update must be signed as the running app is.
#[allow(clippy::too_many_arguments)]
pub(super) async fn fetch(
    release: Release,
    asset: Asset,
    package: Package,
    install: Option<Install>,
    folder: PathBuf,
    work: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
    key: Option<Vec<u8>>,
) -> Result<Downloaded, String> {
    let file = download_signed(release, asset, folder, progress, key).await?;
    // The bundle unpacked on macOS is this app's own: its download needs no hold.
    let staged = match package {
        Package::MacZip => Downloaded {
            path: unpack(&file.path, &work)?,
            hold: None,
        },
        _ => file,
    };
    if let Some(install) = install {
        signed_as_running(package, &install, &staged.path)?;
    }
    Ok(staged)
}

/// The update patch from this installation's version `current` to `release`, when the release
/// has one for the package this app updates from and it is the smaller download.
pub(super) fn patch_asset(release: &Release, package: Package, current: &str) -> Option<Asset> {
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
pub(super) async fn fetch_update(
    release: Release,
    asset: Asset,
    package: Package,
    install: Option<Install>,
    current: String,
    folder: PathBuf,
    work: PathBuf,
    progress: futures::channel::mpsc::UnboundedSender<(u64, u64)>,
    key: Option<Vec<u8>>,
) -> Result<Downloaded, String> {
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
            Ok(staged) => {
                return Ok(Downloaded {
                    path: staged,
                    hold: None,
                });
            }
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
pub(super) async fn fetch_patch(
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
    let file = download_signed(release, delta, folder, progress, key)
        .await?
        .path;
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

/// Where this identity's downloads go: a folder of this user's, never the shared temporary folder,
/// where another user could replace a checked package before it is installed.
pub(super) fn downloads_root() -> Result<PathBuf, String> {
    let cache = dirs::cache_dir().ok_or("KChess has no folder to download updates to")?;
    downloads_in(&cache)
}

/// `kchess-update/PREFIX` in `cache`, each folder private to this user.
pub(super) fn downloads_in(cache: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(cache).map_err(|cause| cause.to_string())?;
    let parent = cache.join("kchess-update");
    private_dir(&parent, cache)?;
    let root = parent.join(PREFIX);
    private_dir(&root, cache)?;
    Ok(root)
}

/// Make `dir` a folder only its owner can use, refusing a link or another user's folder (its
/// owner must be `owner`'s).
pub(super) fn private_dir(dir: &Path, owner: &Path) -> Result<(), String> {
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
pub(super) struct Workspace {
    pub(super) path: PathBuf,
    pub(super) _held: std::fs::File,
}

impl Workspace {
    pub(super) fn open(root: &Path) -> Result<Self, String> {
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
