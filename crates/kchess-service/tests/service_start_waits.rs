//! `connect_or_start` with a service binary that only notes, in `<profile>/launched`, whether the
//! profile's `core.json` was still there when it started, then exits without serving.
//!
//! The runtime folder is set for the whole process (`kchess_core::env`), so every test here
//! shares one, and the one script copied into it.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use kchess_service::protocol::{self, Frame, RefuseReason, read_frame, write_frame};
use kchess_service::{
    Capability, ConnectError, Hello, Start, connect_or_start,
    connect_or_start_within, discovery,
};

const SCRIPT: &str = "#!/bin/sh
if [ -e \"$3/core.json\" ]; then echo early > \"$3/launched\"; else echo after > \"$3/launched\"; fi
";

/// The runtime folder and the script, made once for the process.
fn start() -> Start {
    static FOLDERS: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    let (_, binary) = FOLDERS.get_or_init(|| {
        let folder = tempfile::tempdir().expect("folder");
        kchess_core::env::set("KCHESS_RUNTIME_DIR", folder.path().join("runtime"));
        let binary = folder.path().join("kchess-service");
        std::fs::write(&binary, SCRIPT).expect("script");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("mode");
        (folder, binary)
    });
    Start {
        binary: binary.clone(),
        config: kchess_core::Config::default(),
    }
}

fn hello() -> Hello {
    Hello {
        kind: "gpui".into(),
        capabilities: vec![Capability::Browser],
    }
}

/// A short deadline in real time: the client's wait is the same at any length.
#[tokio::test]
async fn a_core_that_never_starts_fails_at_the_deadline() {
    let profile = tempfile::tempdir().expect("profile");
    let start = start();
    let deadline = Duration::from_millis(500);
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        connect_or_start_within(profile.path(), &hello(), &start, deadline),
    )
    .await
    .expect("gave up within the start deadline");
    let Err(ConnectError::Failed(message)) = outcome else {
        panic!("a core that never starts is an error");
    };
    assert_eq!(message, "The KChess core did not start.");
    assert!(started.elapsed() >= deadline);
}

/// A stand-in core of another release, used by nobody: it refuses with `sole`, as a real one
/// does before it closes, and lets the profile go `closing` later.
async fn closing_core(profile: &Path, closing: Duration) -> tokio::task::JoinHandle<()> {
    let socket = profile.join("fake.sock");
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    let info = discovery::CoreInfo {
        pid: std::process::id(),
        socket: socket.to_string_lossy().into(),
        release: "1999.1.0".into(),
        protocol: protocol::protocol(),
        started_at: 0,
    };
    let info_file = profile.join(discovery::INFO_FILE);
    discovery::write_private(&info_file, &serde_json::to_vec(&info).expect("info"))
        .expect("core.json");
    discovery::write_private(&profile.join(discovery::TOKEN_FILE), b"test-token").expect("token");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        read_frame(&mut stream).await.expect("hello");
        let refuse = Frame::Refuse {
            reason: RefuseReason::Version,
            message: "Another KChess release is open.".into(),
            sole: true,
            clients: Vec::new(),
        };
        write_frame(&mut stream, &refuse).await.expect("refuse");
        tokio::time::sleep(closing).await;
        std::fs::remove_file(&info_file).expect("closed");
    })
}

/// The launched script's note, once it appears.
async fn launched(profile: &Path) -> String {
    let note = profile.join("launched");
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match std::fs::read_to_string(&note) {
                Ok(text) if text.ends_with('\n') => return text.trim().to_owned(),
                _ => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .expect("the service was started")
}

/// The new release's core starts only once the closing one has let the profile go: started
/// earlier, it would find the profile locked and exit.
#[tokio::test(flavor = "multi_thread")]
async fn a_closing_core_of_another_release_is_waited_for() {
    let profile = tempfile::tempdir().expect("profile");
    let start = start();
    let core = closing_core(profile.path(), Duration::from_millis(300)).await;
    let path = profile.path().to_path_buf();
    let connecting =
        tokio::spawn(async move { connect_or_start(&path, &hello(), &start).await.err() });
    assert_eq!(launched(profile.path()).await, "after");
    core.await.expect("the core closed");
    connecting.abort();
}
