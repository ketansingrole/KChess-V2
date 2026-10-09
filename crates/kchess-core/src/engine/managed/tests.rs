//! Ports `core/tests/unit/managed-engine.test.ts` and adds the install security cases: digest,
//! archive traversal and links, failed handoff, failed probe, cancellation. A local HTTP server
//! stands in for GitHub; archives are generated in the test.

use std::collections::HashMap;
use std::future::Future;
use std::io::{Cursor, Write};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

const EXE: &str = "stockfish";
const LINUX_ASSET: &str = "stockfish-linux-x86-64-universal.tar.gz";

#[derive(Default)]
struct RecordingHost {
    events: Mutex<Vec<(String, Value)>>,
    logs: Mutex<Vec<String>>,
}

impl Host for RecordingHost {
    fn log(&self, _level: Level, scope: &str, message: &str) {
        self.logs
            .lock()
            .unwrap()
            .push(format!("[{scope}] {message}"));
    }

    fn emit(&self, event: &str, payload: Value) {
        self.events
            .lock()
            .unwrap()
            .push((event.to_string(), payload));
    }
}

impl RecordingHost {
    fn usage_events(&self) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(event, _)| event == "host:usage")
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

type Hits = Arc<Mutex<Vec<String>>>;

/// A route: path, status and body.
struct Route(&'static str, u16, Vec<u8>);

/// Serve `routes` (built with the server's base URL) on 127.0.0.1 until the test ends.
async fn serve(build: impl FnOnce(&str) -> Vec<Route>) -> (String, Hits) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes: Arc<HashMap<String, (u16, Vec<u8>)>> = Arc::new(
        build(&base)
            .into_iter()
            .map(|Route(path, status, body)| (path.to_string(), (status, body)))
            .collect(),
    );
    let hits: Hits = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&hits);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let routes = Arc::clone(&routes);
            let hits = Arc::clone(&served);
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while head.len() < 16_384 {
                    match socket.read(&mut byte).await {
                        Ok(1) => head.push(byte[0]),
                        _ => break,
                    }
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                hits.lock().unwrap().push(path.clone());
                let (status, body) = routes
                    .get(&path)
                    .cloned()
                    .unwrap_or((404, b"missing".to_vec()));
                let response = format!(
                    "HTTP/1.1 {status} Status\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (base, hits)
}

fn release(tag: &str, assets: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({ "tag_name": tag, "assets": assets })).unwrap()
}

fn asset(name: &str, url: &str, size: usize, digest: Option<String>) -> Value {
    json!({ "name": name, "browser_download_url": url, "size": size, "digest": digest })
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

/// A fake UCI engine: answers `uci` with `uciok`, then exits at end of input.
const FAKE_ENGINE: &str = "#!/bin/sh\nwhile read line; do\n  if [ \"$line\" = uci ]; then echo id name Fake; echo uciok; fi\ndone\n";

/// One archive member.
enum Member<'a> {
    File(&'a str, &'a [u8]),
    Dir(&'a str),
    Link(&'a str, &'a str),
}

fn set_raw_name(header: &mut tar::Header, name: &str) {
    // Bypasses `Header::set_path`, so hostile names (`../x`, absolute) reach the extractor.
    let bytes = name.as_bytes();
    header.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
}

fn tar_gz(members: &[Member]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for member in members {
        let mut header = tar::Header::new_old();
        match member {
            Member::File(name, data) => {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(data.len() as u64);
                header.set_mode(0o755);
                set_raw_name(&mut header, name);
                header.set_cksum();
                builder.append(&header, *data).unwrap();
            }
            Member::Dir(name) => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                header.set_mode(0o755);
                set_raw_name(&mut header, name);
                header.set_cksum();
                builder.append(&header, std::io::empty()).unwrap();
            }
            Member::Link(name, target) => {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777);
                set_raw_name(&mut header, name);
                header.set_link_name(target).unwrap();
                header.set_cksum();
                builder.append(&header, std::io::empty()).unwrap();
            }
        }
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&tar_bytes).unwrap();
    encoder.finish().unwrap()
}

fn zip_bytes(files: &[(&str, &[u8])], symlink: Option<(&str, &str)>) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, data) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    if let Some((name, target)) = symlink {
        writer.add_symlink(name, target, options).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn location(dir: &Path, base: &str) -> ManagedLocation {
    let mut location = ManagedLocation::new(dir.to_path_buf());
    location.release_url = format!("{base}/release");
    location.target = Target {
        platform: "linux",
        arch: "x64",
    };
    location
}

/// Install a version that is already on disk, to be replaced by an install.
fn seed_installed(dir: &Path, version: &str, binary: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(EXE), binary).unwrap();
    std::fs::write(dir.join(VERSION_FILE), format!("{version}\n")).unwrap();
}

fn assert_kept_old_version(dir: &Path) {
    assert_eq!(
        std::fs::read_to_string(dir.join(EXE)).unwrap(),
        "old engine"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(VERSION_FILE))
            .unwrap()
            .trim(),
        "sf_16"
    );
    assert!(!dir.join(format!("{EXE}.pending")).exists());
    assert!(!dir.join(format!("{VERSION_FILE}.pending")).exists());
}

struct Resume(Arc<AtomicUsize>);

impl Drop for Resume {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy)]
enum HandoffMode {
    Allow,
    Refuse,
    Hang,
}

struct TestHandoff {
    mode: HandoffMode,
    suspended: Arc<AtomicUsize>,
    resumed: Arc<AtomicUsize>,
}

impl TestHandoff {
    fn new(mode: HandoffMode) -> Arc<TestHandoff> {
        Arc::new(TestHandoff {
            mode,
            suspended: Arc::new(AtomicUsize::new(0)),
            resumed: Arc::new(AtomicUsize::new(0)),
        })
    }
}

impl EngineHandoff for TestHandoff {
    fn suspend(&self) -> Pin<Box<dyn Future<Output = Result<EngineGuard>> + Send + '_>> {
        Box::pin(async move {
            match self.mode {
                HandoffMode::Allow => {
                    self.suspended.fetch_add(1, Ordering::SeqCst);
                    Ok(Box::new(Resume(Arc::clone(&self.resumed))) as EngineGuard)
                }
                HandoffMode::Refuse => Err(CoreError::new("owners refused")),
                HandoffMode::Hang => std::future::pending().await,
            }
        })
    }
}

// --- Ported from core/tests/unit/managed-engine.test.ts ---

#[tokio::test]
async fn reports_missing_when_the_directory_or_binary_is_absent() {
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty");
    assert_eq!(
        managed_engine(&host, &empty).await,
        ManagedEngine {
            installed: false,
            path: empty.join(executable_name()),
            version: None,
        }
    );
    let nested = dir.path().join("notadir").join("sub");
    std::fs::create_dir_all(&nested).unwrap();
    assert!(!managed_engine(&host, &nested).await.installed);
}

#[tokio::test]
async fn reports_the_recorded_version_and_none_when_the_file_is_missing() {
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(executable_name()), "#!/bin/sh\nexit 0\n").unwrap();
    let state = managed_engine(&host, dir.path()).await;
    assert!(state.installed);
    assert_eq!(state.version, None);
    std::fs::write(dir.path().join(VERSION_FILE), "sf_17.1\n").unwrap();
    let state = managed_engine(&host, dir.path()).await;
    assert_eq!(state.version.as_deref(), Some("sf_17.1"));
}

#[tokio::test]
async fn deletes_only_its_own_directory_under_engine_maintenance() {
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    std::fs::create_dir_all(&engine).unwrap();
    std::fs::write(engine.join(executable_name()), "binary").unwrap();
    let handoff = TestHandoff::new(HandoffMode::Allow);
    let mut location = ManagedLocation::new(engine.clone());
    location.handoff = Some(handoff.clone());
    delete_managed_engine(&host, &location).await.unwrap();
    assert_eq!(handoff.suspended.load(Ordering::SeqCst), 1);
    assert_eq!(handoff.resumed.load(Ordering::SeqCst), 1);
    assert!(!engine.exists());
}

#[tokio::test]
async fn skips_the_download_when_the_installed_engine_matches_the_release() {
    let (base, hits) = serve(|_| {
        vec![
            Route("/release", 200, release("sf_17.1", vec![])),
            Route("/download", 200, b"should not be fetched".to_vec()),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_17.1", "binary");
    let result = install_managed_engine(&host, &location(&engine, &base))
        .await
        .unwrap();
    assert_eq!(
        result,
        InstallResult {
            installed: true,
            path: engine.join(executable_name()),
            version: "sf_17.1".to_string(),
            updated: false,
        }
    );
    assert_eq!(*hits.lock().unwrap(), ["/release"]);
}

#[tokio::test]
async fn rejects_invalid_release_metadata_instead_of_downloading() {
    let (base, hits) = serve(|_| vec![Route("/release", 200, br#"{"nope":true}"#.to_vec())]).await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let error = install_managed_engine(&host, &location(dir.path(), &base))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Invalid Stockfish release metadata.");
    assert_eq!(hits.lock().unwrap().len(), 1);
}

// --- Install: verified replacement ---

#[cfg(unix)]
#[tokio::test]
async fn installs_the_verified_release_and_records_its_version() {
    let archive = tar_gz(&[
        Member::Dir("stockfish/"),
        Member::File("stockfish/README.md", b"docs"),
        Member::File("stockfish/stockfish-ubuntu", FAKE_ENGINE.as_bytes()),
    ]);
    let digest = sha256(&archive);
    let size = archive.len();
    let (base, hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_18",
                    vec![asset(
                        LINUX_ASSET,
                        &format!("{base}/asset"),
                        size,
                        Some(digest),
                    )],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    let result = install_managed_engine(&host, &location(&engine, &base))
        .await
        .unwrap();
    assert_eq!(result.version, "sf_18");
    assert!(result.updated && result.installed);
    assert_eq!(
        std::fs::read_to_string(engine.join(VERSION_FILE)).unwrap(),
        "sf_18\n"
    );
    let mode = std::os::unix::fs::PermissionsExt::mode(
        &std::fs::metadata(engine.join(EXE)).unwrap().permissions(),
    );
    assert_eq!(mode & 0o777, 0o755);
    assert!(!engine.join(format!("{EXE}.pending")).exists());
    assert_eq!(*hits.lock().unwrap(), ["/release", "/asset"]);
    let usage = host.usage_events();
    assert_eq!(usage.first().unwrap()["requests"], 1);
    assert_eq!(usage.last().unwrap()["bytes"], size as u64);
    assert_eq!(
        usage
            .iter()
            .map(|u| u["bytes"].as_u64().unwrap())
            .sum::<u64>(),
        size as u64
    );
}

#[tokio::test]
async fn rejects_a_download_that_fails_its_digest_and_keeps_the_old_engine() {
    let archive = tar_gz(&[Member::File("stockfish", FAKE_ENGINE.as_bytes())]);
    let size = archive.len();
    let (base, _hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_17",
                    vec![asset(
                        LINUX_ASSET,
                        &format!("{base}/asset"),
                        size,
                        Some(format!("sha256:{}", "0".repeat(64))),
                    )],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let error = install_managed_engine(&host, &location(&engine, &base))
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "Stockfish download failed its SHA-256 check."
    );
    assert_kept_old_version(&engine);
}

#[tokio::test]
async fn rejects_a_release_without_a_sha256_digest() {
    let archive = tar_gz(&[Member::File("stockfish", FAKE_ENGINE.as_bytes())]);
    let size = archive.len();
    let (base, hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_17",
                    vec![asset(LINUX_ASSET, &format!("{base}/asset"), size, None)],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let error = install_managed_engine(&host, &location(&engine, &base))
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "The Stockfish release has no SHA-256 digest, so it cannot be verified."
    );
    // Nothing is downloaded without a digest to check it against.
    assert_eq!(*hits.lock().unwrap(), ["/release"]);
    assert_kept_old_version(&engine);
}

#[tokio::test]
async fn rejects_a_download_shorter_than_the_release_size() {
    let archive = tar_gz(&[Member::File("stockfish", FAKE_ENGINE.as_bytes())]);
    let digest = sha256(&archive);
    let declared = archive.len() + 10;
    let (base, _hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_17",
                    vec![asset(
                        LINUX_ASSET,
                        &format!("{base}/asset"),
                        declared,
                        Some(digest),
                    )],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let error = install_managed_engine(&host, &location(&engine, &base))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Stockfish download was incomplete.");
    assert_kept_old_version(&engine);
}

#[tokio::test]
async fn refuses_an_oversized_release_before_downloading() {
    let (base, hits) = serve(|base| {
        vec![Route(
            "/release",
            200,
            release(
                "sf_17",
                vec![asset(
                    LINUX_ASSET,
                    &format!("{base}/asset"),
                    (MAX_DOWNLOAD_BYTES + 1) as usize,
                    Some(format!("sha256:{}", "a".repeat(64))),
                )],
            ),
        )]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let error = install_managed_engine(&host, &location(dir.path(), &base))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Stockfish download is unexpectedly large.");
    assert_eq!(*hits.lock().unwrap(), ["/release"]);
}

#[tokio::test]
async fn reports_a_failed_download_status() {
    let (base, _hits) = serve(|base| {
        vec![Route(
            "/release",
            200,
            release(
                "sf_17",
                vec![asset(
                    LINUX_ASSET,
                    &format!("{base}/asset"),
                    10,
                    Some(format!("sha256:{}", "a".repeat(64))),
                )],
            ),
        )]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let error = install_managed_engine(&host, &location(dir.path(), &base))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Stockfish download failed (404).");
}

#[cfg(unix)]
#[tokio::test]
async fn a_binary_that_does_not_answer_uci_keeps_the_old_engine() {
    let archive = tar_gz(&[Member::File(
        "stockfish",
        b"#!/bin/sh\necho hello\nexit 1\n",
    )]);
    let digest = sha256(&archive);
    let size = archive.len();
    let (base, _hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_17",
                    vec![asset(
                        LINUX_ASSET,
                        &format!("{base}/asset"),
                        size,
                        Some(digest),
                    )],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let error = install_managed_engine(&host, &location(&engine, &base))
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "The downloaded Stockfish could not start. The previous engine was retained."
    );
    assert_kept_old_version(&engine);
}

#[cfg(unix)]
#[tokio::test]
async fn an_archive_without_an_engine_is_rejected() {
    let archive = tar_gz(&[Member::File("README.md", b"docs only")]);
    let digest = sha256(&archive);
    let size = archive.len();
    let (base, _hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_17",
                    vec![asset(
                        LINUX_ASSET,
                        &format!("{base}/asset"),
                        size,
                        Some(digest),
                    )],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let error = install_managed_engine(&host, &location(&engine, &base))
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "Stockfish executable was missing from the release."
    );
    assert_kept_old_version(&engine);
}

// --- Archive safety ---

#[tokio::test]
async fn rejects_path_traversal_in_a_tar_archive() {
    let archive = tar_gz(&[
        Member::File("../escape", b"outside"),
        Member::File("stockfish", FAKE_ENGINE.as_bytes()),
    ]);
    let digest = sha256(&archive);
    let size = archive.len();
    let (base, _hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_17",
                    vec![asset(
                        LINUX_ASSET,
                        &format!("{base}/asset"),
                        size,
                        Some(digest),
                    )],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let error = install_managed_engine(&host, &location(&engine, &base))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Stockfish archive has an unsafe path.");
    assert!(!dir.path().join("escape").exists());
    assert_kept_old_version(&engine);
}

#[tokio::test]
async fn rejects_a_symbolic_link_in_a_tar_archive() {
    let archive = tar_gz(&[
        Member::Link("stockfish", "/bin/sh"),
        Member::File("stockfish", FAKE_ENGINE.as_bytes()),
    ]);
    let digest = sha256(&archive);
    let size = archive.len();
    let (base, _hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_17",
                    vec![asset(
                        LINUX_ASSET,
                        &format!("{base}/asset"),
                        size,
                        Some(digest),
                    )],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let error = install_managed_engine(&host, &location(&engine, &base))
        .await
        .unwrap_err();
    assert_eq!(error.message, "Stockfish archive contains a link.");
    assert_kept_old_version(&engine);
}

#[test]
fn zip_archives_refuse_traversal_absolute_paths_and_links() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("a.zip");
    let cases: [(Vec<u8>, &str); 3] = [
        (
            zip_bytes(&[("../escape", b"x")], None),
            "Stockfish archive has an unsafe path.",
        ),
        (
            zip_bytes(&[("/abs/escape", b"x")], None),
            "Stockfish archive has an unsafe path.",
        ),
        (
            zip_bytes(&[], Some(("stockfish", "/bin/sh"))),
            "Stockfish archive contains a link.",
        ),
    ];
    for (index, (bytes, expected)) in cases.iter().enumerate() {
        std::fs::write(&archive, bytes).unwrap();
        let out = root.path().join(format!("out{index}"));
        let error = extract(ArchiveKind::Zip, &archive, &out).unwrap_err();
        assert_eq!(error.message, *expected);
    }
    assert!(!root.path().join("escape").exists());
}

#[test]
fn archive_names_select_the_format_case_insensitively() {
    assert_eq!(archive_kind("a.ZIP"), Some(ArchiveKind::Zip));
    assert_eq!(archive_kind("a.tar.gz"), Some(ArchiveKind::TarGz));
    assert_eq!(archive_kind("a.TGZ"), Some(ArchiveKind::TarGz));
    assert_eq!(archive_kind("a.tar"), Some(ArchiveKind::Tar));
    assert_eq!(archive_kind("a.7z"), None);
}

#[test]
fn safe_relative_keeps_only_normal_components() {
    assert_eq!(safe_relative("./a/b").unwrap(), Path::new("a/b"));
    assert!(safe_relative("../a").is_err());
    assert!(safe_relative("a/../../b").is_err());
    assert!(safe_relative("/etc/passwd").is_err());
    assert!(safe_relative("a\\..\\b").is_err());
}

// --- Handoff, cancellation and deadlines ---

#[tokio::test]
async fn a_refused_handoff_keeps_the_old_engine_and_resumes_nothing() {
    let archive = tar_gz(&[Member::File("stockfish", FAKE_ENGINE.as_bytes())]);
    let digest = sha256(&archive);
    let size = archive.len();
    let (base, _hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_17",
                    vec![asset(
                        LINUX_ASSET,
                        &format!("{base}/asset"),
                        size,
                        Some(digest),
                    )],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let handoff = TestHandoff::new(HandoffMode::Refuse);
    let mut location = location(&engine, &base);
    location.handoff = Some(handoff.clone());
    let error = install_managed_engine(&host, &location).await.unwrap_err();
    assert_eq!(error.message, "owners refused");
    assert_eq!(handoff.resumed.load(Ordering::SeqCst), 0);
    assert_kept_old_version(&engine);
}

#[tokio::test]
async fn owners_that_do_not_exit_time_out_and_keep_the_old_engine() {
    let archive = tar_gz(&[Member::File("stockfish", FAKE_ENGINE.as_bytes())]);
    let digest = sha256(&archive);
    let size = archive.len();
    let (base, _hits) = serve(|base| {
        vec![
            Route(
                "/release",
                200,
                release(
                    "sf_17",
                    vec![asset(
                        LINUX_ASSET,
                        &format!("{base}/asset"),
                        size,
                        Some(digest),
                    )],
                ),
            ),
            Route("/asset", 200, archive),
        ]
    })
    .await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let mut location = location(&engine, &base);
    location.handoff = Some(TestHandoff::new(HandoffMode::Hang));
    location.exit_deadline = Duration::from_millis(100);
    let error = install_managed_engine(&host, &location).await.unwrap_err();
    assert_eq!(
        error.message,
        "Stockfish did not exit. The previous engine was retained."
    );
    assert_kept_old_version(&engine);
}

#[tokio::test]
async fn a_cancelled_install_fails_as_an_abort() {
    let (base, _hits) = serve(|_| vec![Route("/release", 200, release("sf_17", vec![]))]).await;
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let (cancel, receiver) = watch::channel(false);
    cancel.send(true).unwrap();
    let mut location = location(dir.path(), &base);
    location.cancel = Some(receiver);
    let error = install_managed_engine(&host, &location).await.unwrap_err();
    assert!(error.aborted);
    assert_eq!(error.message, "This operation was aborted");
}

#[tokio::test]
async fn delete_reports_the_refused_handoff_and_removes_nothing() {
    let host = RecordingHost::default();
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("current");
    seed_installed(&engine, "sf_16", "old engine");
    let mut location = ManagedLocation::new(engine.clone());
    location.handoff = Some(TestHandoff::new(HandoffMode::Refuse));
    let error = delete_managed_engine(&host, &location).await.unwrap_err();
    assert_eq!(error.message, "owners refused");
    assert!(engine.join(EXE).exists());
}

// --- Asset choice ---

fn names(names: &[&str]) -> Vec<ReleaseAsset> {
    names
        .iter()
        .map(|name| ReleaseAsset {
            name: name.to_string(),
            browser_download_url: None,
            size: None,
            digest: None,
        })
        .collect()
}

#[test]
fn picks_the_macos_build_for_the_cpu_then_the_universal_one() {
    let assets = names(&[
        "stockfish-macos-x86-64.tar",
        "stockfish-macos-m1-apple-silicon.tar",
        "stockfish-macos-universal.tar",
    ]);
    let pick =
        |platform, arch| pick_stockfish_asset(&assets, platform, arch).map(|a| a.name.clone());
    assert_eq!(
        pick("darwin", "arm64").as_deref(),
        Some("stockfish-macos-m1-apple-silicon.tar")
    );
    assert_eq!(
        pick("darwin", "x64").as_deref(),
        Some("stockfish-macos-x86-64.tar")
    );
    assert_eq!(pick("darwin", "ia32"), None);
    let universal = names(&["stockfish-macos-universal.tar"]);
    assert_eq!(
        pick_stockfish_asset(&universal, "darwin", "arm64").map(|a| a.name.as_str()),
        Some("stockfish-macos-universal.tar")
    );
}

#[test]
fn picks_the_linux_and_windows_universal_builds_by_exact_name() {
    let assets = names(&[
        "stockfish-linux-x86-64-universal.tar.gz",
        "stockfish-windows-x86-64-universal.zip",
    ]);
    assert_eq!(
        pick_stockfish_asset(&assets, "linux", "x64").map(|a| a.name.as_str()),
        Some("stockfish-linux-x86-64-universal.tar.gz")
    );
    assert_eq!(
        pick_stockfish_asset(&assets, "win32", "x64").map(|a| a.name.as_str()),
        Some("stockfish-windows-x86-64-universal.zip")
    );
    assert!(pick_stockfish_asset(&assets, "linux", "arm64").is_none());
    assert!(pick_stockfish_asset(&assets, "freebsd", "x64").is_none());
}

#[test]
fn expected_digest_requires_a_sha256_hex_digest() {
    let hex64 = "ab".repeat(32);
    assert_eq!(
        expected_digest(Some(&format!("sha256:{}", hex64.to_uppercase()))),
        Some(hex64.clone())
    );
    assert_eq!(expected_digest(Some("md5:abc")), None);
    assert_eq!(expected_digest(Some("sha256:short")), None);
    assert_eq!(expected_digest(None), None);
}
