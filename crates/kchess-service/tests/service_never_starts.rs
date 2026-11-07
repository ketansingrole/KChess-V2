//! A service binary that exits at once without serving: `connect_or_start` gives up at its start
//! deadline (30 s) instead of retrying for ever. The clock is paused, so the wait is instant.
//!
//! It points the runtime folder at a test folder for the whole process (`kchess_core::env`), so
//! it is the only test in this binary.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use kchess_service::{Capability, ConnectError, Hello, ServiceConfig, Start, connect_or_start};

#[tokio::test(start_paused = true)]
async fn a_core_that_never_starts_fails_at_the_deadline() {
    let profile = tempfile::tempdir().expect("profile");
    let runtime = tempfile::tempdir().expect("runtime");
    kchess_core::env::set("KCHESS_RUNTIME_DIR", runtime.path());
    let bin = tempfile::tempdir().expect("bin");
    let binary = bin.path().join("kchess-service");
    std::fs::write(&binary, "#!/bin/sh\nexit 0\n").expect("script");
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("mode");
    let start = Start {
        binary,
        config: ServiceConfig::default(),
    };
    let hello = Hello {
        kind: "gpui".into(),
        capabilities: vec![Capability::Browser],
    };
    let started = tokio::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        connect_or_start(profile.path(), &hello, &start),
    )
    .await
    .expect("gave up within the start deadline");
    let Err(ConnectError::Failed(message)) = outcome else {
        panic!("a core that never starts is an error");
    };
    assert_eq!(message, "The KChess core did not start.");
    assert!(started.elapsed() >= Duration::from_secs(30));
}
