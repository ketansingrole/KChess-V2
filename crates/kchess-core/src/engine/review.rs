//! Game review (`core/src/services/review.ts`): Stockfish scores every position of a game, one
//! after another, in its own process (the analysis board and the computer opponent keep theirs).
//! Reviews someone asks for run at once, first a quick pass so labels show within seconds and
//! then a deeper one that refines them. Automatic reviews of synced games run one at a time on a
//! single thread, and only while nothing else needs the computer: no online game, no engine in
//! use, and (unless allowed) not on battery. A Lichess game is first looked up on Lichess, in
//! case it was already analysed there.
//!
//! The TypeScript module state lives in `Reviews`' `State`, one per core instance. Storage is
//! reached through `ReviewStorage` and the host through `ReviewHost`, so the core can plug in the
//! real store and event delivery later. Battery state comes from the engine scheduler.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use kchess_domain::replay::{ReplayedPosition, replay_positions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

use super::status::{engine_identity, engine_status};
use super::uci::{UciController, lock, search_cancelled, signal_aborted};
use super::{EngineContext, after, clear_timer, domain, now_ms, parse_info, replace_timer};
use crate::error::{CoreError, Result};
use crate::host::Level;

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
const IDLE_MS: u64 = 60_000;
/// Automatic reviews of games from this long ago (`reviewAuto: 'recent'`).
const RECENT_MS: i64 = 30 * 86_400_000;
/// Principal variation moves kept per evaluation.
const PV_LENGTH: usize = 10;
/// The message of a review that failed without one of its own.
const NO_ENGINE: &str = "Stockfish could not be found. Choose an engine in Settings.";

/// The settings a review reads (`CoreSettings`' review fields).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSettings {
    /// `'off'`, `'recent'` or `'all'`.
    #[serde(default)]
    pub review_auto: String,
    #[serde(default)]
    pub review_on_battery: bool,
    #[serde(default)]
    pub engine_path: String,
}

/// Why automatic reviews are on hold (`ReviewStatus.paused`). `busy` reports only `Engine` and
/// `Online`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pause {
    Battery,
    Engine,
    Online,
    Off,
}

/// The score of one position (`ReviewEval`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEval {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate: Option<f64>,
    /// The engine's move in this position (UCI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best: Option<String>,
    /// The line the engine expects after its move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pv: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
}

/// A review as stored (`StoredReview`). Fields this module does not read are kept in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredReview {
    /// `reviewKey(fen, moves)`: the same moves from the same start share one review.
    pub key: String,
    pub fen: String,
    /// UCI, from `fen`.
    pub moves: Vec<String>,
    /// `'local'` or `'lichess'`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Index 0 is the starting position, index i the position after move i.
    pub evals: Vec<Option<ReviewEval>>,
    pub depth: u32,
    pub complete: bool,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What a store answers when it saves a review: the review kept and its summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewUpdate {
    pub review: StoredReview,
    /// `ReviewSummary`, as the store computes it.
    pub summary: Value,
}

/// The review being worked on (`ReviewStatus.current`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewProgress {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    pub done: usize,
    pub total: usize,
    pub background: bool,
}

/// The last review asked for that could not be done (`ReviewStatus.failed`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewFailure {
    pub key: String,
    pub message: String,
}

/// The payload of `review:status`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<ReviewProgress>,
    /// Reviews waiting, asked for and automatic.
    pub waiting: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<Pause>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<ReviewFailure>,
}

/// A game someone asked to review (`ReviewRequest`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub fen: String,
    pub moves: Vec<String>,
    /// A Lichess game: its own analysis is fetched first when it has one.
    #[serde(default)]
    pub game_id: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
}

/// A synced game that may be reviewed automatically (`GameToReview`).
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

/// The store the review reads and writes (`reviewStore.ts`'s functions, as the review calls
/// them). Implementations must be safe to call from any thread.
pub trait ReviewStorage: Send + Sync + 'static {
    /// `readReview(key)`.
    fn read_review(&self, key: &str) -> Result<Option<StoredReview>>;
    /// `writeReview(review)`: saves `review` unless a better one is stored (a Lichess review
    /// beats a local one, a finished one beats one in progress), links its game, and returns
    /// what is stored with its summary.
    fn write_review(&self, review: StoredReview) -> Result<ReviewUpdate>;
    /// `hasAccount(username)`: whether the account is still on this device.
    fn has_account(&self, username: &str) -> Result<bool>;
    /// `markChecked(ids)`: Lichess was asked for these games' analysis.
    fn mark_checked(&self, ids: &[String]) -> Result<()>;
    /// `gamesToReview(accounts, since)`: finished games of these accounts with no finished
    /// review, newest first, played after `since` (ms).
    fn games_to_review(&self, accounts: &[String], since: i64) -> Result<Vec<GameToReview>>;
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
    /// `review:update`.
    fn update(&self, update: ReviewUpdate);
    /// `review:status`.
    fn status(&self, status: ReviewStatus);
}

/// One review job: the moves of a game and where its work stands.
struct Job {
    key: String,
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

#[derive(Clone)]
struct ReviewEngine {
    key: String,
    uci: Arc<UciController>,
}

/// `serviceState` of `review.ts`.
#[derive(Default)]
struct State {
    host: Option<Arc<dyn ReviewHost>>,
    /// Reviews someone asked for, the most recent first.
    asked: Vec<Arc<Job>>,
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
    engine: Option<ReviewEngine>,
    /// The search in progress, with the ticket that identifies it (`searchController`).
    search: Option<(u64, CancellationToken)>,
    search_ticket: u64,
    idle: Option<AbortHandle>,
}

struct Shared {
    ctx: EngineContext,
    storage: Arc<dyn ReviewStorage>,
    state: Mutex<State>,
    /// `true` while no pump runs; `stop_reviews` waits for it (`drain`).
    idle_tx: watch::Sender<bool>,
}

/// The review queue and worker. Cloning shares the same queue and engine.
#[derive(Clone)]
pub struct Reviews {
    shared: Arc<Shared>,
}

impl Reviews {
    pub fn new(ctx: EngineContext, storage: Arc<dyn ReviewStorage>) -> Reviews {
        let (idle_tx, _) = watch::channel(true);
        Reviews {
            shared: Arc::new(Shared {
                ctx,
                storage,
                state: Mutex::new(State::default()),
                idle_tx,
            }),
        }
    }

    /// `setupReviews`: installs the host and looks for automatic work soon.
    pub fn setup_reviews(&self, host: Arc<dyn ReviewHost>) {
        lock(&self.shared.state).host = Some(host);
        schedule_recheck(&self.shared, 10_000);
    }

    /// `reviewStatus`.
    pub fn review_status(&self) -> ReviewStatus {
        review_status_of(&self.shared)
    }

    /// `getReview(fen, moves)`: the stored review of these moves, if any.
    pub fn get_review(&self, fen: &str, moves: &[String]) -> Result<Option<StoredReview>> {
        let positions = replay_positions(fen, moves);
        if positions.is_empty() {
            return Ok(None);
        }
        let upto = (positions.len() - 1).min(moves.len());
        let key = review_key(&positions[0].fen, &moves[..upto])?;
        self.shared.storage.read_review(&key)
    }

    /// `requestReview(request)`: reviews a game now, ahead of automatic reviews. Returns what is
    /// already stored (complete or not); updates arrive as the review goes on.
    pub fn request_review(&self, request: ReviewRequest) -> Result<Option<StoredReview>> {
        let s = &self.shared;
        let requested = {
            let mut state = lock(&s.state);
            state.request_seq += 1;
            state.request_seq
        };
        let Some(job) = job_for(&request, false, requested)? else {
            return Ok(None);
        };
        // A request sent just before its account was logged out or removed stores nothing.
        if let Some(account) = &job.account
            && !s.storage.has_account(account)?
        {
            return Ok(None);
        }
        let cached = s.storage.read_review(&job.key)?;
        // Link the requesting game even when cached output is returned or its search is queued.
        let stored = match &job.game_id {
            Some(game) => {
                let mut base = cached.unwrap_or_else(|| empty_review(&job));
                base.game_id = Some(game.clone());
                Some(s.storage.write_review(base)?.review)
            }
            None => cached,
        };
        if let Some(review) = &stored
            && review.complete
            && review.source == "lichess"
        {
            return Ok(Some(review.clone()));
        }
        let key = job.key.clone();
        {
            let mut state = lock(&s.state);
            if let Some(running) = state.running.clone().filter(|running| running.key == key) {
                running.background.store(false, Ordering::SeqCst);
                return Ok(stored);
            }
            if let Some(queued) = state.asked.iter().position(|other| other.key == key) {
                state.asked.remove(queued);
            }
            // The game asked for most recently is the one being looked at: it goes first.
            state.asked.insert(0, Arc::new(job));
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

    /// `cancelReview(key)`: stops reviewing these moves (what is done so far is kept).
    pub fn cancel_review(&self, key: &str) {
        {
            let mut state = lock(&self.shared.state);
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

    /// `discardAccountReviews(accounts)`: logout must prevent an interrupted review from restoring
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

    /// `reviewsChanged`: settings changed or a sync finished; look again for work.
    pub fn reviews_changed(&self) {
        kick(&self.shared);
    }

    /// `restartReviewEngine`: the engine was deleted or replaced. The review in progress stops,
    /// keeping its work, and the engine is dropped.
    pub fn restart_review_engine(&self) {
        let mut state = lock(&self.shared.state);
        if let Some(running) = &state.running {
            running.cancelled.store(true, Ordering::SeqCst);
        }
        close_engine_locked(&mut state);
    }

    /// `stopReviews`: the app is quitting. The queue empties, the running review stops, and this
    /// returns once no pump is running.
    pub async fn stop_reviews(&self) {
        let s = &self.shared;
        {
            let mut state = lock(&s.state);
            state.review_epoch += 1;
            state.asked.clear();
            if let Some(running) = &state.running {
                running.cancelled.store(true, Ordering::SeqCst);
            }
            clear_timer(&mut state.recheck);
            close_engine_locked(&mut state);
            state.host = None;
        }
        let mut idle = s.idle_tx.subscribe();
        let _ = idle.wait_for(|idle| *idle).await;
        let mut state = lock(&s.state);
        clear_timer(&mut state.idle);
        state.skipped.clear();
        state.progress = None;
        state.paused = None;
        state.failed = None;
        state.waiting_background = 0;
    }
}

/// `reviewKey(fen, moves)` for a list of moves.
fn review_key(fen: &str, moves: &[String]) -> Result<String> {
    let key = domain("reviewKey", vec![json!(fen), json!(moves)])?;
    key.as_str()
        .map(str::to_string)
        .ok_or_else(|| CoreError::new("reviewKey returned no key"))
}

/// `jobFor(request, background)`: the job for a request, or None when its moves reach no position.
/// `requested` orders requests (0 for automatic reviews).
fn job_for(request: &ReviewRequest, background: bool, requested: u64) -> Result<Option<Job>> {
    let positions = replay_positions(&request.fen, &request.moves);
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
    let key = review_key(&fen, &moves)?;
    Ok(Some(Job {
        key,
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
        source: "local".to_string(),
        engine: None,
        evals: vec![None; job.positions.len()],
        depth: FULL_DEPTH,
        complete: false,
        updated_at: now_ms(),
        game_id: job.game_id.clone(),
        extra: Map::new(),
    }
}

/// `hasScore`: whether the evaluation carries a score (the rules decide).
fn has_score(score: Option<&ReviewEval>) -> bool {
    let value = score.map_or(Value::Null, |score| {
        serde_json::to_value(score).unwrap_or(Value::Null)
    });
    domain("hasScore", vec![value])
        .ok()
        .and_then(|result| result.as_bool())
        .unwrap_or(false)
}

/// `endEval(end)`: the score of a finished game's last position.
fn end_eval(end: &str) -> Result<ReviewEval> {
    let score = domain("endEval", vec![json!(end)])?;
    serde_json::from_value(score).map_err(|cause| CoreError::new(cause.to_string()))
}

/// `isReviewablePerf(perf)`: standard speeds only.
fn is_reviewable_perf(perf: &str) -> Result<bool> {
    Ok(domain("isReviewablePerf", vec![json!(perf)])?
        .as_bool()
        .unwrap_or(false))
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

/// `sendStatus`: the host hears of the status outside the state lock.
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

/// `scheduleRecheck(ms)`: looks again for work after `ms`.
fn schedule_recheck(s: &Arc<Shared>, ms: u64) {
    let weak: Weak<Shared> = Arc::downgrade(s);
    let timer = after(ms, move || {
        if let Some(shared) = weak.upgrade() {
            kick(&shared);
        }
    });
    replace_timer(&mut lock(&s.state).recheck, timer);
}

/// `closeEngine` under the state lock: forgets the engine and stops its search.
fn close_engine_locked(state: &mut State) {
    clear_timer(&mut state.idle);
    if let Some((_, token)) = &state.search {
        token.cancel();
    }
    if let Some(engine) = state.engine.take() {
        engine.uci.close();
    }
}

fn close_engine(s: &Shared) {
    close_engine_locked(&mut lock(&s.state));
}

/// Why automatic reviews may not run now, if they may not (`backgroundBlock`).
async fn background_block(s: &Arc<Shared>) -> Result<Option<Pause>> {
    let Some(host) = host_of(s) else {
        return Ok(Some(Pause::Off));
    };
    let settings = host.settings().await?;
    if settings.review_auto == "off" {
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

/// The next synced game to review automatically, looking it up on Lichess first.
async fn next_background_job(s: &Arc<Shared>) -> Result<Option<Arc<Job>>> {
    let current_epoch = |s: &Shared| lock(&s.state).review_epoch;
    loop {
        let epoch = current_epoch(s);
        let Some(host) = host_of(s) else {
            return Ok(None);
        };
        let settings = host.settings().await?;
        if host_of(s).is_none() || epoch != current_epoch(s) {
            return Ok(None);
        }
        let accounts = host.accounts().await?;
        if epoch != current_epoch(s) {
            return Ok(None);
        }
        let since = if settings.review_auto == "recent" {
            now_ms() - RECENT_MS
        } else {
            0
        };
        let skipped = lock(&s.state).skipped.clone();
        let mut games = Vec::new();
        for game in s.storage.games_to_review(&accounts, since)? {
            if !skipped.contains(&game.id) && is_reviewable_perf(&game.perf)? {
                games.push(game);
            }
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
                        let update = s.storage.write_review(review)?;
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
                    s.storage.mark_checked(&ids)?;
                }
            }
            continue;
        }
        for game in games {
            let (fen, moves) =
                kchess_domain::review::lichess_line(&game.moves, game.pgn.as_deref(), None);
            let request = ReviewRequest {
                fen,
                moves,
                game_id: Some(game.id.clone()),
                account: Some(game.account.clone()),
            };
            match job_for(&request, true, 0)? {
                Some(job) => {
                    // Reviewed already (opened from the analysis board): link it to the game.
                    if let Some(existing) = s.storage.read_review(&job.key)?
                        && existing.complete
                    {
                        s.storage.write_review(StoredReview {
                            game_id: Some(game.id.clone()),
                            ..existing
                        })?;
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

/// `pump`: works through the queue while nothing else needs the computer. Only one pump runs.
async fn pump(s: Arc<Shared>) {
    {
        let mut state = lock(&s.state);
        if state.pumping || state.host.is_none() {
            return;
        }
        state.pumping = true;
        s.idle_tx.send_replace(false);
    }
    if let Err(cause) = pump_loop(&s).await {
        host_log(&s, Level::Warn, &format!("Review queue failed: {cause}"));
    }
    {
        let mut state = lock(&s.state);
        state.pumping = false;
        state.progress = None;
        s.idle_tx.send_replace(true);
    }
    send_status(&s);
    let arm_idle = {
        let state = lock(&s.state);
        state.host.is_some() && state.running.is_none() && state.asked.is_empty()
    };
    if arm_idle {
        let weak = Arc::downgrade(&s);
        let timer = after(IDLE_MS, move || {
            if let Some(shared) = weak.upgrade() {
                close_engine(&shared);
            }
        });
        replace_timer(&mut lock(&s.state).idle, timer);
    }
}

async fn pump_loop(s: &Arc<Shared>) -> Result<()> {
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
                        break;
                    }
                }
            }
        };
        set_paused(s, None);
        run(s, job).await;
    }
    Ok(())
}

/// Saves what a job produced, unless it was discarded (`publish`).
fn publish(s: &Shared, job: &Job, review: StoredReview) -> Result<()> {
    if job.discarded.load(Ordering::SeqCst) {
        return Ok(());
    }
    let Some(host) = host_of(s) else {
        return Ok(());
    };
    let update = s.storage.write_review(review)?;
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
    let mut review = match s.storage.read_review(&job.key) {
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
    if review.source == "lichess" && review.complete {
        lock(&s.state).running = None;
        return;
    }
    if let Err(cause) = run_job(s, &job, &mut review).await {
        let search_aborted = {
            let state = lock(&s.state);
            state
                .search
                .as_ref()
                .is_some_and(|(_, token)| token.is_cancelled())
                && state
                    .engine
                    .as_ref()
                    .is_none_or(|engine| engine.uci.failed().is_none())
        };
        let interrupted = job.cancelled.load(Ordering::SeqCst) || cause.aborted || search_aborted;
        if interrupted {
            host_log(
                s,
                Level::Debug,
                &format!("Review interrupted: {} {cause}", job.key),
            );
            if review.evals.iter().any(Option::is_some)
                && let Err(saved) = publish(s, &job, review.clone())
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
                && let Err(saved) = publish(s, &job, review.clone())
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
            return publish(s, job, matched);
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
    let target = open_engine(s, &settings.engine_path).await?;
    if review.engine.as_deref() != Some(target.key.as_str()) {
        review.engine = Some(target.key.clone());
        review.complete = false;
        review.evals = vec![None; total];
    }
    target.uci.write("ucinewgame")?;
    target.uci.write("setoption name Hash value 64")?;
    let passes: Vec<(u32, u64)> = if job.background.load(Ordering::SeqCst) {
        vec![(FULL_DEPTH, FULL_MS)]
    } else {
        vec![(QUICK_DEPTH, QUICK_MS), (FULL_DEPTH, FULL_MS)]
    };
    for (depth, ms) in passes {
        let mut since_save = 0usize;
        for index in 0..total {
            let known = review.evals[index].clone();
            if let Some(end) = job.positions[index].end {
                review.evals[index] = Some(end_eval(end)?);
            } else if !has_score(known.as_ref())
                || known.as_ref().and_then(|known| known.depth).unwrap_or(0.0) < f64::from(depth)
            {
                if should_yield(s, job).await? {
                    publish(s, job, review.clone())?;
                    if !job.cancelled.load(Ordering::SeqCst)
                        && !job.background.load(Ordering::SeqCst)
                    {
                        lock(&s.state).asked.push(Arc::clone(job));
                    }
                    return Ok(());
                }
                let score = search(s, &target, job, index, depth, ms).await?;
                if has_score(Some(&score)) {
                    review.evals[index] = Some(score);
                }
                since_save += 1;
            }
            lock(&s.state).progress = Some(ReviewProgress {
                key: job.key.clone(),
                game_id: job.game_id.clone(),
                // Positions finished so far: every position counts, searched or already known.
                done: index + 1,
                total,
                background: job.background.load(Ordering::SeqCst),
            });
            send_status(s);
            // The quick pass shows as it goes; the deep one replaces it a few positions at a time.
            if depth == QUICK_DEPTH || since_save >= SAVE_EVERY {
                publish(s, job, review.clone())?;
                since_save = 0;
            }
        }
    }
    review.complete = review
        .evals
        .iter()
        .all(|eval| eval.as_ref().is_some_and(|eval| has_score(Some(eval))));
    publish(s, job, review.clone())?;
    if !review.complete {
        lock(&s.state)
            .skipped
            .insert(job.game_id.clone().unwrap_or_else(|| job.key.clone()));
    }
    Ok(())
}

/// The review engine for `configured`, started or reused (`openEngine`).
async fn open_engine(s: &Arc<Shared>, configured: &str) -> Result<ReviewEngine> {
    clear_timer(&mut lock(&s.state).idle);
    let status = engine_status(&*s.ctx.host, &s.ctx.locations, configured).await;
    if !status.ready {
        return Err(CoreError::new(NO_ENGINE));
    }
    let key = engine_identity(&status).await?;
    let engine = {
        let mut state = lock(&s.state);
        let current = state
            .engine
            .as_ref()
            .is_some_and(|engine| engine.key == key && engine.uci.failed().is_none());
        if !current {
            close_engine_locked(&mut state);
            let uci = Arc::new(s.ctx.spawn(&status)?);
            state.engine = Some(ReviewEngine {
                key: key.clone(),
                uci,
            });
        }
        state
            .engine
            .clone()
            .ok_or_else(|| CoreError::new("The engine is unavailable."))?
    };
    engine.uci.ready().await?;
    Ok(engine)
}

/// One position's score at `depth` within `ms` (`search`). Cancelling it reaches the engine.
async fn search(
    s: &Arc<Shared>,
    target: &ReviewEngine,
    job: &Arc<Job>,
    index: usize,
    depth: u32,
    ms: u64,
) -> Result<ReviewEval> {
    let token = CancellationToken::new();
    let ticket = {
        let mut state = lock(&s.state);
        state.search_ticket += 1;
        let ticket = state.search_ticket;
        state.search = Some((ticket, token.clone()));
        ticket
    };
    let outcome = search_once(s, target, job, index, depth, ms, &token).await;
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
        other => other,
    }
}

async fn search_once(
    s: &Arc<Shared>,
    target: &ReviewEngine,
    job: &Arc<Job>,
    index: usize,
    depth: u32,
    ms: u64,
    token: &CancellationToken,
) -> Result<ReviewEval> {
    let score = Arc::new(Mutex::new(ReviewEval::default()));
    let stop_token = token.clone();
    s.ctx
        .lease(1, move || stop_token.cancel(), token, async {
            let host = host_of(s);
            if job.cancelled.load(Ordering::SeqCst)
                || host
                    .as_ref()
                    .is_none_or(|host| host.busy() == Some(Pause::Online))
            {
                return Err(search_cancelled());
            }
            // Threads is constant within a job (1 in the background, the budget otherwise), so
            // confirming it once avoids a round-trip per position.
            let threads = if job.background.load(Ordering::SeqCst) {
                1
            } else {
                s.ctx.scheduler.search_threads()
            };
            let threads_text = threads.to_string();
            target
                .uci
                .ensure_options(&[("Threads", threads_text.as_str())])
                .await?;
            if token.is_cancelled() {
                return Err(signal_aborted());
            }
            target
                .uci
                .write(&format!("position fen {}{}", job.fen, job.prefixes[index]))?;
            let white = job.positions[index].turn == "white";
            let sink = Arc::clone(&score);
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
        })
        .await
}
