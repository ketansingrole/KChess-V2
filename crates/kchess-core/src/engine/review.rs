//! Game review: Stockfish scores every position of a game in processes of its own (the analysis
//! board and the computer opponent keep theirs). Reviews someone asks for run at once, first a
//! quick pass so labels show within seconds and then a deeper one that refines them, on several
//! engines at a time: searching one position to a set depth, four threads do 2.6 times the work
//! of one, so each thread gets an engine and a part of the game instead
//! (`docs/benchmarks/2026-11-20-review-engines.md`). Automatic reviews of synced games run one at
//! a time on a single thread, and only while nothing else needs the computer: no online game, no
//! engine in use, and (unless allowed) not on battery. A Lichess game is first looked up on
//! Lichess, in case it was already analysed there.
//!
//! Reviews are stored through `ReviewStore` and the host is reached through `ReviewHost`. Battery
//! state comes from the engine scheduler.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use kchess_domain::records::review::{Score, is_reviewable_perf};
use kchess_domain::replay::{ReplayedPosition, replay_positions_in};
use kchess_domain::variant::Variant;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

use super::job;
use super::scheduler::Priority;
use super::slot::{EngineSlot, OpenEngine};
use super::status::fairy_variant;
use super::uci::search_cancelled;
use super::{EngineContext, after, clear_timer, replace_timer};
use crate::host::Level;
use crate::store::{Database, StoreContext, reviews};
use crate::tasks::{aborted as signal_aborted, lock};
use crate::time::now_ms;
use kchess_contracts::error::{CoreError, Result};
use kchess_domain::training::uci_info::parse_info;

/// Depth of the quick first pass, and of the review that is kept.
const QUICK_DEPTH: u32 = 10;
/// The review that is kept.
pub const FULL_DEPTH: u32 = 18;
const QUICK_MS: u64 = 400;
const FULL_MS: u64 = 5_000;
/// Positions between saves of a review in progress.
const SAVE_EVERY: usize = 6;
/// How often paused automatic reviews look again whether they may run.
const RECHECK_MS: u64 = 30_000;
/// Free the engine after this long with nothing to review.
const IDLE: Duration = Duration::from_secs(60);
/// Automatic reviews of games from this long ago (`ReviewAuto::Recent`).
const RECENT_MS: i64 = 30 * 86_400_000;
/// Principal variation moves kept per evaluation.
const PV_LENGTH: usize = 10;
/// Positions an engine takes at a time. They follow one another in the game, so what the engine
/// learned of one helps with the next; few enough that an engine on a slower core keeps the
/// others waiting for no more than one run. Over an 80-move game on four engines, runs of 5
/// search 7% more than four equal parts do, and runs of 3 21% more.
const RUN: usize = 5;
/// The hash (MiB) the engines of a review share, each given at least `MIN_HASH`.
const HASH: usize = 64;
const MIN_HASH: usize = 16;

/// The settings a review reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewSettings {
    pub review_auto: ReviewAuto,
    pub review_on_battery: bool,
    pub engine_path: String,
}

use kchess_contracts::types::{ReviewAuto, ReviewSource};
pub use kchess_contracts::types::{
    ReviewCurrent as ReviewProgress, ReviewEval, ReviewFailure, ReviewPause as Pause,
    ReviewRequest, ReviewStatus, ReviewUpdate, StoredReview,
};

/// A synced game that may be reviewed automatically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameToReview {
    pub id: String,
    pub account: String,
    pub moves: String,
    pub pgn: Option<String>,
    pub perf: String,
    pub created_at: i64,
    /// Lichess has already been asked for its analysis.
    pub checked: bool,
}

/// The automatic reviews look at most this many games per pass.
const REVIEW_LIMIT: usize = 300;

/// The reviews in `kchess.db`, through the one shared connection.
#[derive(Clone)]
pub struct ReviewStore(Arc<Database>);

pub(crate) fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|cause| CoreError::new(cause.to_string()))
}

impl ReviewStore {
    pub fn new(database: Arc<Database>) -> ReviewStore {
        ReviewStore(database)
    }

    /// The review stored under `key`.
    pub async fn read_review(&self, key: &str) -> Result<Option<StoredReview>> {
        let key = key.to_string();
        self.0
            .read_async(move |ctx| reviews::read_review(ctx.host, ctx.db, &key))
            .await
    }

    /// Saves `review` unless a better one is stored (a Lichess review beats a local one, a
    /// finished one beats one in progress), links its game, and returns what is stored with its
    /// summary. Nothing is written, and None returned, when `current` no longer holds once the
    /// database admits the write.
    pub async fn write_review_if(
        &self,
        review: StoredReview,
        current: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<ReviewUpdate>> {
        self.0
            .write_admitted(|| {
                let wanted = current();
                Ok(move |ctx: &StoreContext| {
                    wanted
                        .then(|| reviews::write_review(ctx.host, ctx.db, review))
                        .transpose()
                })
            })
            .await
    }

    /// Whether the account is still on this device.
    pub async fn has_account(&self, username: &str) -> Result<bool> {
        let username = username.to_string();
        self.0
            .read_async(move |ctx| reviews::has_account(ctx.db, &username))
            .await
    }

    /// Marks these games as looked up on Lichess, unless `current` no longer holds once the
    /// database admits the write. Whether they were marked.
    pub async fn mark_checked_if(
        &self,
        ids: &[String],
        current: &(dyn Fn() -> bool + Sync),
    ) -> Result<bool> {
        let ids = ids.to_vec();
        self.0
            .write_admitted(|| {
                let wanted = current();
                let at = now_ms() as f64;
                Ok(move |ctx: &StoreContext| {
                    if wanted {
                        reviews::mark_checked(ctx.db, &ids, at)?;
                    }
                    Ok(wanted)
                })
            })
            .await
    }

    /// Finished games of these accounts with no finished review, newest first, played after
    /// `since` (ms).
    pub async fn games_to_review(
        &self,
        accounts: &[String],
        since: i64,
    ) -> Result<Vec<GameToReview>> {
        let accounts = accounts.to_vec();
        let value = self
            .0
            .read_async(move |ctx| {
                reviews::games_to_review(ctx.db, &accounts, since as f64, REVIEW_LIMIT as f64)
            })
            .await?;
        parse(value)
    }
}

/// The host the review talks to: settings, the accounts, what else needs the computer, Lichess,
/// and where updates and status go (`review:update` and `review:status`).
pub trait ReviewHost: Send + Sync + 'static {
    fn settings(&self) -> BoxFuture<'_, Result<ReviewSettings>>;
    /// The accounts whose games are reviewed automatically.
    fn accounts(&self) -> BoxFuture<'_, Result<Vec<String>>>;
    /// Something in the foreground wants the computer: `Engine` (the analysis board or the
    /// computer opponent) or `Online` (a live game). `None` when nothing does.
    fn busy(&self) -> Option<Pause>;
    /// Asks Lichess for its analysis of these games of `account`; returns what it has. The host
    /// marks the games checked.
    fn fetch_lichess(
        &self,
        account: &str,
        ids: &[String],
    ) -> BoxFuture<'_, Result<Vec<StoredReview>>>;
    fn update(&self, update: ReviewUpdate);
    fn status(&self, status: ReviewStatus);
}

/// One review job: the moves of a game and where its work stands.
struct Job {
    key: String,
    /// The Lichess key of a variant game Fairy-Stockfish reviews; empty for standard chess and
    /// Chess960.
    variant: String,
    fen: String,
    moves: Vec<String>,
    positions: Vec<ReplayedPosition>,
    /// The `position` suffix per ply (`""` or `" moves …"`), precomputed so the search loop does
    /// not re-join the moves on every position. The full history is kept for repetition context.
    prefixes: Vec<String>,
    game_id: Option<String>,
    account: Option<String>,
    background: AtomicBool,
    cancelled: AtomicBool,
    discarded: AtomicBool,
    /// When someone asked for this review (a sequence number; 0 for automatic reviews). A running
    /// review gives way only to a request made after it.
    requested: u64,
}

/// The review service's state.
#[derive(Default)]
struct State {
    host: Option<Arc<dyn ReviewHost>>,
    /// Reviews someone asked for, the most recent first.
    asked: Vec<Arc<Job>>,
    /// Requests admitted before their asynchronous storage reads finish.
    pending: Vec<Arc<Job>>,
    review_epoch: u64,
    /// Requests made so far (the `requested` of the next one is this plus one).
    request_seq: u64,
    running: Option<Arc<Job>>,
    progress: Option<ReviewProgress>,
    paused: Option<Pause>,
    waiting_background: usize,
    failed: Option<ReviewFailure>,
    /// Games not to try again this session.
    skipped: HashSet<String>,
    pumping: bool,
    recheck: Option<AbortHandle>,
    /// The search in progress, with the ticket that identifies it.
    search: Option<(u64, CancellationToken)>,
    search_ticket: u64,
}

struct Shared {
    ctx: EngineContext,
    /// The review's engines, made as a review first needs them. The first is kept between
    /// reviews; the others only while a review someone asked for uses them.
    engines: Mutex<Vec<Arc<EngineSlot>>>,
    storage: ReviewStore,
    state: Mutex<State>,
    /// `true` while no pump runs; `stop_reviews` waits for it (`drain`).
    idle_tx: watch::Sender<bool>,
}

/// A dropped request relinquishes admission even while storage is still pending.
struct PendingReview {
    shared: Arc<Shared>,
    requested: u64,
}

impl Drop for PendingReview {
    fn drop(&mut self) {
        lock(&self.shared.state)
            .pending
            .retain(|job| job.requested != self.requested);
    }
}

/// The review queue and worker. Cloning shares the same queue and engine.
#[derive(Clone)]
pub struct Reviews {
    shared: Arc<Shared>,
}

impl Reviews {
    pub fn new(ctx: EngineContext, database: Arc<Database>) -> Reviews {
        let storage = ReviewStore::new(database);
        let (idle_tx, _) = watch::channel(true);
        Reviews {
            shared: Arc::new(Shared {
                engines: Mutex::new(Vec::new()),
                ctx,
                storage,
                state: Mutex::new(State::default()),
                idle_tx,
            }),
        }
    }

    /// Installs the host and looks for automatic work soon.
    pub fn setup_reviews(&self, host: Arc<dyn ReviewHost>) {
        lock(&self.shared.state).host = Some(host);
        schedule_recheck(&self.shared, 10_000);
    }

    pub fn review_status(&self) -> ReviewStatus {
        review_status_of(&self.shared)
    }

    /// The stored review of these moves, if any.
    pub async fn get_review(
        &self,
        fen: &str,
        moves: &[String],
        variant: Option<&str>,
    ) -> Result<Option<StoredReview>> {
        let variant = fairy_key(variant);
        let positions = replay_positions_in(variant, fen, moves);
        if positions.is_empty() {
            return Ok(None);
        }
        let upto = (positions.len() - 1).min(moves.len());
        let key = review_key(&positions[0].fen, &moves[..upto], variant);
        self.shared.storage.read_review(&key).await
    }

    /// Reviews a game now, ahead of automatic reviews. Returns what is
    /// already stored (complete or not); updates arrive as the review goes on.
    pub async fn request_review(&self, request: ReviewRequest) -> Result<Option<StoredReview>> {
        let s = &self.shared;
        let (requested, epoch) = {
            let mut state = lock(&s.state);
            state.request_seq += 1;
            (state.request_seq, state.review_epoch)
        };
        let Some(job) = job_for(&request, false, requested)? else {
            return Ok(None);
        };
        let job = Arc::new(job);
        lock(&s.state).pending.push(Arc::clone(&job));
        let _pending = PendingReview {
            shared: Arc::clone(s),
            requested,
        };
        let current =
            || !job.cancelled.load(Ordering::SeqCst) && lock(&s.state).review_epoch == epoch;
        // A request sent just before its account was logged out or removed stores nothing.
        if let Some(account) = &job.account
            && !s.storage.has_account(account).await?
        {
            return Ok(None);
        }
        let cached = s.storage.read_review(&job.key).await?;
        if !current() {
            return Ok(None);
        }
        // Link the requesting game even when cached output is returned or its search is queued.
        let stored = match &job.game_id {
            Some(game) => {
                let mut base = cached.unwrap_or_else(|| empty_review(&job));
                base.game_id = Some(game.clone());
                match s.storage.write_review_if(base, &current).await? {
                    Some(update) => Some(update.review),
                    None => return Ok(None),
                }
            }
            None => cached,
        };
        if let Some(review) = &stored
            && review.complete
            && review.source == ReviewSource::Lichess
        {
            return Ok(Some(review.clone()));
        }
        let key = job.key.clone();
        {
            let mut state = lock(&s.state);
            if state.review_epoch != epoch || job.cancelled.load(Ordering::SeqCst) {
                return Ok(stored);
            }
            if let Some(running) = state.running.clone().filter(|running| running.key == key) {
                running.background.store(false, Ordering::SeqCst);
                return Ok(stored);
            }
            if let Some(queued) = state.asked.iter().position(|other| other.key == key) {
                state.asked.remove(queued);
            }
            // The game asked for most recently is the one being looked at: it goes first.
            state.asked.insert(0, Arc::clone(&job));
            state.skipped.remove(&key);
            if state
                .failed
                .as_ref()
                .is_some_and(|failed| failed.key == key)
            {
                state.failed = None;
            }
        }
        send_status(s);
        kick(s);
        Ok(stored)
    }

    /// Stops reviewing these moves (what is done so far is kept).
    pub fn cancel_review(&self, key: &str) {
        {
            let mut state = lock(&self.shared.state);
            for job in state.pending.iter().filter(|job| job.key == key) {
                job.cancelled.store(true, Ordering::SeqCst);
            }
            state.asked.retain(|job| job.key != key);
            if let Some(running) = state.running.clone()
                && running.key == key
            {
                running.cancelled.store(true, Ordering::SeqCst);
                if let Some((_, token)) = &state.search {
                    token.cancel();
                }
            }
        }
        send_status(&self.shared);
    }

    /// Logout must prevent an interrupted review from restoring
    /// deleted account data. Queued reviews of these accounts go, and the one running is
    /// cancelled and its output dropped.
    pub fn discard_account_reviews(&self, accounts: &[String]) {
        let names: HashSet<String> = accounts
            .iter()
            .map(|account| account.to_lowercase())
            .collect();
        {
            let mut state = lock(&self.shared.state);
            state.review_epoch += 1;
            for job in state.pending.iter().filter(|job| account_in(job, &names)) {
                job.cancelled.store(true, Ordering::SeqCst);
                job.discarded.store(true, Ordering::SeqCst);
            }
            state.asked.retain(|job| !account_in(job, &names));
            if let Some(running) = state.running.clone()
                && account_in(&running, &names)
            {
                running.cancelled.store(true, Ordering::SeqCst);
                running.discarded.store(true, Ordering::SeqCst);
                if let Some((_, token)) = &state.search {
                    token.cancel();
                }
            }
        }
        send_status(&self.shared);
    }

    /// Settings changed or a sync finished; look again for work.
    pub fn reviews_changed(&self) {
        kick(&self.shared);
    }

    /// The engine was deleted or replaced. The review in progress stops,
    /// keeping its work, and the engine is dropped.
    pub fn restart_review_engine(&self) {
        let state = lock(&self.shared.state);
        if let Some(running) = &state.running {
            running.cancelled.store(true, Ordering::SeqCst);
        }
        close_engine_locked(&self.shared, &state);
    }

    /// The app is quitting. The queue empties, the running review stops, and this
    /// returns once no pump is running.
    pub async fn stop_reviews(&self) {
        let s = &self.shared;
        {
            let mut state = lock(&s.state);
            state.review_epoch += 1;
            state.asked.clear();
            for job in &state.pending {
                job.cancelled.store(true, Ordering::SeqCst);
            }
            if let Some(running) = &state.running {
                running.cancelled.store(true, Ordering::SeqCst);
            }
            clear_timer(&mut state.recheck);
            close_engine_locked(s, &state);
            state.host = None;
        }
        let mut idle = s.idle_tx.subscribe();
        let _ = idle.wait_for(|idle| *idle).await;
        close_engines(s, 0);
        let mut state = lock(&s.state);
        state.skipped.clear();
        state.progress = None;
        state.paused = None;
        state.failed = None;
        state.waiting_background = 0;
    }
}

/// The variant a review plays by: a variant Fairy-Stockfish reviews, or standard chess (for
/// Chess960 too).
fn fairy_key(variant: Option<&str>) -> Variant {
    variant
        .and_then(Variant::from_key)
        .filter(|variant| variant.fairy_name().is_some())
        .unwrap_or_default()
}

/// The key a review of `moves` from `fen` is stored under.
fn review_key(fen: &str, moves: &[String], variant: Variant) -> String {
    kchess_domain::records::review::review_key(fen, moves, Some(variant))
}

/// The job for a request, or None when its moves reach no position.
/// `requested` orders requests (0 for automatic reviews).
fn job_for(request: &ReviewRequest, background: bool, requested: u64) -> Result<Option<Job>> {
    let variant = fairy_key(request.variant.as_deref());
    let positions = replay_positions_in(variant, &request.fen, &request.moves);
    if positions.len() < 2 {
        return Ok(None);
    }
    let moves = request.moves[..positions.len() - 1].to_vec();
    let fen = positions[0].fen.clone();
    let mut prefixes = vec![String::new()];
    let mut acc = String::new();
    for mv in &moves {
        if acc.is_empty() {
            acc.push_str(mv);
        } else {
            acc.push(' ');
            acc.push_str(mv);
        }
        prefixes.push(format!(" moves {acc}"));
    }
    let key = review_key(&fen, &moves, variant);
    Ok(Some(Job {
        key,
        variant: if variant.is_standard() {
            String::new()
        } else {
            variant.key().to_string()
        },
        fen,
        moves,
        positions,
        prefixes,
        game_id: request.game_id.clone(),
        account: request.account.clone(),
        background: AtomicBool::new(background),
        cancelled: AtomicBool::new(false),
        discarded: AtomicBool::new(false),
        requested,
    }))
}

fn account_in(job: &Job, names: &HashSet<String>) -> bool {
    job.account
        .as_ref()
        .is_some_and(|account| names.contains(&account.to_lowercase()))
}

fn empty_review(job: &Job) -> StoredReview {
    StoredReview {
        key: job.key.clone(),
        fen: job.fen.clone(),
        moves: job.moves.clone(),
        source: ReviewSource::Local,
        engine: None,
        evals: vec![None; job.positions.len()],
        depth: FULL_DEPTH,
        complete: false,
        updated_at: now_ms(),
        game_id: job.game_id.clone(),
        judgments: None,
        accuracy: None,
        variant: (!job.variant.is_empty()).then(|| job.variant.clone()),
        rest: Map::new(),
    }
}

/// Whether the evaluation carries a score.
fn has_score(score: Option<&ReviewEval>) -> bool {
    score.is_some_and(|score| score.cp.is_some() || score.mate.is_some())
}

/// The score of a finished game's last position.
fn end_eval(end: &str, turn: &str) -> ReviewEval {
    let (cp, mate) = match kchess_domain::records::review::end_eval(end, turn) {
        Score::Cp(cp) => (Some(cp), None),
        Score::Mate(mate) => (None, Some(mate)),
    };
    ReviewEval {
        cp,
        mate,
        ..ReviewEval::default()
    }
}

/// The variant a synced game of `perf` is reviewed in, if it is reviewed automatically: standard
/// chess for the standard speeds, else the variant its perf names (Stockfish reviews Chess960,
/// Fairy-Stockfish the others).
fn synced_variant(perf: &str) -> Option<&str> {
    if is_reviewable_perf(perf) {
        Some("standard")
    } else if perf == "chess960" || fairy_variant(perf).is_some() {
        Some(perf)
    } else {
        None
    }
}

fn host_of(s: &Shared) -> Option<Arc<dyn ReviewHost>> {
    lock(&s.state).host.clone()
}

fn set_paused(s: &Shared, pause: Option<Pause>) {
    lock(&s.state).paused = pause;
}

fn review_status_of(s: &Shared) -> ReviewStatus {
    let state = lock(&s.state);
    ReviewStatus {
        current: state.progress.clone(),
        waiting: state.asked.len() + state.waiting_background,
        paused: state.paused,
        failed: state.failed.clone(),
    }
}

/// The host hears of the status outside the state lock.
fn send_status(s: &Shared) {
    let Some(host) = host_of(s) else { return };
    host.status(review_status_of(s));
}

/// Starts a pump on the runtime, if there is one.
fn kick(s: &Arc<Shared>) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    runtime.spawn(pump(Arc::clone(s)));
}

/// Looks again for work after `ms`.
fn schedule_recheck(s: &Arc<Shared>, ms: u64) {
    let weak: Weak<Shared> = Arc::downgrade(s);
    let timer = after(ms, move || {
        if let Some(shared) = weak.upgrade() {
            kick(&shared);
        }
    });
    replace_timer(&mut lock(&s.state).recheck, timer);
}

/// Under the state lock: stops the searches and closes the engines.
fn close_engine_locked(s: &Shared, state: &State) {
    if let Some((_, token)) = &state.search {
        token.cancel();
    }
    close_engines(s, 0);
}

/// The review's first `count` engines, made as they are first needed.
fn engines(s: &Shared, count: usize) -> Vec<Arc<EngineSlot>> {
    let mut engines = lock(&s.engines);
    let wanted = count.max(engines.len());
    engines.resize_with(wanted, || EngineSlot::new(s.ctx.clone(), IDLE));
    engines.iter().take(count).cloned().collect()
}

/// Closes the review's engines past the first `keep`: each takes about 160 MiB, so one that
/// has no part in the work at hand is not left open.
fn close_engines(s: &Shared, keep: usize) {
    for spare in lock(&s.engines).iter().skip(keep) {
        spare.close();
    }
}

/// Nothing is left to review: only the first engine stays, until it has been idle long enough.
fn rest(s: &Shared) {
    close_engines(s, 1);
    if let Some(first) = lock(&s.engines).first() {
        first.release();
    }
}

fn close_engine(s: &Shared) {
    close_engine_locked(s, &lock(&s.state));
}

/// Why automatic reviews may not run now, if they may not.
async fn background_block(s: &Arc<Shared>) -> Result<Option<Pause>> {
    let Some(host) = host_of(s) else {
        return Ok(Some(Pause::Off));
    };
    let settings = host.settings().await?;
    if settings.review_auto == ReviewAuto::Off {
        return Ok(Some(Pause::Off));
    }
    if let Some(busy) = host.busy() {
        return Ok(Some(busy));
    }
    if !settings.review_on_battery && s.ctx.scheduler.on_battery() {
        return Ok(Some(Pause::Battery));
    }
    Ok(None)
}

/// The next synced game to review automatically, looking it up on Lichess first. Stopping clears
/// the host and moves the epoch together, so an unchanged epoch says the pass is still wanted.
async fn next_background_job(s: &Arc<Shared>) -> Result<Option<Arc<Job>>> {
    let current_epoch = |s: &Shared| lock(&s.state).review_epoch;
    loop {
        let epoch = current_epoch(s);
        let current = || epoch == current_epoch(s);
        let Some(host) = host_of(s) else {
            return Ok(None);
        };
        let settings = host.settings().await?;
        if epoch != current_epoch(s) {
            return Ok(None);
        }
        let accounts = host.accounts().await?;
        if epoch != current_epoch(s) {
            return Ok(None);
        }
        let since = if settings.review_auto == ReviewAuto::Recent {
            now_ms() - RECENT_MS
        } else {
            0
        };
        let skipped = lock(&s.state).skipped.clone();
        let mut games = Vec::new();
        for game in s.storage.games_to_review(&accounts, since).await? {
            if !skipped.contains(&game.id) && synced_variant(&game.perf).is_some() {
                games.push(game);
            }
        }
        if !current() {
            return Ok(None);
        }
        lock(&s.state).waiting_background = games.len();
        // Games not yet looked up on Lichess go there first, a batch per account.
        let unchecked: Vec<&GameToReview> = games.iter().filter(|game| !game.checked).collect();
        if let Some(first) = unchecked.first() {
            let account = first.account.clone();
            let ids: Vec<String> = unchecked
                .iter()
                .filter(|game| game.account == account)
                .map(|game| game.id.clone())
                .collect();
            match host.fetch_lichess(&account, &ids).await {
                Ok(found) => {
                    if epoch != current_epoch(s) {
                        return Ok(None);
                    }
                    for review in found {
                        let Some(update) = s.storage.write_review_if(review, &current).await?
                        else {
                            return Ok(None);
                        };
                        host.update(update);
                    }
                }
                Err(cause) => {
                    host_log(
                        s,
                        Level::Warn,
                        &format!(
                            "Lichess review lookup failed, reviewing locally: {account} {cause}"
                        ),
                    );
                    // Offline or rate limited: review locally rather than wait.
                    if epoch != current_epoch(s) {
                        return Ok(None);
                    }
                    if !s.storage.mark_checked_if(&ids, &current).await? {
                        return Ok(None);
                    }
                }
            }
            continue;
        }
        for game in games {
            let (fen, moves) = match synced_variant(&game.perf) {
                Some("standard") | None => {
                    kchess_domain::review::lichess_line(&game.moves, game.pgn.as_deref(), None)
                }
                Some(variant) => Variant::from_key(variant)
                    .and_then(|variant| {
                        kchess_domain::review::lichess_line_in(
                            variant,
                            &game.moves,
                            game.pgn.as_deref(),
                            None,
                        )
                    })
                    .unwrap_or_default(),
            };
            let variant = fairy_variant(&game.perf).map(|_| game.perf.clone());
            let request = ReviewRequest {
                fen,
                moves,
                game_id: Some(game.id.clone()),
                account: Some(game.account.clone()),
                variant,
            };
            match job_for(&request, true, 0)? {
                Some(job) => {
                    // Reviewed already (opened from the analysis board): link it to the game.
                    let existing = s.storage.read_review(&job.key).await?;
                    if !current() {
                        return Ok(None);
                    }
                    if let Some(existing) = existing.filter(|review| review.complete) {
                        let linked = StoredReview {
                            game_id: Some(game.id.clone()),
                            ..existing
                        };
                        if s.storage.write_review_if(linked, &current).await?.is_none() {
                            return Ok(None);
                        }
                        continue;
                    }
                    return Ok(Some(Arc::new(job)));
                }
                None => {
                    lock(&s.state).skipped.insert(game.id.clone());
                }
            }
        }
        lock(&s.state).waiting_background = 0;
        return Ok(None);
    }
}

fn host_log(s: &Shared, level: Level, message: &str) {
    s.ctx.host.log(level, "review", message);
}

/// Works through the queue while nothing else needs the computer. Only one pump runs.
async fn pump(s: Arc<Shared>) {
    {
        let mut state = lock(&s.state);
        if state.pumping || state.host.is_none() {
            return;
        }
        state.pumping = true;
        s.idle_tx.send_replace(false);
    }
    // A failed or panicking pass still ends as a pass: `pumping` and `idle` must be reset, or no
    // review runs again and quitting waits on `idle` forever. It looks again later, since the
    // timer that started it has fired and nothing else may.
    let (idle, failure) = match crate::tasks::caught(pump_loop(&s)).await {
        Ok(Ok(idle)) => (idle, None),
        Ok(Err(cause)) => (false, Some(cause.to_string())),
        Err(_) => (false, Some("a review step panicked".to_string())),
    };
    if let Some(cause) = failure {
        host_log(&s, Level::Warn, &format!("Review queue failed: {cause}"));
        schedule_recheck(&s, RECHECK_MS);
    }
    {
        let mut state = lock(&s.state);
        state.pumping = false;
        state.progress = None;
        // Only a pass runs reviews: one that failed or panicked mid-review left its job named
        // as running, and every later request for that game joined the dead job instead.
        state.running = None;
        state.search = None;
        s.idle_tx.send_replace(true);
    }
    send_status(&s);
    let (arm_idle, asked) = {
        let state = lock(&s.state);
        after_pass(
            state.host.is_some(),
            state.running.is_some(),
            !state.asked.is_empty(),
        )
    };
    // A review asked for while this pass was deciding it had nothing to do found it still
    // pumping and left the work to it: it would otherwise wait for the next recheck. Only then:
    // a paused pass leaves its queue for its recheck, or it would start again at once, forever.
    if idle && asked {
        kick(&s);
    }
    if arm_idle {
        rest(&s);
    }
}

/// After a pass, with the queue still set up (`present`): whether the engine's idle countdown
/// starts (nothing runs or is asked for), and whether reviews are asked for.
fn after_pass(present: bool, running: bool, asked: bool) -> (bool, bool) {
    (present && !running && !asked, present && asked)
}

/// Works through the queue; whether it ended because it found nothing to do. A pass that ends
/// paused (a live game, the setting, battery) has scheduled its own recheck instead.
async fn pump_loop(s: &Arc<Shared>) -> Result<bool> {
    loop {
        let Some(host) = host_of(s) else {
            break;
        };
        if host.busy() == Some(Pause::Online) {
            set_paused(s, Some(Pause::Online));
            send_status(s);
            schedule_recheck(s, RECHECK_MS);
            break;
        }
        let queued = {
            let mut state = lock(&s.state);
            if state.asked.is_empty() {
                None
            } else {
                Some(state.asked.remove(0))
            }
        };
        let job = match queued {
            Some(job) => job,
            None => {
                let blocked = background_block(s).await?;
                if host_of(s).is_none() {
                    break;
                }
                if let Some(pause) = blocked {
                    set_paused(s, Some(pause));
                    send_status(s);
                    schedule_recheck(s, RECHECK_MS);
                    break;
                }
                let next = next_background_job(s).await?;
                if host_of(s).is_none() {
                    break;
                }
                match next {
                    Some(job) => job,
                    None => {
                        send_status(s);
                        schedule_recheck(s, 5 * 60_000);
                        return Ok(true);
                    }
                }
            }
        };
        set_paused(s, None);
        run(s, job).await;
    }
    Ok(false)
}

/// Saves what a job produced, unless it was discarded (`publish`).
async fn publish(s: &Shared, job: &Job, review: StoredReview) -> Result<()> {
    if job.discarded.load(Ordering::SeqCst) {
        return Ok(());
    }
    let Some(host) = host_of(s) else {
        return Ok(());
    };
    let current = || !job.discarded.load(Ordering::SeqCst) && host_of(s).is_some();
    let Some(update) = s.storage.write_review_if(review, &current).await? else {
        return Ok(());
    };
    // A live game began meanwhile: keep what was done, but tell no window until it is over
    // (the pages read it again then).
    if host.busy() == Some(Pause::Online) {
        return Ok(());
    }
    host.update(update);
    Ok(())
}

/// Should the job in progress give way (to a game someone asked for, or to the foreground)?
async fn should_yield(s: &Arc<Shared>, job: &Job) -> Result<bool> {
    let host = host_of(s);
    if job.cancelled.load(Ordering::SeqCst)
        || host
            .as_ref()
            .is_none_or(|host| host.busy() == Some(Pause::Online))
    {
        return Ok(true);
    }
    // A game asked for more recently goes first; this one carries on after it. Only a newer
    // request counts: a review that gave way to an older one would otherwise yield back and forth
    // with it, and neither would search.
    if lock(&s.state)
        .asked
        .iter()
        .any(|other| other.requested > job.requested)
    {
        return Ok(true);
    }
    if !job.background.load(Ordering::SeqCst) {
        return Ok(false);
    }
    Ok(background_block(s).await?.is_some())
}

/// `run(job)`: reviews one game, keeping what is done if it is interrupted.
async fn run(s: &Arc<Shared>, job: Arc<Job>) {
    if host_of(s).is_none() {
        return;
    }
    lock(&s.state).running = Some(Arc::clone(&job));
    let mut review = match s.storage.read_review(&job.key).await {
        Ok(Some(stored)) => stored,
        Ok(None) => empty_review(&job),
        Err(cause) => {
            host_log(
                s,
                Level::Warn,
                &format!("Review failed: {} {cause}", job.key),
            );
            lock(&s.state).running = None;
            return;
        }
    };
    if review.source == ReviewSource::Lichess && review.complete {
        lock(&s.state).running = None;
        return;
    }
    if let Err(cause) = run_job(s, &job, &mut review).await {
        let search_aborted = lock(&s.state)
            .search
            .as_ref()
            .is_some_and(|(_, token)| token.is_cancelled())
            && lock(&s.engines).iter().all(|engine| engine.healthy());
        let interrupted = job.cancelled.load(Ordering::SeqCst) || cause.aborted || search_aborted;
        if interrupted {
            host_log(
                s,
                Level::Debug,
                &format!("Review interrupted: {} {cause}", job.key),
            );
            if review.evals.iter().any(Option::is_some)
                && let Err(saved) = publish(s, &job, review.clone()).await
            {
                host_log(
                    s,
                    Level::Warn,
                    &format!("Review save failed: {} {saved}", job.key),
                );
            }
            if !job.cancelled.load(Ordering::SeqCst)
                && !job.background.load(Ordering::SeqCst)
                && host_of(s).is_some()
            {
                lock(&s.state).asked.push(Arc::clone(&job));
            }
        } else {
            host_log(
                s,
                Level::Warn,
                &format!(
                    "Review failed: {} {} {cause}",
                    job.key,
                    job.game_id.clone().unwrap_or_default()
                ),
            );
            // The engine is missing or failed: try other games, and this one again next session.
            lock(&s.state)
                .skipped
                .insert(job.game_id.clone().unwrap_or_else(|| job.key.clone()));
            if !job.background.load(Ordering::SeqCst) {
                lock(&s.state).failed = Some(ReviewFailure {
                    key: job.key.clone(),
                    message: if cause.message.is_empty() {
                        "Stockfish could not review this game.".to_string()
                    } else {
                        cause.message.clone()
                    },
                });
            }
            close_engine(s);
            if review.evals.iter().any(Option::is_some)
                && let Err(saved) = publish(s, &job, review.clone()).await
            {
                host_log(
                    s,
                    Level::Warn,
                    &format!("Review save failed: {} {saved}", job.key),
                );
            }
        }
    }
    lock(&s.state).running = None;
}

/// The passes over one game: quick then deep when someone asked, deep only in the background.
async fn run_job(s: &Arc<Shared>, job: &Arc<Job>, review: &mut StoredReview) -> Result<()> {
    let total = job.positions.len();
    // A Lichess game someone opened: Lichess may have analysed it already.
    if !job.background.load(Ordering::SeqCst)
        && let (Some(game), Some(account), Some(host)) =
            (job.game_id.as_ref(), job.account.as_ref(), host_of(s))
    {
        let found = match host
            .fetch_lichess(account, std::slice::from_ref(game))
            .await
        {
            Ok(found) => found,
            Err(cause) => {
                host_log(
                    s,
                    Level::Debug,
                    &format!("Lichess review fetch failed: {game} {cause}"),
                );
                Vec::new()
            }
        };
        if let Some(matched) = found.into_iter().find(|found| found.key == job.key) {
            return publish(s, job, matched).await;
        }
    }
    let previous_game = review.game_id.take();
    review.game_id = job.game_id.clone().or(previous_game);
    while review.evals.len() < total {
        review.evals.push(None);
    }
    let Some(host) = host_of(s) else {
        return Ok(());
    };
    let settings = host.settings().await?;
    if job.cancelled.load(Ordering::SeqCst) || host_of(s).is_none() {
        return Ok(());
    }
    if !job.variant.is_empty() {
        review.variant = Some(job.variant.clone());
    }
    // The first engine names the engine the review is of.
    let first = open_engine(&engines(s, 1)[0], &settings.engine_path, &job.variant).await?;
    if review.engine.as_deref() != Some(first.key.as_str()) {
        review.engine = Some(first.key.clone());
        review.complete = false;
        review.evals = vec![None; total];
    }
    first.uci.write("ucinewgame")?;
    // How many of the review's engines have been told of the new game.
    let mut fresh = 1;
    loop {
        let passes: &[(u32, u64)] = if job.background.load(Ordering::SeqCst) {
            &[(FULL_DEPTH, FULL_MS)]
        } else {
            &[(QUICK_DEPTH, QUICK_MS), (FULL_DEPTH, FULL_MS)]
        };
        let mut ended = Ended::Done;
        for &(depth, ms) in passes {
            let pass = Pass {
                engine_path: &settings.engine_path,
                depth,
                ms,
            };
            ended = run_pass(s, job, review, &pass, &mut fresh).await?;
            if ended != Ended::Done {
                break;
            }
        }
        match ended {
            // Someone asked for the game an automatic review was on: the passes of a review
            // asked for, from the first, on as many engines.
            Ended::Promoted => {}
            Ended::Yielded => {
                publish(s, job, review.clone()).await?;
                if !job.cancelled.load(Ordering::SeqCst) && !job.background.load(Ordering::SeqCst) {
                    lock(&s.state).asked.push(Arc::clone(job));
                }
                return Ok(());
            }
            Ended::Done | Ended::Short => break,
        }
    }
    review.complete = review
        .evals
        .iter()
        .all(|eval| eval.as_ref().is_some_and(|eval| has_score(Some(eval))));
    publish(s, job, review.clone()).await?;
    if !review.complete {
        lock(&s.state)
            .skipped
            .insert(job.game_id.clone().unwrap_or_else(|| job.key.clone()));
    }
    Ok(())
}

/// One pass over a game: every position scored to `depth`, each search given `ms`.
struct Pass<'a> {
    engine_path: &'a str,
    depth: u32,
    ms: u64,
}

/// How a pass over a game ended.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ended {
    /// Every position was searched.
    Done,
    /// The review gives way (`should_yield`); it carries on later.
    Yielded,
    /// Someone asked for the game an automatic review was working on.
    Promoted,
    /// One of several engines ran out of time before the depth: a thread of this computer is
    /// too slow for an engine of its own.
    Short,
}

/// How a pass shares the computer among its engines.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Share {
    engines: usize,
    /// Search threads of each engine.
    threads: usize,
    /// Hash of each engine, in MiB.
    hash: usize,
}

/// The share of a pass with `runs` runs to search, on a computer that gives a review `threads`
/// search threads and has room for `room` engines. An automatic review keeps to one engine on
/// one thread. One asked for gets an engine per run, as many as there is room for; they divide
/// the threads and the hash, so a lone engine searches with every thread, as it did before a
/// review had several.
fn share(background: bool, threads: usize, room: usize, runs: usize) -> Share {
    if background {
        return Share {
            engines: 1,
            threads: 1,
            hash: HASH,
        };
    }
    let engines = room.min(runs).max(1);
    Share {
        engines,
        threads: (threads / engines).max(1),
        hash: (HASH / engines).max(MIN_HASH),
    }
}

/// Whether the position has a score searched to `depth` or deeper.
fn scored_to(eval: Option<&ReviewEval>, depth: u32) -> bool {
    has_score(eval) && eval.and_then(|eval| eval.depth).unwrap_or(0.0) >= f64::from(depth)
}

/// What the engines of a round share: the runs of positions not yet taken, the review they fill
/// in, and how the round ends.
struct Board<'a> {
    runs: Mutex<VecDeque<Vec<usize>>>,
    review: Mutex<&'a mut StoredReview>,
    /// Positions finished: those that needed no search, and those searched since.
    done: AtomicUsize,
    /// Positions searched since the review was last saved. Held while it is saved, so the saves
    /// follow one another and none stores less than the one before.
    unsaved: tokio::sync::Mutex<usize>,
    /// Set by the engine that ends the round early; the others stop after their position.
    ended: Mutex<Option<Ended>>,
    /// The round began as an automatic review's.
    background: bool,
}

impl Board<'_> {
    /// Ends the round, unless another engine has.
    fn end(&self, ended: Ended) {
        lock(&self.ended).get_or_insert(ended);
    }

    /// A position is searched: the status counts it, and the review is saved, at every position
    /// of the quick pass (it shows as it goes) and every few of the deep one.
    async fn searched(&self, s: &Shared, job: &Job, depth: u32) -> Result<()> {
        {
            // Counted under the state's lock, so the status never counts back.
            let mut state = lock(&s.state);
            let done = self.done.fetch_add(1, Ordering::SeqCst) + 1;
            state.progress = Some(progress(job, done));
        }
        send_status(s);
        let mut unsaved = self.unsaved.lock().await;
        *unsaved += 1;
        if depth == QUICK_DEPTH || *unsaved >= SAVE_EVERY {
            *unsaved = 0;
            let review = StoredReview::clone(&lock(&self.review));
            publish(s, job, review).await?;
        }
        Ok(())
    }
}

/// The status of a review with `done` of its positions finished: every position counts,
/// searched or already known.
fn progress(job: &Job, done: usize) -> ReviewProgress {
    ReviewProgress {
        key: job.key.clone(),
        game_id: job.game_id.clone(),
        done,
        total: job.positions.len(),
        background: job.background.load(Ordering::SeqCst),
    }
}

/// One pass: on as many engines as the computer has room for, and where one of several runs out
/// of time before the depth, again on a single engine with every thread, as before a review had
/// several, for the positions still short of it.
async fn run_pass(
    s: &Arc<Shared>,
    job: &Arc<Job>,
    review: &mut StoredReview,
    pass: &Pass<'_>,
    fresh: &mut usize,
) -> Result<Ended> {
    let room = s.ctx.scheduler.review_engines();
    let ended = round(s, job, review, pass, room, fresh).await?;
    if ended != Ended::Short {
        return Ok(ended);
    }
    round(s, job, review, pass, 1, fresh).await
}

/// One round of a pass: the positions not yet scored to its depth, dealt in runs to at most
/// `room` engines at a time, under one lease of the engine budget. Cancelling the round's
/// search reaches every engine. `fresh` counts the engines told of the new game.
async fn round(
    s: &Arc<Shared>,
    job: &Arc<Job>,
    review: &mut StoredReview,
    pass: &Pass<'_>,
    room: usize,
    fresh: &mut usize,
) -> Result<Ended> {
    let mut todo = Vec::new();
    for (index, position) in job.positions.iter().enumerate() {
        if let Some(end) = position.end {
            review.evals[index] = Some(end_eval(end, position.turn));
        } else if !scored_to(review.evals[index].as_ref(), pass.depth) {
            todo.push(index);
        }
    }
    let known = job.positions.len() - todo.len();
    lock(&s.state).progress = Some(progress(job, known));
    send_status(s);
    if todo.is_empty() {
        return Ok(Ended::Done);
    }
    // The quick pass shows the game from its first move. The deep one works from the last: an
    // engine has then just searched the position its next one leads to (9% less to search).
    if pass.depth != QUICK_DEPTH {
        todo.reverse();
    }
    let background = job.background.load(Ordering::SeqCst);
    let threads = s.ctx.scheduler.search_threads();
    let share = share(background, threads, room, todo.len().div_ceil(RUN));
    let board = Board {
        runs: Mutex::new(todo.chunks(RUN).map(<[usize]>::to_vec).collect()),
        review: Mutex::new(review),
        done: AtomicUsize::new(known),
        unsaved: tokio::sync::Mutex::new(0),
        ended: Mutex::new(None),
        background,
    };
    let token = CancellationToken::new();
    let ticket = {
        let mut state = lock(&s.state);
        state.search_ticket += 1;
        let ticket = state.search_ticket;
        state.search = Some((ticket, token.clone()));
        ticket
    };
    let outcome = s
        .ctx
        .lease(Priority::Review, &token, async {
            // Engines past the first start once the budget is the review's, and those a round
            // on fewer leaves out close.
            let mut opened = Vec::with_capacity(share.engines);
            for (index, slot) in engines(s, share.engines).iter().enumerate() {
                let engine = open_engine(slot, pass.engine_path, &job.variant).await?;
                if index >= *fresh {
                    engine.uci.write("ucinewgame")?;
                }
                opened.push(engine);
            }
            *fresh = (*fresh).max(share.engines);
            close_engines(s, share.engines);
            // Every engine's search ends before the round does: a search left behind would
            // answer the next one on its engine.
            let searches = opened
                .iter()
                .map(|engine| work(s, job, engine, &board, pass, share, &token));
            futures_util::future::join_all(searches)
                .await
                .into_iter()
                .collect::<Result<()>>()
        })
        .await;
    {
        let mut state = lock(&s.state);
        if state
            .search
            .as_ref()
            .is_some_and(|(current, _)| *current == ticket)
        {
            state.search = None;
        }
    }
    match outcome {
        Err(_) if token.is_cancelled() => Err(search_cancelled()),
        Err(cause) => Err(cause),
        Ok(()) => Ok(lock(&board.ended).unwrap_or(Ended::Done)),
    }
}

/// One engine's part of a round: run after run of positions, until none is left or the round
/// ends.
async fn work(
    s: &Arc<Shared>,
    job: &Arc<Job>,
    engine: &OpenEngine,
    board: &Board<'_>,
    pass: &Pass<'_>,
    share: Share,
    token: &CancellationToken,
) -> Result<()> {
    loop {
        let Some(run) = lock(&board.runs).pop_front() else {
            return Ok(());
        };
        for index in run {
            if lock(&board.ended).is_some() {
                return Ok(());
            }
            if should_yield(s, job).await? {
                board.end(Ended::Yielded);
                return Ok(());
            }
            if board.background && !job.background.load(Ordering::SeqCst) {
                board.end(Ended::Promoted);
                return Ok(());
            }
            let score = search(engine, job, index, pass, share, token).await?;
            let short = share.engines > 1 && !scored_to(Some(&score), pass.depth);
            if has_score(Some(&score)) {
                lock(&board.review).evals[index] = Some(score);
            }
            board.searched(s, job, pass.depth).await?;
            if short {
                board.end(Ended::Short);
                return Ok(());
            }
        }
    }
}

/// The review engine in `slot` for `configured`, started or reused. A variant game (`variant`,
/// a Lichess key) gets the bundled Fairy-Stockfish.
async fn open_engine(
    slot: &Arc<EngineSlot>,
    configured: &str,
    variant: &str,
) -> Result<OpenEngine> {
    slot.keep();
    let located = slot.locate(configured, !variant.is_empty()).await?;
    let engine = slot.open(&located)?;
    engine.uci.ready().await?;
    Ok(engine)
}

/// One position's score at the pass's depth within its time, on one of the round's engines
/// with its share of the threads and the hash. Cancelling `token` reaches the engine.
async fn search(
    target: &OpenEngine,
    job: &Job,
    index: usize,
    pass: &Pass<'_>,
    share: Share,
    token: &CancellationToken,
) -> Result<ReviewEval> {
    let score = Arc::new(Mutex::new(ReviewEval::default()));
    let (threads, hash) = (share.threads.to_string(), share.hash.to_string());
    // `ensure_options` sends an option only when it changes, so there is no round-trip per
    // position. The rules are as for analysis (`job::rules_option`): a standard-mode engine
    // never answers `go` after a king-takes-rook castling. A variant game tells
    // Fairy-Stockfish its variant instead.
    let rules = job::rules_option(fairy_variant(&job.variant));
    target
        .uci
        .ensure_options(&[
            ("Threads", threads.as_str()),
            ("Hash", hash.as_str()),
            rules,
        ])
        .await?;
    if token.is_cancelled() {
        return Err(signal_aborted());
    }
    target
        .uci
        .write(&format!("position fen {}{}", job.fen, job.prefixes[index]))?;
    let white = job.positions[index].turn == "white";
    let sink = Arc::clone(&score);
    let (depth, ms) = (pass.depth, pass.ms);
    target
        .uci
        .search(
            &format!("go depth {depth} movetime {ms}"),
            move |line: &str| {
                let Some(parsed) = parse_info(line, white) else {
                    return;
                };
                if parsed.line.rank != 1.0 {
                    return;
                }
                let line = parsed.line;
                let mut best = lock(&sink);
                *best = ReviewEval {
                    cp: if line.mate.is_some() { None } else { line.cp },
                    mate: line.mate,
                    best: line.pv.first().cloned(),
                    pv: Some(line.pv.iter().take(PV_LENGTH).cloned().collect()),
                    depth: Some(line.depth),
                };
            },
            Some(Duration::from_millis(ms + 10_000)),
            Some(token),
        )
        .await?;
    let result = lock(&score).clone();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_score_is_centipawns_or_mate() {
        let cp = ReviewEval {
            cp: Some(0.0),
            ..ReviewEval::default()
        };
        let mate = ReviewEval {
            mate: Some(2.0),
            ..ReviewEval::default()
        };
        assert!(has_score(Some(&cp)) && has_score(Some(&mate)));
        assert!(!has_score(Some(&ReviewEval::default())));
        assert!(!has_score(None));
    }

    #[test]
    fn the_engine_idles_only_when_nothing_runs_or_is_asked_for() {
        for (present, running, asked, expected) in [
            (true, false, false, (true, false)),
            (true, false, true, (false, true)),
            (true, true, false, (false, false)),
            (false, false, false, (false, false)),
            (false, false, true, (false, false)),
        ] {
            assert_eq!(
                after_pass(present, running, asked),
                expected,
                "{present} {running} {asked}"
            );
        }
    }

    /// A review asked for gets an engine for each run it has room for, sharing the threads and
    /// the hash; an automatic one keeps to one engine on one thread.
    #[test]
    fn a_pass_shares_the_threads_and_the_hash_among_its_engines() {
        let of = |engines, threads, hash| Share {
            engines,
            threads,
            hash,
        };
        assert_eq!(share(false, 4, 4, 16), of(4, 1, 16));
        assert_eq!(share(false, 4, 3, 16), of(3, 1, 21));
        assert_eq!(share(false, 4, 2, 16), of(2, 2, 32));
        assert_eq!(share(false, 4, 1, 16), of(1, 4, 64));
        // No more engines than runs to give them, and one even with nothing to search.
        assert_eq!(share(false, 4, 4, 2), of(2, 2, 32));
        assert_eq!(share(false, 4, 4, 1), of(1, 4, 64));
        assert_eq!(share(false, 4, 4, 0), of(1, 4, 64));
        assert_eq!(share(false, 2, 2, 9), of(2, 1, 32));
        // An engine never gets less than a thread, whatever room is claimed.
        assert_eq!(share(false, 2, 4, 9), of(4, 1, 16));
        assert_eq!(share(true, 4, 4, 16), of(1, 1, 64));
    }

    #[test]
    fn a_position_is_scored_to_a_depth_by_a_score_searched_that_deep() {
        let at = |depth| ReviewEval {
            cp: Some(12.0),
            depth,
            ..ReviewEval::default()
        };
        assert!(scored_to(Some(&at(Some(18.0))), 18));
        assert!(scored_to(Some(&at(Some(18.0))), 10));
        assert!(!scored_to(Some(&at(Some(10.0))), 18));
        assert!(!scored_to(Some(&at(None)), 10));
        assert!(!scored_to(None, 10));
        // A depth with no score is a search that reported nothing to keep.
        let unscored = ReviewEval {
            depth: Some(18.0),
            ..ReviewEval::default()
        };
        assert!(!scored_to(Some(&unscored), 18));
    }

    /// A game's end needs no engine: a draw is level, a mate is mate.
    #[test]
    fn the_end_of_a_game_is_scored_without_the_engine() {
        assert_eq!(end_eval("stalemate", "white").cp, Some(0.0));
        assert_eq!(end_eval("stalemate", "white").mate, None);
        assert_eq!(end_eval("checkmate", "black").mate, Some(0.0));
        assert_eq!(end_eval("checkmate", "black").cp, None);
    }
}
