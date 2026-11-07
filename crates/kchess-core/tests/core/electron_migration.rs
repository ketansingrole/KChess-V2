//! A profile the Electron app last used is taken over at the first start: its `localStorage`
//! (studies, view preferences) and every Lichess login Electron's `safeStorage` sealed, in use or
//! not, sealed again with the profile's credentials. The profile records it only once all of that
//! worked; a failure keeps every original and the next start tries again.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use kchess_core::capabilities::Capabilities;
use kchess_core::electron_migration::{Migration, Outcome};
use kchess_core::secrets::{Credentials, KeyStore, is_sealed};
use kchess_core::store::Database;
use kchess_core::{Config, CoreError, Host, Level};
use serde_json::{Value, json};

/// A host that opens what "Electron sealed" (base64 here), except the logins it is told to refuse.
struct Electron {
    capabilities: Mutex<Option<Arc<Capabilities>>>,
    asked: Mutex<Vec<String>>,
    refuses: Mutex<HashSet<String>>,
    logs: Mutex<Vec<String>>,
}

impl Host for Electron {
    fn log(&self, _: Level, _: &str, message: &str) {
        self.logs.lock().unwrap().push(message.to_string());
    }

    fn emit(&self, event: &str, payload: Value) {
        if event != "host:request" {
            return;
        }
        let kind = payload["kind"].as_str().unwrap_or_default().to_string();
        self.asked.lock().unwrap().push(kind.clone());
        let capabilities = self.capabilities.lock().unwrap().clone().unwrap();
        let sealed = payload["payload"].as_str().unwrap_or_default().to_string();
        let refused = self.refuses.lock().unwrap().contains(&sealed);
        tokio::spawn(async move {
            use base64::Engine;
            let answer = if kind == "secrets.decrypt" && !refused {
                let plain = base64::engine::general_purpose::STANDARD
                    .decode(&sealed)
                    .unwrap();
                json!({ "value": String::from_utf8(plain).unwrap() })
            } else {
                json!({ "error": "The keychain refused." })
            };
            capabilities.reply(payload["id"].as_u64().unwrap(), &answer);
        });
    }
}

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

/// Three accounts' logins as Electron sealed them: one connected, two not in use.
const LOGINS: [(&str, &str, &str); 3] = [
    ("alice", "lip_alice", "bGlwX2FsaWNl"),
    ("bob", "lip_bob", "bGlwX2JvYg=="),
    ("carol", "lip_carol", "bGlwX2Nhcm9s"),
];

struct Profile {
    dir: tempfile::TempDir,
    host: Arc<Electron>,
    database: Arc<Database>,
    keys: Memory,
    capabilities: Arc<Capabilities>,
}

impl Profile {
    fn migration(&self) -> Migration {
        Migration::new(
            Arc::clone(&self.database),
            Arc::clone(&self.capabilities),
            self.host.clone(),
            self.dir.path().to_path_buf(),
            Arc::new(AtomicBool::new(false)),
        )
    }

    async fn stored(&self, account: &str) -> String {
        self.database
            .call_async("store.games.tokenCiphertext", &[json!(account)])
            .await
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn marker(&self) -> Value {
        self.database
            .call_async("store.electron.migrated", &[])
            .await
            .unwrap()
    }

    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut *self.host.asked.lock().unwrap())
    }
}

fn copy_local_storage(profile: &Path) {
    let target = profile.join("Local Storage").join("leveldb");
    std::fs::create_dir_all(&target).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../kchess-electron-legacy/tests/fixtures/local-storage");
    for entry in std::fs::read_dir(fixture).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
    }
}

async fn profile(local_storage: bool, logins: bool) -> Profile {
    let dir = tempfile::tempdir().unwrap();
    if local_storage {
        copy_local_storage(dir.path());
    }
    let host = Arc::new(Electron {
        capabilities: Mutex::new(None),
        asked: Mutex::new(Vec::new()),
        refuses: Mutex::new(HashSet::new()),
        logs: Mutex::new(Vec::new()),
    });
    let keys = Memory::default();
    let capabilities = Arc::new(Capabilities::with_credentials(
        host.clone(),
        Some(Arc::new(Credentials::new(Box::new(keys.clone())))),
    ));
    *host.capabilities.lock().unwrap() = Some(Arc::clone(&capabilities));
    let database = Arc::new(Database::new(
        Config::new(dir.path().to_path_buf()),
        host.clone(),
    ));
    if logins {
        for (account, _, sealed) in LOGINS {
            database
                .call_async(
                    "store.games.importTokenCiphertext",
                    &[json!(account), json!(sealed)],
                )
                .await
                .unwrap();
        }
    }
    Profile {
        dir,
        host,
        database,
        keys,
        capabilities,
    }
}

#[tokio::test]
async fn an_electron_profile_is_taken_over_whole_at_the_first_start() {
    let profile = profile(true, true).await;
    assert_eq!(profile.migration().run().await.unwrap(), Outcome::Migrated);

    let library = profile
        .database
        .call_async("store.library.library", &[json!(0)])
        .await
        .unwrap();
    assert_eq!(library["imported"], json!(true));
    assert_eq!(library["studies"][0]["name"], "Ünïcode ♞ study");
    let preferences = profile
        .database
        .call_async("store.viewPreferences.all", &[])
        .await
        .unwrap();
    assert_eq!(preferences["kchess:studies-sort"], json!("updated"));

    // Every login, in use or not, now opens with the profile's own key, here or in another app.
    let other = Credentials::new(Box::new(profile.keys.clone()));
    for (account, token, _) in LOGINS {
        let stored = profile.stored(account).await;
        assert!(is_sealed(&stored), "{account}: {stored}");
        assert_eq!(other.open(&stored).unwrap(), token);
    }
    assert_eq!(
        profile.marker().await,
        json!({ "release": env!("CARGO_PKG_VERSION") })
    );
    // The logs name accounts and causes, never a token.
    for line in profile.host.logs.lock().unwrap().iter() {
        assert!(!line.contains("lip_"), "{line}");
    }
}

#[tokio::test]
async fn a_failed_login_keeps_its_original_and_the_next_start_finishes() {
    let profile = profile(true, true).await;
    profile
        .host
        .refuses
        .lock()
        .unwrap()
        .insert(LOGINS[1].2.to_string());
    let failed = profile.migration().run().await.unwrap_err();
    assert!(
        failed.message.contains("1 saved login"),
        "{}",
        failed.message
    );
    assert_eq!(profile.marker().await, Value::Null, "not marked as done");
    assert_eq!(
        profile.stored("bob").await,
        LOGINS[1].2,
        "the original stays"
    );
    assert!(is_sealed(&profile.stored("alice").await));
    assert!(is_sealed(&profile.stored("carol").await));

    // The next start: the keychain answers now, and only bob's login is still to open.
    profile.host.refuses.lock().unwrap().clear();
    profile.asked();
    assert_eq!(profile.migration().run().await.unwrap(), Outcome::Migrated);
    assert_eq!(profile.asked(), vec!["secrets.decrypt".to_string()]);
    let other = Credentials::new(Box::new(profile.keys.clone()));
    assert_eq!(other.open(&profile.stored("bob").await).unwrap(), "lip_bob");
    assert!(!profile.marker().await.is_null());
}

#[tokio::test]
async fn an_unreadable_local_storage_is_not_marked_and_is_tried_again() {
    let profile = profile(true, true).await;
    let manifest = profile
        .dir
        .path()
        .join("Local Storage/leveldb/MANIFEST-000001");
    let kept = std::fs::read(&manifest).unwrap();
    std::fs::write(&manifest, b"not a manifest").unwrap();
    assert!(profile.migration().run().await.is_err());
    assert_eq!(profile.marker().await, Value::Null);
    // The logins were resealed all the same; only the documents wait.
    assert!(is_sealed(&profile.stored("alice").await));

    std::fs::write(&manifest, kept).unwrap();
    assert_eq!(profile.migration().run().await.unwrap(), Outcome::Migrated);
    let library = profile
        .database
        .call_async("store.library.library", &[json!(0)])
        .await
        .unwrap();
    assert_eq!(library["studies"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_second_start_does_nothing() {
    let profile = profile(true, true).await;
    assert_eq!(profile.migration().run().await.unwrap(), Outcome::Migrated);
    let before: Vec<String> = stored_logins(&profile).await;
    profile.asked();
    // A later change is not undone by the leftovers, which are still on disk.
    profile
        .database
        .call_async(
            "store.viewPreferences.set",
            &[json!("kchess:studies-sort"), json!("name")],
        )
        .await
        .unwrap();
    assert_eq!(
        profile.migration().run().await.unwrap(),
        Outcome::AlreadyDone
    );
    assert!(profile.asked().is_empty(), "nothing is opened again");
    assert_eq!(stored_logins(&profile).await, before);
    let preferences = profile
        .database
        .call_async("store.viewPreferences.all", &[])
        .await
        .unwrap();
    assert_eq!(preferences["kchess:studies-sort"], json!("name"));
}

#[tokio::test]
async fn a_profile_without_electron_data_is_left_alone() {
    let profile = profile(false, false).await;
    assert_eq!(
        profile.migration().run().await.unwrap(),
        Outcome::NothingToDo
    );
    assert!(profile.asked().is_empty(), "the secret store is not asked");
    assert_eq!(profile.marker().await, Value::Null);
}

#[tokio::test]
async fn the_core_takes_over_an_electron_profile_once_started() {
    let dir = tempfile::tempdir().unwrap();
    copy_local_storage(dir.path());
    let core =
        kchess_core::Core::open(Config::new(dir.path().to_path_buf()), Arc::new(Quiet)).unwrap();
    // The first call starts the core, whose background run takes the profile over; this one
    // waits for it or does it.
    core.call("loadData", vec![]).await.unwrap();
    let outcome = core.migrate_electron().await.unwrap();
    assert!(
        matches!(outcome, Outcome::Migrated | Outcome::AlreadyDone),
        "{outcome:?}"
    );
    assert_eq!(core.migrate_electron().await.unwrap(), Outcome::AlreadyDone);
    let library = core.call("library", vec![]).await.unwrap();
    assert_eq!(library["studies"].as_array().unwrap().len(), 1);
    core.close().await;
}

struct Quiet;
impl Host for Quiet {
    fn log(&self, _: Level, _: &str, _: &str) {}
    fn emit(&self, _: &str, _: Value) {}
}

async fn stored_logins(profile: &Profile) -> Vec<String> {
    let mut stored = Vec::new();
    for (account, _, _) in LOGINS {
        stored.push(profile.stored(account).await);
    }
    stored
}
