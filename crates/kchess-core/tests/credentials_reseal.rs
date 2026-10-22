//! Tokens an earlier release's host sealed (Electron's `safeStorage`) are resealed with the
//! profile's own credentials the first time they are needed, so the GPUI app opens them too.

use std::sync::{Arc, Mutex};

use kchess_core::capabilities::Capabilities;
use kchess_core::lichess::adapters::DbTokens;
use kchess_core::lichess::client::TokenSource;
use kchess_core::secrets::{Credentials, KeyStore, is_sealed};
use kchess_core::store::Database;
use kchess_core::{Config, CoreError, Host, Level};
use serde_json::{Value, json};

/// A host whose secret store opens base64 text, as `fakeSecrets` does, or refuses like a host
/// that cannot read `safeStorage` (the GPUI app).
struct Legacy {
    capabilities: Mutex<Option<Arc<Capabilities>>>,
    asked: Mutex<Vec<String>>,
    opens: bool,
}

impl Host for Legacy {
    fn log(&self, _: Level, _: &str, _: &str) {}

    fn emit(&self, event: &str, payload: Value) {
        if event != "host:request" {
            return;
        }
        let kind = payload["kind"].as_str().unwrap_or_default().to_string();
        self.asked.lock().unwrap().push(kind.clone());
        let capabilities = self.capabilities.lock().unwrap().clone().unwrap();
        let opens = self.opens;
        tokio::spawn(async move {
            use base64::Engine;
            let answer = match (kind.as_str(), opens) {
                ("secrets.decrypt", true) => {
                    let plain = base64::engine::general_purpose::STANDARD
                        .decode(payload["payload"].as_str().unwrap())
                        .unwrap();
                    json!({ "value": String::from_utf8(plain).unwrap() })
                }
                _ => json!({ "error": "This login was saved by an earlier KChess release." }),
            };
            capabilities.reply(payload["id"].as_u64().unwrap(), &answer);
        });
    }
}

/// A credential store in memory, shared by every `Credentials` made over it.
#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Option<String>>>);

impl KeyStore for Memory {
    fn available(&self) -> bool {
        true
    }
    fn read(&self) -> Result<Option<String>, CoreError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn write(&self, key: &str) -> Result<(), CoreError> {
        *self.0.lock().unwrap() = Some(key.to_string());
        Ok(())
    }
}

struct Profile {
    _dir: tempfile::TempDir,
    host: Arc<Legacy>,
    database: Arc<Database>,
    tokens: DbTokens,
}

/// "lip_legacy" as the legacy host sealed it.
const LEGACY: &str = "bGlwX2xlZ2FjeQ==";

async fn profile(opens: bool, keys: Memory) -> Profile {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Legacy {
        capabilities: Mutex::new(None),
        asked: Mutex::new(Vec::new()),
        opens,
    });
    let capabilities = Arc::new(Capabilities::with_credentials(
        host.clone(),
        Some(Arc::new(Credentials::new(Box::new(keys)))),
    ));
    *host.capabilities.lock().unwrap() = Some(Arc::clone(&capabilities));
    let database = Arc::new(Database::new(
        Config::new(dir.path().to_path_buf()),
        host.clone(),
    ));
    database
        .call_async(
            "store.games.importTokenCiphertext",
            &[json!("tester"), json!(LEGACY)],
        )
        .await
        .unwrap();
    let tokens = DbTokens::new(Arc::clone(&database), capabilities);
    Profile {
        _dir: dir,
        host,
        database,
        tokens,
    }
}

async fn stored(database: &Arc<Database>) -> String {
    database
        .call_async("store.games.tokenCiphertext", &[json!("tester")])
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn an_old_token_is_resealed_when_first_read_and_then_opens_without_the_host() {
    let keys = Memory::default();
    let profile = profile(true, keys.clone()).await;
    assert_eq!(
        profile.tokens.token("tester").await.unwrap().as_deref(),
        Some("lip_legacy")
    );
    let resealed = stored(&profile.database).await;
    assert!(is_sealed(&resealed), "{resealed}");

    profile.host.asked.lock().unwrap().clear();
    // Names match without case, as `tokenCiphertext` reads them.
    assert_eq!(
        profile.tokens.token("Tester").await.unwrap().as_deref(),
        Some("lip_legacy")
    );
    assert!(profile.host.asked.lock().unwrap().is_empty());

    // Another app over the same profile and key store opens it.
    let other = Credentials::new(Box::new(keys));
    assert_eq!(other.open(&resealed).unwrap(), "lip_legacy");
}

#[tokio::test]
async fn a_host_that_cannot_open_an_old_token_leaves_it_for_one_that_can() {
    let profile = profile(false, Memory::default()).await;
    let error = profile.tokens.token("tester").await.unwrap_err();
    assert_eq!(
        error.message,
        "This login was saved by an earlier KChess release."
    );
    assert_eq!(stored(&profile.database).await, LEGACY);
}

#[tokio::test]
async fn a_reseal_never_replaces_a_login_saved_in_the_meantime() {
    let profile = profile(true, Memory::default()).await;
    profile
        .database
        .call_async(
            "store.games.importTokenCiphertext",
            &[json!("tester"), json!("kcs1:newer")],
        )
        .await
        .unwrap();
    let replaced = profile
        .database
        .call_async(
            "store.games.resealTokenCiphertext",
            &[json!("tester"), json!(LEGACY), json!("kcs1:stale")],
        )
        .await
        .unwrap();
    assert_eq!(replaced, json!(false));
    assert_eq!(stored(&profile.database).await, "kcs1:newer");
}
