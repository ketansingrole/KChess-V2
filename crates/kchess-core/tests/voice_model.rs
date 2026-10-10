//! Ports `core/tests/unit/voice-model.test.ts`: the cache's install check, shared preparation,
//! offline reuse, checksum and corruption handling, failed and interrupted downloads, unsafe
//! archives, and responsiveness while a large model is prepared. A local HTTP server stands in
//! for the model's host, and archives are generated in the test.

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kchess_core::host::{Host, Level};
use kchess_core::voice::{
    Archive, Listener, MAX_DOWNLOAD_BYTES, Phase, Progress, Status, VoiceModel, VoiceModelCache,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

const NAME: &str = "test-model";

/// Keeps every log line, as `Recording` does for the engine tests.
#[derive(Default)]
struct Logs(Mutex<Vec<String>>);

impl Host for Logs {
    fn log(&self, level: Level, scope: &str, message: &str) {
        self.0
            .lock()
            .expect("log lock")
            .push(format!("{} [{scope}] {message}", level.name()));
    }

    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
}

/// One scripted answer to a request.
#[derive(Clone)]
enum Reply {
    /// A complete response with this status and body.
    Body(u16, Vec<u8>),
    /// A 200 whose declared length is longer than the body that follows; the connection closes.
    Interrupted(Vec<u8>),
    /// A 200 that declares this many bytes and sends none.
    Declared(u64),
    /// Waits for the gate, then answers with the body.
    Gate(Arc<Notify>, Vec<u8>),
}

struct Server {
    base: String,
    requests: Arc<AtomicUsize>,
}

impl Server {
    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// Serve `handler(n)` as the reply to the n-th request (counting from zero).
async fn serve(handler: impl Fn(usize) -> Reply + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&requests);
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let index = counter.fetch_add(1, Ordering::SeqCst);
            let reply = handler(index);
            tokio::spawn(async move {
                let _ = answer(&mut socket, reply).await;
            });
        }
    });
    Server { base, requests }
}

async fn answer(socket: &mut TcpStream, reply: Reply) -> std::io::Result<()> {
    read_head(socket).await?;
    match reply {
        Reply::Body(status, body) => {
            socket
                .write_all(head(status, body.len() as u64).as_bytes())
                .await?;
            socket.write_all(&body).await?;
        }
        Reply::Interrupted(body) => {
            socket
                .write_all(head(200, body.len() as u64 + 1_000).as_bytes())
                .await?;
            socket.write_all(&body).await?;
        }
        Reply::Declared(length) => {
            socket.write_all(head(200, length).as_bytes()).await?;
        }
        Reply::Gate(gate, body) => {
            gate.notified().await;
            socket
                .write_all(head(200, body.len() as u64).as_bytes())
                .await?;
            socket.write_all(&body).await?;
        }
    }
    socket.shutdown().await
}

fn head(status: u16, length: u64) -> String {
    let reason = match status {
        200 => "OK",
        503 => "Service Unavailable",
        _ => "Status",
    };
    format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
}

async fn read_head(socket: &mut TcpStream) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = socket.read(&mut chunk).await?;
        if read == 0 || buffer.len() > 64 * 1024 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(());
        }
    }
}

/// A zip with the given members, deflated unless `stored` (large fixtures are stored, so the
/// test measures the preparation rather than the compression).
fn zip_of(entries: &[(&str, &[u8])], stored: bool) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let method = if stored {
        zip::CompressionMethod::Stored
    } else {
        zip::CompressionMethod::Deflated
    };
    let options = SimpleFileOptions::default().compression_method(method);
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_directory(format!("{NAME}/"), options)
        .expect("directory");
    for (name, data) in entries {
        writer.start_file(*name, options).expect("start");
        writer.write_all(data).expect("write");
    }
    writer.finish().expect("finish").into_inner()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn model_for(base: &str, zip: &[u8]) -> VoiceModel {
    VoiceModel {
        name: NAME.into(),
        url: format!("{base}/model.zip"),
        sha256: sha256(zip),
    }
}

fn cache_in(directory: &Path, base: &str, zip: &[u8]) -> (VoiceModelCache, Arc<Logs>) {
    let logs = Arc::new(Logs::default());
    let cache = VoiceModelCache::new(
        logs.clone(),
        directory.to_path_buf(),
        Archive::Url(format!("{base}/model.zip")),
        model_for(base, zip),
    );
    (cache, logs)
}

fn listener_of(phases: &Arc<Mutex<Vec<Phase>>>) -> Listener {
    let phases = Arc::clone(phases);
    Arc::new(move |progress: &Progress| {
        phases.lock().expect("phase lock").push(progress.phase);
    })
}

fn entries(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("read directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn model_zip() -> Vec<u8> {
    zip_of(&[("test-model/am/final.mdl", b"test model")], false)
}

fn idle(installed: bool, bytes: u64) -> Status {
    Status {
        installed,
        bytes,
        busy: false,
        progress: None,
    }
}

#[tokio::test]
async fn checks_installation_without_downloading_and_rejects_a_damaged_cache() {
    let zip = model_zip();
    let body = zip.clone();
    let server = serve(move |_| Reply::Body(200, body.clone())).await;
    let directory = tempfile::tempdir().expect("directory");
    let (cache, _) = cache_in(directory.path(), &server.base, &zip);

    assert_eq!(cache.status().await, idle(false, 0));
    assert_eq!(server.requests(), 0);
    cache.ensure(None).await.expect("ensure");
    let archive = fs::read(cache.path()).expect("archive");
    assert_eq!(cache.status().await, idle(true, archive.len() as u64));
    assert_eq!(server.requests(), 1);

    let mut damaged = archive.clone();
    *damaged.last_mut().expect("byte") ^= 1;
    fs::write(cache.path(), &damaged).expect("damage");
    assert_eq!(cache.status().await, idle(false, 0));
    assert_eq!(server.requests(), 1);
}

#[tokio::test]
async fn reports_shared_preparation_while_a_download_is_in_flight() {
    let zip = model_zip();
    let gate = Arc::new(Notify::new());
    let held = Arc::clone(&gate);
    let body = zip.clone();
    let server = serve(move |_| Reply::Gate(Arc::clone(&held), body.clone())).await;
    let directory = tempfile::tempdir().expect("directory");
    let (cache, _) = cache_in(directory.path(), &server.base, &zip);

    let pending = {
        let cache = cache.clone();
        tokio::spawn(async move { cache.ensure(None).await })
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        while server.requests() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("request");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let busy = cache.status().await;
    assert!(busy.busy);
    assert!(!busy.installed);
    assert_eq!(
        busy.progress.map(|progress| progress.phase),
        Some(Phase::Downloading)
    );

    gate.notify_one();
    pending.await.expect("join").expect("ensure");
    assert_eq!(
        cache.status().await,
        idle(true, fs::metadata(cache.path()).expect("file").len())
    );
}

#[tokio::test]
async fn shares_concurrent_downloads_and_reuses_a_verified_cache_offline() {
    let zip = model_zip();
    let body = zip.clone();
    let server = serve(move |_| Reply::Body(200, body.clone())).await;
    let directory = tempfile::tempdir().expect("directory");
    let (cache, _) = cache_in(directory.path(), &server.base, &zip);
    let phases = Arc::new(Mutex::new(Vec::new()));

    let (first, second) =
        tokio::join!(cache.ensure(Some(listener_of(&phases))), cache.ensure(None));
    let first = first.expect("first");
    assert_eq!(first, second.expect("second"));
    assert_eq!(server.requests(), 1);
    let seen = phases.lock().expect("phases").clone();
    assert!(seen.contains(&Phase::Downloading));
    assert!(seen.contains(&Phase::Preparing));

    let bytes = fs::read(&first).expect("archive");
    assert_eq!(&bytes[..2], &[0x1f, 0x8b]);

    // A new instance with no network reuses the verified cache.
    let (offline, _) = cache_at_unreachable(directory.path(), &zip);
    assert_eq!(offline.ensure(None).await.expect("offline"), first);
    assert_eq!(server.requests(), 1);
    assert_eq!(
        entries(directory.path()),
        vec!["model.tar.gz".to_string(), "model.tar.gz.json".to_string()]
    );
}

fn cache_at_unreachable(directory: &Path, zip: &[u8]) -> (VoiceModelCache, Arc<Logs>) {
    cache_in(directory, "http://127.0.0.1:9", zip)
}

#[tokio::test]
async fn rejects_a_bad_checksum_without_publishing_a_cache_then_permits_retry() {
    let zip = model_zip();
    let body = zip.clone();
    let server = serve(move |index| {
        if index == 0 {
            Reply::Body(200, b"corrupted download".to_vec())
        } else {
            Reply::Body(200, body.clone())
        }
    })
    .await;
    let directory = tempfile::tempdir().expect("directory");
    let (cache, logs) = cache_in(directory.path(), &server.base, &zip);

    let error = cache.ensure(None).await.expect_err("checksum");
    assert!(
        error.message.contains("checksum mismatch"),
        "{}",
        error.message
    );
    assert!(entries(directory.path()).is_empty());
    assert!(
        logs.0
            .lock()
            .expect("logs")
            .iter()
            .any(|line| line.contains("preparation failed"))
    );

    assert_eq!(cache.ensure(None).await.expect("retry"), cache.path());
    assert_eq!(server.requests(), 2);
}

#[tokio::test]
async fn detects_same_size_cache_corruption_and_downloads_a_replacement() {
    let zip = model_zip();
    let body = zip.clone();
    let server = serve(move |_| Reply::Body(200, body.clone())).await;
    let directory = tempfile::tempdir().expect("directory");
    let (cache, _) = cache_in(directory.path(), &server.base, &zip);

    let path = cache.ensure(None).await.expect("ensure");
    let original = fs::read(&path).expect("original");
    let mut damaged = original.clone();
    *damaged.last_mut().expect("byte") ^= 1;
    fs::write(&path, &damaged).expect("damage");

    cache.ensure(None).await.expect("replacement");
    assert_eq!(server.requests(), 2);
    assert_ne!(fs::read(&path).expect("replaced"), damaged);

    cache.ensure(None).await.expect("verified");
    assert_eq!(server.requests(), 2);
}

#[tokio::test]
async fn cleans_up_failed_and_interrupted_responses_and_allows_another_attempt() {
    let zip = model_zip();
    let body = zip.clone();
    let server = serve(move |index| match index {
        0 => Reply::Body(503, Vec::new()),
        1 => Reply::Interrupted(body.clone()),
        _ => Reply::Body(200, body.clone()),
    })
    .await;
    let directory = tempfile::tempdir().expect("directory");
    let (cache, _) = cache_in(directory.path(), &server.base, &zip);

    let failed = cache.ensure(None).await.expect_err("503");
    assert_eq!(
        failed.message,
        "Couldn’t prepare voice input. Voice model download failed: HTTP 503. Connect to the internet and try again."
    );
    let interrupted = cache.ensure(None).await.expect_err("interrupted");
    assert!(
        interrupted
            .message
            .starts_with("Couldn’t prepare voice input. "),
        "{}",
        interrupted.message
    );
    assert!(entries(directory.path()).is_empty());
    cache.ensure(None).await.expect("third attempt");
}

#[tokio::test]
async fn refuses_a_download_declared_larger_than_the_limit() {
    let zip = model_zip();
    let server = serve(|_| Reply::Declared(MAX_DOWNLOAD_BYTES + 1)).await;
    let directory = tempfile::tempdir().expect("directory");
    let (cache, _) = cache_in(directory.path(), &server.base, &zip);
    let error = cache.ensure(None).await.expect_err("too large");
    assert!(
        error
            .message
            .ends_with("Voice model download is too large. Connect to the internet and try again."),
        "{}",
        error.message
    );
    assert!(entries(directory.path()).is_empty());
}

#[tokio::test]
async fn rejects_paths_outside_the_expected_model_directory_even_with_a_matching_digest() {
    let unsafe_zip = zip_of(&[("../escape", &[1])], false);
    let body = unsafe_zip.clone();
    let server = serve(move |_| Reply::Body(200, body.clone())).await;
    let directory = tempfile::tempdir().expect("directory");
    let (cache, _) = cache_in(directory.path(), &server.base, &unsafe_zip);

    let error = cache.ensure(None).await.expect_err("unsafe");
    assert!(
        error.message.contains("Invalid voice model archive."),
        "{}",
        error.message
    );
    assert!(entries(directory.path()).is_empty());
    assert!(
        !directory
            .path()
            .parent()
            .expect("parent")
            .join("escape")
            .exists()
    );
}

#[tokio::test]
async fn refuses_links_and_archives_without_model_files() {
    let directory = tempfile::tempdir().expect("directory");

    let empty = zip_of(&[], false);
    let body = empty.clone();
    let server = serve(move |_| Reply::Body(200, body.clone())).await;
    let (cache, _) = cache_in(directory.path(), &server.base, &empty);
    let error = cache.ensure(None).await.expect_err("empty");
    assert!(
        error.message.contains("Voice model archive is empty."),
        "{}",
        error.message
    );
    assert!(entries(directory.path()).is_empty());

    let link = {
        use zip::write::SimpleFileOptions;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink(
                format!("{NAME}/link"),
                "/etc/passwd",
                SimpleFileOptions::default(),
            )
            .expect("symlink");
        writer.finish().expect("finish").into_inner()
    };
    let body = link.clone();
    let server = serve(move |_| Reply::Body(200, body.clone())).await;
    let (cache, _) = cache_in(directory.path(), &server.base, &link);
    let error = cache.ensure(None).await.expect_err("link");
    assert!(
        error.message.contains("Invalid voice model archive."),
        "{}",
        error.message
    );
    assert!(entries(directory.path()).is_empty());
}

#[tokio::test]
async fn keeps_the_main_event_loop_responsive_while_preparing_a_large_model() {
    let large = zip_of(
        &[("test-model/am/final.mdl", &vec![42_u8; 32 * 1024 * 1024])],
        true,
    );
    let body = large.clone();
    let server = serve(move |_| Reply::Body(200, body.clone())).await;
    let directory = tempfile::tempdir().expect("directory");
    let (cache, _) = cache_in(directory.path(), &server.base, &large);

    let preparing = Arc::new(AtomicBool::new(false));
    let ticks = Arc::new(AtomicUsize::new(0));
    let ticker = {
        let (preparing, ticks) = (Arc::clone(&preparing), Arc::clone(&ticks));
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(5));
            loop {
                interval.tick().await;
                if preparing.load(Ordering::SeqCst) {
                    ticks.fetch_add(1, Ordering::SeqCst);
                }
            }
        })
    };
    let listener: Listener = {
        let preparing = Arc::clone(&preparing);
        Arc::new(move |progress: &Progress| {
            preparing.store(progress.phase == Phase::Preparing, Ordering::SeqCst);
        })
    };
    let outcome = cache.ensure(Some(listener)).await;
    ticker.abort();
    outcome.expect("ensure");
    assert!(
        ticks.load(Ordering::SeqCst) > 2,
        "ticks {}",
        ticks.load(Ordering::SeqCst)
    );
}
