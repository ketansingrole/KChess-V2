//! A core over a scratch profile and a local Lichess, with `Alice` signed in: the host's secret
//! store opens her stored login as written (`secrets.decrypt`). Include with
//! `#[path = "support/signed_in.rs"] mod signed_in;` from a test binary of one test, since it sets
//! the process environment.
#![allow(dead_code)]

use std::sync::{Arc, Mutex, OnceLock, Weak};

use kchess_core::{Config, Core, Host, Level};
use serde_json::{Value, json};

/// The login `Alice` has, as Lichess sees it in `Authorization`.
pub const ALICE_LOGIN: &str = "Bearer lip_alice";

#[derive(Default)]
pub struct SignedIn {
    core: OnceLock<Weak<Core>>,
    pub logs: Mutex<Vec<String>>,
    pub events: Mutex<Vec<(String, Value)>>,
}

impl Host for SignedIn {
    fn log(&self, _: Level, scope: &str, message: &str) {
        self.logs
            .lock()
            .unwrap()
            .push(format!("[{scope}] {message}"));
    }

    fn emit(&self, event: &str, payload: Value) {
        if event != "host:request" {
            self.events
                .lock()
                .unwrap()
                .push((event.to_string(), payload));
            return;
        }
        let Some(core) = self.core.get().and_then(Weak::upgrade) else {
            return;
        };
        let answer = match payload["kind"].as_str() {
            Some("secrets.decrypt") => json!({ "value": payload["payload"] }),
            _ => json!({ "error": "Not in this test." }),
        };
        tokio::spawn(async move {
            core.call("host.reply", vec![payload["id"].clone(), answer])
                .await
                .unwrap();
        });
    }
}

pub struct Profile {
    pub core: Arc<Core>,
    pub host: Arc<SignedIn>,
    _dir: tempfile::TempDir,
}

/// Opens the core against the Lichess at `base` with `Alice` connected.
pub async fn open(base: &str) -> Profile {
    // SAFETY: each test binary including this has one test, which calls this before any other
    // thread reads the environment.
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("KCHESS_STORE_DEBUG", "1");
        std::env::set_var("KCHESS_TEST_LICHESS_BASE", base);
    }
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(SignedIn::default());
    let core = Arc::new(
        Core::open(
            Config::new(dir.path().to_path_buf()),
            Arc::clone(&host) as Arc<dyn Host>,
        )
        .unwrap(),
    );
    host.core.set(Arc::downgrade(&core)).unwrap();
    core.call("store.games.addAccount", vec![json!("Alice"), json!(true)])
        .await
        .unwrap();
    core.call(
        "store.games.importTokenCiphertext",
        vec![json!("Alice"), json!("lip_alice")],
    )
    .await
    .unwrap();
    Profile {
        core,
        host,
        _dir: dir,
    }
}
