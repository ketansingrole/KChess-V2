//! The core's one-time import of an earlier release's database, as its first call runs it: the
//! plaintext logins are sealed with the host's secret store and then removed from the earlier
//! file.

use std::sync::{Arc, OnceLock, Weak};

use kchess_contracts::{CoreEvent, HostReply, HostRequest};
use kchess_core::store::{Database, games};
use kchess_core::{Config, Core, Host, Level};
use rusqlite::Connection;
use serde_json::json;

/// A host whose secret store seals a login as `sealed:<login>`, once `hold` (when set) lets it.
#[derive(Default)]
struct Sealer {
    core: OnceLock<Weak<Core>>,
    hold: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    asked: std::sync::atomic::AtomicBool,
}

impl Host for Sealer {
    fn log(&self, _: Level, _: &str, _: &str) {}

    fn emit(&self, event: CoreEvent) {
        let CoreEvent::HostRequest(ask) = event else {
            return;
        };
        let Some(core) = self.core.get().and_then(Weak::upgrade) else {
            return;
        };
        let mut held = None;
        let answer = match ask.request {
            HostRequest::SecretsAvailable => HostReply::Value(json!(true)),
            HostRequest::SecretsEncrypt(login) => {
                self.asked.store(true, std::sync::atomic::Ordering::SeqCst);
                held = self.hold.lock().unwrap().take();
                HostReply::Value(json!(format!("sealed:{login}")))
            }
            _ => HostReply::Error("Not in this test.".into()),
        };
        tokio::spawn(async move {
            if let Some(held) = held {
                held.await.ok();
            }
            core.host_reply(ask.id, answer).unwrap();
        });
    }
}

/// An earlier release's database holding Carol's plaintext login, and its path.
fn earlier_database(earlier: &tempfile::TempDir) -> std::path::PathBuf {
    let database = Database::new(
        Config::new(earlier.path().to_path_buf()),
        kchess_core::test_host::TestHost::quiet(),
    );
    database
        .read(|_| Ok(()))
        .expect("the earlier database opens");
    database.close();
    let legacy = earlier.path().join("kchess.db");
    Connection::open(&legacy)
        .and_then(|connection| {
            connection.execute_batch(
                "CREATE TABLE lichess_accounts (username TEXT, auth_kind TEXT, last_synced_at INTEGER);
                 INSERT INTO lichess_accounts VALUES ('Carol', 'oauth', 5);
                 CREATE TABLE lichess_tokens (username TEXT, token TEXT);
                 INSERT INTO lichess_tokens VALUES ('Carol', 'lip_carol');",
            )
        })
        .expect("the earlier database is written");
    legacy
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_earlier_databases_logins_are_sealed_and_removed_from_it() {
    let earlier = tempfile::tempdir().unwrap();
    let legacy = earlier_database(&earlier);
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Sealer::default());
    let config = Config {
        legacy_database_path: Some(legacy.clone()),
        ..Config::new(dir.path().to_path_buf())
    };
    let core = Arc::new(Core::open(config, Arc::clone(&host) as Arc<dyn Host>).unwrap());
    host.core.set(Arc::downgrade(&core)).unwrap();

    let data = core.call("loadData", vec![]).await.expect("loadData");
    assert_eq!(data["accounts"][0]["username"], json!("Carol"));
    assert_eq!(data["accounts"][0]["connected"], json!(true));
    let sealed = core
        .database_for_tests()
        .read(|c| games::token_ciphertext(c.db, "Carol"))
        .expect("the token");
    assert_eq!(sealed.as_deref(), Some("sealed:lip_carol"));
    let left: i64 = Connection::open(&legacy)
        .and_then(|earlier| {
            earlier.query_row("SELECT COUNT(*) FROM lichess_tokens", [], |row| row.get(0))
        })
        .expect("the earlier database");
    assert_eq!(left, 0, "the plaintext login is gone");
    core.close().await;
}

/// 2026-11-15: the first call ran the import itself, so a call cancelled while the keychain was
/// asked (its window closed) stopped the import after Carol was committed as connected and
/// before her login was: she stayed connected without one, and the import never ran again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_import_whose_first_call_is_cancelled_still_seals_the_logins() {
    let earlier = tempfile::tempdir().unwrap();
    let legacy = earlier_database(&earlier);
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Sealer::default());
    let (release, held) = tokio::sync::oneshot::channel();
    *host.hold.lock().unwrap() = Some(held);
    let config = Config {
        legacy_database_path: Some(legacy),
        ..Config::new(dir.path().to_path_buf())
    };
    let core = Arc::new(Core::open(config, Arc::clone(&host) as Arc<dyn Host>).unwrap());
    host.core.set(Arc::downgrade(&core)).unwrap();

    let first = tokio::spawn({
        let core = Arc::clone(&core);
        async move { core.call("loadData", vec![]).await }
    });
    crate::support::wait_until(
        std::time::Duration::from_secs(10),
        "the keychain to be asked",
        || {
            host.asked
                .load(std::sync::atomic::Ordering::SeqCst)
                .then_some(())
        },
    )
    .await;
    first.abort();
    let _ = first.await;
    release.send(()).unwrap();

    let data = core.call("loadData", vec![]).await.expect("loadData");
    assert_eq!(data["accounts"][0]["connected"], json!(true));
    let sealed = core
        .database_for_tests()
        .read(|c| games::token_ciphertext(c.db, "Carol"))
        .expect("the token");
    assert_eq!(sealed.as_deref(), Some("sealed:lip_carol"));
    core.close().await;
}
