//! Lichess's cloud evaluation (`core/src/services/cloudEval.ts`) and the explicit position lookups
//! of the opening explorer, masters, player and tablebase databases
//! (`core/src/services/positionLookup.ts`).
//!
//! The saved lookups live behind `LookupCache` (the `store::lookups` table in production). Legal
//! moves and SAN come from `kchess_domain::replay`, and every remote answer is validated before it
//! is trusted: a move that is not legal from its position fails the lookup.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;

use super::client::Failure;
use super::reviews::split_whitespace;
use crate::error::{CoreError, Result as CoreResult};
use kchess_domain::{js, misc, position, replay};

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
            depth: data.depth.clone(),
            cp: pv.cp.clone(),
            mate: pv.mate.clone(),
            pv: moves[..legal].iter().map(|m| m.to_string()).collect(),
        });
    }
    let result = CloudEval {
        fen: fen.to_string(),
        depth: data.depth,
        knodes: data.knodes,
        lines: result_lines,
    };
    cache.set(key, Some(result.clone()));
    Ok(Some(result))
}

struct CloudPv {
    cp: Option<Value>,
    mate: Option<Value>,
    moves: String,
}

struct CloudData {
    depth: Value,
    knodes: Value,
    pvs: Vec<CloudPv>,
}

/// The cloud evaluation's shape: finite non-negative depth and nodes, at most five lines, each
/// with a score (`cp` or `mate`) and at most 2000 characters of moves.
fn decode_cloud(raw: &Value) -> CoreResult<CloudData> {
    let unreadable = || CoreError::new("Lichess sent an unreadable cloud evaluation.");
    let finite_count = |value: Option<&Value>| -> Option<Value> {
        value
            .filter(|v| v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0))
            .cloned()
    };
    let depth = finite_count(raw.get("depth")).ok_or_else(unreadable)?;
    let knodes = finite_count(raw.get("knodes")).ok_or_else(unreadable)?;
    let items = raw
        .get("pvs")
        .and_then(Value::as_array)
        .filter(|items| items.len() <= 5)
        .ok_or_else(unreadable)?;
    let mut pvs = Vec::with_capacity(items.len());
    for item in items {
        let moves = item
            .get("moves")
            .and_then(Value::as_str)
            .filter(|moves| moves.len() <= 2000)
            .ok_or_else(unreadable)?;
        let finite = |key: &str| {
            item.get(key)
                .filter(|v| v.as_f64().is_some_and(f64::is_finite))
                .cloned()
        };
        let (cp, mate) = match (finite("cp"), finite("mate")) {
            (Some(cp), _) => (Some(cp), None),
            (None, Some(mate)) => (None, Some(mate)),
            (None, None) => return Err(unreadable()),
        };
        pvs.push(CloudPv {
            cp,
            mate,
            moves: moves.to_string(),
        });
    }
    Ok(CloudData { depth, knodes, pvs })
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
pub trait LookupCache: Send + Sync {
    /// The saved JSON for `key`, if any (a missing or unreadable entry is None).
    fn read(&self, key: &str) -> CoreResult<Option<Value>>;
    /// Saves `value` under `key`.
    fn write(&self, key: &str, value: &Value) -> CoreResult<()>;
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
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        Endpoints {
            opening: "https://explorer.lichess.org/lichess".into(),
            masters: "https://explorer.lichess.org/masters".into(),
            player: "https://explorer.lichess.org/player".into(),
            tablebase: "https://tablebase.lichess.org/standard".into(),
            masters_game: "https://explorer.lichess.org/masters/pgn".into(),
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
        }
    }

    /// `lookup(kind, fen, options)`.
    pub async fn lookup(
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
        let candidate = decode_cached(self.cache.read(&key)?.as_ref());
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
        let known = lock(&self.pending).get(&key).cloned();
        if let Some(known) = known {
            return wait_for(known).await;
        }
        if lock(&self.pending).len() >= PENDING_MAX {
            return Err(CoreError::new(
                "Too many position lookups are pending. Try again shortly.",
            ));
        }
        let (send, receive) = watch::channel(None);
        lock(&self.pending).insert(key.clone(), receive);
        let result = self.load(kind, &fen, &options, &key, saved.as_ref()).await;
        let _ = send.send(Some(result.clone()));
        lock(&self.pending).remove(&key);
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
            self.cache.write(
                key,
                &serde_json::to_value(&result).map_err(|cause| {
                    CoreError::new(format!("Position lookup could not be saved: {cause}"))
                })?,
            )?;
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
                query.append_pair("recentGames", "0");
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
                    query.append_pair("recentGames", "8");
                }
                Kind::Tablebase => {}
            }
        }
        Ok(url)
    }

    /// Requests a lookup and reads its body, bounded by the kind's size limit.
    async fn fetch(&self, kind: Kind, fen: &str, options: &LookupOptions) -> CoreResult<String> {
        let url = self.url(kind, fen, options)?;
        let mut response = self.http.get(url).send().await.map_err(|_| {
            CoreError::new("Position lookup failed. Check your connection and retry.")
        })?;
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
        let valid = id.len() == 8 && id.bytes().all(|b| b.is_ascii_alphanumeric());
        if !valid {
            return Err(CoreError::new("Invalid id."));
        }
        let url = format!("{}/{id}", self.endpoints.masters_game);
        let attempt = async {
            let response = self
                .http
                .get(url)
                .send()
                .await
                .map_err(|_| CoreError::new("The master game could not be loaded."))?;
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

fn tablebase_result(fen: &str, now: i64, raw: &Value) -> CoreResult<Cached> {
    let unreadable = || CoreError::new("Position lookup returned unreadable data.");
    let category = category_of(raw.get("category")).ok_or_else(unreadable)?;
    let dtz = dtz_of(raw.get("dtz")).ok_or_else(unreadable)?;
    let items = raw
        .get("moves")
        .and_then(Value::as_array)
        .filter(|items| items.len() <= MAX_MOVES)
        .ok_or_else(unreadable)?;
    let mut moves = Vec::with_capacity(items.len());
    for item in items {
        let uci = uci_of(item.get("uci")).ok_or_else(unreadable)?;
        let entry_category = category_of(item.get("category")).ok_or_else(unreadable)?;
        let entry_dtz = dtz_of(item.get("dtz")).ok_or_else(unreadable)?;
        let san = legal_san(fen, &uci)
            .ok_or_else(|| CoreError::new("Position lookup returned an illegal move."))?;
        moves.push(LookupMove {
            uci,
            san,
            category: Some(entry_category),
            dtz: Some(entry_dtz),
            ..LookupMove::default()
        });
    }
    Ok(Cached {
        kind: "tablebase".into(),
        fen: fen.to_string(),
        fetched_at: now,
        total: None,
        opening: None,
        category: Some(category),
        dtz: Some(dtz),
        games: None,
        moves,
    })
}

fn explorer_result(kind: Kind, fen: &str, now: i64, raw: &Value) -> CoreResult<Cached> {
    let unreadable = || CoreError::new("Position lookup returned unreadable data.");
    let count = |value: Option<&Value>| -> Result<Option<i64>, CoreError> {
        match value {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_i64()
                .filter(|n| *n >= 0)
                .map(Some)
                .ok_or_else(unreadable),
        }
    };
    let items = raw
        .get("moves")
        .and_then(Value::as_array)
        .filter(|items| items.len() <= MAX_MOVES)
        .ok_or_else(unreadable)?;
    let (white, draws, black) = (
        count(raw.get("white"))?.unwrap_or(0),
        count(raw.get("draws"))?.unwrap_or(0),
        count(raw.get("black"))?.unwrap_or(0),
    );
    let opening = match raw.get("opening") {
        None | Some(Value::Null) => None,
        Some(opening) => {
            let name = opening
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(unreadable)?;
            let eco = opening
                .get("eco")
                .and_then(Value::as_str)
                .ok_or_else(unreadable)?;
            if name.chars().count() > 200 {
                return Err(unreadable());
            }
            Some(format!("{eco} · {name}"))
        }
    };
    let mut moves = Vec::with_capacity(items.len());
    for item in items {
        let uci = uci_of(item.get("uci")).ok_or_else(unreadable)?;
        // Each move's counts are required, unlike the position's own totals.
        let required = |key: &str| count(item.get(key))?.ok_or_else(unreadable);
        let (entry_white, entry_draws, entry_black) =
            (required("white")?, required("draws")?, required("black")?);
        let san = legal_san(fen, &uci)
            .ok_or_else(|| CoreError::new("Position lookup returned an illegal move."))?;
        moves.push(LookupMove {
            uci,
            san,
            white: Some(entry_white),
            draws: Some(entry_draws),
            black: Some(entry_black),
            ..LookupMove::default()
        });
    }
    let mut listed: Vec<&Value> = Vec::new();
    for key in ["topGames", "recentGames"] {
        if let Some(games) = raw.get(key).and_then(Value::as_array) {
            if games.len() > MAX_CACHED_GAMES {
                return Err(unreadable());
            }
            listed.extend(games.iter());
        }
    }
    listed.truncate(12);
    let mut games = Vec::with_capacity(listed.len());
    for game in listed {
        games.push(explorer_game(game, fen, &unreadable)?);
    }
    let total = white + draws + black;
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

fn explorer_game(
    game: &Value,
    fen: &str,
    unreadable: &dyn Fn() -> CoreError,
) -> CoreResult<ExplorerGame> {
    let id = game
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(unreadable)?;
    if id.len() != 8 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(unreadable());
    }
    let player = |key: &str| -> Result<(String, Option<f64>), CoreError> {
        let side = game.get(key).ok_or_else(unreadable)?;
        let name = side
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(unreadable)?;
        if name.chars().count() > 80 {
            return Err(unreadable());
        }
        let rating = side.get("rating").and_then(|value| match value {
            Value::Null => None,
            other => other.as_f64(),
        });
        Ok((name.to_string(), rating))
    };
    let (white, white_rating) = player("white")?;
    let (black, black_rating) = player("black")?;
    let winner = match game.get("winner").and_then(Value::as_str) {
        Some(color @ ("white" | "black")) => Some(color.to_string()),
        _ => None,
    };
    // The move a listed game played from this position, as SAN, when it is legal there.
    let san = game
        .get("uci")
        .and_then(Value::as_str)
        .filter(|uci| uci.len() <= 5)
        .and_then(|uci| legal_san(fen, uci));
    Ok(ExplorerGame {
        id: id.to_string(),
        white,
        black,
        white_rating,
        black_rating,
        winner,
        year: game.get("year").and_then(Value::as_i64),
        month: game
            .get("month")
            .and_then(Value::as_str)
            .map(str::to_string),
        san,
    })
}

/// `decodeLookupCache`: a saved lookup with the shape this module writes, or None.
fn decode_cached(raw: Option<&Value>) -> Option<Cached> {
    let raw = raw?;
    let object = raw.as_object()?;
    let kind = object.get("kind")?.as_str()?;
    Kind::parse(kind)?;
    let fen = object.get("fen")?.as_str()?;
    if fen.len() > 100 {
        return None;
    }
    let fetched_at = object.get("fetchedAt")?.as_i64().filter(|n| *n >= 0)?;
    let entries = object.get("moves")?.as_array()?;
    if entries.len() > MAX_MOVES {
        return None;
    }
    let mut moves = Vec::with_capacity(entries.len());
    for entry in entries {
        let uci = entry.get("uci")?.as_str()?;
        let san = entry.get("san")?.as_str()?;
        if uci.len() > 5 || san.len() > 32 {
            return None;
        }
        moves.push(serde_json::from_value::<LookupMove>(entry.clone()).ok()?);
    }
    let games = match object.get("games") {
        None => None,
        Some(games) => {
            let list = games.as_array()?;
            if list.len() > MAX_CACHED_GAMES {
                return None;
            }
            Some(
                list.iter()
                    .map(|game| serde_json::from_value::<ExplorerGame>(game.clone()).ok())
                    .collect::<Option<Vec<_>>>()?,
            )
        }
    };
    Some(Cached {
        kind: kind.to_string(),
        fen: fen.to_string(),
        fetched_at,
        total: object.get("total").and_then(Value::as_i64),
        opening: object
            .get("opening")
            .and_then(Value::as_str)
            .map(str::to_string),
        category: object
            .get("category")
            .and_then(Value::as_str)
            .map(str::to_string),
        dtz: match object.get("dtz") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(value) => Some(Some(value.as_i64()?)),
        },
        games,
        moves,
    })
}

fn category_of(value: Option<&Value>) -> Option<String> {
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
    let text = value?.as_str()?;
    CATEGORIES.contains(&text).then(|| text.to_string())
}

fn dtz_of(value: Option<&Value>) -> Option<Option<i64>> {
    match value {
        None => Some(None),
        Some(Value::Null) => Some(None),
        Some(value) => value.as_i64().map(Some),
    }
}

fn uci_of(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    (text.len() <= 5).then(|| text.to_string())
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
