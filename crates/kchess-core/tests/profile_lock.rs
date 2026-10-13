//! Cross-process single-instance protection (`ProfileClaim`): a second core over the
//! same profile is refused even when the first core lives in another process (a second
//! app launch, the CLI on the same data dir), where the in-process map cannot see it.

use std::fs::File;
use std::sync::Arc;

use kchess_core::facade::ProfileClaim;
use kchess_core::{Config, Core, Host, Level};
use serde_json::Value;

/// The lock file the claim holds (`PROFILE_LOCK`, `crates/kchess-core/src/facade.rs`).
const LOCK: &str = "profile.lock";
/// The refusal a second core sees while the profile is claimed.
const RUNNING: &str = "A KChess core is already running for this profile.";

/// Records nothing; none of these calls needs a host answer.
#[derive(Default)]
struct Wire;

impl Host for Wire {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {}

    fn emit(&self, _event: &str, _payload: Value) {}
}

/// Holds the OS lock from a separately opened handle: `flock`/`LockFileEx` conflict
/// across open file descriptions, which is exactly how a second process opens the file.
fn hold(lock: &std::path::Path) -> File {
    let file = File::create(lock).expect("lock file");
    file.try_lock().expect("holder locks");
    file
}

#[test]
fn a_lock_held_elsewhere_refuses_the_claim() {
    let dir = tempfile::tempdir().expect("profile");
    let _holder = hold(&dir.path().join(LOCK));
    let refused = ProfileClaim::claim(dir.path());
    let err = refused.err().expect("held profile is refused");
    assert_eq!(err.to_string(), RUNNING);
    // Releasing the holder frees the profile without any core involved.
    drop(_holder);
    let claim = ProfileClaim::claim(dir.path()).expect("released profile claims");
    drop(claim);
}

#[test]
fn dropping_the_claim_releases_the_os_lock() {
    let dir = tempfile::tempdir().expect("profile");
    let lock = dir.path().join(LOCK);
    let claim = ProfileClaim::claim(dir.path()).expect("profile claims");
    // While claimed, a second open description cannot lock (the cross-process case).
    {
        let probe = File::open(&lock).expect("lock file");
        assert!(probe.try_lock().is_err(), "a live claim holds the lock");
    }
    drop(claim);
    // Release on drop frees it: a later holder (or core) locks again.
    {
        let probe = File::open(&lock).expect("lock file");
        probe.try_lock().expect("dropped claim releases the lock");
    }
}

#[tokio::test]
async fn a_second_process_lock_refuses_core_open() {
    let dir = tempfile::tempdir().expect("profile");
    let host = Arc::new(Wire);
    let holder = hold(&dir.path().join(LOCK));
    let refused = Core::open(Config::new(dir.path().to_path_buf()), host.clone());
    let err = refused.err().expect("held profile refuses the core");
    assert_eq!(err.to_string(), RUNNING);
    drop(holder);
    let core = Arc::new(
        Core::open(Config::new(dir.path().to_path_buf()), host.clone()).expect("profile opens"),
    );
    // The open core holds the lock against any later process for its lifetime.
    let lock = dir.path().join(LOCK);
    {
        let probe = File::open(&lock).expect("lock file");
        assert!(probe.try_lock().is_err(), "an open core holds the lock");
    }
    core.close().await;
    {
        // Scoped: a successful probe holds the lock until its handle drops.
        let probe = File::open(&lock).expect("lock file");
        probe.try_lock().expect("a closed core releases the lock");
    }
    let reopened = Core::open(Config::new(dir.path().to_path_buf()), host.clone());
    assert!(
        reopened.is_ok(),
        "a closed profile reopens: {:?}",
        reopened.err().map(|cause| cause.to_string())
    );
    reopened.expect("reopened").close().await;
}
