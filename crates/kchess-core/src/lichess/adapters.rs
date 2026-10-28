//! The Lichess services' storage and credentials, over the core's one database
//! (`store::Database::call`) and the host's secret store (`Capabilities`). Each trait method is a
//! storage method of the same meaning the service calls, so the tables and their writes are
//! unchanged.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::accounts::{AccountRow, CachedValue, LichessGame, LichessStore};
use super::client::TokenSource;
use super::lookups::{ExplorerLogin, LookupCache};
use crate::capabilities::Capabilities;
use crate::error::{CoreError, Result as CoreResult};
use crate::host::Level;
use crate::secrets;
use crate::store::Database;

/// `LichessStore` over `kchess.db`.
pub struct DbLichessStore {
    database: Arc<Database>,
}

impl DbLichessStore {
    pub fn new(database: Arc<Database>) -> DbLichessStore {
        DbLichessStore { database }
    }

    async fn call(&self, method: &str, args: &[Value]) -> CoreResult<Value> {
        self.database.call_async(method, args).await
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
    fn accounts<'a>(&'a self) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<AccountRow>>> {
        Box::pin(async move {
            let data = self.call("store.games.loadData", &[]).await?;
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
        })
    }

    fn dismissed_friends<'a>(
        &'a self,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<String>>> {
        Box::pin(async move {
            Ok(Self::strings(
                self.call("store.games.dismissedFriends", &[]).await?,
            ))
        })
    }

    fn pending_game_ids<'a>(
        &'a self,
        account: &'a str,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<String>>> {
        Box::pin(async move {
            Ok(Self::strings(
                self.call("store.games.pendingGameIds", &[json!(account)])
                    .await?,
            ))
        })
    }

    fn save_games_page<'a>(
        &'a self,
        account: &'a str,
        games: &'a [LichessGame],
        reviews: &'a [Value],
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            // The store writes the games and their reviews in one transaction, and checks `current`
            // once more before it writes.
            self.database
                .call_async_with("store.games.saveGamesPage", || {
                    Ok(vec![
                        json!(account),
                        json!(games),
                        json!(reviews),
                        json!(current()),
                    ])
                })
                .await
                .map(|_| ())
        })
    }

    fn save_games_cursor<'a>(
        &'a self,
        account: &'a str,
        synced_at: i64,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            self.database
                .call_async_with("store.games.saveGames", || {
                    Ok(vec![
                        json!(account),
                        json!([]),
                        json!(synced_at),
                        json!(current()),
                    ])
                })
                .await
                .map(|_| ())
        })
    }

    fn read_api_cache<'a>(
        &'a self,
        key: &'a str,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Option<CachedValue>>> {
        Box::pin(async move {
            let row = self.call("store.games.readApiCache", &[json!(key)]).await?;
            Ok(match row {
                Value::Object(object) => Some(CachedValue {
                    value: object.get("value").cloned().unwrap_or(Value::Null),
                    fetched_at: object.get("fetchedAt").and_then(Value::as_i64).unwrap_or(0),
                }),
                _ => None,
            })
        })
    }

    fn write_api_cache<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            self.database
                .call_async_with("store.games.writeApiCache", || {
                    Ok(vec![json!(key), value.clone(), json!(current())])
                })
                .await
                .map(|_| ())
        })
    }

    fn save_login<'a>(
        &'a self,
        username: &'a str,
        ciphertext: &'a str,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            self.database
                .call_async_with("store.games.saveTokenCiphertext", || {
                    if !current() {
                        return Err(CoreError::aborted("Login was cancelled by logout."));
                    }
                    Ok(vec![json!(username), json!(ciphertext)])
                })
                .await
                .map(|_| ())
        })
    }

    fn save_reviews<'a>(
        &'a self,
        account: &'a str,
        reviews: &'a [Value],
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<Value>>> {
        Box::pin(async move {
            let _ = account;
            if !current() {
                return Err(CoreError::new("Sync was cancelled."));
            }
            // One transaction: a cancelled or failed batch keeps the previous reviews.
            let stored = self
                .database
                .call_async_with("store.reviewStore.writeReviews", || {
                    if !current() {
                        return Err(CoreError::new("Sync was cancelled."));
                    }
                    Ok(vec![Value::Array(reviews.to_vec())])
                })
                .await?;
            Ok(stored.as_array().cloned().unwrap_or_default())
        })
    }

    fn receives_challenges<'a>(&'a self) -> BoxFuture<'a, CoreResult<bool>> {
        Box::pin(async move {
            let settings = self.call("store.games.getSettings", &[]).await?;
            Ok(settings["receiveChallenges"].as_bool().unwrap_or(true))
        })
    }

    fn mark_checked<'a>(
        &'a self,
        ids: &'a [String],
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            let now = super::accounts::now_ms();
            self.database
                .call_async_with("store.reviewStore.markChecked", || {
                    if !current() {
                        return Err(CoreError::aborted("Sync was cancelled."));
                    }
                    Ok(vec![json!(ids), json!(now)])
                })
                .await
                .map(|_| ())
        })
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
                .call_async("store.games.tokenCiphertext", &[json!(account)])
                .await?;
            let Some(ciphertext) = ciphertext.as_str() else {
                return Ok(None);
            };
            let token = self.capabilities.decrypt(ciphertext).await?;
            if self.capabilities.reseals() && !secrets::is_sealed(ciphertext) {
                self.reseal(account, ciphertext, &token).await;
            }
            Ok(Some(token))
        })
    }
}

impl DbTokens {
    /// Reseal a token an earlier release's host sealed (Electron's `safeStorage`) with the
    /// profile's credentials, so every KChess app opens it. This runs when the token is first
    /// needed, as opening it already used the OS credential store; never at startup, where a
    /// keychain prompt once hung the app (`core_startup.rs`). A token that cannot be resealed
    /// keeps working through the host and is tried again next time.
    async fn reseal(&self, account: &str, previous: &str, token: &str) {
        let resealed = async {
            let sealed = self.capabilities.encrypt(token).await?;
            self.database
                .call_async(
                    "store.games.resealTokenCiphertext",
                    &[json!(account), json!(previous), json!(sealed)],
                )
                .await
        }
        .await;
        if let Err(cause) = resealed {
            self.capabilities.log(
                Level::Warn,
                "credentials",
                &format!(
                    "The saved login of {account} was not resealed: {}",
                    cause.message
                ),
            );
        }
    }
}

/// The explorer's login: the account `setupPositionLookup` is set to use, and its token.
pub struct DbExplorerLogin {
    database: Arc<Database>,
    tokens: Arc<dyn TokenSource>,
}

impl DbExplorerLogin {
    pub fn new(database: Arc<Database>, tokens: Arc<dyn TokenSource>) -> DbExplorerLogin {
        DbExplorerLogin { database, tokens }
    }
}

impl ExplorerLogin for DbExplorerLogin {
    fn token(&self) -> BoxFuture<'_, CoreResult<Option<String>>> {
        Box::pin(async move {
            let account = self
                .database
                .call_async("store.setupPositionLookup.explorerAccount", &[])
                .await?;
            match account.as_str() {
                Some(account) => self.tokens.token(account).await,
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
    fn read<'a>(
        &'a self,
        key: &'a str,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Option<Value>>> {
        Box::pin(async move {
            // The store answers the saved text, or null when it is missing or too large to read; text
            // that is not JSON is an unreadable entry, which the explorer treats as missing.
            let text = self
                .database
                .call_async("store.setupPositionLookup.read", &[json!(key)])
                .await?;
            Ok(text
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok()))
        })
    }

    fn write<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            self.database
                .call_async(
                    "store.setupPositionLookup.write",
                    &[json!(key), value.clone()],
                )
                .await
                .map(|_| ())
        })
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
    #[tokio::test]
    async fn a_saved_lookup_reads_back_as_the_json_it_was_written_as() {
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
        assert_eq!(cache.read("opening:fen").await.expect("read"), None);
        cache.write("opening:fen", &saved).await.expect("write");
        assert_eq!(cache.read("opening:fen").await.expect("read"), Some(saved));
    }
}
