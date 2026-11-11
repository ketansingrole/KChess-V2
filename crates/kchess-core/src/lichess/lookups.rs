//! Lichess's cloud evaluation and the explicit position lookups
//! of the opening explorer, masters, player and tablebase databases.
//!
//! The saved lookups live behind `LookupCache` (the `store::lookups` table in production). Legal
//! moves and SAN come from `kchess_domain::replay`, and every remote answer is validated before it
//! is trusted: a move that is not legal from its position fails the lookup.

use kchess_domain::variant::Variant;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kchess_contracts::check::EXPLORER_RATINGS;
use kchess_contracts::types::{
    CloudEval, CloudLine, ExplorerGame, ExplorerSpeed, LookupMove, LookupOptions, PositionLookup,
    PositionLookupKind as Kind, SavedLookup as Cached, UsageKind,
};

use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::client::{Auth, Lane, LichessClient};
use super::decode::{Loose, Text, read};
use super::policy::{Clock, Policy, timed_out};
use super::reviews::split_whitespace;
use crate::host::Host;
use kchess_contracts::error::{CoreError, ErrorKind, Result as CoreResult};
use kchess_domain::color::{self, Color};
use kchess_domain::{js, position, replay};

mod opening_info;

const CLOUD_TTL: Duration = Duration::from_secs(10 * 60);
const CLOUD_MAX: usize = 500;
const PENDING_MAX: usize = 16;
const DEFAULT_OPENING: &str = r#"{"speeds":["blitz","rapid","classical"]}"#;
const MAX_CACHED_GAMES: usize = 30;
const MAX_MOVES: usize = 256;
/// How long a lookup's answer may take by default (the player database's stream excepted).
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the player database may take to finish its stream of ever more complete results.
const PLAYER_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_EXPLORER: &str = "Connect a Lichess account in Settings to use the opening explorer.";

/* ── Cloud evaluation ── */

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

    /// Forgets every cached evaluation (`lichess.reset`, when the core closes).
    pub fn clear(&self) {
        lock(&self.entries).clear();
    }
}

use crate::tasks::lock;

/// Lichess's cloud evaluation, for positions someone analysed deeply on
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
    let raw = lichess
        .client()
        .get("/api/cloud-eval", &lichess.anonymous(UsageKind::Analysis))
        .query(&[("fen", fen.to_string()), ("multiPv", lines.to_string())])
        .read_json::<Value>()
        .await;
    let raw = match raw {
        Ok(raw) => raw,
        Err(cause) if cause.kind == Some(ErrorKind::NotFound) => {
            cache.set(key, None);
            return Ok(None);
        }
        Err(cause) => return Err(cause),
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
            rank: index as u32 + 1,
            depth: data.depth,
            // `cp` wins when both are sent.
            cp: pv.cp.0,
            mate: pv.cp.0.is_none().then_some(pv.mate.0).flatten(),
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

#[derive(Deserialize)]
struct CloudPv {
    #[serde(default)]
    cp: Loose<i64>,
    #[serde(default)]
    mate: Loose<i64>,
    moves: String,
}

#[derive(Deserialize)]
struct CloudData {
    depth: u64,
    knodes: u64,
    pvs: Vec<CloudPv>,
}

/// The cloud evaluation's shape: whole non-negative depth and nodes, at most five lines, each
/// with a whole score (`cp` or `mate`) and at most 2000 characters of moves.
fn decode_cloud(raw: &Value) -> CoreResult<CloudData> {
    read::<CloudData>(raw)
        .filter(|data| {
            data.pvs.len() <= 5
                && data
                    .pvs
                    .iter()
                    .all(|pv| pv.moves.len() <= 2000 && (pv.cp.0.is_some() || pv.mate.0.is_some()))
        })
        .ok_or_else(|| CoreError::new("Lichess sent an unreadable cloud evaluation."))
}

/* ── Position lookups ── */

/// The stored saved-lookup cache: the newest entries, as the store keeps them.
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

/// How long a saved lookup is fresh: master games change rarely, a player's games often.
fn fresh_ms(kind: Kind) -> i64 {
    match kind {
        Kind::Opening => 86_400_000,
        Kind::Masters | Kind::Tablebase => 30 * 86_400_000,
        Kind::Player => 3_600_000,
    }
}

fn max_bytes(kind: Kind) -> usize {
    match kind {
        Kind::Player => 4_000_000,
        _ => 512_000,
    }
}

/// The origins the lookups are sent to. Production uses the Lichess explorer and tablebase.
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// The opening explorer: `/lichess`, `/masters`, `/player`, `/masters/pgn/{id}` and
    /// `/lichess/history` under it.
    pub explorer: String,
    /// The tablebase: `/standard` under it.
    pub tablebase: String,
    /// The Wikibooks API, for opening descriptions.
    pub wikibooks: String,
}

impl Endpoints {
    /// The endpoints under a test origin (`KCHESS_TEST_LICHESS_BASE`), with the same paths as
    /// production.
    pub fn at(origin: &str) -> Endpoints {
        Endpoints {
            explorer: format!("{origin}/explorer"),
            tablebase: format!("{origin}/tablebase"),
            wikibooks: format!("{origin}/wikibooks/w/api.php"),
        }
    }
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        Endpoints {
            explorer: "https://explorer.lichess.org".into(),
            tablebase: "https://tablebase.lichess.org".into(),
            wikibooks: "https://en.wikibooks.org/w/api.php".into(),
        }
    }
}

type Pending = watch::Receiver<Option<Result<PositionLookup, CoreError>>>;

/// The position explorer: deduplicated and bounded lookups with validated legal moves, stale
/// recovery when a refresh fails, and the master game PGNs.
pub struct PositionLookups {
    host: Arc<dyn Host>,
    http: reqwest::Client,
    endpoints: Endpoints,
    /// The explorer and the tablebase, each with its own policy: a 429 from one pauses only it.
    explorer: LichessClient,
    tablebase: LichessClient,
    cache: Arc<dyn LookupCache>,
    now: Clock,
    pending: Mutex<HashMap<String, Pending>>,
    /// The login the explorer is asked with; None sends no login (tests).
    login: Option<Arc<dyn ExplorerLogin>>,
    lifetime: CancellationToken,
    /// How long a lookup's answer may take (the player database's stream excepted).
    timeout: Duration,
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
/// to use, and its token. Tablebase lookups never carry it.
pub trait ExplorerLogin: Send + Sync {
    /// The token of the explorer's account, or None when no account is set or it has no login.
    fn token(&self) -> BoxFuture<'_, CoreResult<Option<String>>>;
}

impl PositionLookups {
    /// `now` is the clock in ms since the epoch; it also times the origins' 429 cooldowns.
    pub fn new(
        host: Arc<dyn Host>,
        http: reqwest::Client,
        endpoints: Endpoints,
        cache: Arc<dyn LookupCache>,
        now: Clock,
    ) -> PositionLookups {
        let lifetime = CancellationToken::new();
        let (explorer, tablebase) =
            clients(&host, &http, &endpoints, &lifetime, &now, LOOKUP_TIMEOUT);
        PositionLookups {
            host,
            http,
            endpoints,
            explorer,
            tablebase,
            cache,
            now,
            pending: Mutex::new(HashMap::new()),
            login: None,
            lifetime,
            timeout: LOOKUP_TIMEOUT,
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
        self.rebuild()
    }

    /// How long a lookup's answer may take (the tests' unanswered lookups).
    pub fn with_timeout(mut self, timeout: Duration) -> PositionLookups {
        self.timeout = timeout;
        self.rebuild()
    }

    fn rebuild(mut self) -> PositionLookups {
        (self.explorer, self.tablebase) = clients(
            &self.host,
            &self.http,
            &self.endpoints,
            &self.lifetime,
            &self.now,
            self.timeout,
        );
        self
    }

    /// Who the explorer (not the tablebase) is asked as: the explorer's login when one is set.
    async fn explorer_auth(&self) -> CoreResult<Auth> {
        let auth = self.explorer.anonymous(UsageKind::Analysis);
        let Some(login) = &self.login else {
            return Ok(auth);
        };
        let token = login
            .token()
            .await?
            .ok_or_else(|| CoreError::new(CONNECT_EXPLORER).with_kind(ErrorKind::Login))?;
        Ok(auth.with_token(token))
    }

    /// The position `fen` looked up in `kind`'s database with `options` (checked as the
    /// method's arguments are).
    pub async fn lookup(
        &self,
        kind: Kind,
        fen: &str,
        options: Option<&LookupOptions>,
    ) -> CoreResult<PositionLookup> {
        tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => Err(CoreError::aborted("Position lookup was cancelled.")),
            result = self.lookup_inner(kind, fen, options) => result,
        }
    }

    async fn lookup_inner(
        &self,
        kind: Kind,
        fen: &str,
        options: Option<&LookupOptions>,
    ) -> CoreResult<PositionLookup> {
        let options = normalize_options(kind, options.unwrap_or(&LookupOptions::default()))?;
        let variant = options.variant.unwrap_or_default();
        if !is_legal_position(variant, fen) {
            return Err(CoreError::new("That position is not legal."));
        }
        if kind == Kind::Tablebase {
            let (pieces, castling) = material(fen);
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
                format!("{}:{fen}", kind.as_str())
            } else {
                format!("{}:{filters}:{fen}", kind.as_str())
            };
        let candidate = decode_cached(self.cache.read(&key).await?.as_ref());
        let saved = candidate.filter(|candidate| {
            candidate.fen == fen
                && candidate.kind == kind
                && candidate.moves.iter().all(|entry| {
                    legal_san(variant, fen, &entry.uci).as_deref() == Some(entry.san.as_str())
                })
        });
        if let Some(saved) = &saved
            && (self.now)() - saved.fetched_at < fresh_ms(kind)
        {
            return Ok(PositionLookup {
                saved: saved.clone(),
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
        let result = self.load(kind, fen, &options, &key, saved.as_ref()).await;
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
            let text = self.fetch(kind, fen, options).await?;
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
                explorer_result(
                    kind,
                    options.variant.unwrap_or_default(),
                    fen,
                    (self.now)(),
                    &raw,
                )?
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
                saved: result,
                cached: false,
                stale: false,
                message: None,
            })
        };
        match attempt.await {
            Ok(result) => Ok(result),
            Err(cause) => match saved {
                Some(saved) => Ok(PositionLookup {
                    saved: saved.clone(),
                    cached: true,
                    stale: true,
                    message: Some("Could not refresh. Showing the saved lookup.".into()),
                }),
                None => Err(cause),
            },
        }
    }

    /// The query of a lookup, with the filters the database understands.
    fn query(kind: Kind, fen: &str, options: &LookupOptions) -> Vec<(&'static str, String)> {
        let mut query = Pairs::default();
        {
            query.append_pair("fen", fen);
            if kind != Kind::Tablebase {
                query.append_pair("moves", "12");
                // Once per URL: Lichess reads the first of repeated parameters.
                let recent = if kind == Kind::Player { "8" } else { "0" };
                query.append_pair("recentGames", recent);
            }
            match kind {
                Kind::Opening => {
                    query.append_pair("variant", options.variant.unwrap_or_default().key());
                    query.append_pair(
                        "speeds",
                        &speeds(options.speeds.as_deref().unwrap_or_default()),
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
                    let side = options.color.unwrap_or(Color::White);
                    query.append_pair("color", color::name(side));
                    if let Some(variant) = options.variant {
                        query.append_pair("variant", variant.key());
                    }
                    if let Some(listed) = options.speeds.as_ref().filter(|s| !s.is_empty()) {
                        query.append_pair("speeds", &speeds(listed));
                    }
                    if let Some(modes) = options.modes.as_ref().filter(|m| !m.is_empty()) {
                        let modes: Vec<&str> = modes.iter().map(|mode| mode.as_str()).collect();
                        query.append_pair("modes", &modes.join(","));
                    }
                    if let Some(since) = &options.since {
                        query.append_pair("since", since);
                    }
                }
                Kind::Tablebase => {}
            }
        }
        query.0
    }

    /// Requests a lookup and reads its body, bounded by the kind's size limit.
    async fn fetch(&self, kind: Kind, fen: &str, options: &LookupOptions) -> CoreResult<String> {
        let query = PositionLookups::query(kind, fen, options);
        let failed = |cause: CoreError| lookup_error(cause, kind, options);
        let (client, auth, path) = match kind {
            Kind::Tablebase => (
                &self.tablebase,
                self.tablebase.anonymous(UsageKind::Analysis),
                "/standard",
            ),
            Kind::Opening => (&self.explorer, self.explorer_auth().await?, "/lichess"),
            Kind::Masters => (&self.explorer, self.explorer_auth().await?, "/masters"),
            Kind::Player => (&self.explorer, self.explorer_auth().await?, "/player"),
        };
        let request = client
            .get(path, &auth)
            .query(&query)
            .max_bytes(max_bytes(kind), "Position lookup returned too much data.");
        if kind != Kind::Player {
            return request.read_text(None).await.map_err(failed);
        }
        // The player database streams its answer, so only the whole of it is bounded.
        let read = request.lane(Lane::Feed).read_text(None);
        match tokio::time::timeout(PLAYER_TIMEOUT, read).await {
            Ok(text) => text.map_err(failed),
            Err(_) => Err(timed_out()),
        }
    }

    /// A master game's PGN; master games never change.
    pub async fn masters_game(&self, id: &str) -> CoreResult<String> {
        tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => Err(CoreError::aborted("Position lookup was cancelled.")),
            result = self.masters_game_inner(id) => result,
        }
    }

    async fn masters_game_inner(&self, id: &str) -> CoreResult<String> {
        if !super::is_lichess_id(id) {
            return Err(CoreError::new("Invalid id."));
        }
        let text = self
            .explorer
            .get(&format!("/masters/pgn/{id}"), &self.explorer_auth().await?)
            .read_text(None)
            .await
            .map_err(|cause| match cause.kind {
                Some(ErrorKind::Login) => {
                    CoreError::new(CONNECT_EXPLORER).with_kind(ErrorKind::Login)
                }
                Some(ErrorKind::RateLimited | ErrorKind::TimedOut) => cause,
                _ if cause.aborted => cause,
                _ => CoreError {
                    message: "The master game could not be loaded.".into(),
                    ..cause
                },
            })?;
        if js::utf16_len(&text) > 200_000 || !text.contains('[') {
            return Err(CoreError::new("That game is not a PGN."));
        }
        Ok(text)
    }
}

/// What a person is told when a lookup fails: never Lichess's status, and a refused login asks to
/// connect an account only for the explorer, which needs one.
fn lookup_error(cause: CoreError, kind: Kind, options: &LookupOptions) -> CoreError {
    match cause.kind {
        Some(ErrorKind::Login) if kind != Kind::Tablebase => {
            CoreError::new(CONNECT_EXPLORER).with_kind(ErrorKind::Login)
        }
        Some(ErrorKind::Offline) => CoreError {
            message: "Position lookup failed. Check your connection and retry.".into(),
            ..cause
        },
        Some(ErrorKind::NotFound) if kind == Kind::Player => CoreError {
            message: format!(
                "No Lichess player named \"{}\".",
                options.player.as_deref().unwrap_or("")
            ),
            ..cause
        },
        _ => cause,
    }
}

/// Query pairs in the order they are added.
#[derive(Default)]
struct Pairs(Vec<(&'static str, String)>);

impl Pairs {
    fn append_pair(&mut self, key: &'static str, value: &str) {
        self.0.push((key, value.to_string()));
    }
}

/// The explorer's and the tablebase's clients, stopped by `lifetime`.
fn clients(
    host: &Arc<dyn Host>,
    http: &reqwest::Client,
    endpoints: &Endpoints,
    lifetime: &CancellationToken,
    now: &Clock,
    timeout: Duration,
) -> (LichessClient, LichessClient) {
    let client = |base: &str| {
        let policy = Policy::for_lookups(
            Arc::clone(host),
            http.clone(),
            lifetime.clone(),
            timeout,
            Arc::clone(now),
        );
        LichessClient::with_base(base, policy)
    };
    (client(&endpoints.explorer), client(&endpoints.tablebase))
}

/// The speeds as a query lists them.
fn speeds(speeds: &[ExplorerSpeed]) -> String {
    let names: Vec<&str> = speeds.iter().map(|speed| speed.as_str()).collect();
    names.join(",")
}

/// `speeds` in the order the contract lists them, each once.
fn in_order(speeds: &[ExplorerSpeed]) -> Vec<ExplorerSpeed> {
    ExplorerSpeed::ALL
        .iter()
        .copied()
        .filter(|speed| speeds.contains(speed))
        .collect()
}

/// The filters each database understands, in a fixed order; standard chess has no variant.
fn normalize_options(kind: Kind, options: &LookupOptions) -> CoreResult<LookupOptions> {
    let variant = options
        .variant
        .filter(|variant| *variant != Variant::Standard);
    if variant.is_some() && matches!(kind, Kind::Masters | Kind::Tablebase) {
        return Err(CoreError::new(
            "Masters games and tablebases are standard chess only.",
        ));
    }
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
                options
                    .speeds
                    .as_deref()
                    .filter(|speeds| !speeds.is_empty())
                    .map_or_else(
                        || {
                            vec![
                                ExplorerSpeed::Blitz,
                                ExplorerSpeed::Rapid,
                                ExplorerSpeed::Classical,
                            ]
                        },
                        in_order,
                    ),
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
            variant,
            ..LookupOptions::default()
        }),
        Kind::Player => {
            let Some(player) = options.player.clone().filter(|p| !p.is_empty()) else {
                return Err(CoreError::new("Choose whose games to explore."));
            };
            Ok(LookupOptions {
                player: Some(player),
                color: Some(options.color.unwrap_or(Color::White)),
                speeds: options
                    .speeds
                    .as_deref()
                    .filter(|speeds| !speeds.is_empty())
                    .map(in_order),
                modes: options.modes.clone().filter(|modes| !modes.is_empty()),
                since: options.since.clone(),
                variant,
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
            fields.push(text("color", json!(options.color.map_or("", color::name))));
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
    if let Some(variant) = &options.variant {
        fields.push(text("variant", json!(variant)));
    }
    format!("{{{}}}", fields.join(","))
}

/// The rating groups, comma-separated.
fn join_numbers(values: &[u32]) -> String {
    let listed: Vec<String> = values.iter().map(u32::to_string).collect();
    listed.join(",")
}

/// A legal position of `variant`.
fn is_legal_position(variant: Variant, fen: &str) -> bool {
    if variant == Variant::Standard {
        position::fen_problem(fen).is_none()
    } else {
        kchess_domain::rules::setup_start(variant, fen).is_some()
    }
}

/// The piece count and whether any castling right remains, for the tablebase limits.
fn material(fen: &str) -> (usize, bool) {
    let Some(replay) = replay::replay_setup::<&str>(Variant::Standard, fen, &[]) else {
        return (usize::MAX, true);
    };
    let mut fields = replay.fen.split(' ');
    let board = fields.next().unwrap_or("");
    let pieces = board.chars().filter(|c| c.is_ascii_alphabetic()).count();
    let castling = fields.nth(1).is_some_and(|rights| rights != "-");
    (pieces, castling)
}

/// The SAN of a legal move from `fen`, or None when it is not legal there.
fn legal_san(variant: Variant, fen: &str, uci: &str) -> Option<String> {
    let replay = replay::replay_setup(variant, fen, &[uci])?;
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
        let san = legal_san(Variant::Standard, fen, &item.uci)
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
        kind: Kind::Tablebase,
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

fn explorer_result(
    kind: Kind,
    variant: Variant,
    fen: &str,
    now: i64,
    raw: &Value,
) -> CoreResult<Cached> {
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
        let san = legal_san(variant, fen, &item.uci)
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
        .map(|game| explorer_game(game, variant, fen).ok_or_else(unreadable))
        .collect::<CoreResult<Vec<_>>>()?;
    let total = [explorer.white, explorer.draws, explorer.black]
        .into_iter()
        .map(|n| count(n.unwrap_or(0)))
        .fold(0i64, i64::saturating_add);
    Ok(Cached {
        kind,
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
fn explorer_game(game: &Value, variant: Variant, fen: &str) -> Option<ExplorerGame> {
    let game: RawExplorerGame = read(game)?;
    let id_ok = super::is_lichess_id(&game.id);
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
        winner: game.winner.0.as_deref().and_then(color::from_name),
        // The move a listed game played from this position, as SAN, when it is legal there.
        san: game
            .uci
            .0
            .filter(|uci| uci.len() <= 5)
            .and_then(|uci| legal_san(variant, fen, &uci)),
        year: game.year.0,
        month: game.month.0,
    })
}

/// A saved lookup with the shape this module writes, or None.
fn decode_cached(raw: Option<&Value>) -> Option<Cached> {
    let cached: Cached = read(raw?)?;
    let fits = cached.fen.len() <= 100
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
