//! A core over a scratch profile and a local Lichess, with `Alice` signed in: the host's secret
//! store opens her stored login as written (`secrets.decrypt`).

use kchess_contracts::{CoreEvent, HostAsk, HostReply, HostRequest};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use kchess_core::store::games;
use kchess_core::{Config, Core, Host, Level, TestSwitches};
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
        let Ok(ask) = serde_json::from_value::<HostAsk>(payload) else {
            return;
        };
        let answer = match ask.request {
            HostRequest::SecretsDecrypt(sealed) => HostReply::Value(json!(sealed)),
            _ => HostReply::Error("Not in this test.".into()),
        };
        tokio::spawn(async move {
            core.host_reply(ask.id, answer).unwrap();
        });
    }
}

pub struct Profile {
    pub core: Arc<Core>,
    _dir: tempfile::TempDir,
}

/// Opens the core against the Lichess at `base` with `Alice` connected.
pub async fn open(base: &str) -> Profile {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(SignedIn::default());
    let config = Config {
        test: Some(TestSwitches {
            lichess_origin: Some(base.to_string()),
            ..TestSwitches::default()
        }),
        ..Config::new(dir.path().to_path_buf())
    };
    let core = Arc::new(Core::open(config, Arc::clone(&host) as Arc<dyn Host>).unwrap());
    host.core.set(Arc::downgrade(&core)).unwrap();
    core.database_for_tests()
        .write(|c| {
            games::add_account(c.db, "Alice", true)?;
            games::import_token_ciphertext(c.db, Some("Alice"), "lip_alice")
        })
        .unwrap();
    Profile { core, _dir: dir }
}
