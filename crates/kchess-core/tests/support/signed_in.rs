//! A core over a scratch profile and a local Lichess, with `Alice` signed in: the host's secret
//! store opens her stored login as written (`secrets.decrypt`). Include with
//! `#[path = "support/signed_in.rs"] mod signed_in;` from a test binary of one test, since it sets
//! the process environment.
#![allow(dead_code)]

use kchess_contracts::CoreEvent;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use kchess_core::store::games;
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

    fn emit(&self, event: CoreEvent) {
        let (event, payload) = (event.name(), event.into_payload());
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
    // The core reads its Lichess from the process's switches when it opens; tests sharing a binary
    // under `cargo test` set them in turn, so each sets and opens before the next sets.
    static OPENING: Mutex<()> = Mutex::new(());
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(SignedIn::default());
    let core = {
        let _opening = OPENING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        kchess_core::env::set("KCHESS_STORE_DEBUG", "1");
        kchess_core::env::set("KCHESS_TEST_LICHESS_BASE", base);
        Arc::new(
            Core::open(
                Config::new(dir.path().to_path_buf()),
                Arc::clone(&host) as Arc<dyn Host>,
            )
            .unwrap(),
        )
    };
    host.core.set(Arc::downgrade(&core)).unwrap();
    core.database_for_tests()
        .write(|c| {
            games::add_account(c.db, "Alice", true)?;
            games::import_token_ciphertext(c.db, Some("Alice"), "lip_alice")
        })
        .unwrap();
    Profile {
        core,
        host,
        _dir: dir,
    }
}
