//! Chess.com game sync over its public API (`api.chess.com/pub`), which needs no login: a
//! player's monthly archives are read and their games kept as synced games
//! (`store.chesscom.*`, `kchess_domain::chesscom::game_row`).
//!
//! Chess.com asks API clients to send requests one at a time, so every request waits its turn
//! here. A rate-limited request (429) waits as long as the answer asks (capped) and is tried again;
//! only reads are sent, so a retry can never repeat a change. A month that has ended and was read
//! in full is not asked for again, and an unchanged month is answered `304 Not Modified`, so a
//! sync reads one small list and usually one month.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, RETRY_AFTER};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::error::{CoreError, Result};
use crate::host::{Host, Level};
use crate::store::Database;
use crate::store::games::MAX_GAMES;
use crate::tasks::lock;
use kchess_domain::chesscom::{archive_month, game_row};

/// Chess.com's public API.
pub const BASE_URL: &str = "https://api.chess.com";

/// The longest a rate-limited request waits before it is tried again.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);
/// How often a rate-limited request is tried again before the sync gives up.
const RETRIES: usize = 2;
/// A whole request, headers and body.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// The largest answer read: a month of a very busy player is a few megabytes.
const MAX_BODY: usize = 64 * 1024 * 1024;

type Outcome = watch::Receiver<Option<std::result::Result<usize, CoreError>>>;

/// The Chess.com sync of one core.
pub struct Chesscom {
    http: reqwest::Client,
    base: String,
    database: Arc<Database>,
    host: Arc<dyn Host>,
    lifetime: CancellationToken,
    /// One request at a time, as Chess.com asks.
    turn: tokio::sync::Mutex<()>,
    /// The sync running per player (lowercase name), which a second request joins.
    syncs: Mutex<HashMap<String, (u64, Outcome)>>,
    next_sync: AtomicU64,
    /// Bumped when a player is removed, so a sync that started before writes nothing after.
    epochs: Mutex<HashMap<String, u64>>,
}

/// An answer: its status, validators and body.
struct Answer {
    status: StatusCode,
    etag: Option<String>,
    last_modified: Option<String>,
    body: Vec<u8>,
}

/// One month of a player's archive as the store keeps it: its validators for the next request,
/// whether the month is over, and the games read from it.
struct MonthArchive<'a> {
    month: &'a str,
    etag: Option<&'a str>,
    last_modified: Option<&'a str>,
    complete: bool,
    games: &'a [Value],
}

fn unreachable() -> CoreError {
    CoreError::new("Chess.com could not be reached. Try again later.")
}

fn no_such_player(name: &str) -> CoreError {
    CoreError::new(format!("No Chess.com player is called {name}."))
}

/// The current month (UTC) as an archive names it, `2024/03`.
fn this_month() -> String {
    let days = crate::clock::now_ms().div_euclid(86_400_000);
    let (year, month, _) = kchess_domain::calendar::civil_from_days(days).unwrap_or((1970, 1, 1));
    format!("{year:04}/{month:02}")
}

impl Chesscom {
    /// The sync over the core's database; `origin` replaces Chess.com (only the tests set it).
    pub fn new(
        database: Arc<Database>,
        host: Arc<dyn Host>,
        origin: Option<String>,
        lifetime: CancellationToken,
    ) -> Chesscom {
        let http = reqwest::Client::builder()
            .user_agent(format!("{} (game sync)", crate::USER_AGENT))
            .connect_timeout(Duration::from_secs(15))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Chesscom {
            http,
            base: origin.unwrap_or_else(|| BASE_URL.to_string()),
            database,
            host,
            lifetime,
            turn: tokio::sync::Mutex::new(()),
            syncs: Mutex::new(HashMap::new()),
            next_sync: AtomicU64::new(0),
            epochs: Mutex::new(HashMap::new()),
        }
    }

    /// Cancels the requests in flight; called when the core closes.
    pub fn abort(&self) {
        self.lifetime.cancel();
    }

    fn epoch(&self, name: &str) -> u64 {
        lock(&self.epochs)
            .get(&name.to_lowercase())
            .copied()
            .unwrap_or(0)
    }

    /// A GET, in its turn, tried again after a rate limit.
    async fn get(&self, path: &str, validators: (Option<&str>, Option<&str>)) -> Result<Answer> {
        let url = format!("{}{path}", self.base);
        for attempt in 0..=RETRIES {
            let answer = {
                let _turn = self.turn.lock().await;
                let mut request = self.http.get(&url).header("Accept", "application/json");
                if let Some(etag) = validators.0 {
                    request = request.header(IF_NONE_MATCH, etag);
                }
                if let Some(modified) = validators.1 {
                    request = request.header(IF_MODIFIED_SINCE, modified);
                }
                tokio::select! {
                    () = self.lifetime.cancelled() => return Err(CoreError::new("Sync was cancelled.")),
                    answer = Self::read(request) => answer?,
                }
            };
            if answer.status != StatusCode::TOO_MANY_REQUESTS || attempt == RETRIES {
                return Ok(answer);
            }
            let wait = answer
                .etag
                .as_deref()
                .and_then(|seconds| seconds.parse::<u64>().ok())
                .map_or(Duration::from_secs(5), Duration::from_secs)
                .min(MAX_RETRY_WAIT);
            self.host.log(
                Level::Info,
                "chesscom",
                &format!("Rate limited; trying again in {}s", wait.as_secs()),
            );
            tokio::select! {
                () = self.lifetime.cancelled() => return Err(CoreError::new("Sync was cancelled.")),
                () = tokio::time::sleep(wait) => {}
            }
        }
        Err(unreachable())
    }

    /// Sends a request and reads its answer, bounded. A 429's `Retry-After` is carried in `etag`.
    async fn read(request: reqwest::RequestBuilder) -> Result<Answer> {
        let mut response = request.send().await.map_err(|_| unreachable())?;
        let status = response.status();
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value: &reqwest::header::HeaderValue| value.to_str().ok())
                .map(str::to_string)
        };
        let (etag, last_modified) = if status == StatusCode::TOO_MANY_REQUESTS {
            (header(RETRY_AFTER), None)
        } else {
            (header(ETAG), header(LAST_MODIFIED))
        };
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| unreachable())? {
            if body.len() + chunk.len() > MAX_BODY {
                return Err(CoreError::new("Chess.com sent more than KChess reads."));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Answer {
            status,
            etag,
            last_modified,
            body,
        })
    }

    fn json(answer: &Answer) -> Result<Value> {
        serde_json::from_slice(&answer.body)
            .map_err(|_| CoreError::new("Chess.com sent an answer KChess cannot read."))
    }

    /// Whether Chess.com knows the player: their name as Chess.com spells it.
    async fn player(&self, name: &str) -> Result<String> {
        let answer = self
            .get(
                &format!("/pub/player/{}", name.to_lowercase()),
                (None, None),
            )
            .await?;
        match answer.status {
            StatusCode::OK => {
                let profile = Self::json(&answer)?;
                // The profile's `url` ends with the name in its own case; `username` is lowercase.
                let spelled = profile
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(|url| url.trim_end_matches('/').rsplit('/').next())
                    .filter(|spelled| spelled.eq_ignore_ascii_case(name))
                    .map_or_else(|| name.to_string(), str::to_string);
                Ok(spelled)
            }
            StatusCode::NOT_FOUND | StatusCode::GONE => Err(no_such_player(name)),
            _ => Err(unreachable()),
        }
    }

    /// `addChesscomAccount`: adds a player Chess.com knows, then syncs their games.
    pub async fn add(&self, name: &str) -> Result<Value> {
        let spelled = self.player(name).await?;
        self.database
            .call_async("store.chesscom.addAccount", &[json!(spelled)])
            .await?;
        // The player stays added when their games cannot be read yet: Sync tries again.
        self.sync(Some(&spelled)).await
    }

    /// `removeChesscomAccount`: stops a sync in flight from writing, then deletes the player.
    pub async fn remove(&self, name: &str) -> Result<Value> {
        let key = name.to_lowercase();
        *lock(&self.epochs).entry(key.clone()).or_insert(0) += 1;
        lock(&self.syncs).remove(&key);
        self.database
            .call_async("store.chesscom.removeAccount", &[json!(name)])
            .await
    }

    /// `syncChesscomGames`: one player's games, or every player's in turn. Every player still
    /// syncs when one fails; the first failure is reported afterwards.
    pub async fn sync(&self, name: Option<&str>) -> Result<Value> {
        let accounts = self
            .database
            .call_async("store.chesscom.accounts", &[])
            .await?;
        let names: Vec<String> = accounts
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|row| row.get("username").and_then(Value::as_str))
            .filter(|stored| name.is_none_or(|wanted| stored.eq_ignore_ascii_case(wanted)))
            .map(str::to_string)
            .collect();
        let mut first_error = None;
        for name in &names {
            if let Err(cause) = self.sync_one(name).await {
                self.host.log(
                    Level::Warn,
                    "chesscom",
                    &format!("Game sync failed: {name} {}", cause.message),
                );
                first_error.get_or_insert(cause);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => {
                self.database
                    .call_async("store.chesscom.accounts", &[])
                    .await
            }
        }
    }

    /// One player's sync; a second request for the same player joins the running one.
    async fn sync_one(&self, name: &str) -> Result<usize> {
        let key = name.to_lowercase();
        let joined = lock(&self.syncs)
            .get(&key)
            .map(|(_, outcome)| outcome.clone());
        if let Some(mut outcome) = joined {
            loop {
                if let Some(result) = outcome.borrow().clone() {
                    return result;
                }
                if outcome.changed().await.is_err() {
                    return Err(CoreError::new("Sync was cancelled."));
                }
            }
        }
        let generation = self.next_sync.fetch_add(1, Ordering::SeqCst) + 1;
        let (send, receive) = watch::channel(None);
        lock(&self.syncs).insert(key.clone(), (generation, receive));
        // Forgotten however the run ends, a cancelled call included (see `sync_account`).
        let forget = crate::tasks::on_drop(|| {
            let mut syncs = lock(&self.syncs);
            if syncs
                .get(&key)
                .is_some_and(|(current, _)| *current == generation)
            {
                syncs.remove(&key);
            }
        });
        let result = self.run(name).await;
        drop(forget);
        let _ = send.send(Some(result.clone()));
        if let Ok(fetched) = &result {
            self.host.log(
                Level::Info,
                "chesscom",
                &format!("Game sync completed: {name} fetched={fetched}"),
            );
        }
        result
    }

    async fn run(&self, name: &str) -> Result<usize> {
        let epoch = self.epoch(name);
        let current = || self.epoch(name) == epoch && !self.lifetime.is_cancelled();
        let started_at = crate::clock::now_ms();
        let player = name.to_lowercase();
        let list = self
            .get(
                &format!("/pub/player/{player}/games/archives"),
                (None, None),
            )
            .await?;
        match list.status {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND | StatusCode::GONE => return Err(no_such_player(name)),
            _ => return Err(unreachable()),
        }
        let mut months: Vec<String> = Self::json(&list)?
            .get("archives")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .filter_map(archive_month)
            .collect();
        months.sort();
        months.dedup();
        let stored = self
            .database
            .call_async("store.chesscom.archives", &[json!(name)])
            .await?;
        let known: HashMap<String, &Value> = stored
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|row| Some((row.get("month")?.as_str()?.to_string(), row)))
            .collect();
        let now = this_month();
        let mut fetched = 0usize;
        // Newest first: the games kept are the newest, so once there are enough the older
        // months are marked read without being downloaded.
        for month in months.iter().rev() {
            let previous = known.get(month);
            let complete_before = previous
                .and_then(|row| row.get("complete"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if complete_before {
                continue;
            }
            let ended = month.as_str() < now.as_str();
            if fetched >= MAX_GAMES as usize {
                let archive = MonthArchive {
                    month,
                    etag: None,
                    last_modified: None,
                    complete: ended,
                    games: &[],
                };
                self.save(name, archive, &current).await?;
                continue;
            }
            let validator = |key| {
                previous
                    .and_then(|row| row.get(key))
                    .and_then(Value::as_str)
            };
            let answer = self
                .get(
                    &format!("/pub/player/{player}/games/{month}"),
                    (validator("etag"), validator("lastModified")),
                )
                .await?;
            match answer.status {
                StatusCode::NOT_MODIFIED => {
                    if ended {
                        let archive = MonthArchive {
                            month,
                            etag: validator("etag"),
                            last_modified: validator("lastModified"),
                            complete: true,
                            games: &[],
                        };
                        self.save(name, archive, &current).await?;
                    }
                }
                StatusCode::OK => {
                    let games: Vec<Value> = Self::json(&answer)?
                        .get("games")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|game| game_row(name, game))
                        .collect();
                    fetched += games.len();
                    let archive = MonthArchive {
                        month,
                        etag: answer.etag.as_deref(),
                        last_modified: answer.last_modified.as_deref(),
                        complete: ended,
                        games: &games,
                    };
                    self.save(name, archive, &current).await?;
                }
                StatusCode::NOT_FOUND | StatusCode::GONE => {}
                _ => return Err(unreachable()),
            }
        }
        if !current() {
            return Err(CoreError::new("Sync was cancelled."));
        }
        self.database
            .call_async_with("store.chesscom.finishSync", || {
                Ok(vec![json!(name), json!(started_at), json!(current())])
            })
            .await?;
        Ok(fetched)
    }

    async fn save(
        &self,
        name: &str,
        archive: MonthArchive<'_>,
        current: &(dyn Fn() -> bool + Sync),
    ) -> Result<()> {
        if !current() {
            return Err(CoreError::new("Sync was cancelled."));
        }
        let archive = json!({
            "username": name,
            "month": archive.month,
            "etag": archive.etag,
            "lastModified": archive.last_modified,
            "complete": archive.complete,
            "games": archive.games,
        });
        self.database
            .call_async_with("store.chesscom.saveArchive", || {
                Ok(vec![archive.clone(), json!(current())])
            })
            .await
            .map(|_| ())
    }
}
