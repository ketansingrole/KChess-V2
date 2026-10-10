//! The core as hosts drive it: one asynchronous `call(method, args)` with JSON arguments
//! and results, and events through the host.
//!
//! `Core` owns one engine context (the host, the process registry, the engine budget and where
//! engines are found) and the three engine services built on it: computer moves (`search`),
//! analysis and game review. They share the one database (`store::Database`) and the battery
//! reading the scheduler consults.

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{OnceCell, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::capabilities::Capabilities;
use crate::engine::EngineContext;
use crate::engine::analysis::{ANALYSIS_EVENT, Analysis, AnalysisSink, AnalysisUpdate};
use crate::engine::bridge::{Battery, EngineOwners, ReviewBridge, ReviewRecords};
use crate::engine::managed::{
    ManagedLocation, ReleaseAsset, Target, default_dir, delete_managed_engine,
    install_managed_engine, managed_engine, pick_mac_asset, pick_stockfish_asset,
};
use crate::engine::review::{ReviewRequest, Reviews};
use crate::engine::scheduler::Scheduler;
use crate::engine::search::Search;
use crate::engine::status::{EngineLocations, EngineStatus, engine_identity, engine_status};
use crate::engine::uci::EngineHub;
use crate::error::{CoreError, Result};
use crate::facade::{GatedHost, ProfileClaim};
use crate::host::{Config, Host};
use crate::lichess::services::{LiveGame, Services};
use crate::puzzles::PuzzleService;
use crate::store::Database;
use crate::store::debug;
use crate::usage::{UsageBatch, UsageHost};

/// A managed engine change that replaces or removes the downloaded executable.
#[derive(Clone, Copy)]
enum Mutation {
    Install,
    Delete,
}

pub struct Core {
    pub(crate) host: Arc<dyn Host>,
    pub(crate) puzzles: PuzzleService,
    /// What the core asks of its host (secrets, browser, focus, power).
    pub(crate) capabilities: Arc<Capabilities>,
    /// `kchess.db`, shared by the storage calls and the review queue.
    pub(crate) database: Arc<Database>,
    /// The battery reading the engine scheduler consults.
    pub(crate) battery: Arc<Battery>,
    battery_started: AtomicBool,
    pub(crate) ctx: EngineContext,
    pub(crate) search: Search,
    pub(crate) analysis: Analysis,
    pub(crate) reviews: Reviews,
    /// The Lichess services (client, online session, spectator, explorer, cloud evaluation).
    pub(crate) services: Services,
    /// Network accounting counters, batched before they reach the database.
    pub(crate) usage: Arc<UsageBatch>,
    /// Managed engine installs and deletes in progress, by request id; `true` cancels one.
    installs: Mutex<HashMap<u64, watch::Sender<bool>>>,
    /// Whether a live online game (or its recovery) blocks the engines, as of the last state.
    busy: Arc<AtomicBool>,
    /// Set when `close` starts: later facade calls fail and events stop.
    pub(crate) closed: Arc<AtomicBool>,
    /// The facade calls in flight; closing waits for them.
    pub(crate) tracker: TaskTracker,
    /// The one-time import of an earlier release (`ensureMigrated`).
    pub(crate) migrated: OnceCell<()>,
    /// The close in progress, shared by every `close` call.
    pub(crate) closing: OnceCell<()>,
    /// Ids for the managed engine requests the facade starts.
    pub(crate) requests: AtomicU64,
    /// This core's claim on its profile directory, released when it closes.
    pub(crate) claim: ProfileClaim,
}

fn arg<T: DeserializeOwned>(args: &[Value], i: usize, name: &str) -> Result<T> {
    serde_json::from_value(args.get(i).cloned().unwrap_or(Value::Null))
        .map_err(|e| CoreError::new(format!("Invalid {name}: {e}")))
}

/// An optional argument: absent or null is the default.
fn optional<T: DeserializeOwned + Default>(args: &[Value], i: usize, name: &str) -> Result<T> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(T::default()),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|e| CoreError::new(format!("Invalid {name}: {e}"))),
    }
}

/// An argument that is passed through as JSON (moves, options, validated by the service).
fn raw(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Null)
}

fn json<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| CoreError::new(e.to_string()))
}

/// Where a managed engine call works, for tests only: a directory and a release endpoint. The
/// renderer never sends these; they are honoured only when the store's test switch is on
/// (`KCHESS_STORE_DEBUG=1`), and otherwise refused.
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocationOverride {
    #[serde(default)]
    dir: Option<PathBuf>,
    #[serde(default)]
    release_url: Option<String>,
}

fn location_override(args: &[Value], i: usize) -> Result<LocationOverride> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(LocationOverride::default()),
        Some(value) => {
            if !debug::enabled() {
                return Err(CoreError::new("Engine locations are only set by tests."));
            }
            serde_json::from_value(value.clone())
                .map_err(|e| CoreError::new(format!("Invalid location: {e}")))
        }
    }
}

/// A test-only Lichess origin (`KCHESS_TEST_LICHESS_BASE`), honoured only when the test switch
/// is on (`KCHESS_STORE_DEBUG=1`), so the end-to-end suite serves Lichess from a local fixture.
fn test_lichess_origin() -> Option<String> {
    if !debug::enabled() {
        return None;
    }
    std::env::var("KCHESS_TEST_LICHESS_BASE")
        .ok()
        .map(|origin| origin.trim_end_matches('/').to_string())
        .filter(|origin| !origin.is_empty())
}

fn engine_locations(config: &Config) -> EngineLocations {
    EngineLocations {
        managed_dir: default_dir(&config.data_dir, config.managed_engine_dir.as_deref()),
        target: Target::current(),
        bundled_script: config.bundled_engine_path.clone().unwrap_or_default(),
        node_path: config
            .node_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("node")),
        node_env: config.node_env.clone(),
    }
}

impl Core {
    /// Opens the core over `config`'s profile. Fails when a live core already owns that profile.
    pub fn open(config: Config, host: Arc<dyn Host>) -> Result<Core> {
        let claim = ProfileClaim::claim(&config.data_dir)?;
        let closed = Arc::new(AtomicBool::new(false));
        let host: Arc<dyn Host> = Arc::new(GatedHost::new(host, Arc::clone(&closed)));
        let database = Arc::new(Database::new(config.clone(), Arc::clone(&host)));
        let capabilities = Arc::new(Capabilities::new(Arc::clone(&host)));
        let battery = Battery::new(Arc::clone(&host));
        let scheduler = {
            let battery = Arc::clone(&battery);
            Arc::new(Scheduler::new(move || battery.value()))
        };
        let hub = Arc::new(EngineHub::new(Arc::clone(&host)));
        let mut ctx =
            EngineContext::new(Arc::clone(&host), hub, scheduler, engine_locations(&config));
        {
            let battery = Arc::clone(&battery);
            let capabilities = Arc::clone(&capabilities);
            ctx.before_lease = Arc::new(move || battery.refresh(&capabilities));
        }
        let search = Search::new(ctx.clone());
        let analysis = Analysis::new(ctx.clone());
        let reviews = Reviews::new(ctx.clone(), Arc::new(ReviewRecords(Arc::clone(&database))));
        let busy = Arc::new(AtomicBool::new(false));
        // A live game stops the computer, the analysis and the review engine before it is reported.
        let game_stops: Arc<dyn Fn() + Send + Sync> = {
            let (search, analysis, reviews) = (search.clone(), analysis.clone(), reviews.clone());
            Arc::new(move || {
                search.stop_engine(false);
                analysis.stop_analysis(false);
                reviews.restart_review_engine();
            })
        };
        // Usage events are counted in batches (`usage.ts`) before the services' host sees them.
        let usage = UsageBatch::new(
            Arc::clone(&database),
            Arc::clone(&host),
            config.data_dir.clone(),
        );
        let services_host: Arc<dyn Host> = Arc::new(UsageHost {
            inner: Arc::clone(&host),
            batch: Arc::clone(&usage),
        });
        let services = Services::new(
            Arc::clone(&services_host),
            Arc::clone(&database),
            Arc::clone(&capabilities),
            Arc::clone(&usage),
            CancellationToken::new(),
            test_lichess_origin(),
            LiveGame {
                stops: game_stops,
                busy: Arc::clone(&busy),
            },
        );
        Ok(Core {
            puzzles: PuzzleService::new(config, services_host),
            host,
            capabilities,
            database,
            battery,
            battery_started: AtomicBool::new(false),
            ctx,
            search,
            analysis,
            reviews,
            services,
            usage,
            installs: Mutex::new(HashMap::new()),
            busy,
            closed,
            tracker: TaskTracker::new(),
            migrated: OnceCell::new(),
            closing: OnceCell::new(),
            requests: AtomicU64::new(0),
            claim,
        })
    }

    /// Run one method: a facade method (`CoreApi`) or an internal service method. Unknown
    /// methods and malformed arguments are errors.
    pub async fn call(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        self.start();
        if crate::facade::is_facade_method(method) {
            return Box::pin(self.facade(method, args)).await;
        }
        Box::pin(self.dispatch(method, args)).await
    }

    /// The internal service methods, by their service names.
    pub(crate) async fn dispatch(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        match method {
            "host.reply" => {
                let id: u64 = arg(&args, 0, "id")?;
                self.capabilities
                    .reply(id, args.get(1).unwrap_or(&Value::Null));
                Ok(Value::Null)
            }
            // Test-only: ask the host for a capability and return its answer.
            "host.ask" if debug::enabled() => {
                let kind: String = arg(&args, 0, "kind")?;
                self.capabilities
                    .request(&kind, args.get(1).cloned().unwrap_or(Value::Null))
                    .await
            }
            "puzzles.status" => json(self.puzzles.status().await?),
            "puzzles.install" => {
                let url: String = arg(&args, 0, "url")?;
                json(self.puzzles.install(url).await?)
            }
            "puzzles.cancel" => {
                self.puzzles.cancel();
                Ok(Value::Null)
            }
            "puzzles.delete" => json(self.puzzles.delete().await?),
            "puzzles.query" => json(self.puzzles.query(arg(&args, 0, "query")?).await?),
            "puzzles.ladder" => json(self.puzzles.ladder(arg(&args, 0, "query")?).await?),
            "engine.bestMove" => {
                let level: String = arg(&args, 1, "level")?;
                let configured: String = optional(&args, 2, "configured")?;
                let mv = self
                    .search
                    .best_move(raw(&args, 0), &level, &configured, raw(&args, 3))
                    .await?;
                json(mv)
            }
            "engine.status" => {
                let configured: String = optional(&args, 0, "configured")?;
                json(engine_status(self.host.as_ref(), &self.ctx.locations, &configured).await)
            }
            "engine.identity" => {
                let status: EngineStatus = arg(&args, 0, "status")?;
                json(engine_identity(&status).await?)
            }
            "analysis.start" => {
                let configured: String = optional(&args, 1, "configured")?;
                let host = Arc::clone(&self.host);
                let sink: AnalysisSink = Arc::new(move |update: AnalysisUpdate| {
                    host.emit(ANALYSIS_EVENT, json!(update));
                });
                json(
                    self.analysis
                        .start_analysis(raw(&args, 0), &configured, sink)?,
                )
            }
            "reviews.setup" => {
                self.reviews.setup_reviews(Arc::new(self.review_bridge()));
                Ok(Value::Null)
            }
            "reviews.request" => json(
                self.reviews
                    .request_review(arg::<ReviewRequest>(&args, 0, "request")?)?,
            ),
            "reviews.changed" => {
                self.reviews.reviews_changed();
                Ok(Value::Null)
            }
            "reviews.stop" => {
                self.reviews.stop_reviews().await;
                Ok(Value::Null)
            }
            "managedEngine.status" => {
                let dir = self.managed_dir(&location_override(&args, 0)?);
                json(managed_engine(self.host.as_ref(), &dir).await)
            }
            "managedEngine.install" => {
                let id: u64 = arg(&args, 0, "requestId")?;
                let location = location_override(&args, 1)?;
                self.managed_request(id, Mutation::Install, &location).await
            }
            "managedEngine.delete" => {
                let id: u64 = arg(&args, 0, "requestId")?;
                let location = location_override(&args, 1)?;
                self.managed_request(id, Mutation::Delete, &location).await
            }
            "managedEngine.cancel" => {
                let id: u64 = arg(&args, 0, "requestId")?;
                self.cancel_install(id);
                Ok(Value::Null)
            }
            "engines.maintenance" => {
                self.ctx
                    .hub
                    .with_maintenance(|| self.stop_owners(), || async { Ok(()) })
                    .await?;
                Ok(Value::Null)
            }
            "engines.close" => {
                self.ctx.hub.close_engines().await?;
                Ok(Value::Null)
            }
            _ => {
                if let Some(result) = Box::pin(self.services.call(method, &args)).await {
                    return result;
                }
                self.call_sync(method, args)
            }
        }
    }

    /// Run one synchronous method: storage, and the engine state that a synchronous caller reads
    /// or changes without starting work (`engine.stop`, `engine.computerPlaying`,
    /// `reviews.status`, ...). Unknown methods and malformed arguments are errors.
    pub fn call_sync(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        if let Some(result) = self.engine_sync(method, &args) {
            return result;
        }
        if let Some(result) = self.services.call_sync(method, &args) {
            return result;
        }
        if let Some(result) = self.usage_sync(method, &args) {
            return result;
        }
        if let Some(result) = self.facade_sync(method, &args) {
            return result;
        }
        if !method.starts_with("store.") {
            return Err(CoreError::new(format!("Unknown core method {method}.")));
        }
        self.database.call(method, &args)
    }

    /// The network accounting methods (`usage.ts`).
    fn usage_sync(&self, method: &str, args: &[Value]) -> Option<Result<Value>> {
        let result = match method {
            "usage.flush" => {
                self.usage.flush();
                Ok(Value::Null)
            }
            "usage.report" => self.usage.report(),
            "usage.reset" => self.usage.reset(),
            "usage.close" => {
                self.usage.close();
                Ok(Value::Null)
            }
            "usage.forget" => optional::<Vec<String>>(args, 0, "accounts").map(|accounts| {
                self.services.forget_usage(&accounts);
                Value::Null
            }),
            _ => return None,
        };
        Some(result)
    }

    /// The engine methods that start no work and need no runtime: safe from a synchronous caller.
    fn engine_sync(&self, method: &str, args: &[Value]) -> Option<Result<Value>> {
        let result = match method {
            "engine.stop" => optional::<bool>(args, 0, "close").map(|close| {
                self.search.stop_engine(close);
                Value::Null
            }),
            "engine.reset" => {
                self.search.reset_engine();
                Ok(Value::Null)
            }
            "engine.computerPlaying" => arg::<i64>(args, 0, "withinMs")
                .and_then(|within| json(self.search.computer_playing(within))),
            "engine.threads" => json(self.search.engine_threads()),
            "engine.trust" => arg::<String>(args, 0, "path").map(|path| {
                self.search.trust_engine_path(&PathBuf::from(path));
                Value::Null
            }),
            "engine.isTrusted" => arg::<String>(args, 0, "path")
                .and_then(|path| json(self.search.is_trusted_engine_path(&PathBuf::from(path)))),
            "analysis.running" => json(self.analysis.analysis_running()),
            "analysis.stop" => optional::<bool>(args, 0, "kill").map(|kill| {
                self.analysis.stop_analysis(kill);
                Value::Null
            }),
            // Test-only: a live online game blocks the engines (`online:state` sets it in service).
            "engine.setBusy" if debug::enabled() => arg::<bool>(args, 0, "online").map(|online| {
                self.busy.store(online, Ordering::SeqCst);
                Value::Null
            }),
            "managedEngine.pickMacAsset" => {
                arg::<Vec<ReleaseAsset>>(args, 0, "assets").and_then(|assets| {
                    let cpu: String = arg(args, 1, "arch")?;
                    json(pick_mac_asset(&assets, &cpu).cloned())
                })
            }
            "managedEngine.pickAsset" => {
                arg::<Vec<ReleaseAsset>>(args, 0, "assets").and_then(|assets| {
                    let platform: String = arg(args, 1, "platform")?;
                    let cpu: String = arg(args, 2, "arch")?;
                    json(pick_stockfish_asset(&assets, &platform, &cpu).cloned())
                })
            }
            "reviews.status" => json(self.reviews.review_status()),
            "reviews.get" => arg::<String>(args, 0, "fen").and_then(|fen| {
                let moves: Vec<String> = optional(args, 1, "moves")?;
                json(self.reviews.get_review(&fen, &moves)?)
            }),
            "reviews.cancel" => arg::<String>(args, 0, "key").map(|key| {
                self.reviews.cancel_review(&key);
                Value::Null
            }),
            "reviews.discardAccounts" => {
                optional::<Vec<String>>(args, 0, "accounts").map(|accounts| {
                    self.reviews.discard_account_reviews(&accounts);
                    Value::Null
                })
            }
            "reviews.restartEngine" => {
                self.reviews.restart_review_engine();
                Ok(Value::Null)
            }
            _ => return None,
        };
        Some(result)
    }

    /// Starts the background work once, on the first call (which is always in a runtime): the
    /// battery refresh, and the review queue, which looks for work as soon as it starts.
    fn start(&self) {
        if !self.battery_started.swap(true, Ordering::SeqCst) {
            self.battery.refresh(&self.capabilities);
            self.battery
                .start_refreshing(Arc::clone(&self.capabilities));
            self.reviews.setup_reviews(Arc::new(self.review_bridge()));
        }
    }

    /// The review queue's host for this core.
    fn review_bridge(&self) -> ReviewBridge {
        ReviewBridge {
            database: Arc::clone(&self.database),
            host: Arc::clone(&self.host),
            capabilities: Arc::clone(&self.capabilities),
            lichess: Arc::clone(self.services.lichess()),
            search: self.search.clone(),
            analysis: self.analysis.clone(),
            online: {
                let busy = Arc::clone(&self.busy);
                Arc::new(move || busy.load(Ordering::SeqCst))
            },
        }
    }

    /// The managed engine's directory: the core's, or a test's.
    fn managed_dir(&self, overridden: &LocationOverride) -> PathBuf {
        overridden
            .dir
            .clone()
            .unwrap_or_else(|| self.ctx.locations.managed_dir.clone())
    }

    /// `managedEngine.install` / `managedEngine.delete`: runs under a request id so the host can
    /// cancel it, and suspends the engines while the executable is replaced.
    async fn managed_request(
        &self,
        id: u64,
        mutation: Mutation,
        overridden: &LocationOverride,
    ) -> Result<Value> {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.installs
            .lock()
            .map_err(|_| CoreError::new("The engine is unavailable."))?
            .insert(id, cancel_tx);
        let mut location = ManagedLocation::new(self.managed_dir(overridden));
        if let Some(url) = &overridden.release_url {
            location.release_url = url.clone();
        }
        location.handoff = Some(Arc::new(EngineOwners {
            hub: Arc::clone(&self.ctx.hub),
            search: self.search.clone(),
            analysis: self.analysis.clone(),
            reviews: self.reviews.clone(),
        }));
        location.cancel = Some(cancel_rx);
        let result = match mutation {
            Mutation::Install => install_managed_engine(self.host.as_ref(), &location)
                .await
                .and_then(json),
            Mutation::Delete => delete_managed_engine(self.host.as_ref(), &location)
                .await
                .map(|()| Value::Null),
        };
        if let Ok(mut installs) = self.installs.lock() {
            installs.remove(&id);
        }
        result
    }

    fn cancel_install(&self, id: u64) {
        if let Ok(installs) = self.installs.lock()
            && let Some(sender) = installs.get(&id)
        {
            sender.send_replace(true);
        }
    }

    /// `withEngineMaintenance`'s owners: the computer's search, the analysis and the review
    /// engine stop before engines are closed.
    fn stop_owners(&self) {
        self.search.stop_engine(true);
        self.analysis.stop_analysis(true);
        self.reviews.restart_review_engine();
    }

    /// The host capabilities, for services that need them.
    pub fn capabilities(&self) -> Arc<Capabilities> {
        Arc::clone(&self.capabilities)
    }

    /// Cancels the managed engine installs in progress.
    pub(crate) fn cancel_installs(&self) {
        if let Ok(installs) = self.installs.lock() {
            for sender in installs.values() {
                sender.send_replace(true);
            }
        }
    }
}
