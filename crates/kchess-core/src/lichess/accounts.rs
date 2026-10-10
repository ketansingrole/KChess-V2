//! Lichess accounts, profiles and game sync (the account half of `crates/kchess-node/js/lichess.ts`):
//! `asOwner`, `asAnyAccount`, `asAccount`, `cancelAccountSyncs`, `invalidateLogin`, the profile
//! cache, `followedUsers`, `primeProfiles`, `profile`, `playerPerf`, `exportGame`, `recentGames`,
//! `sendMessage`, `crosstable`, `ratingHistory` and `syncGames`.
//!
//! Storage goes through `LichessStore`, which mirrors the store calls `lichess.ts` makes, and
//! tokens through `TokenSource` (`client.rs`). The sync also saves Lichess's analysis of each page
//! (`review_from_lichess`, in `reviews.rs`) through the store's `save_reviews` and `mark_checked`,
//! which default to no-ops until the store wiring implements them. Not in this module: online
//! sessions and challenges (`online.rs`, `challenges.rs`), tournaments, and the puzzle, study,
//! watch and lookup services in their own modules.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::client::{Failure, LichessClient, TokenSource, authorize};
use super::reviews::review_from_lichess;
use crate::capabilities::Capabilities;
use crate::error::{CoreError, Result as CoreResult};
use crate::host::{Host, Level};

/// `MAX_GAMES` of `store.ts`: the most games one sync keeps walking back through.
pub const MAX_GAMES: usize = 5000;
/// Lichess returns at most this many games per page of a sync.
const SYNC_PAGE: usize = 1000;
/// Overlap of the incremental sync window, for clock skew.
const SYNC_OVERLAP_MS: i64 = 60_000;
/// Profile data changes slowly; five minutes of reuse saves a round trip per visit.
const PROFILE_TTL: Duration = Duration::from_secs(5 * 60);
const PROFILE_CACHE_MAX: usize = 100;
/// `recentGames`: how many games a recent-games request returns.
const RECENT_GAMES: usize = 10;
const RATED_GAMES: usize = 200;
/// `exportGame`: a longer PGN is refused (UTF-16 units, as `pgn.length` counts them).
const MAX_PGN_UNITS: usize = 200_000;

/// The account rows the store keeps (`loadData().accounts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub username: String,
    pub connected: bool,
    pub last_synced_at: Option<i64>,
}

/// A value of the persisted API cache with the time it was fetched (`readApiCache`).
#[derive(Clone, Debug, PartialEq)]
pub struct CachedValue {
    pub value: Value,
    pub fetched_at: i64,
}

/// What `asAccount` answers: the result, or that the account must connect again (no login, or
/// one Lichess refuses).
#[derive(Debug, PartialEq)]
pub enum Reply<T> {
    Done(T),
    NeedsReconnect,
}

/// The storage `lichess.ts` reads and writes, behind a trait so the wiring can plug the store in.
/// Every method returns the TypeScript result; `current` is the caller's "still signed in"
/// check, read again by the store right before it writes.
pub trait LichessStore: Send + Sync {
    /// `loadData().accounts`.
    fn accounts(&self) -> CoreResult<Vec<AccountRow>>;
    /// Lowercase usernames the user dismissed as friends (`dismissedFriends`).
    fn dismissed_friends(&self) -> CoreResult<Vec<String>>;
    /// Games of `account` that were unfinished at the last sync (`pendingGameIds`).
    fn pending_game_ids(&self, account: &str) -> CoreResult<Vec<String>>;
    /// Saves a page of games and the reviews Lichess returned with them, in one transaction, unless
    /// `current` turned false (`saveGamesPage`).
    fn save_games_page(
        &self,
        account: &str,
        games: &[LichessGame],
        reviews: &[Value],
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<()>;
    /// Moves the sync cursor of `account` to `synced_at` (`saveGames(username, [], startedAt)`).
    fn save_games_cursor(
        &self,
        account: &str,
        synced_at: i64,
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<()>;
    /// `readApiCache(key)`.
    fn read_api_cache(&self, key: &str) -> CoreResult<Option<CachedValue>>;
    /// `writeApiCache(key, value, current)`.
    fn write_api_cache(
        &self,
        key: &str,
        value: &Value,
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<()>;
    /// `saveLogin`'s write: the encrypted token and the connected account, together.
    fn save_login(&self, username: &str, ciphertext: &str) -> CoreResult<()>;
    /// Reviews without a page of games (the review queue's fetch): written, and the stored reviews
    /// returned (`writeReview` for each).
    fn save_reviews(
        &self,
        account: &str,
        reviews: &[Value],
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<Vec<Value>>;
    /// Games Lichess was asked about, so they are not asked again soon (`markChecked`).
    fn mark_checked(&self, ids: &[String]) -> CoreResult<()>;
}

/// A game as the store keeps it and the UI shows it (`LichessGame` of `contracts/types.ts`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LichessGame {
    pub id: String,
    pub account: String,
    pub created_at: i64,
    pub last_move_at: i64,
    pub rated: bool,
    pub speed: String,
    pub perf: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub winner: Option<String>,
    pub color: String,
    pub opponent: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub opponent_rating: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub player_rating: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rating_diff: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub opening: Option<String>,
    pub moves: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pgn: Option<String>,
}

/// The parts of a `GameJson` (`@lichess-org/types`) the sync reads.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameJson {
    pub id: String,
    pub created_at: i64,
    pub last_move_at: i64,
    #[serde(default)]
    pub rated: bool,
    #[serde(default)]
    pub speed: String,
    #[serde(default)]
    pub perf: String,
    pub status: String,
    #[serde(default)]
    pub winner: Option<String>,
    pub players: Players,
    #[serde(default)]
    pub opening: Option<Opening>,
    #[serde(default)]
    pub moves: Option<String>,
    #[serde(default)]
    pub pgn: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Players {
    pub white: Player,
    pub black: Player,
}

/// A side of a game: a Lichess user, or the Stockfish level an AI played.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    #[serde(default)]
    pub user: Option<PlayerUser>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ai_level: Option<u32>,
    #[serde(default)]
    pub rating: Option<i64>,
    #[serde(default)]
    pub rating_diff: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PlayerUser {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Opening {
    pub name: String,
}

/// `playerName`: a user's name, the AI level, or "Anonymous".
fn player_name(player: &Player) -> String {
    if let Some(user) = &player.user {
        return user
            .name
            .clone()
            .or_else(|| player.name.clone())
            .unwrap_or_else(|| "Anonymous".to_string());
    }
    match player.ai_level {
        Some(level) => format!("Stockfish level {level}"),
        None => "Anonymous".to_string(),
    }
}

/// `normalizeGame`: the game seen from `account`'s side.
pub fn normalize_game(raw: &GameJson, account: &str) -> LichessGame {
    let white = player_name(&raw.players.white);
    let black = player_name(&raw.players.black);
    let color = if white.to_lowercase() == account.to_lowercase() {
        "white"
    } else {
        "black"
    };
    let (player, opponent, opponent_name) = if color == "white" {
        (&raw.players.white, &raw.players.black, black)
    } else {
        (&raw.players.black, &raw.players.white, white)
    };
    LichessGame {
        id: raw.id.clone(),
        account: account.to_string(),
        created_at: raw.created_at,
        last_move_at: raw.last_move_at,
        rated: raw.rated,
        speed: raw.speed.clone(),
        perf: raw.perf.clone(),
        status: raw.status.clone(),
        winner: raw.winner.clone(),
        color: color.to_string(),
        opponent: opponent_name,
        opponent_rating: opponent.rating,
        player_rating: player.rating,
        rating_diff: player.rating_diff,
        opening: raw.opening.as_ref().map(|opening| opening.name.clone()),
        moves: raw.moves.clone().unwrap_or_default(),
        pgn: raw.pgn.clone(),
    }
}

/// `Date.now()`.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Percent-encodes one path segment (a username), as openapi-fetch does for `{username}`.
pub fn path_segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn profile_key(username: &str, kind: &str) -> String {
    format!("{}:{kind}", username.to_lowercase())
}

/// The Lichess service: one per process. It holds the client, the storage and token traits,
/// the login and account epochs (a logout or removal ends work started under an older epoch),
/// the profile cache and the running syncs.
pub struct Lichess {
    client: LichessClient,
    capabilities: Arc<Capabilities>,
    store: Arc<dyn LichessStore>,
    tokens: Arc<dyn TokenSource>,
    host: Arc<dyn Host>,
    lifetime: CancellationToken,
    login_epoch: AtomicU64,
    account_epochs: Mutex<HashMap<String, u64>>,
    profiles: Mutex<HashMap<String, (Instant, Value)>>,
    last_following: Mutex<HashMap<String, Value>>,
    syncs: Mutex<HashMap<String, (u64, SyncOutcome)>>,
    next_sync: AtomicU64,
}

type SyncOutcome = watch::Receiver<Option<Result<usize, CoreError>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl Lichess {
    pub fn new(
        client: LichessClient,
        capabilities: Arc<Capabilities>,
        store: Arc<dyn LichessStore>,
        tokens: Arc<dyn TokenSource>,
        host: Arc<dyn Host>,
        lifetime: CancellationToken,
    ) -> Lichess {
        Lichess {
            client,
            capabilities,
            store,
            tokens,
            host,
            lifetime,
            login_epoch: AtomicU64::new(0),
            account_epochs: Mutex::new(HashMap::new()),
            profiles: Mutex::new(HashMap::new()),
            last_following: Mutex::new(HashMap::new()),
            syncs: Mutex::new(HashMap::new()),
            next_sync: AtomicU64::new(0),
        }
    }

    pub fn client(&self) -> &LichessClient {
        &self.client
    }

    pub fn capabilities(&self) -> &Arc<Capabilities> {
        &self.capabilities
    }

    pub fn store(&self) -> &Arc<dyn LichessStore> {
        &self.store
    }

    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    pub fn lifetime(&self) -> &CancellationToken {
        &self.lifetime
    }

    pub fn login_epoch(&self) -> u64 {
        self.login_epoch.load(Ordering::SeqCst)
    }

    pub(crate) fn bump_login_epoch(&self) {
        self.login_epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// The epoch of an account (lowercase); work from an older epoch is discarded.
    pub fn account_epoch(&self, account: &str) -> u64 {
        lock(&self.account_epochs)
            .get(&account.to_lowercase())
            .copied()
            .unwrap_or(0)
    }

    /// `invalidateLogin`: a logout. The login epoch moves, the followed-users cache is dropped
    /// and the syncs of these accounts are cancelled.
    pub fn invalidate_login(&self, accounts: &[String]) {
        self.bump_login_epoch();
        lock(&self.last_following).clear();
        self.cancel_account_syncs(accounts);
    }

    /// `cancelAccountSyncs`: stops the running syncs of these accounts before they save anything
    /// more, without ending the login.
    pub fn cancel_account_syncs(&self, accounts: &[String]) {
        for name in accounts {
            let key = name.to_lowercase();
            {
                let mut epochs = lock(&self.account_epochs);
                let next = epochs.get(&key).copied().unwrap_or(0) + 1;
                epochs.insert(key.clone(), next);
            }
            self.forget_profile(name);
            lock(&self.syncs).remove(&key);
        }
    }

    /// `resetLichess`: ends every login epoch and running sync, and drops the in-memory profiles.
    pub fn reset(&self) {
        self.login_epoch.fetch_add(1, Ordering::SeqCst);
        let running: Vec<String> = lock(&self.syncs).keys().cloned().collect();
        self.cancel_account_syncs(&running);
        lock(&self.profiles).clear();
        lock(&self.last_following).clear();
    }

    /// `forgetProfile`: the in-memory copies of an account's profile and rating history.
    pub fn forget_profile(&self, username: &str) {
        let mut profiles = lock(&self.profiles);
        profiles.remove(&profile_key(username, "profile"));
        profiles.remove(&profile_key(username, "rating"));
    }

    /// `asOwner`: run `call` with the account's own token when it has one. A rejected token
    /// falls back to the anonymous request, so a sync is never worse than an anonymous one.
    /// `call` receives the `Authorization` header value, or None.
    pub async fn as_owner<T, F, Fut>(&self, account: &str, call: F) -> Result<T, Failure>
    where
        F: Fn(Option<String>) -> Fut,
        Fut: Future<Output = Result<T, Failure>>,
    {
        let token = match self.tokens.token(account).await {
            Ok(token) => token,
            Err(cause) => {
                self.host.log(
                    Level::Debug,
                    "lichess",
                    &format!("Stored login is unavailable: {account} {}", cause.message),
                );
                None
            }
        };
        let Some(token) = token else {
            return call(None).await;
        };
        match call(Some(authorize(&token))).await {
            Err(failure) if failure.status() == Some(401) => call(None).await,
            answer => answer,
        }
    }

    /// `asAnyAccount`: some public lookups answer only to a signed-in app. Use a connected login,
    /// preferring `account`; a refused login falls back to asking anonymously.
    pub async fn as_any_account<T, F, Fut>(
        &self,
        account: Option<&str>,
        call: F,
    ) -> Result<T, Failure>
    where
        F: Fn(Option<String>) -> Fut,
        Fut: Future<Output = Result<T, Failure>>,
    {
        let connected: Vec<String> = match self.store.accounts() {
            Ok(rows) => rows
                .into_iter()
                .filter(|row| row.connected)
                .map(|row| row.username)
                .collect(),
            Err(cause) => {
                self.host.log(
                    Level::Debug,
                    "lichess",
                    &format!("Stored accounts are unavailable: {}", cause.message),
                );
                Vec::new()
            }
        };
        let owner = account
            .and_then(|wanted| {
                connected
                    .iter()
                    .find(|name| name.to_lowercase() == wanted.to_lowercase())
                    .cloned()
            })
            .or_else(|| connected.first().cloned());
        match owner {
            Some(owner) => self.as_owner(&owner, call).await,
            None => call(None).await,
        }
    }

    /// `asAccount`: run `work` with the account's token. A missing login, or one Lichess refuses
    /// (401/403), answers "reconnect" instead of failing.
    pub async fn as_account<T, F, Fut>(
        &self,
        account: &str,
        kind: &str,
        work: F,
    ) -> CoreResult<Reply<T>>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<T, Failure>>,
    {
        let Some(token) = self.tokens.token(account).await? else {
            return Ok(Reply::NeedsReconnect);
        };
        match self
            .client
            .policy()
            .with_usage(account, kind, work(token))
            .await
        {
            Ok(value) => Ok(Reply::Done(value)),
            Err(failure) if matches!(failure.status(), Some(401) | Some(403)) => {
                Ok(Reply::NeedsReconnect)
            }
            Err(failure) => Err(failure.into()),
        }
    }

    /// `cachedFetch`: memory first (five minutes), then the network. The result is persisted;
    /// when Lichess is unreachable the last persisted copy is served instead of an error.
    async fn cached_fetch<F, Fut>(&self, key: &str, account: &str, load: F) -> CoreResult<Value>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, Failure>>,
    {
        if let Some(value) = self.memory_value(key) {
            return Ok(value);
        }
        let epoch = self.account_epoch(account);
        match load().await {
            Ok(value) => {
                let current = || self.account_epoch(account) == epoch;
                if !current() {
                    return Err(CoreError::new("Account was logged out."));
                }
                if let Err(cause) = self.store.write_api_cache(key, &value, &current) {
                    self.host.log(
                        Level::Warn,
                        "lichess-cache",
                        &format!("Profile cache write failed: {key} {}", cause.message),
                    );
                }
                self.remember(key, &value);
                Ok(value)
            }
            Err(failure) if failure.status() == Some(404) => Err(failure.into()),
            Err(failure) => {
                let stale = match self.store.read_api_cache(key) {
                    Ok(stale) => stale,
                    Err(cause) => {
                        self.host.log(
                            Level::Debug,
                            "lichess",
                            &format!("API cache read failed: {key} {}", cause.message),
                        );
                        None
                    }
                };
                match stale {
                    Some(stale) => Ok(stale.value),
                    None => Err(failure.into()),
                }
            }
        }
    }

    fn memory_value(&self, key: &str) -> Option<Value> {
        let mut profiles = lock(&self.profiles);
        let fresh = profiles
            .get(key)
            .filter(|(stored, _)| stored.elapsed() < PROFILE_TTL)
            .map(|(_, value)| value.clone());
        if fresh.is_none() {
            profiles.remove(key);
        }
        fresh
    }

    fn remember(&self, key: &str, value: &Value) {
        let mut profiles = lock(&self.profiles);
        if profiles.len() >= PROFILE_CACHE_MAX && !profiles.contains_key(key) {
            // Evict the oldest entry: the cache is a convenience, not a record.
            if let Some(oldest) = profiles
                .iter()
                .min_by_key(|(_, (stored, _))| *stored)
                .map(|(key, _)| key.clone())
            {
                profiles.remove(&oldest);
            }
        }
        profiles.insert(key.to_string(), (Instant::now(), value.clone()));
    }

    /// `cachedProfile`: the last persisted profile and rating history, with no network.
    pub fn cached_profile(&self, username: &str) -> CoreResult<CachedProfile> {
        let profile = self.read_cache_or_log(&profile_key(username, "profile"), username);
        let rating = self.read_cache_or_log(&profile_key(username, "rating"), username);
        Ok(CachedProfile {
            profile_fetched_at: profile.as_ref().map(|hit| hit.fetched_at),
            profile: profile.map(|hit| hit.value),
            rating_history: rating.map(|hit| hit.value),
        })
    }

    fn read_cache_or_log(&self, key: &str, username: &str) -> Option<CachedValue> {
        match self.store.read_api_cache(key) {
            Ok(hit) => hit,
            Err(cause) => {
                self.host.log(
                    Level::Debug,
                    "lichess",
                    &format!("Cached profile read failed: {username} {}", cause.message),
                );
                None
            }
        }
    }

    /// `followedUsers`: everyone the connected accounts follow. Needs `follow:read`; accounts
    /// connected before it was requested come back as a problem to reconnect.
    pub async fn followed_users(&self) -> CoreResult<FollowingReport> {
        let rows = self.store.accounts()?;
        let dismissed = self.store.dismissed_friends()?;
        let added: Vec<String> = rows.iter().map(|row| row.username.to_lowercase()).collect();
        let epoch = self.login_epoch();
        let mut by_name: BTreeMap<String, FollowedUser> = BTreeMap::new();
        let mut following: HashMap<String, Value> = HashMap::new();
        let mut problems = Vec::new();
        for row in rows.iter().filter(|row| row.connected) {
            let token = match self.tokens.token(&row.username).await? {
                Some(token) => token,
                None => {
                    problems.push(FollowingProblem {
                        account: row.username.clone(),
                        message: "Its Lichess login is unavailable.".to_string(),
                        needs_reconnect: true,
                    });
                    continue;
                }
            };
            let mut lines = Vec::new();
            let outcome = self
                .client
                .policy()
                .with_usage(&row.username, "profile", async {
                    self.client
                        .ndjson(
                            reqwest::Method::GET,
                            "/api/rel/following",
                            &[],
                            None,
                            Some(&authorize(&token)),
                            &self.lifetime,
                            |line| {
                                lines.push(line.to_string());
                                Ok(())
                            },
                        )
                        .await
                })
                .await;
            match outcome {
                Ok(()) => {
                    for line in &lines {
                        let user: Value = serde_json::from_str(line)
                            .map_err(|cause| CoreError::new(cause.to_string()))?;
                        let name = user["username"].as_str().unwrap_or_default().to_string();
                        let key = name.to_lowercase();
                        let entry = by_name.entry(key.clone()).or_insert_with(|| FollowedUser {
                            username: name.clone(),
                            title: user["title"].as_str().map(str::to_string),
                            ratings: FollowedRatings {
                                bullet: user["perfs"]["bullet"]["rating"].as_i64(),
                                blitz: user["perfs"]["blitz"]["rating"].as_i64(),
                                rapid: user["perfs"]["rapid"]["rating"].as_i64(),
                                classical: user["perfs"]["classical"]["rating"].as_i64(),
                            },
                            followed_by: Vec::new(),
                            already_added: added.contains(&key),
                            dismissed: dismissed.contains(&key),
                        });
                        entry.followed_by.push(row.username.clone());
                        following.insert(key, user);
                    }
                }
                Err(failure) => {
                    let denied = matches!(failure.status(), Some(401) | Some(403));
                    self.host.log(
                        Level::Warn,
                        "lichess",
                        &format!(
                            "Could not load followed users: {} {}",
                            row.username,
                            failure.message()
                        ),
                    );
                    problems.push(FollowingProblem {
                        account: row.username.clone(),
                        message: if denied {
                            "Lichess needs your permission to read who this account follows."
                                .to_string()
                        } else {
                            failure.message()
                        },
                        needs_reconnect: denied,
                    });
                }
            }
        }
        // A logout during the download: this list belonged to an account that is gone.
        if epoch != self.login_epoch() {
            return Err(CoreError::new("Login was cancelled by logout."));
        }
        {
            let mut last = lock(&self.last_following);
            last.clear();
            last.extend(following);
        }
        let mut users: Vec<FollowedUser> = by_name.into_values().collect();
        users.sort_by(|a, b| a.username.to_lowercase().cmp(&b.username.to_lowercase()));
        users.truncate(1000);
        Ok(FollowingReport { users, problems })
    }

    /// `primeProfiles`: saves the profiles `followedUsers` just fetched for the players added.
    pub fn prime_profiles(&self, usernames: &[String]) -> CoreResult<()> {
        let last = lock(&self.last_following).clone();
        for name in usernames {
            let Some(user) = last.get(&name.to_lowercase()) else {
                continue;
            };
            let key = profile_key(name, "profile");
            let always = || true;
            if let Err(cause) = self.store.write_api_cache(&key, user, &always) {
                self.host.log(
                    Level::Debug,
                    "lichess",
                    &format!("Profile prime write failed: {name} {}", cause.message),
                );
            }
        }
        Ok(())
    }

    /// `profile`: a Lichess user as Lichess sent it, cached. A missing user names the user.
    pub async fn profile(&self, username: &str) -> CoreResult<Value> {
        let key = profile_key(username, "profile");
        self.cached_fetch(&key, username, || async {
            self.client
                .policy()
                .with_usage(username, "profile", async {
                    self.client
                        .get_json::<Value>(
                            &format!("/api/user/{}", path_segment(username)),
                            &[],
                            None,
                            &self.lifetime,
                        )
                        .await
                        .map_err(|failure| no_such_user(username, failure))
                })
                .await
        })
        .await
    }

    /// `playerPerf`: one player's record in one rating category (public).
    pub async fn player_perf(&self, username: &str, perf: &str) -> CoreResult<PerfStats> {
        let raw: PerfResponse = self
            .client
            .policy()
            .with_usage("", "profile", async {
                self.client
                    .get_json(
                        &format!(
                            "/api/user/{}/perf/{}",
                            path_segment(username),
                            path_segment(perf)
                        ),
                        &[],
                        None,
                        &self.lifetime,
                    )
                    .await
                    .map_err(|failure| no_such_user(username, failure))
            })
            .await?;
        Ok(perf_stats(perf, raw))
    }

    /// `exportGame`: a finished game as PGN with clocks, for the analysis board.
    pub async fn export_game(&self, id: &str) -> CoreResult<String> {
        let pgn = self
            .client
            .policy()
            .with_usage("", "games", async {
                self.client
                    .get_text(
                        &format!("/game/export/{}", path_segment(id)),
                        &[
                            ("clocks", "true".to_string()),
                            ("evals", "false".to_string()),
                            ("opening", "true".to_string()),
                        ],
                        "application/x-chess-pgn",
                        None,
                        &self.lifetime,
                    )
                    .await
            })
            .await?;
        if pgn.encode_utf16().count() > MAX_PGN_UNITS {
            return Err(CoreError::new("That game is too long to open."));
        }
        Ok(pgn)
    }

    /// `recentGames`: a player's latest public games, newest first, seen from their side. `rated`
    /// asks for more of their rated games, to chart their ratings.
    pub async fn recent_games(&self, username: &str, rated: bool) -> CoreResult<Vec<LichessGame>> {
        let max = if rated { RATED_GAMES } else { RECENT_GAMES };
        self.client
            .policy()
            .with_usage("", "profile", async {
                self.as_any_account(None, |auth| async move {
                    let mut query = vec![
                        ("max", max.to_string()),
                        ("moves", "false".to_string()),
                        ("opening", (!rated).to_string()),
                        ("ongoing", "false".to_string()),
                    ];
                    if rated {
                        query.push(("rated", "true".to_string()));
                    }
                    let mut lines = Vec::new();
                    self.client
                        .ndjson(
                            reqwest::Method::GET,
                            &format!("/api/games/user/{}", path_segment(username)),
                            &query,
                            None,
                            auth.as_deref(),
                            &self.lifetime,
                            |line| {
                                lines.push(line.to_string());
                                Ok(())
                            },
                        )
                        .await?;
                    Ok::<Vec<String>, Failure>(lines)
                })
                .await
            })
            .await
            .map_err(CoreError::from)
            .and_then(|lines| {
                let mut games = Vec::new();
                for line in lines {
                    if games.len() >= max {
                        break;
                    }
                    let raw: GameJson = parse_game(&line)?;
                    games.push(normalize_game(&raw, username));
                }
                Ok(games)
            })
    }

    /// `sendMessage`: a private message from `account`. A login made before messaging was
    /// requested answers "reconnect".
    pub async fn send_message(
        &self,
        account: &str,
        username: &str,
        text: &str,
    ) -> CoreResult<Reply<()>> {
        self.as_account(account, "profile", |token| async move {
            self.client
                .post_form::<Value>(
                    &format!("/inbox/{}", path_segment(username)),
                    &[("text", text)],
                    Some(&authorize(&token)),
                    &self.lifetime,
                )
                .await
                .map(|_| ())
        })
        .await
    }

    /// `crosstable`: lifetime score between two players, and their current matchup if playing.
    pub async fn crosstable(&self, a: &str, b: &str) -> CoreResult<Crosstable> {
        let raw: CrosstableRaw = self
            .client
            .policy()
            .with_usage("", "profile", async {
                self.client
                    .get_json(
                        &format!("/api/crosstable/{}/{}", path_segment(a), path_segment(b)),
                        &[("matchup", "true".to_string())],
                        None,
                        &self.lifetime,
                    )
                    .await
            })
            .await?;
        Ok(Crosstable {
            users: lower_keys(raw.users),
            nb_games: raw.nb_games,
            matchup: raw.matchup.map(|matchup| CrosstableMatchup {
                users: lower_keys(matchup.users),
                nb_games: matchup.nb_games,
            }),
        })
    }

    /// `ratingHistory`: the rating history of a player, cached.
    pub async fn rating_history(&self, username: &str) -> CoreResult<Value> {
        let key = profile_key(username, "rating");
        self.cached_fetch(&key, username, || async {
            self.client
                .policy()
                .with_usage(username, "profile", async {
                    self.as_any_account(Some(username), |auth| async move {
                        self.client
                            .get_json::<Value>(
                                &format!("/api/user/{}/rating-history", path_segment(username)),
                                &[],
                                auth.as_deref(),
                                &self.lifetime,
                            )
                            .await
                    })
                    .await
                    .map_err(|failure| no_such_user(username, failure))
                })
                .await
        })
        .await
    }

    /// `syncGames`: syncs the connected accounts (or one of them), in turn. Every account still
    /// syncs when one fails; the first failure is reported afterwards.
    pub async fn sync_games(&self, username: Option<&str>) -> CoreResult<()> {
        let rows = self.store.accounts()?;
        let mut first_error: Option<CoreError> = None;
        for row in rows.iter().filter(|row| {
            username.is_none_or(|wanted| row.username.to_lowercase() == wanted.to_lowercase())
        }) {
            if let Err(cause) = self.sync_account(row).await {
                self.host.log(
                    Level::Warn,
                    "lichess",
                    &format!("Game sync failed: {} {}", row.username, cause.message),
                );
                first_error.get_or_insert(cause);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// One sync per account at a time: a second request joins the running one.
    async fn sync_account(&self, row: &AccountRow) -> CoreResult<usize> {
        let key = row.username.to_lowercase();
        let joined = lock(&self.syncs)
            .get(&key)
            .map(|(_, outcome)| outcome.clone());
        if let Some(outcome) = joined {
            return wait_for(outcome).await;
        }
        let generation = self.next_sync.fetch_add(1, Ordering::SeqCst) + 1;
        let (send, receive) = watch::channel(None);
        lock(&self.syncs).insert(key.clone(), (generation, receive));
        let result = self.run_sync(row).await;
        {
            let mut syncs = lock(&self.syncs);
            if syncs
                .get(&key)
                .is_some_and(|(current, _)| *current == generation)
            {
                syncs.remove(&key);
            }
        }
        let shared = result.clone();
        let _ = send.send(Some(shared));
        if let Ok(fetched) = &result {
            self.host.log(
                Level::Info,
                "lichess",
                &format!("Game sync completed: {} fetched={fetched}", row.username),
            );
        }
        result
    }

    async fn run_sync(&self, row: &AccountRow) -> CoreResult<usize> {
        let name = row.username.as_str();
        let epoch = self.account_epoch(name);
        let current = || self.account_epoch(name) == epoch;
        let started_at = now_ms();
        self.client
            .policy()
            .with_usage(name, "games", async {
                self.refresh_pending_games(name, &current).await?;
                // Incremental sync: Lichess filters by `since` (ms), overlapping 60 s for skew.
                // It returns newest first and caps each page, so the walk pages backwards with
                // `until`; a long gap between syncs must not silently lose games.
                let since = row.last_synced_at.map(|at| at - SYNC_OVERLAP_MS);
                let mut fetched = 0usize;
                let mut until: Option<i64> = None;
                while fetched < MAX_GAMES {
                    let (games, reviews, settled) =
                        self.fetch_games_page(name, since, until).await?;
                    if !current() {
                        return Err(sync_cancelled());
                    }
                    if !games.is_empty() {
                        self.store
                            .save_games_page(name, &games, &reviews, &current)?;
                    }
                    if !current() {
                        return Err(sync_cancelled());
                    }
                    self.store.mark_checked(&settled)?;
                    fetched += games.len();
                    if games.len() < SYNC_PAGE {
                        break;
                    }
                    let oldest = games.iter().map(|game| game.created_at).min().unwrap_or(0);
                    if until.is_some_and(|bound| oldest >= bound) {
                        break;
                    }
                    until = Some(oldest);
                }
                self.store.save_games_cursor(name, started_at, &current)?;
                self.forget_profile(name);
                Ok::<usize, CoreError>(fetched)
            })
            .await
    }

    /// One page of the sync: its games, Lichess's reviews of them, and the finished games it
    /// has settled (reviewed, or old enough that Lichess has had its chance to analyse them).
    async fn fetch_games_page(
        &self,
        account: &str,
        since: Option<i64>,
        until: Option<i64>,
    ) -> CoreResult<(Vec<LichessGame>, Vec<Value>, Vec<String>)> {
        let mut query = vec![
            ("max", SYNC_PAGE.to_string()),
            ("ongoing", "true".to_string()),
        ];
        if let Some(since) = since {
            query.push(("since", since.to_string()));
        }
        if let Some(until) = until {
            query.push(("until", until.to_string()));
        }
        for flag in [
            "opening",
            "moves",
            "pgnInJson",
            "clocks",
            "evals",
            "accuracy",
        ] {
            query.push((flag, "true".to_string()));
        }
        let path = format!("/api/games/user/{}", path_segment(account));
        let lines = self
            .collect_lines(account, reqwest::Method::GET, &path, &query, None)
            .await?;
        let mut games = Vec::with_capacity(lines.len());
        let mut reviews = Vec::new();
        let mut settled = Vec::new();
        let now = now_ms();
        for line in &lines {
            let raw = parse_game(line)?;
            games.push(normalize_game(&raw, account));
            let value: Value = serde_json::from_str(line).unwrap_or(Value::Null);
            let review = review_from_lichess(&value);
            if !is_game_in_progress(&value)
                && (review.is_some() || now - raw.last_move_at > ANALYSIS_SETTLED_MS)
            {
                settled.push(raw.id.clone());
            }
            reviews.extend(review);
        }
        Ok((games, reviews, settled))
    }

    /// Unfinished games are re-fetched by ID, even when their creation predates the next window.
    async fn refresh_pending_games(
        &self,
        account: &str,
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<()> {
        let ids = self.store.pending_game_ids(account)?;
        let query: Vec<(&str, String)> = [
            "moves",
            "pgnInJson",
            "clocks",
            "opening",
            "evals",
            "accuracy",
        ]
        .into_iter()
        .map(|flag| (flag, "true".to_string()))
        .collect();
        for chunk in ids.chunks(300) {
            let requested: std::collections::HashSet<&str> =
                chunk.iter().map(String::as_str).collect();
            let lines = self
                .collect_lines(
                    account,
                    reqwest::Method::POST,
                    "/api/games/export/_ids",
                    &query,
                    Some(chunk.join(",")),
                )
                .await?;
            let mut games = Vec::new();
            let mut reviews = Vec::new();
            for line in &lines {
                let raw = parse_game(line)?;
                if requested.contains(raw.id.as_str()) {
                    games.push(normalize_game(&raw, account));
                    let value: Value = serde_json::from_str(line).unwrap_or(Value::Null);
                    reviews.extend(review_from_lichess(&value));
                }
            }
            // A missing ID stays pending; a failed request never moves the cursor.
            self.store
                .save_games_page(account, &games, &reviews, current)?;
        }
        Ok(())
    }

    /// The lines of an NDJSON response for `account`, read with its own token when it has one.
    async fn collect_lines(
        &self,
        account: &str,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<String>,
    ) -> CoreResult<Vec<String>> {
        let lines = self
            .client
            .policy()
            .with_usage(account, "games", async {
                self.as_owner(account, |auth| {
                    let method = method.clone();
                    let body = body.clone();
                    async move {
                        let mut lines = Vec::new();
                        self.client
                            .ndjson(
                                method,
                                path,
                                query,
                                body,
                                auth.as_deref(),
                                &self.lifetime,
                                |line| {
                                    lines.push(line.to_string());
                                    Ok(())
                                },
                            )
                            .await?;
                        Ok::<Vec<String>, Failure>(lines)
                    }
                })
                .await
            })
            .await?;
        Ok(lines)
    }
}

fn sync_cancelled() -> CoreError {
    CoreError::new("Sync was cancelled.")
}

/// Waits for a running sync (joined by a second request) to finish, and shares its result.
async fn wait_for(mut outcome: SyncOutcome) -> CoreResult<usize> {
    loop {
        if let Some(result) = outcome.borrow().clone() {
            return result;
        }
        if outcome.changed().await.is_err() {
            return Err(CoreError::new("Sync was cancelled."));
        }
    }
}

fn parse_game(line: &str) -> CoreResult<GameJson> {
    serde_json::from_str(line)
        .map_err(|cause| CoreError::new(format!("Lichess sent an unreadable game record: {cause}")))
}

/// Games that ended this long ago have had their chance to be analysed on Lichess.
const ANALYSIS_SETTLED_MS: i64 = 86_400_000;

/// `isGameInProgress(raw.status)` for a game record.
fn is_game_in_progress(raw: &Value) -> bool {
    let status = raw.get("status").cloned().unwrap_or(Value::Null);
    kchess_domain::records::call("isGameInProgress", &[status])
        .and_then(Result::ok)
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

/// `noSuchUser`: Lichess's generic 404 becomes a message that names the user.
fn no_such_user(username: &str, failure: Failure) -> Failure {
    match failure {
        Failure::Lichess(error) if error.status == 404 => {
            Failure::Lichess(super::client::LichessError {
                status: 404,
                endpoint: error.endpoint,
                detail: format!("no such Lichess user \"{username}\""),
            })
        }
        other => other,
    }
}

fn lower_keys<V>(map: BTreeMap<String, V>) -> BTreeMap<String, V> {
    map.into_iter()
        .map(|(name, value)| (name.to_lowercase(), value))
        .collect()
}

/// A profile as the cache holds it, with when it was fetched.
#[derive(Clone, Debug, PartialEq)]
pub struct CachedProfile {
    pub profile: Option<Value>,
    pub rating_history: Option<Value>,
    pub profile_fetched_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowedRatings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bullet: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blitz: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rapid: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classical: Option<i64>,
}

/// `FollowedUser` (`contracts/types.ts`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowedUser {
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub ratings: FollowedRatings,
    pub followed_by: Vec<String>,
    pub already_added: bool,
    pub dismissed: bool,
}

/// `FollowingProblem`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowingProblem {
    pub account: String,
    pub message: String,
    pub needs_reconnect: bool,
}

/// `FollowingReport`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FollowingReport {
    pub users: Vec<FollowedUser>,
    pub problems: Vec<FollowingProblem>,
}

#[derive(Debug, Deserialize)]
struct PerfResponse {
    perf: PerfInfo,
    #[serde(default)]
    rank: Option<i64>,
    #[serde(default)]
    percentile: Option<f64>,
    stat: PerfStat,
}

#[derive(Debug, Deserialize)]
struct PerfInfo {
    #[serde(default)]
    glicko: Option<Glicko>,
    #[serde(default)]
    progress: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Glicko {
    rating: f64,
    deviation: f64,
    #[serde(default)]
    provisional: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PerfStat {
    #[serde(default)]
    count: Value,
    #[serde(default)]
    highest: Option<PerfPoint>,
    #[serde(default)]
    lowest: Option<PerfPoint>,
    #[serde(default, rename = "bestWins")]
    best_wins: Option<PerfResults>,
    #[serde(default, rename = "worstLosses")]
    worst_losses: Option<PerfResults>,
    #[serde(default, rename = "resultStreak")]
    result_streak: Option<ResultStreak>,
}

#[derive(Debug, Deserialize)]
struct PerfPoint {
    int: f64,
    at: String,
    #[serde(rename = "gameId")]
    game_id: String,
}

#[derive(Debug, Deserialize)]
struct PerfResults {
    #[serde(default)]
    results: Vec<PerfResult>,
}

#[derive(Debug, Deserialize)]
struct PerfResult {
    #[serde(default, rename = "opRating")]
    op_rating: f64,
    #[serde(default, rename = "opId")]
    op_id: Option<OpponentId>,
    at: String,
    #[serde(rename = "gameId")]
    game_id: String,
}

#[derive(Debug, Deserialize)]
struct OpponentId {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ResultStreak {
    win: StreakSide,
    loss: StreakSide,
}

#[derive(Debug, Deserialize)]
struct StreakSide {
    cur: StreakValue,
    max: StreakValue,
}

#[derive(Debug, Deserialize)]
struct StreakValue {
    v: i64,
}

/// `PerfPoint` of `PerfStats`: a rating and the game it came from.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfMark {
    pub rating: f64,
    pub at: String,
    pub game_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfGame {
    pub opponent: String,
    pub opponent_rating: f64,
    pub at: String,
    pub game_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Streak {
    pub current: i64,
    pub best: i64,
}

/// `PerfStats` (`contracts/types.ts`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfStats {
    pub perf: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deviation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisional: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentile: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<i64>,
    pub count: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highest: Option<PerfMark>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lowest: Option<PerfMark>,
    pub best_wins: Vec<PerfGame>,
    pub worst_losses: Vec<PerfGame>,
    pub win_streak: Streak,
    pub loss_streak: Streak,
}

fn perf_stats(perf: &str, raw: PerfResponse) -> PerfStats {
    let results = |list: Option<PerfResults>| -> Vec<PerfGame> {
        list.map(|list| list.results)
            .unwrap_or_default()
            .into_iter()
            .take(5)
            .map(|entry| PerfGame {
                opponent: entry
                    .op_id
                    .map(|id| id.name)
                    .unwrap_or_else(|| "?".to_string()),
                opponent_rating: entry.op_rating,
                at: entry.at,
                game_id: entry.game_id,
            })
            .collect()
    };
    let mark = |point: Option<PerfPoint>| {
        point.map(|point| PerfMark {
            rating: point.int,
            at: point.at,
            game_id: point.game_id,
        })
    };
    let streak = |side: fn(&ResultStreak) -> (i64, i64)| {
        raw.stat
            .result_streak
            .as_ref()
            .map(side)
            .map(|(current, best)| Streak { current, best })
            .unwrap_or(Streak {
                current: 0,
                best: 0,
            })
    };
    PerfStats {
        perf: perf.to_string(),
        rating: raw.perf.glicko.as_ref().map(|glicko| glicko.rating),
        deviation: raw.perf.glicko.as_ref().map(|glicko| glicko.deviation),
        provisional: raw
            .perf
            .glicko
            .as_ref()
            .and_then(|glicko| glicko.provisional),
        rank: raw.rank,
        percentile: raw.percentile,
        progress: raw.perf.progress,
        count: raw.stat.count.clone(),
        highest: mark(raw.stat.highest),
        lowest: mark(raw.stat.lowest),
        best_wins: results(raw.stat.best_wins),
        worst_losses: results(raw.stat.worst_losses),
        win_streak: streak(|streaks| (streaks.win.cur.v, streaks.win.max.v)),
        loss_streak: streak(|streaks| (streaks.loss.cur.v, streaks.loss.max.v)),
    }
}

#[derive(Debug, Deserialize)]
struct CrosstableRaw {
    #[serde(default)]
    users: BTreeMap<String, f64>,
    #[serde(default, rename = "nbGames")]
    nb_games: i64,
    #[serde(default)]
    matchup: Option<CrosstableMatchupRaw>,
}

#[derive(Debug, Deserialize)]
struct CrosstableMatchupRaw {
    #[serde(default)]
    users: BTreeMap<String, f64>,
    #[serde(default, rename = "nbGames")]
    nb_games: i64,
}

/// `Crosstable` (`contracts/types.ts`); names are lowercase.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crosstable {
    pub users: BTreeMap<String, f64>,
    pub nb_games: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matchup: Option<CrosstableMatchup>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrosstableMatchup {
    pub users: BTreeMap<String, f64>,
    pub nb_games: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segments_are_encoded() {
        assert_eq!(path_segment("Magnus_C-1"), "Magnus_C-1");
        assert_eq!(path_segment("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn a_game_is_seen_from_its_account() {
        let raw: GameJson = serde_json::from_str(
            r#"{"id":"abcdefgh","createdAt":1,"lastMoveAt":2,"rated":true,"speed":"blitz",
                "perf":"blitz","status":"mate","winner":"black",
                "players":{"white":{"user":{"name":"Bob"},"rating":1400},
                           "black":{"user":{"name":"Alice"},"rating":1500,"ratingDiff":-8}},
                "opening":{"name":"Sicilian"},"moves":"e4 c5"}"#,
        )
        .unwrap();
        let game = normalize_game(&raw, "alice");
        assert_eq!(game.color, "black");
        assert_eq!(game.opponent, "Bob");
        assert_eq!(game.opponent_rating, Some(1400));
        assert_eq!(game.player_rating, Some(1500));
        assert_eq!(game.rating_diff, Some(-8));
        assert_eq!(game.opening.as_deref(), Some("Sicilian"));
    }

    #[test]
    fn an_ai_opponent_is_named_by_its_level() {
        let raw: GameJson = serde_json::from_str(
            r#"{"id":"x","createdAt":1,"lastMoveAt":2,"status":"resign",
                "players":{"white":{"user":{"name":"Alice"}},"black":{"aiLevel":3}}}"#,
        )
        .unwrap();
        assert_eq!(normalize_game(&raw, "Alice").opponent, "Stockfish level 3");
    }
}
