//! The Lichess services' storage and credentials, over the core's one database
//! (`store::Database::call`) and the host's secret store (`Capabilities`). Each trait method is a
//! storage method of the same meaning the TypeScript service called (`store.ts`, `reviewStore.ts`,
//! `setupPositionLookup.ts`), so the tables and their writes are unchanged.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::accounts::{AccountRow, CachedValue, LichessGame, LichessStore};
use super::client::TokenSource;
use super::lookups::LookupCache;
use crate::capabilities::Capabilities;
use crate::error::{CoreError, Result as CoreResult};
use crate::store::Database;

/// `LichessStore` over `kchess.db`.
pub struct DbLichessStore {
    database: Arc<Database>,
}

impl DbLichessStore {
    pub fn new(database: Arc<Database>) -> DbLichessStore {
        DbLichessStore { database }
    }

    fn call(&self, method: &str, args: &[Value]) -> CoreResult<Value> {
        self.database.call(method, args)
    }

    fn strings(value: Value) -> Vec<String> {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl LichessStore for DbLichessStore {
    fn accounts(&self) -> CoreResult<Vec<AccountRow>> {
        let data = self.call("store.games.loadData", &[])?;
        let rows = data
            .get("accounts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(rows
            .iter()
            .filter_map(|row| {
                Some(AccountRow {
                    username: row.get("username")?.as_str()?.to_string(),
                    connected: row
                        .get("connected")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    last_synced_at: row.get("lastSyncedAt").and_then(Value::as_i64),
                })
            })
            .collect())
    }

    fn dismissed_friends(&self) -> CoreResult<Vec<String>> {
        Ok(Self::strings(
            self.call("store.games.dismissedFriends", &[])?,
        ))
    }

    fn pending_game_ids(&self, account: &str) -> CoreResult<Vec<String>> {
        Ok(Self::strings(
            self.call("store.games.pendingGameIds", &[json!(account)])?,
        ))
    }

    fn save_games_page(
        &self,
        account: &str,
        games: &[LichessGame],
        reviews: &[Value],
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<()> {
        // The store writes the games and their reviews in one transaction, and checks `current`
        // once more before it writes.
        self.call(
            "store.games.saveGamesPage",
            &[
                json!(account),
                json!(games),
                json!(reviews),
                json!(current()),
            ],
        )
        .map(|_| ())
    }

    fn save_games_cursor(
        &self,
        account: &str,
        synced_at: i64,
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<()> {
        self.call(
            "store.games.saveGames",
            &[
                json!(account),
                json!([]),
                json!(synced_at),
                json!(current()),
            ],
        )
        .map(|_| ())
    }

    fn read_api_cache(&self, key: &str) -> CoreResult<Option<CachedValue>> {
        let row = self.call("store.games.readApiCache", &[json!(key)])?;
        Ok(match row {
            Value::Object(object) => Some(CachedValue {
                value: object.get("value").cloned().unwrap_or(Value::Null),
                fetched_at: object.get("fetchedAt").and_then(Value::as_i64).unwrap_or(0),
            }),
            _ => None,
        })
    }

    fn write_api_cache(
        &self,
        key: &str,
        value: &Value,
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<()> {
        self.call(
            "store.games.writeApiCache",
            &[json!(key), value.clone(), json!(current())],
        )
        .map(|_| ())
    }

    fn save_login(&self, username: &str, ciphertext: &str) -> CoreResult<()> {
        self.call(
            "store.games.saveTokenCiphertext",
            &[json!(username), json!(ciphertext)],
        )
        .map(|_| ())
    }

    fn save_reviews(
        &self,
        account: &str,
        reviews: &[Value],
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<Vec<Value>> {
        let _ = account;
        if !current() {
            return Err(CoreError::new("Sync was cancelled."));
        }
        let mut stored = Vec::with_capacity(reviews.len());
        for review in reviews {
            let written = self.call(
                "store.reviewStore.writeReview",
                std::slice::from_ref(review),
            )?;
            stored.push(written.get("review").cloned().unwrap_or(Value::Null));
        }
        Ok(stored)
    }

    fn mark_checked(&self, ids: &[String]) -> CoreResult<()> {
        let now = super::accounts::now_ms();
        self.call("store.reviewStore.markChecked", &[json!(ids), json!(now)])
            .map(|_| ())
    }
}

/// `TokenSource` over the encrypted tokens of `kchess.db`, decrypted by the host's secret store.
pub struct DbTokens {
    database: Arc<Database>,
    capabilities: Arc<Capabilities>,
}

impl DbTokens {
    pub fn new(database: Arc<Database>, capabilities: Arc<Capabilities>) -> DbTokens {
        DbTokens {
            database,
            capabilities,
        }
    }
}

impl TokenSource for DbTokens {
    fn token<'a>(&'a self, account: &'a str) -> BoxFuture<'a, CoreResult<Option<String>>> {
        Box::pin(async move {
            let ciphertext = self
                .database
                .call("store.games.tokenCiphertext", &[json!(account)])?;
            match ciphertext.as_str() {
                Some(ciphertext) => self.capabilities.decrypt(ciphertext).await.map(Some),
                None => Ok(None),
            }
        })
    }
}

/// `LookupCache` over the saved position lookups (`setupPositionLookup`).
pub struct DbLookups {
    database: Arc<Database>,
}

impl DbLookups {
    pub fn new(database: Arc<Database>) -> DbLookups {
        DbLookups { database }
    }
}

impl LookupCache for DbLookups {
    fn read(&self, key: &str) -> CoreResult<Option<Value>> {
        // The store answers the saved text, or null when it is missing or too large to read; text
        // that is not JSON is an unreadable entry, which the explorer treats as missing.
        let text = self
            .database
            .call("store.setupPositionLookup.read", &[json!(key)])?;
        Ok(text
            .as_str()
            .and_then(|text| serde_json::from_str::<Value>(text).ok()))
    }

    fn write(&self, key: &str, value: &Value) -> CoreResult<()> {
        self.database
            .call(
                "store.setupPositionLookup.write",
                &[json!(key), value.clone()],
            )
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Config, Host, Level};

    struct Quiet;

    impl Host for Quiet {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, _: &str, _: Value) {}
    }

    /// A saved explorer lookup written by the cache comes back as the same JSON, so a refresh that
    /// fails can show it (`lookups.rs` reads the entry the store keeps as text).
    #[test]
    fn a_saved_lookup_reads_back_as_the_json_it_was_written_as() {
        let dir = tempfile::tempdir().expect("temp dir");
        let host: Arc<dyn Host> = Arc::new(Quiet);
        let database = Arc::new(Database::new(
            Config::new(dir.path().to_path_buf()),
            Arc::clone(&host),
        ));
        let cache = DbLookups::new(database);
        let saved = json!({
            "kind": "opening",
            "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "fetchedAt": 1,
            "moves": [{ "uci": "e2e4", "san": "e4", "white": 100, "draws": 20, "black": 50 }],
        });
        assert_eq!(cache.read("opening:fen").expect("read"), None);
        cache.write("opening:fen", &saved).expect("write");
        assert_eq!(cache.read("opening:fen").expect("read"), Some(saved));
    }
}
