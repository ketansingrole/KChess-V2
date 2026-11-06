//! Starting the service binary (`connect_or_start`): the first app starts it from this
//! release's copy in the runtime folder, the second reuses it, and it stops when both are gone.
//!
//! It points the runtime folder at a test folder through the process environment, so it is the
//! only test in this binary: no other test can read the environment while it changes.
// Tests set process environment (`std::env::set_var`), which is unsafe in edition 2024.
#![allow(unsafe_code)]

use std::path::Path;
use std::time::Duration;

use kchess_service::{
    Capability, Hello, ServiceConfig, Start, connect_or_start, discovery, protocol,
};

fn window(kind: &str) -> Hello {
    Hello {
        kind: kind.into(),
        capabilities: vec![Capability::Browser, Capability::Focus],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_app_starts_the_service_and_the_second_reuses_it() {
    let profile = tempfile::tempdir().expect("profile");
    let runtime = tempfile::tempdir().expect("runtime");
    // SAFETY: the only test in this binary sets it before it starts anything that reads the
    // environment.
    unsafe { std::env::set_var("KCHESS_RUNTIME_DIR", runtime.path()) };
    // An earlier release's copy, which the started service removes.
    let earlier = runtime.path().join("core").join("1999.1.0");
    std::fs::create_dir_all(&earlier).expect("earlier release");
    let start = Start {
        binary: env!("CARGO_BIN_EXE_kchess-service").into(),
        config: ServiceConfig::default(),
    };
    let (gpui, _gpui_events) = connect_or_start(profile.path(), &window("gpui"), &start)
        .await
        .expect("started");
    let info = discovery::read_info(profile.path()).expect("core.json");
    assert_ne!(info.pid, std::process::id(), "a process of its own");
    let copied = runtime.path().join("core").join(protocol::release()).join(
        Path::new(env!("CARGO_BIN_EXE_kchess-service"))
            .file_name()
            .expect("name"),
    );
    assert!(copied.exists(), "it runs from this release's copy");
    assert!(!earlier.exists(), "other releases' copies are removed");

    let (electron, _electron_events) =
        connect_or_start(profile.path(), &window("electron"), &start)
            .await
            .expect("reused");
    assert_eq!(
        discovery::read_info(profile.path()).expect("core.json").pid,
        info.pid
    );
    assert_ne!(gpui.connection(), electron.connection());
    electron.call("library", vec![]).await.expect("works");

    // Both gone: the service stops after its idle period (10 s by default).
    drop((gpui, electron));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while discovery::read_info(profile.path()).is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the service stops when idle"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
