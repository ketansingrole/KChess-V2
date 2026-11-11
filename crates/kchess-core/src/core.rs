//! The core as hosts drive it: one asynchronous `call(method, args)` with JSON arguments
//! and results, run by the `CoreApi` dispatch, and events through the host.
//!
//! `Core` owns one engine context (the host, the process registry, the engine budget and where
//! engines are found) and the three engine services built on it: computer moves (`search`),
//! analysis and game review. They share the one database (`store::Database`) and the battery
//! reading the scheduler consults.

use kchess_contracts::{CoreEvent, HostReply};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::capabilities::Capabilities;
use crate::connection::{APP, ClientInfo, Connections};
use crate::engine::EngineContext;
use crate::engine::analysis::Analysis;
use crate::engine::bridge::{Battery, EngineOwners, ReviewBridge};
use crate::engine::managed::{
    ManagedLocation, Target, default_dir, delete_managed_engine, install_managed_engine,
};
use crate::engine::review::Reviews;
use crate::engine::scheduler::Scheduler;
use crate::engine::search::Search;
use crate::engine::status::EngineLocations;
use crate::engine::uci::EngineHub;
use crate::error::{CoreError, Result};
use crate::facade::GatedHost;
use crate::host::{Config, Host};
use crate::lichess::services::{LiveGame, Services};
use crate::profile_lock::ProfileClaim;
use crate::puzzles::PuzzleService;
use crate::secrets::Credentials;
use crate::store::Database;
use crate::usage::UsageBatch;
use crate::voice::recognizer::VoiceRecognizer;

/// A managed engine change that replaces or removes the downloaded executable.
#[derive(Clone, Copy)]
pub(crate) enum Mutation {
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
    /// ChatGPT plan usage.
    pub(crate) ai: crate::ai::Ai,
    /// The Lichess services (client, online session, spectator, explorer, cloud evaluation).
    pub(crate) services: Services,
    /// Chess.com game sync.
    pub(crate) chesscom: crate::chesscom::Chesscom,
    /// Offline speech recognition through the `kchess-voice` helper.
    pub(crate) voice: Arc<VoiceRecognizer>,
    /// The client listening through `voice`: one microphone at a time. Held across each
    /// recognizer call, so a change of owner and the session it starts or ends happen together.
    pub(crate) voice_owner: Arc<AsyncMutex<Option<u64>>>,
    /// The offline speech model `voice` loads: downloaded and verified once per profile.
    pub(crate) voice_model: crate::voice::VoiceModelCache,
    /// Network accounting counters, batched before they reach the database.
    pub(crate) usage: Arc<UsageBatch>,
    /// Managed engine installs and deletes in progress, by request id.
    installs: Mutex<HashMap<u64, CancellationToken>>,
    /// Whether a live online game (or its recovery) blocks the engines, as of the last state.
    busy: Arc<AtomicBool>,
    /// Set when `close` starts: later facade calls fail and events stop.
    pub(crate) closed: Arc<AtomicBool>,
    /// The facade calls in flight; closing waits for them.
    pub(crate) tracker: TaskTracker,
    /// The one-time import of an earlier release.
    pub(crate) migrated: OnceCell<()>,
    /// The close in progress, shared by every `close` call.
    pub(crate) closing: OnceCell<()>,
    /// Ids for the managed engine requests the facade starts.
    pub(crate) requests: AtomicU64,
    /// This core's claim on its profile directory, released when it closes.
    pub(crate) claim: ProfileClaim,
    /// The profile directory (`Config::data_dir`).
    pub(crate) data_dir: PathBuf,
    /// Its clients: the embedding app, or the shared service's connections.
    pub(crate) connections: Arc<Connections>,
    /// The alert owner last reported (`alerts:owner`); `u64::MAX` none, `APP` at first.
    alert_owner: AtomicU64,
    /// Held while the alert owner is reported, so a joining client's snapshot is not stale.
    alert_reports: Mutex<()>,
    /// Held while settings are read, merged and written, so two windows' saves never interleave.
    pub(crate) settings_writes: AsyncMutex<()>,
}

fn json<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| CoreError::wrap("core result encoding", e))
}

/// Where a managed engine change works: a test's directory and release endpoint, or the core's.
#[derive(Default)]
pub(crate) struct LocationOverride {
    dir: Option<PathBuf>,
    release_url: Option<String>,
}

/// Runs a call of client `connection`; a panic in it is an error that names `method`.
async fn caught_as(
    connection: u64,
    method: &str,
    call: impl std::future::Future<Output = Result<Value>>,
) -> Result<Value> {
    crate::tasks::caught(crate::connection::scope(connection, call))
        .await
        .unwrap_or_else(|_| Err(CoreError::new(format!("KChess could not finish {method}."))))
}

fn engine_locations(config: &Config) -> EngineLocations {
    EngineLocations {
        managed_dir: default_dir(&config.data_dir, config.managed_engine_dir.as_deref()),
        target: Target::current(),
        bundled: config.bundled_engine_path.clone().unwrap_or_default(),
        variant: config.variant_engine_path.clone().unwrap_or_default(),
    }
}

impl Core {
    /// Opens the core over `config`'s profile. Fails when a live core already owns that profile.
    pub fn open(config: Config, host: Arc<dyn Host>) -> Result<Core> {
        let claim = ProfileClaim::claim(&config.data_dir)?;
        let test = config.test_switches().cloned().unwrap_or_default();
        let testing = config.test_switches().is_some();
        let connections = Arc::new(Connections::new());
        let closed = Arc::new(AtomicBool::new(false));
        let host: Arc<dyn Host> = Arc::new(GatedHost::new(host, Arc::clone(&closed)));
        crate::panics::report_to(&host);
        let database = Arc::new(Database::new(config.clone(), Arc::clone(&host)));
        let credentials = config
            .system_credentials
            .then(|| Arc::new(Credentials::for_profile(&config.data_dir, testing)));
        let capabilities = Arc::new(
            Capabilities::with_credentials(Arc::clone(&host), credentials)
                .with_connections(Arc::clone(&connections)),
        );
        let battery = Battery::new(Arc::clone(&host), Arc::clone(&capabilities));
        let scheduler = Arc::new(Scheduler::new(Arc::clone(&battery)));
        let hub = Arc::new(EngineHub::new(Arc::clone(&host)));
        let ctx = EngineContext::new(Arc::clone(&host), hub, scheduler, engine_locations(&config));
        let search = Search::new(ctx.clone());
        let analysis = Analysis::new(ctx.clone());
        let reviews = Reviews::new(ctx.clone(), Arc::clone(&database));
        let busy = Arc::new(AtomicBool::new(false));
        // A live game stops the computer, the analysis and the review engine before it is reported.
        let game_stops: Arc<dyn Fn() + Send + Sync> = {
            let (search, analysis, reviews) = (search.clone(), analysis.clone(), reviews.clone());
            Arc::new(move || crate::engine::stop_engines(&search, &analysis, &reviews, false))
        };
        let usage = UsageBatch::new(
            Arc::clone(&database),
            Arc::clone(&host),
            config.data_dir.clone(),
        );
        let services = Services::new(
            Arc::clone(&host),
            Arc::clone(&database),
            Arc::clone(&capabilities),
            Arc::clone(&usage),
            CancellationToken::new(),
            test.lichess_origin,
            LiveGame {
                stops: game_stops,
                busy: Arc::clone(&busy),
            },
        );
        let voice = Arc::new(VoiceRecognizer::new(
            Arc::clone(&host),
            config.voice_helper_path.clone(),
            crate::voice::model_dir(&config.data_dir),
        ));
        let (voice_archive, voice_model) = crate::voice::active_model(test.voice_model.as_ref());
        let voice_model = crate::voice::VoiceModelCache::new(
            Arc::clone(&host),
            usage.clone(),
            crate::voice::cache_dir(&config.data_dir),
            voice_archive,
            voice_model,
        );
        let chesscom = crate::chesscom::Chesscom::new(
            Arc::clone(&database),
            Arc::clone(&host),
            test.chesscom_origin,
            CancellationToken::new(),
        );
        let data_dir = config.data_dir.clone();
        Ok(Core {
            data_dir,
            connections,
            alert_owner: AtomicU64::new(APP),
            alert_reports: Mutex::new(()),
            settings_writes: AsyncMutex::new(()),
            puzzles: PuzzleService::new(config, Arc::clone(&host), usage.clone()),
            voice,
            voice_owner: Arc::new(AsyncMutex::new(None)),
            voice_model,
            host,
            capabilities,
            database,
            battery,
            battery_started: AtomicBool::new(false),
            ctx,
            search,
            analysis,
            reviews,
            ai: crate::ai::Ai::new(test.openai_origin, CancellationToken::new())?,
            chesscom,
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

    pub(crate) fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    /// Run one method of `CoreApi`, or `host.reply` (a host's answer to a capability request).
    /// Unknown methods and malformed arguments are errors.
    pub async fn call(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        self.call_as(APP, method, args).await
    }

    /// Run one method for client `connection` (`connect`): what it starts (an analysis, a watch,
    /// voice input) is its own, and its events go to it.
    ///
    /// A method that panics fails like any other: its caller (a window, the command line, a
    /// socket client whose call has no deadline) is answered rather than left waiting forever.
    /// The panic hook has logged where.
    pub async fn call_as(&self, connection: u64, method: &str, args: Vec<Value>) -> Result<Value> {
        caught_as(connection, method, self.call_unscoped(method, args)).await
    }

    /// A new client of this core (the shared service's socket connections); its connection id.
    pub fn connect(&self, info: ClientInfo) -> u64 {
        let id = self.connections.connect(info);
        self.report_alert_owner();
        id
    }

    /// The frontend that embeds this core, with the host requests it can answer: the others are
    /// refused rather than sent to it (an embedded command line has no window to focus).
    pub fn embedded_by(&self, info: ClientInfo) {
        self.connections.set_app(info);
        self.report_alert_owner();
    }

    /// The shared service serves its own clients: the embedding app's connection goes.
    pub fn serve_connections(&self) {
        self.connections.without_app();
        self.report_alert_owner();
    }

    /// A client went away: what it started stops.
    pub fn disconnect(&self, connection: u64) {
        self.end_connection(connection);
        self.connections.disconnect(connection);
        self.report_alert_owner();
    }

    /// A client has no window open (`suspend`): what it started stops, and every other client's
    /// work carries on.
    pub fn suspend_connection(&self, connection: u64) {
        self.end_connection(connection);
    }

    /// A client's window gained or lost focus; the window focused last alerts the person.
    pub fn focus(&self, connection: u64, focused: bool) {
        self.connections.focus(connection, focused);
        self.report_alert_owner();
    }

    /// Tells every client which window alerts now (`alerts:owner`), when that changed.
    fn report_alert_owner(&self) {
        let _reporting = self.alert_reports.lock();
        let owner = self.connections.alert_owner();
        let encoded = owner.unwrap_or(u64::MAX);
        if self.alert_owner.swap(encoded, Ordering::SeqCst) != encoded {
            self.host
                .emit(CoreEvent::AlertsOwner(json!({ "connection": owner })));
        }
    }

    /// Runs `join` with the current `alerts:owner` event while no change of owner is reported:
    /// a client that joins queues it before it receives any event, so it misses no change.
    pub fn with_alert_owner<T>(&self, join: impl FnOnce(CoreEvent) -> T) -> T {
        let _reporting = self.alert_reports.lock();
        join(CoreEvent::AlertsOwner(
            json!({ "connection": self.connections.alert_owner() }),
        ))
    }

    /// The clients connected now.
    pub fn connections(&self) -> &Connections {
        &self.connections
    }

    /// Stops what `connection` started: its analysis, its watch, its voice input.
    fn end_connection(&self, connection: u64) {
        self.search.stop_connection(connection);
        self.analysis.forget(connection);
        self.services.spectator().forget(connection);
        self.release_voice(connection);
    }

    /// A frontend's answer to the host request `id` (`host:request`).
    pub fn host_reply(&self, id: u64, reply: HostReply) -> Result<()> {
        // The services were stopped and the profile released: a late reply must not restart
        // them.
        self.refuse_closed()?;
        self.start();
        self.capabilities.reply(id, reply);
        Ok(())
    }

    async fn call_unscoped(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        // How a socket client's `host_reply` crosses (`kchess-service`).
        if method == kchess_contracts::host::REPLY_METHOD {
            let id = args
                .first()
                .and_then(Value::as_u64)
                .ok_or_else(|| CoreError::new("Invalid id."))?;
            let reply = serde_json::from_value(args.get(1).cloned().unwrap_or_default())
                .map_err(|_| CoreError::new("Invalid host reply."))?;
            return self.host_reply(id, reply).map(|()| Value::Null);
        }
        // Tracked before the closed check: closing waits for every call that got this far.
        let _in_flight = self.tracker.token();
        self.refuse_closed()?;
        // An earlier release's profile is imported before anything reads the store.
        self.ensure_migrated().await?;
        if !self.closed.load(Ordering::SeqCst) {
            self.start();
        }
        // Boxed: the method table is one large future, too big to lay out on a worker's stack.
        Box::pin(kchess_contracts::core_api::dispatch(self, method, args)).await
    }

    /// Tests only: a call of client `connection` that panics, as a defect would.
    #[doc(hidden)]
    pub async fn panic_for_tests(&self, connection: u64) -> Result<Value> {
        caught_as(connection, "a panicking call", async {
            panic!("a panicking call")
        })
        .await
    }

    /// Tests only: a live online game blocks the engines, as `online:state` sets it.
    #[doc(hidden)]
    pub fn set_engines_busy_for_tests(&self, online: bool) {
        self.busy.store(online, Ordering::SeqCst);
    }

    /// Tests only: the automatic reviews look for work again.
    #[doc(hidden)]
    pub fn reviews_changed_for_tests(&self) {
        self.reviews.reviews_changed();
    }

    /// Tests only: the search threads the engines may use now.
    #[doc(hidden)]
    pub fn engine_threads_for_tests(&self) -> usize {
        self.search.engine_threads()
    }

    /// Tests only: whether an analysis is running.
    #[doc(hidden)]
    pub fn analysis_running_for_tests(&self) -> bool {
        self.analysis.analysis_running()
    }

    /// Tests only: installs (or deletes) the managed engine under request `id`, in `dir` and
    /// from `release_url` when given.
    #[doc(hidden)]
    pub async fn managed_engine_for_tests(
        &self,
        install: bool,
        id: u64,
        dir: Option<PathBuf>,
        release_url: Option<String>,
    ) -> Result<Value> {
        let mutation = if install {
            Mutation::Install
        } else {
            Mutation::Delete
        };
        let location = LocationOverride { dir, release_url };
        self.managed_request(id, mutation, &location).await
    }

    /// Tests only: installs the puzzle database from `url`.
    #[doc(hidden)]
    pub async fn install_puzzles_for_tests(&self, url: &str) -> Result<Value> {
        json(self.puzzles.install(url.into()).await?)
    }

    /// Tests only: asynchronous storage calls queue behind the returned guard until it drops, so
    /// a test can observe work that is waiting for the database without depending on timing.
    #[doc(hidden)]
    pub async fn hold_storage_for_tests(&self) -> Result<impl Drop + use<>> {
        self.database.hold_execution().await
    }

    /// Tests only: the profile's database, to seed and read it with the store's typed calls.
    #[doc(hidden)]
    pub fn database_for_tests(&self) -> &Arc<Database> {
        &self.database
    }

    /// `The KChess core is closed.` once `close` has begun.
    fn refuse_closed(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(CoreError::new(crate::facade::CLOSED));
        }
        Ok(())
    }

    /// Starts the background work once, on the first call (which is always in a runtime): the
    /// battery refresh, and the review queue, which looks for work as soon as it starts.
    fn start(&self) {
        if !self.battery_started.swap(true, Ordering::SeqCst) {
            self.battery.refresh();
            self.battery.start_refreshing();
            self.reviews.setup_reviews(Arc::new(self.review_bridge()));
        }
    }

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

    fn managed_dir(&self, overridden: &LocationOverride) -> PathBuf {
        overridden
            .dir
            .clone()
            .unwrap_or_else(|| self.ctx.locations.managed_dir.clone())
    }

    /// Installs or deletes the managed engine under a request id, so closing the core cancels it
    /// (`cancel_installs`), and suspends the engines while the executable is replaced.
    pub(crate) async fn managed_request(
        &self,
        id: u64,
        mutation: Mutation,
        overridden: &LocationOverride,
    ) -> Result<Value> {
        let cancel = CancellationToken::new();
        crate::tasks::lock(&self.installs).insert(id, cancel.clone());
        // Forgotten however the request ends, a cancelled call included.
        let _forget = crate::tasks::on_drop(|| {
            crate::tasks::lock(&self.installs).remove(&id);
        });
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
        location.cancel = cancel;
        location.meter = self.usage.clone();
        match mutation {
            Mutation::Install => install_managed_engine(self.host.as_ref(), &location)
                .await
                .and_then(json),
            Mutation::Delete => delete_managed_engine(self.host.as_ref(), &location)
                .await
                .map(|()| Value::Null),
        }
    }

    /// Tests only: the host capabilities.
    pub fn capabilities(&self) -> Arc<Capabilities> {
        Arc::clone(&self.capabilities)
    }

    pub(crate) fn cancel_installs(&self) {
        for cancel in crate::tasks::lock(&self.installs).values() {
            cancel.cancel();
        }
    }
}
