//! The core's one-time import of an earlier release's database, as its first call runs it: the
//! plaintext logins are sealed with the host's secret store and then removed from the earlier
//! file.

use std::sync::{Arc, OnceLock, Weak};

use kchess_contracts::{CoreEvent, HostReply, HostRequest};
use kchess_core::store::{Database, games};
use kchess_core::{Config, Core, Host, Level};
use rusqlite::Connection;
use serde_json::json;

/// A host whose secret store seals a login as `sealed:<login>`.
#[derive(Default)]
struct Sealer {
    core: OnceLock<Weak<Core>>,
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
        let answer = match ask.request {
            HostRequest::SecretsAvailable => HostReply::Value(json!(true)),
            HostRequest::SecretsEncrypt(login) => {
                HostReply::Value(json!(format!("sealed:{login}")))
            }
            _ => HostReply::Error("Not in this test.".into()),
        };
        tokio::spawn(async move {
            core.host_reply(ask.id, answer).unwrap();
        });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_earlier_databases_logins_are_sealed_and_removed_from_it() {
    let earlier = tempfile::tempdir().unwrap();
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
