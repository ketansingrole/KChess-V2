//! Lichess's cloud evaluation (`crates/kchess-node/js/cloudEval.ts`) and the explicit position lookups
//! of the opening explorer, masters, player and tablebase databases
//! (`crates/kchess-node/js/positionLookup.ts`).
//!
//! The saved lookups live behind `LookupCache` (the `store::lookups` table in production). Legal
//! moves and SAN come from `kchess_domain::replay`, and every remote answer is validated before it
//! is trusted: a move that is not legal from its position fails the lookup.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::client::Failure;
use super::decode::{Loose, Text, read};
use super::reviews::split_whitespace;
use crate::error::{CoreError, Result as CoreResult};
use kchess_domain::{js, misc, position, replay};

mod opening_info;
pub use opening_info::{DescriptionBlock, OpeningDescription, PopularityMonth};

/// `POSITION_LOOKUP_KINDS` of the contracts.
pub const LOOKUP_KINDS: [&str; 4] = ["opening", "masters", "player", "tablebase"];
/// `EXPLORER_SPEEDS` of the contracts, in their order.
pub const EXPLORER_SPEEDS: [&str; 6] = [
    "ultraBullet",
    "bullet",
    "blitz",
    "rapid",
    "classical",
    "correspondence",
];
/// `EXPLORER_RATINGS` of the contracts.
pub const EXPLORER_RATINGS: [f64; 9] = [
    400.0, 1000.0, 1200.0, 1400.0, 1600.0, 1800.0, 2000.0, 2200.0, 2500.0,
];

const CLOUD_TTL: Duration = Duration::from_secs(10 * 60);
const CLOUD_MAX: usize = 500;
const PENDING_MAX: usize = 16;
const DEFAULT_OPENING: &str = r#"{"speeds":["blitz","rapid","classical"]}"#;
const MAX_CACHED_GAMES: usize = 30;
const MAX_MOVES: usize = 256;

/* ── Cloud evaluation ── */

/// A cloud evaluation: lines from White's point of view (`CloudEval`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudEval {
    pub fen: String,
    pub depth: Value,
    pub knodes: Value,
    pub lines: Vec<CloudLine>,
}

/// One line of a cloud evaluation (`EngineLine`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CloudLine {
    pub rank: usize,
    pub depth: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cp: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mate: Option<Value>,
    pub pv: Vec<String>,
}

/// The cloud evaluation cache: ten minutes per entry, the 500 most recently used kept. A position
/// Lichess has no evaluation for is remembered as `None`, so it is not asked again soon.
#[derive(Default)]
pub struct CloudCache {
    entries: Mutex<HashMap<String, CloudEntry>>,
}

struct CloudEntry {
    value: Option<CloudEval>,
    stored: Instant,
    used: Instant,
}

impl CloudCache {
    fn get(&self, key: &str) -> Option<Option<CloudEval>> {
        let mut entries = lock(&self.entries);
        let fresh = entries
            .get(key)
            .is_some_and(|entry| entry.stored.elapsed() < CLOUD_TTL);
        if !fresh {
            entries.remove(key);
            return None;
        }
        let entry = entries.get_mut(key)?;
        entry.used = Instant::now();
        Some(entry.value.clone())
    }

    fn set(&self, key: String, value: Option<CloudEval>) {
        let mut entries = lock(&self.entries);
        if entries.len() >= CLOUD_MAX
            && !entries.contains_key(&key)
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        let now = Instant::now();
        entries.insert(
            key,
            CloudEntry {
                value,
                stored: now,
                used: now,
            },
        );
    }

    /// `clearCloudEval`.
    pub fn clear(&self) {
        lock(&self.entries).clear();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// `cloudEval(fen, lines)`: Lichess's cloud evaluation, for positions someone analysed deeply on
/// Lichess before. Lines are kept only as far as they are legal from the position; unknown
/// positions give None.
pub async fn cloud_eval(
    lichess: &super::accounts::Lichess,
    cache: &CloudCache,
    fen: &str,
    lines: usize,
) -> CoreResult<Option<CloudEval>> {
    let key = format!("{lines}:{fen}");
    if let Some(hit) = cache.get(&key) {
        return Ok(hit);
    }
    let cancel = lichess.lifetime().clone();
    let query = [("fen", fen.to_string()), ("multiPv", lines.to_string())];
    let raw = lichess
        .client()
        .policy()
        .with_usage("", "analysis", async {
            lichess
                .client()
                .get_json::<Value>("/api/cloud-eval", &query, None, &cancel)
                .await
        })
        .await;
    let raw = match raw {
        Ok(raw) => raw,
        Err(Failure::Lichess(error)) if error.status == 404 => {
            cache.set(key, None);
            return Ok(None);
        }
        Err(failure) => return Err(failure.into()),
    };
    let data = decode_cloud(&raw)?;
    let mut result_lines = Vec::new();
    for (index, pv) in data.pvs.iter().enumerate() {
        let trimmed = js::trim(&pv.moves);
        let moves: Vec<&str> = split_whitespace(trimmed).into_iter().take(30).collect();
        // Keep the legal prefix; an illegal first move drops the line.
        let legal = replay::replay_positions(fen, &moves)
            .len()
            .saturating_sub(1);
        if legal < 1 {
            continue;
        }
        result_lines.push(CloudLine {
            rank: index + 1,
            depth: Value::Number(data.depth.clone()),
            // `cp` wins when both are sent.
            cp: pv.cp.0.clone().map(Value::Number),
            mate: pv
                .cp
                .0
                .is_none()
                .then(|| pv.mate.0.clone().map(Value::Number))
                .flatten(),
            pv: moves[..legal].iter().map(|m| m.to_string()).collect(),
        });
    }
    let result = CloudEval {
        fen: fen.to_string(),
        depth: Value::Number(data.depth),
        knodes: Value::Number(data.knodes),
        lines: result_lines,
    };
    cache.set(key, Some(result.clone()));
    Ok(Some(result))
}

#[derive(Deserialize)]
struct CloudPv {
    #[serde(default)]
    cp: Loose<serde_json::Number>,
    #[serde(default)]
    mate: Loose<serde_json::Number>,
    moves: String,
}

#[derive(Deserialize)]
struct CloudData {
    depth: serde_json::Number,
    knodes: serde_json::Number,
    pvs: Vec<CloudPv>,
}

/// The cloud evaluation's shape: non-negative depth and nodes, at most five lines, each with a
/// score (`cp` or `mate`) and at most 2000 characters of moves.
fn decode_cloud(raw: &Value) -> CoreResult<CloudData> {
    let non_negative = |n: &serde_json::Number| n.as_f64().is_some_and(|n| n >= 0.0);
    read::<CloudData>(raw)
        .filter(|data| {
            non_negative(&data.depth)
                && non_negative(&data.knodes)
                && data.pvs.len() <= 5
                && data
                    .pvs
                    .iter()
                    .all(|pv| pv.moves.len() <= 2000 && (pv.cp.0.is_some() || pv.mate.0.is_some()))
        })
        .ok_or_else(|| CoreError::new("Lichess sent an unreadable cloud evaluation."))
}

/* ── Position lookups ── */

/// The filters a database uses, normalized in a stable order (they are part of the cache key).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LookupOptions {
    pub speeds: Option<Vec<String>>,
    pub ratings: Option<Vec<f64>>,
    pub player: Option<String>,
    pub color: Option<String>,
    pub modes: Option<Vec<String>>,
    pub since: Option<String>,
}

/// One move of a lookup: the database's counts and the SAN checked against the position.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LookupMove {
    pub uci: String,
    pub san: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub white: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub draws: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub black: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub category: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "double_option"
    )]
    pub dtz: Option<Option<i64>>,
}

/// A game the explorer lists for a position (`ExplorerGame`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerGame {
    pub id: String,
    pub white: String,
    pub black: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub white_rating: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub black_rating: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub winner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub year: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub month: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub san: Option<String>,
}

/// A lookup as saved (`Omit<PositionLookup, 'cached' | 'stale' | 'message'>`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cached {
    pub kind: String,
    pub fen: String,
    pub fetched_at: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub opening: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub category: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "double_option"
    )]
    pub dtz: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub games: Option<Vec<ExplorerGame>>,
    pub moves: Vec<LookupMove>,
}

/// A lookup as the caller receives it (`PositionLookup`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionLookup {
    #[serde(flatten)]
    pub data: Cached,
    pub cached: bool,
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// `dtz` is present and null, or a number, or absent.
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer).map(Some)
}

/// The stored saved-lookup cache: the newest entries, as the store keeps them.
///
/// Async since the store moved off the executor (`Database::call_async`): both methods return a
/// boxed future, so out-of-tree implementers wrap each body in `Box::pin(async move { ... })` and
/// `.await` the trait at call sites.
pub trait LookupCache: Send + Sync {
    /// The saved JSON for `key`, if any (a missing or unreadable entry is None).
    fn read<'a>(
        &'a self,
        key: &'a str,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<Option<Value>>>;
    /// Saves `value` under `key`.
    fn write<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
    ) -> futures_util::future::BoxFuture<'a, CoreResult<()>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Opening,
    Masters,
    Player,
    Tablebase,
}

impl Kind {
    fn parse(text: &str) -> Option<Kind> {
        Some(match text {
            "opening" => Kind::Opening,
            "masters" => Kind::Masters,
            "player" => Kind::Player,
            "tablebase" => Kind::Tablebase,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Opening => "opening",
            Kind::Masters => "masters",
            Kind::Player => "player",
            Kind::Tablebase => "tablebase",
        }
    }

    /// How long a saved lookup is fresh: master games change rarely, a player's games often.
    fn fresh_ms(self) -> i64 {
        match self {
            Kind::Opening => 86_400_000,
            Kind::Masters | Kind::Tablebase => 30 * 86_400_000,
            Kind::Player => 3_600_000,
        }
    }

    /// The most bytes a response may send.
    fn max_bytes(self) -> usize {
        match self {
            Kind::Player => 4_000_000,
            _ => 512_000,
        }
    }
}

/// Where the lookups are sent. Production uses the Lichess explorer and tablebase.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub opening: String,
    pub masters: String,
    pub player: String,
    pub tablebase: String,
    pub masters_game: String,
    /// The Wikibooks API, for opening descriptions.
    pub wikibooks: String,
}

impl Endpoints {
    /// The explorer and tablebase endpoints under a test origin (`KCHESS_TEST_LICHESS_BASE`), with
    /// the same paths as production.
    pub fn at(origin: &str) -> Endpoints {
        Endpoints {
            opening: format!("{origin}/explorer/lichess"),
            masters: format!("{origin}/explorer/masters"),
            player: format!("{origin}/explorer/player"),
            tablebase: format!("{origin}/tablebase/standard"),
            masters_game: format!("{origin}/explorer/masters/pgn"),
            wikibooks: format!("{origin}/wikibooks/w/api.php"),
        }
    }
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        Endpoints {
            opening: "https://explorer.lichess.org/lichess".into(),
            masters: "https://explorer.lichess.org/masters".into(),
            player: "https://explorer.lichess.org/player".into(),
            tablebase: "https://tablebase.lichess.org/standard".into(),
            masters_game: "https://explorer.lichess.org/masters/pgn".into(),
            wikibooks: "https://en.wikibooks.org/w/api.php".into(),
        }
    }
}

type Pending = watch::Receiver<Option<Result<PositionLookup, CoreError>>>;

/// The position explorer: deduplicated and bounded lookups with validated legal moves, stale
/// recovery when a refresh fails, and the master game PGNs.
pub struct PositionLookups {
    http: reqwest::Client,
    endpoints: Endpoints,
    cache: Arc<dyn LookupCache>,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    pending: Mutex<HashMap<String, Pending>>,
    /// The login the explorer is asked with; None sends no login (tests).
    login: Option<Arc<dyn ExplorerLogin>>,
    lifetime: CancellationToken,
    /// Per origin, when (by `now`) a 429's cooldown ends; until then that origin is not asked.
    cooldowns: Mutex<HashMap<String, i64>>,
}

/// Removing a pending lookup must also happen when its caller is cancelled or dropped.
struct PendingLookup<'a> {
    pending: &'a Mutex<HashMap<String, Pending>>,
    key: String,
}

impl Drop for PendingLookup<'_> {
    fn drop(&mut self) {
        lock(self.pending).remove(&self.key);
    }
}

/// The login the opening explorer and master games are asked with: the account the explorer is set
/// to use, and its token (`setupPositionLookup.ts`). Tablebase lookups never carry it.
pub trait ExplorerLogin: Send + Sync {
    /// The token of the explorer's account, or None when no account is set or it has no login.
    fn token(&self) -> BoxFuture<'_, CoreResult<Option<String>>>;
}

impl PositionLookups {
    /// `now` is the clock in ms since the epoch (`Date.now`).
    pub fn new(
        http: reqwest::Client,
        endpoints: Endpoints,
        cache: Arc<dyn LookupCache>,
        now: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> PositionLookups {
        PositionLookups {
            http,
            endpoints,
            cache,
            now,
            pending: Mutex::new(HashMap::new()),
            login: None,
            lifetime: CancellationToken::new(),
            cooldowns: Mutex::new(HashMap::new()),
        }
    }

    /// Asks the explorer and master games with `login`'s token. Without a token the explorer is
    /// not asked at all: it requires a Lichess login.
    pub fn with_login(mut self, login: Arc<dyn ExplorerLogin>) -> PositionLookups {
        self.login = Some(login);
        self
    }

    /// Share the core's lifetime so close stops login, HTTP, cache waits and deduplicated callers.
    pub fn with_lifetime(mut self, lifetime: CancellationToken) -> PositionLookups {
        self.lifetime = lifetime;
        self
    }

    /// A request to the explorer (not the tablebase), with the explorer's login when one is set.
    async fn explorer_request(&self, url: reqwest::Url) -> CoreResult<reqwest::RequestBuilder> {
        let Some(login) = &self.login else {
            return Ok(self.http.get(url));
        };
        let token = login.token().await?.ok_or_else(|| {
            CoreError::new("Connect a Lichess account in Settings to use the opening explorer.")
        })?;
        Ok(self.http.get(url).bearer_auth(token))
    }

    /// Refuses a request to an origin that answered 429 until its cooldown ends, so the saved
    /// lookup (or the error) comes back at once instead of prolonging the rate limit.
    fn paced(&self, url: &reqwest::Url) -> CoreResult<()> {
        let origin = url.origin().ascii_serialization();
        let until = lock(&self.cooldowns).get(&origin).copied();
        match until {
            Some(until) if until > (self.now)() => Err(slow_down(until - (self.now)())),
            _ => Ok(()),
        }
    }

    /// After a 429, starts its origin's cooldown (`Retry-After`, as the Lichess API's policy
    /// reads it) and fails the request.
    fn note_rate_limit(&self, url: &reqwest::Url, response: &reqwest::Response) -> CoreResult<()> {
        if response.status().as_u16() != 429 {
            return Ok(());
        }
        let wait = i64::try_from(super::policy::cooldown_for(response.headers()).as_millis())
            .unwrap_or(i64::MAX);
        let until = (self.now)().saturating_add(wait);
        let origin = url.origin().ascii_serialization();
        lock(&self.cooldowns).insert(origin, until);
        Err(slow_down(wait))
    }

    /// `lookup(kind, fen, options)`.
    pub async fn lookup(
        &self,
        raw_kind: &Value,
        raw_fen: &Value,
        raw_options: Option<&Value>,
    ) -> CoreResult<PositionLookup> {
        tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => Err(CoreError::aborted("Position lookup was cancelled.")),
            result = self.lookup_inner(raw_kind, raw_fen, raw_options) => result,
        }
    }

    async fn lookup_inner(
        &self,
        raw_kind: &Value,
        raw_fen: &Value,
        raw_options: Option<&Value>,
    ) -> CoreResult<PositionLookup> {
        let kind = raw_kind
            .as_str()
            .and_then(Kind::parse)
            .ok_or_else(|| CoreError::new("Invalid position lookup kind."))?;
        let analysis = misc::call(
            "assertAnalysisRequest",
            &[no_lossless(), json!({ "fen": raw_fen, "lines": 1 })],
        )
        .ok_or_else(|| CoreError::new("Unknown validator."))?
        .map_err(CoreError::new)?;
        let fen = analysis
            .get("fen")
            .and_then(Value::as_str)
            .ok_or_else(|| CoreError::new("Invalid analysis request."))?
            .to_string();
        let options = assert_options(raw_options)?;
        let options = normalize_options(kind, &options)?;
        if !is_legal_position(&fen) {
            return Err(CoreError::new("That position is not legal."));
        }
        if kind == Kind::Tablebase {
            let (pieces, castling) = material(&fen);
            if pieces > 7 || castling {
                return Err(CoreError::new(
                    "Tablebases cover positions with up to seven pieces and no castling rights.",
                ));
            }
        }
        let filters = filters_json(kind, &options);
        // The original keys stay readable for lookups saved before filters existed.
        let key =
            if kind == Kind::Tablebase || (kind == Kind::Opening && filters == DEFAULT_OPENING) {
                format!("{}:{fen}", kind.name())
            } else {
                format!("{}:{filters}:{fen}", kind.name())
            };
        let candidate = decode_cached(self.cache.read(&key).await?.as_ref());
        let saved = candidate.filter(|candidate| {
            candidate.fen == fen
                && candidate.kind == kind.name()
                && candidate
                    .moves
                    .iter()
                    .all(|entry| legal_san(&fen, &entry.uci).as_deref() == Some(entry.san.as_str()))
        });
        if let Some(saved) = &saved
            && (self.now)() - saved.fetched_at < kind.fresh_ms()
        {
            return Ok(PositionLookup {
                data: saved.clone(),
                cached: true,
                stale: false,
                message: None,
            });
        }
        let admitted = {
            let mut pending = lock(&self.pending);
            if let Some(known) = pending.get(&key).cloned() {
                Err(known)
            } else {
                if pending.len() >= PENDING_MAX {
                    return Err(CoreError::new(
                        "Too many position lookups are pending. Try again shortly.",
                    ));
                }
                let (send, receive) = watch::channel(None);
                pending.insert(key.clone(), receive);
                Ok(send)
            }
        };
        let send = match admitted {
            Ok(send) => send,
            Err(known) => return wait_for(known).await,
        };
        let _pending = PendingLookup {
            pending: &self.pending,
            key: key.clone(),
        };
        let result = self.load(kind, &fen, &options, &key, saved.as_ref()).await;
        let _ = send.send(Some(result.clone()));
        result
    }

    async fn load(
        &self,
        kind: Kind,
        fen: &str,
        options: &LookupOptions,
        key: &str,
        saved: Option<&Cached>,
    ) -> CoreResult<PositionLookup> {
        let attempt = async {
            let timeout = if kind == Kind::Player {
                Duration::from_secs(60)
            } else {
                Duration::from_secs(15)
            };
            let text = tokio::time::timeout(timeout, self.fetch(kind, fen, options))
                .await
                .map_err(|_| CoreError::new("The operation was aborted due to timeout"))??;
            let body = if kind == Kind::Player {
                // The player database answers with a stream of ever more complete results.
                text.split('\n')
                    .map(js::trim)
                    .rfind(|line| !line.is_empty())
                    .unwrap_or("{}")
                    .to_string()
            } else {
                text
            };
            let raw: Value = serde_json::from_str(&body)
                .map_err(|_| CoreError::new("Position lookup returned unreadable data."))?;
            let result = if kind == Kind::Tablebase {
                tablebase_result(fen, (self.now)(), &raw)?
            } else {
                explorer_result(kind, fen, (self.now)(), &raw)?
            };
            self.cache
                .write(
                    key,
                    &serde_json::to_value(&result).map_err(|cause| {
                        CoreError::new(format!("Position lookup could not be saved: {cause}"))
                    })?,
                )
                .await?;
            Ok(PositionLookup {
                data: result,
                cached: false,
                stale: false,
                message: None,
            })
        };
        match attempt.await {
            Ok(result) => Ok(result),
            Err(cause) => match saved {
                Some(saved) => Ok(PositionLookup {
                    data: saved.clone(),
                    cached: true,
                    stale: true,
                    message: Some("Could not refresh. Showing the saved lookup.".into()),
                }),
                None => Err(cause),
            },
        }
    }

    /// The URL of a lookup, with the filters the database understands.
    fn url(&self, kind: Kind, fen: &str, options: &LookupOptions) -> CoreResult<reqwest::Url> {
        let base = match kind {
            Kind::Opening => &self.endpoints.opening,
            Kind::Masters => &self.endpoints.masters,
            Kind::Player => &self.endpoints.player,
            Kind::Tablebase => &self.endpoints.tablebase,
        };
        let mut url = reqwest::Url::parse(base)
            .map_err(|cause| CoreError::new(format!("Invalid explorer URL: {cause}")))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("fen", fen);
            if kind != Kind::Tablebase {
                query.append_pair("moves", "12");
                // Once per URL: Lichess reads the first of repeated parameters.
                let recent = if kind == Kind::Player { "8" } else { "0" };
                query.append_pair("recentGames", recent);
            }
            match kind {
                Kind::Opening => {
                    query.append_pair("variant", "standard");
                    query.append_pair(
                        "speeds",
                        &options.speeds.clone().unwrap_or_default().join(","),
                    );
                    if let Some(ratings) = options.ratings.as_ref().filter(|r| !r.is_empty()) {
                        query.append_pair("ratings", &join_numbers(ratings));
                    }
                    query.append_pair("topGames", "0");
                    if let Some(since) = &options.since {
                        query.append_pair("since", since);
                    }
                }
                Kind::Masters => {
                    query.append_pair("topGames", "8");
                    if let Some(since) = &options.since {
                        query.append_pair("since", since);
                    }
                }
                Kind::Player => {
                    query.append_pair("player", options.player.as_deref().unwrap_or(""));
                    query.append_pair("color", options.color.as_deref().unwrap_or("white"));
                    if let Some(speeds) = options.speeds.as_ref().filter(|s| !s.is_empty()) {
                        query.append_pair("speeds", &speeds.join(","));
                    }
                    if let Some(modes) = options.modes.as_ref().filter(|m| !m.is_empty()) {
                        query.append_pair("modes", &modes.join(","));
                    }
                    if let Some(since) = &options.since {
                        query.append_pair("since", since);
                    }
                }
                Kind::Tablebase => {}
            }
        }
        Ok(url)
    }

    /// Requests a lookup and reads its body, bounded by the kind's size limit.
    async fn fetch(&self, kind: Kind, fen: &str, options: &LookupOptions) -> CoreResult<String> {
        let url = self.url(kind, fen, options)?;
        self.paced(&url)?;
        let request = if kind == Kind::Tablebase {
            self.http.get(url.clone())
        } else {
            self.explorer_request(url.clone()).await?
        };
        let mut response = request.send().await.map_err(|_| {
            CoreError::new("Position lookup failed. Check your connection and retry.")
        })?;
        self.note_rate_limit(&url, &response)?;
        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return Err(CoreError::new(
                "Connect a Lichess account in Settings to use the opening explorer.",
            ));
        }
        if status == 404 && kind == Kind::Player {
            return Err(CoreError::new(format!(
                "No Lichess player named \"{}\".",
                options.player.as_deref().unwrap_or("")
            )));
        }
        if !response.status().is_success() {
            return Err(CoreError::new(format!(
                "Position lookup returned {status}. Try again shortly."
            )));
        }
        let limit = kind.max_bytes();
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            let chunk = response.chunk().await.map_err(|_| {
                CoreError::new("Position lookup failed. Check your connection and retry.")
            })?;
            let Some(chunk) = chunk else { break };
            if bytes.len() + chunk.len() > limit {
                return Err(CoreError::new("Position lookup returned too much data."));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// `mastersGame(id)`: a master game's PGN; master games never change.
    pub async fn masters_game(&self, id: &str) -> CoreResult<String> {
        tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => Err(CoreError::aborted("Position lookup was cancelled.")),
            result = self.masters_game_inner(id) => result,
        }
    }

    async fn masters_game_inner(&self, id: &str) -> CoreResult<String> {
        let valid = id.len() == 8 && id.bytes().all(|b| b.is_ascii_alphanumeric());
        if !valid {
            return Err(CoreError::new("Invalid id."));
        }
        let url = reqwest::Url::parse(&format!("{}/{id}", self.endpoints.masters_game))
            .map_err(|_| CoreError::new("Invalid id."))?;
        self.paced(&url)?;
        let attempt = async {
            let response = self
                .explorer_request(url.clone())
                .await?
                .send()
                .await
                .map_err(|_| CoreError::new("The master game could not be loaded."))?;
            self.note_rate_limit(&url, &response)?;
            let status = response.status().as_u16();
            if status == 401 || status == 403 {
                return Err(CoreError::new(
                    "Connect a Lichess account in Settings to use the opening explorer.",
                ));
            }
            if !response.status().is_success() {
                return Err(CoreError::new(format!(
                    "The master game could not be loaded ({status})."
                )));
            }
            response
                .text()
                .await
                .map_err(|_| CoreError::new("The master game could not be loaded."))
        };
        let text = tokio::time::timeout(Duration::from_secs(15), attempt)
            .await
            .map_err(|_| CoreError::new("The operation was aborted due to timeout"))??;
        if js::utf16_len(&text) > 200_000 || !text.contains('[') {
            return Err(CoreError::new("That game is not a PGN."));
        }
        Ok(text)
    }
}

/// The error while an origin's 429 cooldown of `wait_ms` runs.
fn slow_down(wait_ms: i64) -> CoreError {
    let seconds = (wait_ms.max(1) + 999) / 1000;
    let unit = if seconds == 1 { "second" } else { "seconds" };
    CoreError::new(format!("Too many lookups. Try again in {seconds} {unit}."))
}

/// The lossless sidecar of a validator call whose arguments are plain JSON (no `undefined`, NaN or
/// binary values to record).
fn no_lossless() -> Value {
    json!([])
}

/// `assertLookupOptions(raw)`: the filters as the request sent them, validated.
fn assert_options(raw: Option<&Value>) -> CoreResult<LookupOptions> {
    // An absent value is passed as no argument at all, which the validator reads as undefined.
    let mut args = vec![no_lossless()];
    args.extend(raw.cloned());
    let value = misc::call("assertLookupOptions", &args)
        .ok_or_else(|| CoreError::new("Unknown validator."))?
        .map_err(CoreError::new)?;
    let strings = |key: &str| -> Option<Vec<String>> {
        value.get(key).and_then(Value::as_array).map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
    };
    Ok(LookupOptions {
        speeds: strings("speeds"),
        ratings: value
            .get("ratings")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_f64).collect()),
        player: value
            .get("player")
            .and_then(Value::as_str)
            .map(str::to_string),
        color: value
            .get("color")
            .and_then(Value::as_str)
            .map(str::to_string),
        modes: strings("modes"),
        since: value
            .get("since")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// `normalizeOptions(kind, options)`: the filters each database understands, in a fixed order.
fn normalize_options(kind: Kind, options: &LookupOptions) -> CoreResult<LookupOptions> {
    let nonempty = |list: &Option<Vec<String>>| list.as_ref().filter(|l| !l.is_empty()).cloned();
    match kind {
        Kind::Tablebase => Ok(LookupOptions::default()),
        Kind::Masters => Ok(LookupOptions {
            since: options
                .since
                .as_ref()
                .filter(|since| !since.is_empty())
                .map(|since| since.chars().take(4).collect()),
            ..LookupOptions::default()
        }),
        Kind::Opening => Ok(LookupOptions {
            speeds: Some(
                nonempty(&options.speeds)
                    .map(|speeds| {
                        EXPLORER_SPEEDS
                            .iter()
                            .filter(|speed| speeds.iter().any(|s| s == *speed))
                            .map(|speed| speed.to_string())
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["blitz".into(), "rapid".into(), "classical".into()]),
            ),
            ratings: options
                .ratings
                .as_ref()
                .filter(|r| !r.is_empty())
                .map(|ratings| {
                    EXPLORER_RATINGS
                        .iter()
                        .copied()
                        .filter(|rating| ratings.contains(rating))
                        .collect()
                }),
            since: options.since.clone(),
            ..LookupOptions::default()
        }),
        Kind::Player => {
            let Some(player) = options.player.clone().filter(|p| !p.is_empty()) else {
                return Err(CoreError::new("Choose whose games to explore."));
            };
            Ok(LookupOptions {
                player: Some(player),
                color: Some(options.color.clone().unwrap_or_else(|| "white".into())),
                speeds: nonempty(&options.speeds).map(|speeds| {
                    EXPLORER_SPEEDS
                        .iter()
                        .filter(|speed| speeds.iter().any(|s| s == *speed))
                        .map(|speed| speed.to_string())
                        .collect()
                }),
                modes: nonempty(&options.modes),
                since: options.since.clone(),
                ..LookupOptions::default()
            })
        }
    }
}

/// The normalized filters as JSON, with keys in the order the cache keys were written.
fn filters_json(kind: Kind, options: &LookupOptions) -> String {
    let mut fields: Vec<String> = Vec::new();
    let text = |key: &str, value: Value| format!("{}:{}", json!(key), value);
    match kind {
        Kind::Tablebase => {}
        Kind::Masters => {
            if let Some(since) = &options.since {
                fields.push(text("since", json!(since)));
            }
        }
        Kind::Opening => {
            fields.push(text(
                "speeds",
                json!(options.speeds.clone().unwrap_or_default()),
            ));
            if let Some(ratings) = &options.ratings {
                fields.push(format!("\"ratings\":[{}]", join_numbers(ratings)));
            }
            if let Some(since) = &options.since {
                fields.push(text("since", json!(since)));
            }
        }
        Kind::Player => {
            fields.push(text(
                "player",
                json!(options.player.clone().unwrap_or_default()),
            ));
            fields.push(text(
                "color",
                json!(options.color.clone().unwrap_or_default()),
            ));
            if let Some(speeds) = &options.speeds {
                fields.push(text("speeds", json!(speeds)));
            }
            if let Some(modes) = &options.modes {
                fields.push(text("modes", json!(modes)));
            }
            if let Some(since) = &options.since {
                fields.push(text("since", json!(since)));
            }
        }
    }
    format!("{{{}}}", fields.join(","))
}

/// Numbers as JavaScript writes them (`400`, not `400.0`), comma-separated.
fn join_numbers(values: &[f64]) -> String {
    values
        .iter()
        .map(|value| js::number_to_string(*value))
        .collect::<Vec<_>>()
        .join(",")
}

/// `Position.fromFen` accepts the FEN: a legal standard position.
fn is_legal_position(fen: &str) -> bool {
    position::fen_problem(fen).is_none()
}

/// The piece count and whether any castling right remains, for the tablebase limits.
fn material(fen: &str) -> (usize, bool) {
    let Some(replay) = replay::replay_setup::<&str>("standard", fen, &[]) else {
        return (usize::MAX, true);
    };
    let mut fields = replay.fen.split(' ');
    let board = fields.next().unwrap_or("");
    let pieces = board.chars().filter(|c| c.is_ascii_alphabetic()).count();
    let castling = fields.nth(1).is_some_and(|rights| rights != "-");
    (pieces, castling)
}

/// The SAN of a legal move from `fen`, or None when it is not legal there.
fn legal_san(fen: &str, uci: &str) -> Option<String> {
    let replay = replay::replay_setup("standard", fen, &[uci])?;
    replay.played.first().map(|played| played.san.clone())
}

/// A tablebase category (`win`, `syzygy-loss`, …).
struct Category(String);

impl<'de> Deserialize<'de> for Category {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        const CATEGORIES: [&str; 10] = [
            "win",
            "unknown",
            "syzygy-win",
            "maybe-win",
            "cursed-win",
            "draw",
            "blessed-loss",
            "maybe-loss",
            "syzygy-loss",
            "loss",
        ];
        let text = String::deserialize(deserializer)?;
        if CATEGORIES.contains(&text.as_str()) {
            Ok(Category(text))
        } else {
            Err(serde::de::Error::custom("unknown tablebase category"))
        }
    }
}

#[derive(Deserialize)]
struct RawTablebaseMove {
    uci: Text<5>,
    category: Category,
    #[serde(default)]
    dtz: Option<i64>,
}

#[derive(Deserialize)]
struct RawTablebase {
    category: Category,
    #[serde(default)]
    dtz: Option<i64>,
    moves: Vec<RawTablebaseMove>,
}

/// The position's and each move's totals: absent or null is no count.
#[derive(Deserialize)]
struct RawExplorerMove {
    uci: Text<5>,
    white: u64,
    draws: u64,
    black: u64,
}

#[derive(Deserialize)]
struct RawOpening {
    name: String,
    eco: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawExplorer {
    #[serde(default)]
    white: Option<u64>,
    #[serde(default)]
    draws: Option<u64>,
    #[serde(default)]
    black: Option<u64>,
    #[serde(default)]
    opening: Option<RawOpening>,
    moves: Vec<RawExplorerMove>,
    // Only the games kept are read (`explorer_game`); one past them may be malformed.
    #[serde(default)]
    top_games: Loose<Vec<Value>>,
    #[serde(default)]
    recent_games: Loose<Vec<Value>>,
}

#[derive(Deserialize)]
struct RawGamePlayer {
    name: String,
    #[serde(default)]
    rating: Loose<f64>,
}

#[derive(Deserialize)]
struct RawExplorerGame {
    id: String,
    white: RawGamePlayer,
    black: RawGamePlayer,
    #[serde(default)]
    winner: Loose<String>,
    #[serde(default)]
    uci: Loose<String>,
    #[serde(default)]
    year: Loose<i64>,
    #[serde(default)]
    month: Loose<String>,
}

/// Counts as the cache keeps them.
fn count(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn tablebase_result(fen: &str, now: i64, raw: &Value) -> CoreResult<Cached> {
    let unreadable = || CoreError::new("Position lookup returned unreadable data.");
    let tablebase = read::<RawTablebase>(raw)
        .filter(|tablebase| tablebase.moves.len() <= MAX_MOVES)
        .ok_or_else(unreadable)?;
    let mut moves = Vec::with_capacity(tablebase.moves.len());
    for item in tablebase.moves {
        let san = legal_san(fen, &item.uci)
            .ok_or_else(|| CoreError::new("Position lookup returned an illegal move."))?;
        moves.push(LookupMove {
            uci: item.uci.into(),
            san,
            category: Some(item.category.0),
            dtz: Some(item.dtz),
            ..LookupMove::default()
        });
    }
    Ok(Cached {
        kind: "tablebase".into(),
        fen: fen.to_string(),
        fetched_at: now,
        total: None,
        opening: None,
        category: Some(tablebase.category.0),
        dtz: Some(tablebase.dtz),
        games: None,
        moves,
    })
}

fn explorer_result(kind: Kind, fen: &str, now: i64, raw: &Value) -> CoreResult<Cached> {
    let unreadable = || CoreError::new("Position lookup returned unreadable data.");
    let explorer = read::<RawExplorer>(raw)
        .filter(|explorer| {
            explorer.moves.len() <= MAX_MOVES
                && explorer
                    .opening
                    .as_ref()
                    .is_none_or(|opening| opening.name.chars().count() <= 200)
        })
        .ok_or_else(unreadable)?;
    let opening = explorer
        .opening
        .map(|opening| format!("{} · {}", opening.eco, opening.name));
    let mut moves = Vec::with_capacity(explorer.moves.len());
    for item in explorer.moves {
        let san = legal_san(fen, &item.uci)
            .ok_or_else(|| CoreError::new("Position lookup returned an illegal move."))?;
        moves.push(LookupMove {
            uci: item.uci.into(),
            san,
            white: Some(count(item.white)),
            draws: Some(count(item.draws)),
            black: Some(count(item.black)),
            ..LookupMove::default()
        });
    }
    let mut listed: Vec<Value> = Vec::new();
    for games in [explorer.top_games.0, explorer.recent_games.0]
        .into_iter()
        .flatten()
    {
        if games.len() > MAX_CACHED_GAMES {
            return Err(unreadable());
        }
        listed.extend(games);
    }
    listed.truncate(12);
    let games = listed
        .iter()
        .map(|game| explorer_game(game, fen).ok_or_else(unreadable))
        .collect::<CoreResult<Vec<_>>>()?;
    let total = [explorer.white, explorer.draws, explorer.black]
        .into_iter()
        .map(|n| count(n.unwrap_or(0)))
        .fold(0i64, i64::saturating_add);
    Ok(Cached {
        kind: kind.name().into(),
        fen: fen.to_string(),
        fetched_at: now,
        total: (total > 0).then_some(total),
        opening,
        category: None,
        dtz: None,
        games: Some(games),
        moves,
    })
}

/// A game the explorer lists, or None when it is not readable.
fn explorer_game(game: &Value, fen: &str) -> Option<ExplorerGame> {
    let game: RawExplorerGame = read(game)?;
    let id_ok = game.id.len() == 8 && game.id.bytes().all(|b| b.is_ascii_alphanumeric());
    let names_ok = [&game.white.name, &game.black.name]
        .iter()
        .all(|name| name.chars().count() <= 80);
    if !id_ok || !names_ok {
        return None;
    }
    Some(ExplorerGame {
        id: game.id,
        white: game.white.name,
        black: game.black.name,
        white_rating: game.white.rating.0,
        black_rating: game.black.rating.0,
        winner: game
            .winner
            .0
            .filter(|color| color == "white" || color == "black"),
        // The move a listed game played from this position, as SAN, when it is legal there.
        san: game
            .uci
            .0
            .filter(|uci| uci.len() <= 5)
            .and_then(|uci| legal_san(fen, &uci)),
        year: game.year.0,
        month: game.month.0,
    })
}

/// `decodeLookupCache`: a saved lookup with the shape this module writes, or None.
fn decode_cached(raw: Option<&Value>) -> Option<Cached> {
    let cached: Cached = read(raw?)?;
    let fits = Kind::parse(&cached.kind).is_some()
        && cached.fen.len() <= 100
        && cached.fetched_at >= 0
        && cached.moves.len() <= MAX_MOVES
        && cached
            .moves
            .iter()
            .all(|entry| entry.uci.len() <= 5 && entry.san.len() <= 32)
        && cached
            .games
            .as_ref()
            .is_none_or(|games| games.len() <= MAX_CACHED_GAMES);
    fits.then_some(cached)
}

/// Waits for a lookup another caller started. A lookup abandoned before it answers reports so.
async fn wait_for(mut receiver: Pending) -> CoreResult<PositionLookup> {
    loop {
        if let Some(result) = receiver.borrow().clone() {
            return result;
        }
        if receiver.changed().await.is_err() {
            return Err(CoreError::new("Position lookup was cancelled."));
        }
    }
}
