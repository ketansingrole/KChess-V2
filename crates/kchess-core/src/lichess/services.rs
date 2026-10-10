//! The Lichess services as one object the core owns: the Lichess service with its client, the
//! online session (with its challenge inbox), the spectator, the position explorer and the cloud
//! evaluation cache. `Core::call` sends every `lichess.*`, `online.*`, `challenges.*`,
//! `tournaments.*`, `studies.*`, `spectate.*`, `cloudEval.*`, `positionLookup.*` method here; each
//! method keeps the name and result of the TypeScript operation it replaces (`core/src/services`).

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::accounts::{Lichess, Reply};
use super::adapters::{DbExplorerLogin, DbLichessStore, DbLookups, DbTokens};
use super::challenges::ChallengeInfo;
use super::client::{LichessClient, TokenSource};
use super::lookups::{CloudCache, Endpoints, PositionLookups, cloud_eval};
use super::online::{OnlineAction, OnlineOptions, OnlineSession};
use super::policy::Policy;
use super::puzzles::{PuzzleRequest, PuzzleSolveRequest};
use super::reviews::{fetch_lichess_reviews, review_from_lichess};
use super::studies::StudySyncRequest;
use super::tournaments::NewArena;
use super::watch::{HostSink, Spectator, WatchTarget};
use crate::capabilities::Capabilities;
use crate::error::{CoreError, Result};
use crate::host::Host;
use crate::store::Database;
use crate::usage::UsageBatch;

/// The request deadline of the Lichess client (`requestPolicy.ts`).
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The Lichess services of one core.
pub struct Services {
    lichess: Arc<Lichess>,
    tokens: Arc<dyn TokenSource>,
    online: OnlineSession,
    spectator: Spectator,
    lookups: PositionLookups,
    cloud: CloudCache,
    database: Arc<Database>,
    usage: Arc<UsageBatch>,
    capabilities: Arc<Capabilities>,
    lifetime: CancellationToken,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountDays {
    account: String,
    days: u32,
}

/// What the engines do with a live online game: `stops` ends the computer, the analysis and the
/// review engine; `busy` is set while assistance is blocked (read by the review queue).
pub struct LiveGame {
    pub stops: Arc<dyn Fn() + Send + Sync>,
    pub busy: Arc<std::sync::atomic::AtomicBool>,
}

impl Services {
    /// The services over the core's database, capabilities and host (`host` is the usage-batching
    /// host, so the policy's counters reach `kchess.db` through the batch).
    pub fn new(
        host: Arc<dyn Host>,
        database: Arc<Database>,
        capabilities: Arc<Capabilities>,
        usage: Arc<UsageBatch>,
        lifetime: CancellationToken,
        origin: Option<String>,
        live: LiveGame,
    ) -> Services {
        let LiveGame {
            stops: game_stops,
            busy,
        } = live;
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let policy = Policy::new(Arc::clone(&host), http.clone(), lifetime.clone());
        // A test origin replaces Lichess for every request (`origin` is only set by the tests).
        let (client, endpoints) = match &origin {
            Some(base) => (
                LichessClient::with_base(base, Arc::clone(&policy), http.clone()),
                Endpoints::at(base),
            ),
            None => (
                LichessClient::new(Arc::clone(&policy), http.clone()),
                Endpoints::default(),
            ),
        };
        let tokens: Arc<dyn TokenSource> = Arc::new(DbTokens::new(
            Arc::clone(&database),
            Arc::clone(&capabilities),
        ));
        let store = Arc::new(DbLichessStore::new(Arc::clone(&database)));
        let lichess = Arc::new(Lichess::new(
            client,
            Arc::clone(&capabilities),
            store,
            Arc::clone(&tokens),
            Arc::clone(&host),
            lifetime.clone(),
        ));
        let online = OnlineSession::new(Arc::clone(&lichess), Arc::clone(&tokens), http.clone());
        let spectator = Spectator::new(Arc::clone(&lichess), Arc::new(HostSink::new(host.clone())));
        // Before each state is reported: a live game (or the startup check) stops the engines and the
        // watch, and the engines' busy state is taken from the session as it now is.
        {
            let watch = spectator.clone();
            online.set_state_hook(Arc::new(move |live_game, blocked| {
                if live_game {
                    game_stops();
                    watch.stop();
                }
                busy.store(blocked, std::sync::atomic::Ordering::SeqCst);
            }));
        }
        let lookups = PositionLookups::new(
            http,
            endpoints,
            Arc::new(DbLookups::new(Arc::clone(&database))),
            Arc::new(super::accounts::now_ms),
        )
        .with_login(Arc::new(DbExplorerLogin::new(
            Arc::clone(&database),
            Arc::clone(&tokens),
        )));
        Services {
            lichess,
            tokens,
            online,
            spectator,
            lookups,
            cloud: CloudCache::default(),
            database,
            usage,
            capabilities,
            lifetime,
        }
    }

    /// The Lichess service, for the review queue's analysis fetch.
    pub fn lichess(&self) -> &Arc<Lichess> {
        &self.lichess
    }

    /// The online session, for the engines' busy check and the facade.
    pub fn online(&self) -> &OnlineSession {
        &self.online
    }

    /// The spectator (the watch of a TV channel, a game or a broadcast round).
    pub fn spectator(&self) -> &Spectator {
        &self.spectator
    }

    /// Cancels the requests and streams in flight: Lichess calls, the usage counters' pending
    /// writes are kept for the core to flush. Called when the core closes.
    pub fn abort(&self) {
        self.lifetime.cancel();
    }

    /// Runs one asynchronous method. `None` when the method is not one of these services'.
    pub async fn call(&self, method: &str, args: &[Value]) -> Option<Result<Value>> {
        let result = match method {
            "lichess.syncGames" => self.sync_games(args).await,
            "lichess.connectLichess" => self.connect(args).await,
            "lichess.crosstable" => match (text(args, 0), text(args, 1)) {
                (Ok(a), Ok(b)) => to_json(self.lichess.crosstable(&a, &b).await),
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "lichess.ratingHistory" => match text(args, 0) {
                Ok(name) => self.lichess.rating_history(&name).await,
                Err(cause) => Err(cause),
            },
            "lichess.fetchLichessReviews" => match (text(args, 0), strings(args, 1)) {
                (Ok(account), Ok(ids)) => {
                    to_json(fetch_lichess_reviews(&self.lichess, &account, &ids).await)
                }
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "lichess.puzzleNext" => match parse::<PuzzleRequest>(args, 0, "request") {
                Ok(request) => {
                    reply_json(super::puzzles::puzzle_next(&self.lichess, &request).await)
                }
                Err(cause) => Err(cause),
            },
            "lichess.puzzleSolve" => match parse::<PuzzleSolveRequest>(args, 0, "request") {
                Ok(request) => {
                    reply_json(super::puzzles::puzzle_solve(&self.lichess, &request).await)
                }
                Err(cause) => Err(cause),
            },
            "lichess.puzzleDaily" => super::puzzles::puzzle_daily(&self.lichess).await,
            "lichess.puzzleDashboard" => match parse::<AccountDays>(args, 0, "request") {
                Ok(request) => reply_json(
                    super::puzzles::puzzle_dashboard(&self.lichess, &request.account, request.days)
                        .await,
                ),
                Err(cause) => Err(cause),
            },
            "lichess.puzzleActivity" => match (text(args, 0), arg_u32(args, 1)) {
                (Ok(account), Ok(max)) => {
                    reply_json(super::puzzles::puzzle_activity(&self.lichess, &account, max).await)
                }
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "lichess.stormDashboard" => match (text(args, 0), arg_u32(args, 1)) {
                (Ok(name), Ok(days)) => {
                    super::puzzles::storm_dashboard(&self.lichess, &name, days).await
                }
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "lichess.followedUsers" => to_json(self.lichess.followed_users().await),
            "lichess.primeProfiles" => match strings(args, 0) {
                Ok(names) => self.lichess.prime_profiles(&names).map(|()| Value::Null),
                Err(cause) => Err(cause),
            },
            "lichess.profile" => match text(args, 0) {
                Ok(name) => self.lichess.profile(&name).await,
                Err(cause) => Err(cause),
            },
            "lichess.playerPerf" => match (text(args, 0), text(args, 1)) {
                (Ok(name), Ok(perf)) => to_json(self.lichess.player_perf(&name, &perf).await),
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "lichess.exportGame" => match text(args, 0) {
                Ok(id) => to_json(self.lichess.export_game(&id).await),
                Err(cause) => Err(cause),
            },
            "lichess.recentGames" => match (text(args, 0), args.get(1).and_then(Value::as_bool)) {
                (Ok(name), rated) => to_json(
                    self.lichess
                        .recent_games(&name, rated.unwrap_or(false))
                        .await,
                ),
                (Err(cause), _) => Err(cause),
            },
            "lichess.sendMessage" => match (text(args, 0), text(args, 1), text(args, 2)) {
                (Ok(account), Ok(name), Ok(body)) => {
                    match self.lichess.send_message(&account, &name, &body).await {
                        Ok(Reply::Done(())) => Ok(json!({ "sent": true })),
                        Ok(Reply::NeedsReconnect) => Ok(json!({ "needsReconnect": true })),
                        Err(cause) => Err(cause),
                    }
                }
                (Err(cause), _, _) | (_, Err(cause), _) | (_, _, Err(cause)) => Err(cause),
            },
            "lichess.reset" => {
                self.cloud.clear();
                self.lichess.reset();
                Ok(Value::Null)
            }
            "online.start" => match parse::<OnlineOptions>(args, 0, "options") {
                Ok(options) => to_json(self.online.start(&options).await),
                Err(cause) => Err(cause),
            },
            "online.resume" => to_json(self.online.resume().await.map(|resumed| {
                resumed.map(|found| json!({ "id": found.id, "account": found.account }))
            })),
            "online.ongoing" => to_json(self.online.ongoing().await),
            "online.open" => match (text(args, 0), text(args, 1)) {
                (Ok(account), Ok(id)) => self
                    .online
                    .open(&account, &id)
                    .await
                    .map(|()| Value::Null)
                    .map_err(CoreError::from),
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "online.attach" => match (text(args, 0), text(args, 1)) {
                (Ok(account), Ok(id)) => self
                    .online
                    .attach(&account, &id)
                    .await
                    .map(|()| Value::Null)
                    .map_err(CoreError::from),
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "online.acceptChallenge" => match parse::<ChallengeInfo>(args, 0, "challenge") {
                Ok(challenge) => self
                    .online
                    .accept_challenge(&challenge)
                    .await
                    .map(|()| Value::Null)
                    .map_err(CoreError::from),
                Err(cause) => Err(cause),
            },
            "online.declineChallenge" => {
                match (parse::<ChallengeInfo>(args, 0, "challenge"), text(args, 1)) {
                    (Ok(challenge), Ok(reason)) => self
                        .online
                        .decline_challenge(&challenge, &reason)
                        .await
                        .map(|()| Value::Null)
                        .map_err(CoreError::from),
                    (Err(cause), _) | (_, Err(cause)) => Err(cause),
                }
            }
            "online.withdrawChallenge" => match parse::<ChallengeInfo>(args, 0, "challenge") {
                Ok(challenge) => self
                    .online
                    .withdraw_challenge(&challenge)
                    .await
                    .map(|()| Value::Null)
                    .map_err(CoreError::from),
                Err(cause) => Err(cause),
            },
            "online.move" => match (text(args, 0), text(args, 1)) {
                (Ok(id), Ok(uci)) => self
                    .online
                    .move_uci(&id, &uci)
                    .await
                    .map(|()| Value::Null)
                    .map_err(CoreError::from),
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "online.chat" => match text(args, 0) {
                Ok(id) => to_json(self.online.chat(&id).await),
                Err(cause) => Err(cause),
            },
            "online.sendChat" => match (text(args, 0), text(args, 1), text(args, 2)) {
                (Ok(id), Ok(room), Ok(body)) => self
                    .online
                    .send_chat(&id, &room, &body)
                    .await
                    .map(|()| Value::Null)
                    .map_err(CoreError::from),
                (Err(cause), _, _) | (_, Err(cause), _) | (_, _, Err(cause)) => Err(cause),
            },
            "online.presence" => match strings(args, 0) {
                Ok(names) => to_json(self.online.presence(&names).await),
                Err(cause) => Err(cause),
            },
            "online.action" => match (text(args, 0), parse::<OnlineAction>(args, 1, "action")) {
                (Ok(id), Ok(action)) => self
                    .online
                    .action(&id, action)
                    .await
                    .map(|()| Value::Null)
                    .map_err(CoreError::from),
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "online.cancel" => {
                self.online.cancel();
                Ok(Value::Null)
            }
            "online.close" => {
                self.online.close();
                Ok(Value::Null)
            }
            "online.logout" => {
                match args.first().filter(|value| !value.is_null()) {
                    None => self.online.logout(None),
                    Some(_) => {
                        let accounts: Vec<String> = match parse(args, 0, "accounts") {
                            Ok(accounts) => accounts,
                            Err(cause) => return Some(Err(cause)),
                        };
                        self.online.logout(Some(&accounts));
                    }
                }
                Ok(Value::Null)
            }
            "online.stayConnected" => match text(args, 0) {
                Ok(account) => {
                    self.online.stay_connected(&account);
                    Ok(Value::Null)
                }
                Err(cause) => Err(cause),
            },
            "tournaments.list" => match text(args, 0) {
                Ok(account) => {
                    to_json(super::tournaments::tournaments(&self.lichess, &account).await)
                }
                Err(cause) => Err(cause),
            },
            "tournaments.get" => match (
                text(args, 0),
                text(args, 1),
                text(args, 2),
                arg_u32(args, 3),
            ) {
                (Ok(system), Ok(id), Ok(account), Ok(page)) => to_json(
                    super::tournaments::tournament(
                        &self.lichess,
                        self.tokens.as_ref(),
                        &system,
                        &id,
                        &account,
                        page,
                    )
                    .await,
                ),
                (Err(cause), _, _, _)
                | (_, Err(cause), _, _)
                | (_, _, Err(cause), _)
                | (_, _, _, Err(cause)) => Err(cause),
            },
            "tournaments.join" => match (text(args, 0), text(args, 1), text(args, 2)) {
                (Ok(system), Ok(id), Ok(account)) => {
                    let password = optional_text(args, 3);
                    reply_true(
                        super::tournaments::join_tournament(
                            &self.lichess,
                            &system,
                            &id,
                            &account,
                            password.as_deref(),
                        )
                        .await,
                    )
                }
                (Err(cause), _, _) | (_, Err(cause), _) | (_, _, Err(cause)) => Err(cause),
            },
            "tournaments.leave" => match (text(args, 0), text(args, 1), text(args, 2)) {
                (Ok(system), Ok(id), Ok(account)) => reply_true(
                    super::tournaments::leave_tournament(&self.lichess, &system, &id, &account)
                        .await,
                ),
                (Err(cause), _, _) | (_, Err(cause), _) | (_, _, Err(cause)) => Err(cause),
            },
            "tournaments.create" => match (text(args, 0), parse::<NewArena>(args, 1, "arena")) {
                (Ok(account), Ok(arena)) => reply_json(
                    super::tournaments::create_tournament(&self.lichess, &account, &arena).await,
                ),
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "studies.list" => match text(args, 0) {
                Ok(account) => {
                    reply_json(super::studies::lichess_studies(&self.lichess, &account).await)
                }
                Err(cause) => Err(cause),
            },
            "studies.chapters" => match (text(args, 0), text(args, 1)) {
                (Ok(account), Ok(id)) => reply_json(
                    super::studies::lichess_study_chapters(&self.lichess, &account, &id).await,
                ),
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "studies.export" => {
                match (text(args, 0), text(args, 1), text(args, 2), text(args, 3)) {
                    (Ok(account), Ok(study), Ok(name), Ok(pgn)) => {
                        match super::studies::export_to_lichess_study(
                            &self.lichess,
                            &account,
                            &study,
                            &name,
                            &pgn,
                        )
                        .await
                        {
                            Ok(Reply::Done(id)) => Ok(json!({ "id": id })),
                            Ok(Reply::NeedsReconnect) => Ok(json!({ "needsReconnect": true })),
                            Err(cause) => Err(cause),
                        }
                    }
                    (Err(cause), _, _, _)
                    | (_, Err(cause), _, _)
                    | (_, _, Err(cause), _)
                    | (_, _, _, Err(cause)) => Err(cause),
                }
            }
            "studies.sync" => match parse::<StudySyncRequest>(args, 0, "request") {
                Ok(request) => {
                    reply_json(super::studies::sync_lichess_study(&self.lichess, &request).await)
                }
                Err(cause) => Err(cause),
            },
            "spectate.watch" => match parse_target(args, 0) {
                Ok(target) => Ok(json!(self.spectator.watch(target))),
                Err(cause) => Err(cause),
            },
            "spectate.watchRound" => match text(args, 0) {
                Ok(round) => Ok(json!(self.spectator.watch_round(&round))),
                Err(cause) => Err(cause),
            },
            "spectate.tvChannels" => to_json(super::watch::tv_channels(&self.lichess).await),
            "spectate.broadcasts" => {
                let query = optional_text(args, 0);
                to_json(super::watch::broadcasts(&self.lichess, query.as_deref()).await)
            }
            "spectate.broadcastTour" => match text(args, 0) {
                Ok(id) => to_json(super::watch::broadcast_tour(&self.lichess, &id).await),
                Err(cause) => Err(cause),
            },
            "cloudEval.cloudEval" => match (text(args, 0), arg_u32(args, 1)) {
                (Ok(fen), Ok(lines)) => {
                    to_json(cloud_eval(&self.lichess, &self.cloud, &fen, lines as usize).await)
                }
                (Err(cause), _) | (_, Err(cause)) => Err(cause),
            },
            "positionLookup.lookup" => {
                let kind = args.first().cloned().unwrap_or(Value::Null);
                let fen = args.get(1).cloned().unwrap_or(Value::Null);
                let options = args.get(2).filter(|value| !value.is_null());
                to_json(self.lookups.lookup(&kind, &fen, options).await)
            }
            "positionLookup.mastersGame" => match text(args, 0) {
                Ok(id) => to_json(self.lookups.masters_game(&id).await),
                Err(cause) => Err(cause),
            },
            _ => return None,
        };
        Some(result)
    }

    /// The synchronous methods of these services: pure readings and state the TypeScript callers
    /// read without starting work. `None` when the method is not one of them.
    pub fn call_sync(&self, method: &str, args: &[Value]) -> Option<Result<Value>> {
        let result = match method {
            "online.playing" => Ok(json!(self.online.playing())),
            "online.assistanceBlocked" => Ok(json!(self.online.assistance_blocked())),
            "challenges.list" => to_json(Ok::<_, CoreError>(self.online.challenges().list())),
            "challenges.get" => match text(args, 0) {
                Ok(id) => to_json(Ok::<_, CoreError>(self.online.challenges().get(&id))),
                Err(cause) => Err(cause),
            },
            "challenges.ingest" => match (text(args, 0), args.get(1)) {
                (Ok(account), Some(raw)) => to_json(Ok::<_, CoreError>(
                    self.online.challenges().ingest(&account, raw),
                )),
                (Err(cause), _) => Err(cause),
                (_, None) => Err(CoreError::new("Invalid challenge event.")),
            },
            "challenges.remove" => match text(args, 0) {
                Ok(id) => {
                    self.online.challenges().remove(&id);
                    Ok(Value::Null)
                }
                Err(cause) => Err(cause),
            },
            "challenges.forget" => match text(args, 0) {
                Ok(account) => {
                    self.online.challenges().forget(&account);
                    Ok(Value::Null)
                }
                Err(cause) => Err(cause),
            },
            "lichess.cachedProfile" => match text(args, 0) {
                Ok(name) => to_json(self.lichess.cached_profile(&name).map(|cached| {
                    let mut object = serde_json::Map::new();
                    object.insert("profile".into(), cached.profile.unwrap_or(Value::Null));
                    object.insert(
                        "ratingHistory".into(),
                        cached.rating_history.unwrap_or(Value::Null),
                    );
                    if let Some(at) = cached.profile_fetched_at {
                        object.insert("profileFetchedAt".into(), json!(at));
                    }
                    Value::Object(object)
                })),
                Err(cause) => Err(cause),
            },
            "lichess.forgetProfile" => match text(args, 0) {
                Ok(name) => {
                    self.lichess.forget_profile(&name);
                    Ok(Value::Null)
                }
                Err(cause) => Err(cause),
            },
            "lichess.invalidateLogin" => match strings(args, 0) {
                Ok(accounts) => {
                    self.lichess.invalidate_login(&accounts);
                    Ok(Value::Null)
                }
                Err(cause) => Err(cause),
            },
            "lichess.cancelAccountSyncs" => match strings(args, 0) {
                Ok(accounts) => {
                    self.lichess.cancel_account_syncs(&accounts);
                    Ok(Value::Null)
                }
                Err(cause) => Err(cause),
            },
            "cloudEval.clear" => {
                self.cloud.clear();
                Ok(Value::Null)
            }
            "spectate.stop" => {
                self.spectator.stop();
                Ok(Value::Null)
            }
            "spectate.broadcastGame" => match text(args, 0) {
                Ok(pgn) => to_json(Ok::<_, CoreError>(super::watch::broadcast_game(&pgn))),
                Err(cause) => Err(cause),
            },
            "studies.splitPgn" => match text(args, 0) {
                Ok(text) => to_json(Ok::<_, CoreError>(super::studies::split_pgn(&text))),
                Err(cause) => Err(cause),
            },
            "tournaments.arenaExtras" => Ok(json!(super::tournaments::arena_extras(
                args.first().unwrap_or(&Value::Null)
            ))),
            "lichess.reviewFromLichess" => Ok(review_from_lichess(
                args.first().unwrap_or(&Value::Null),
            )
            .unwrap_or(Value::Null)),
            _ => return None,
        };
        Some(result)
    }

    /// `syncGames(username?)`: syncs the accounts, then answers the stored application data.
    async fn sync_games(&self, args: &[Value]) -> Result<Value> {
        let username = optional_text(args, 0);
        self.lichess.sync_games(username.as_deref()).await?;
        self.database.call("store.games.loadData", &[])
    }

    /// `connectLichess(look)`: the browser sign-in; the window is focused when the browser returns.
    async fn connect(&self, args: &[Value]) -> Result<Value> {
        let look = args.first().filter(|value| !value.is_null());
        let capabilities = Arc::clone(&self.capabilities);
        let focus = move || {
            let capabilities = Arc::clone(&capabilities);
            tokio::spawn(async move {
                let _ = capabilities.focus().await;
            });
        };
        let connected = self.lichess.connect_lichess(look, Some(&focus)).await?;
        let data = self.database.call("store.games.loadData", &[])?;
        Ok(json!({ "data": data, "username": connected.username }))
    }

    /// Cancels the running services and drops the usage still pending (`closeUsage`).
    pub async fn close(&self) {
        self.online.close();
        self.spectator.stop();
        self.lifetime.cancel();
        self.usage.close();
    }

    /// Forgets the usage of accounts that were signed out (`forgetUsage`): traffic still in flight
    /// for them is no longer counted, and what is pending is dropped.
    pub fn forget_usage(&self, accounts: &[String]) {
        self.lichess.client().policy().forget_usage(accounts);
        self.usage.forget(accounts);
    }
}

/// A tournament action's answer: `true` when it went through, or `{ needsReconnect: true }`.
fn reply_true<E: Into<CoreError>>(result: std::result::Result<Reply<()>, E>) -> Result<Value> {
    match result.map_err(Into::into)? {
        Reply::Done(()) => Ok(json!(true)),
        Reply::NeedsReconnect => Ok(json!({ "needsReconnect": true })),
    }
}

/// A service's answer as the TypeScript caller reads it: the value, or `{ needsReconnect: true }`
/// when the account's login lacks the permission.
fn reply_json<T: serde::Serialize, E: Into<CoreError>>(
    result: std::result::Result<Reply<T>, E>,
) -> Result<Value> {
    match result.map_err(Into::into)? {
        Reply::Done(value) => {
            serde_json::to_value(value).map_err(|cause| CoreError::new(cause.to_string()))
        }
        Reply::NeedsReconnect => Ok(json!({ "needsReconnect": true })),
    }
}

fn to_json<T: serde::Serialize>(
    result: std::result::Result<T, impl Into<CoreError>>,
) -> Result<Value> {
    match result {
        Ok(value) => serde_json::to_value(value).map_err(|cause| CoreError::new(cause.to_string())),
        Err(cause) => Err(cause.into()),
    }
}

fn text(args: &[Value], i: usize) -> Result<String> {
    args.get(i)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| CoreError::new(format!("Invalid argument {}.", i + 1)))
}

fn optional_text(args: &[Value], i: usize) -> Option<String> {
    args.get(i).and_then(Value::as_str).map(str::to_string)
}

fn strings(args: &[Value], i: usize) -> Result<Vec<String>> {
    parse(args, i, "list")
}

fn arg_u32(args: &[Value], i: usize) -> Result<u32> {
    args.get(i)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| CoreError::new(format!("Invalid argument {}.", i + 1)))
}

fn parse<T: serde::de::DeserializeOwned>(args: &[Value], i: usize, name: &str) -> Result<T> {
    serde_json::from_value(args.get(i).cloned().unwrap_or(Value::Null))
        .map_err(|cause| CoreError::new(format!("Invalid {name}: {cause}")))
}

/// `{ channel }` or `{ gameId }`: what the spectator watches.
fn parse_target(args: &[Value], i: usize) -> Result<WatchTarget> {
    let value = args.get(i).cloned().unwrap_or(Value::Null);
    if let Some(channel) = value.get("channel").and_then(Value::as_str) {
        return Ok(WatchTarget::Channel(channel.to_string()));
    }
    if let Some(game) = value.get("gameId").and_then(Value::as_str) {
        return Ok(WatchTarget::Game(game.to_string()));
    }
    Err(CoreError::new("Invalid watch target."))
}
