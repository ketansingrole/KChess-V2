//! The Lichess services' storage and credentials, over the core's one database
//! (`store::Database`) and the host's secret store (`Capabilities`).

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::Value;

use super::accounts::{AccountRow, CachedValue, LichessGame, LichessStore};
use super::client::TokenSource;
use super::lookups::{ExplorerLogin, LookupCache};
use crate::capabilities::Capabilities;
use crate::error::{CoreError, Result as CoreResult};
use crate::store::{Database, games, lookups, reviews};

/// `LichessStore` over `kchess.db`.
pub struct DbLichessStore {
    database: Arc<Database>,
}

impl DbLichessStore {
    pub fn new(database: Arc<Database>) -> DbLichessStore {
        DbLichessStore { database }
    }
}

/// A sync's write, unless a logout ended the sync while it waited its turn.
fn unless_cancelled(current: &(dyn Fn() -> bool + Sync)) -> CoreResult<()> {
    if current() {
        Ok(())
    } else {
        Err(CoreError::new("Sync was cancelled."))
    }
}

impl LichessStore for DbLichessStore {
    fn accounts<'a>(&'a self) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<AccountRow>>> {
        Box::pin(async move {
            let data = self
                .database
                .read_async(|ctx| games::load_app_data(ctx.db))
                .await?;
            Ok(data
                .accounts
                .into_iter()
                .map(|row| AccountRow {
                    username: row.username,
                    connected: row.connected,
                    last_synced_at: row.last_synced_at,
                })
                .collect())
        })
    }

    fn dismissed_friends<'a>(
        &'a self,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<String>>> {
        Box::pin(async move {
            self.database
                .read_async(|ctx| games::dismissed_friends(ctx.db))
                .await
        })
    }

    fn pending_game_ids<'a>(
        &'a self,
        account: &'a str,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Vec<String>>> {
        Box::pin(async move {
            let account = account.to_string();
            self.database
                .read_async(move |ctx| games::pending_game_ids(ctx.db, &account))
                .await
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
            // The games and their reviews go in one transaction, once the database admits the
            // write and `current` still holds.
            let account = account.to_string();
            let games = games_json(games)?;
            let reviews = reviews.to_vec();
            self.database
                .write_admitted(|| {
                    unless_cancelled(current)?;
                    Ok(move |ctx: &crate::store::StoreContext| {
                        games::save_games_page(ctx.host, ctx.db, &account, &games, &reviews)
                    })
                })
                .await
        })
    }

    fn save_games_cursor<'a>(
        &'a self,
        account: &'a str,
        synced_at: i64,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            let account = account.to_string();
            self.database
                .write_admitted(|| {
                    unless_cancelled(current)?;
                    Ok(move |ctx: &crate::store::StoreContext| {
                        games::save_games(ctx.host, ctx.db, &account, &[], Some(synced_at))
                            .map(|_| ())
                    })
                })
                .await
        })
    }

    fn save_synced_play_time<'a>(
        &'a self,
        account: &'a str,
        play_time: i64,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            let account = account.to_string();
            self.database
                .write_admitted(|| {
                    unless_cancelled(current)?;
                    Ok(move |ctx: &crate::store::StoreContext| {
                        games::save_synced_play_time(ctx.db, &account, play_time)
                    })
                })
                .await
        })
    }

    fn read_api_cache<'a>(
        &'a self,
        key: &'a str,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Option<CachedValue>>> {
        Box::pin(async move {
            let key = key.to_string();
            let row = self
                .database
                .read_async(move |ctx| games::read_api_cache(ctx.host, ctx.db, &key))
                .await?;
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
            // A write a logout overtook is skipped: the account's cache is gone with it.
            let (key, value) = (key.to_string(), value.clone());
            self.database
                .write_admitted(|| {
                    let wanted = current();
                    Ok(move |ctx: &crate::store::StoreContext| {
                        if wanted {
                            games::write_api_cache(ctx.db, &key, &value)
                        } else {
                            Ok(())
                        }
                    })
                })
                .await
        })
    }

    fn save_login<'a>(
        &'a self,
        username: &'a str,
        ciphertext: &'a str,
        current: &'a (dyn Fn() -> bool + Sync),
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>> {
        Box::pin(async move {
            let (username, ciphertext) = (username.to_string(), ciphertext.to_string());
            self.database
                .write_admitted(|| {
                    if !current() {
                        return Err(CoreError::aborted("Login was cancelled by logout."));
                    }
                    Ok(move |ctx: &crate::store::StoreContext| {
                        games::save_login(ctx.db, &username, &ciphertext).map(|_| ())
                    })
                })
                .await
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
            let reviews = Value::Array(reviews.to_vec());
            let stored = self
                .database
                .write_admitted(|| {
                    unless_cancelled(current)?;
                    Ok(move |ctx: &crate::store::StoreContext| {
                        reviews::write_reviews(ctx.host, ctx.db, &reviews)
                    })
                })
                .await?;
            Ok(stored.as_array().cloned().unwrap_or_default())
        })
    }

    fn receives_challenges<'a>(&'a self) -> BoxFuture<'a, CoreResult<bool>> {
        Box::pin(async move {
            let settings = self
                .database
                .read_async(|ctx| games::get_settings(ctx.db))
                .await?;
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
            let ids = ids.to_vec();
            self.database
                .write_admitted(|| {
                    if !current() {
                        return Err(CoreError::aborted("Sync was cancelled."));
                    }
                    Ok(move |ctx: &crate::store::StoreContext| {
                        reviews::mark_checked(ctx.db, &ids, now as f64)
                    })
                })
                .await
        })
    }
}

/// Games as the rows the store writes.
fn games_json(games: &[LichessGame]) -> CoreResult<Vec<Value>> {
    games
        .iter()
        .map(|game| serde_json::to_value(game).map_err(|cause| CoreError::new(cause.to_string())))
        .collect()
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
            let owner = account.to_string();
            let ciphertext = self
                .database
                .read_async(move |ctx| games::token_ciphertext(ctx.db, &owner))
                .await?;
            let Some(ciphertext) = ciphertext else {
                return Ok(None);
            };
            Ok(Some(self.capabilities.decrypt(&ciphertext).await?))
        })
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
                .read_async(|ctx| lookups::explorer_account(ctx.db))
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
            let key = key.to_string();
            let text = self
                .database
                .read_async(move |ctx| lookups::read(ctx.db, &key))
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
            let (key, value) = (key.to_string(), value.clone());
            self.database
                .write_async(move |ctx| lookups::write(ctx.db, &key, &value))
                .await
                .map(|_| ())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Config, Host, Level};
    use serde_json::json;

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
