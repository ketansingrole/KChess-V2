//! The offline puzzle database (`puzzles.db`), which this service alone owns
//! (formerly `crates/kchess-node/js/puzzleWorker.ts`). Lichess publishes every rated puzzle as one
//! CC0 file; the service streams it once, keeps a sample (`kchess_domain::puzzle::Sampler`) and
//! stores it, so Storm, Streak, Rush and offline practice work without the original.

pub mod attempts;
pub mod queries;
mod zst;

use futures_util::StreamExt;
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::error::{CoreError, Result};
use crate::host::{Config, Host, Level};
use kchess_domain::puzzle::{Random, Sampler};
use queries::{LocalLadderQuery, LocalPuzzleQuery, Puzzle, PuzzleDbStatus};

pub const PUZZLE_DB_URL: &str = "https://database.lichess.org/lichess_db_puzzle.csv.zst";

/// One running download: cancelled through the token.
struct Download {
    cancel: CancellationToken,
}

struct Inner {
    config: Config,
    host: Arc<dyn Host>,
    db: Mutex<Option<Connection>>,
    execution: Arc<tokio::sync::Semaphore>,
    work: tokio_util::task::TaskTracker,
    running: Mutex<Option<Arc<Download>>>,
    closed: AtomicBool,
}

pub struct PuzzleService {
    inner: Arc<Inner>,
}

/// What `report` sends: `PuzzleDbProgress`.
#[derive(Clone, Copy)]
enum Phase {
    Downloading,
    Importing,
    Done,
    Cancelled,
    Failed,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Downloading => "downloading",
            Phase::Importing => "importing",
            Phase::Done => "done",
            Phase::Cancelled => "cancelled",
            Phase::Failed => "failed",
        }
    }
}

/// The schema, permissions and one-time import from the old main database.
fn open(config: &Config, host: &dyn Host) -> Result<Connection> {
    let path = config.data_dir.join("puzzles.db");
    let db = Connection::open(&path)
        .map_err(|e| CoreError::wrap("Could not open the puzzle database", e))?;
    db.execute_batch("PRAGMA busy_timeout = 5000;")
        .map_err(|e| CoreError::new(e.to_string()))?;
    queries::create_schema(&db)?;
    attempts::create_schema(&db)?;
    restrict(&path, host);
    let migrated = db
        .query_row("SELECT id FROM worker_meta WHERE id = 1", [], |_| Ok(()))
        .is_ok();
    // Earlier releases kept the puzzles in `kchess.db`, the main database. It stays an intact
    // fallback; all later puzzle writes have one owner.
    let legacy = config.data_dir.join("kchess.db");
    if !migrated && legacy.exists() {
        db.execute(
            "ATTACH DATABASE ?1 AS legacy",
            [legacy.to_string_lossy().as_ref()],
        )
        .map_err(|e| CoreError::new(e.to_string()))?;
        // A main database the store has not migrated yet has no puzzle tables to read; leave the
        // import for a later open rather than marking it done.
        let has_tables: bool = db
            .query_row(
                "SELECT COUNT(*) = 2 FROM legacy.sqlite_master
                 WHERE type = 'table' AND name IN ('puzzles', 'puzzle_meta')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| CoreError::new(e.to_string()))?;
        if has_tables {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                INSERT OR IGNORE INTO puzzles (id, fen, moves, rating, plays, themes)
                    SELECT id, fen, moves, rating, plays, themes FROM legacy.puzzles;
                INSERT OR IGNORE INTO puzzle_meta SELECT id, importedAt, count, bytes FROM legacy.puzzle_meta;
                INSERT OR REPLACE INTO worker_meta VALUES (1);
                COMMIT;",
            )
            .map_err(|e| match db.execute_batch("ROLLBACK") {
                Ok(()) => CoreError::new(e.to_string()),
                Err(rollback) => CoreError::new(e.to_string()).with_rollback_failure(rollback),
            })?;
        }
        db.execute_batch("DETACH DATABASE legacy")
            .map_err(|e| CoreError::new(e.to_string()))?;
    }
    db.execute_batch("INSERT OR IGNORE INTO worker_meta VALUES (1)")
        .map_err(|e| CoreError::new(e.to_string()))?;
    Ok(db)
}

fn restrict(path: &Path, host: &dyn Host) {
    crate::store::restrict_database(path, host, "puzzles");
}

/// A random sampler seed, as `randomInt(2 ** 32)` gave one.
fn seed() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64),
    );
    hasher.finish() as u32
}

/// `Math.random()` for the queries' shuffles.
fn random() -> impl FnMut() -> f64 {
    let mut generator = Random::new(seed());
    move || generator.next_f64()
}

impl Inner {
    /// Run `work` on the database off the async threads, opening it on first use.
    /// Admission is serialized on the execution semaphore so `close` drains cleanly;
    /// the closed flag is rechecked while holding the connection lock to close the
    /// reopen-after-close race.
    async fn with_db<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = Arc::clone(&self.execution)
            .acquire_owned()
            .await
            .map_err(|_| CoreError::new("Puzzle service closed."))?;
        let tracked = self.work.token();
        let inner = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let (_permit, _tracked) = (permit, tracked);
            let mut db = inner
                .db
                .lock()
                .map_err(|_| CoreError::new("Puzzle service stopped. Retry the operation."))?;
            if inner.closed.load(Ordering::SeqCst) {
                return Err(CoreError::new("Puzzle service closed."));
            }
            if db.is_none() {
                *db = Some(open(&inner.config, inner.host.as_ref())?);
                if inner.closed.load(Ordering::SeqCst) {
                    db.take();
                    return Err(CoreError::new("Puzzle service closed."));
                }
            }
            match db.as_ref() {
                Some(connection) => work(connection),
                None => Err(CoreError::new(
                    "Puzzle service stopped. Retry the operation.",
                )),
            }
        })
        .await
        .map_err(|_| CoreError::new("Puzzle service stopped. Retry the operation."))?
    }

    fn running(&self) -> Option<Arc<Download>> {
        self.running.lock().ok().and_then(|r| r.clone())
    }

    fn usage(&self, requests: u32, bytes: u64) {
        self.host.emit(
            "host:usage",
            json!({ "account": "", "category": "database", "requests": requests, "bytes": bytes }),
        );
    }

    fn failed(&self, method: &str, error: &CoreError) {
        self.host.log(
            Level::Warn,
            "puzzles",
            &format!("Puzzle operation {method} failed {}", error.message),
        );
    }
}

impl PuzzleService {
    pub fn new(config: Config, host: Arc<dyn Host>) -> PuzzleService {
        PuzzleService {
            inner: Arc::new(Inner {
                config,
                host,
                db: Mutex::new(None),
                execution: Arc::new(tokio::sync::Semaphore::new(1)),
                work: tokio_util::task::TaskTracker::new(),
                running: Mutex::new(None),
                closed: AtomicBool::new(false),
            }),
        }
    }

    async fn logged<T>(&self, method: &str, result: Result<T>) -> Result<T> {
        if let Err(error) = &result {
            self.inner.failed(method, error);
        }
        result
    }

    pub async fn status(&self) -> Result<PuzzleDbStatus> {
        let running = self.inner.running().is_some();
        let result = self
            .inner
            .with_db(move |db| Ok(queries::read_status(db, running)?))
            .await;
        self.logged("status", result).await
    }

    pub async fn query(&self, query: LocalPuzzleQuery) -> Result<Vec<Puzzle>> {
        let result = self
            .inner
            .with_db(move |db| Ok(queries::query_puzzles(db, &query, &mut random())?))
            .await;
        self.logged("query", result).await
    }

    pub async fn ladder(&self, query: LocalLadderQuery) -> Result<Vec<Puzzle>> {
        let result = self
            .inner
            .with_db(move |db| Ok(queries::query_ladder(db, &query, &mut random())?))
            .await;
        self.logged("ladder", result).await
    }

    /// Keep a training attempt; the progress it leaves, as of the attempt's own day.
    pub async fn record_attempt(
        &self,
        attempt: attempts::AttemptInput,
    ) -> Result<serde_json::Value> {
        let result = self
            .inner
            .with_db(move |db| {
                attempts::record(db, &attempt, queries::now_ms())?;
                let query = attempts::ProgressQuery {
                    today: attempt.day.clone(),
                    limit: attempts::RECENT,
                };
                Ok(attempts::progress(db, &query)?)
            })
            .await;
        self.logged("recordAttempt", result).await
    }

    pub async fn progress(&self, query: attempts::ProgressQuery) -> Result<serde_json::Value> {
        let result = self
            .inner
            .with_db(move |db| Ok(attempts::progress(db, &query)?))
            .await;
        self.logged("progress", result).await
    }

    pub async fn delete(&self) -> Result<PuzzleDbStatus> {
        let result = if self.inner.running().is_some() {
            Err(CoreError::new(
                "Cancel the download before deleting puzzles.",
            ))
        } else {
            self.inner
                .with_db(|db| {
                    queries::clear_stored(db)?;
                    Ok(queries::read_status(db, false)?)
                })
                .await
        };
        self.logged("delete", result).await
    }

    /// Stop a running download; it resolves with the current status.
    pub fn cancel(&self) {
        if let Some(download) = self.inner.running() {
            download.cancel.cancel();
        }
    }

    pub async fn close(&self) {
        self.cancel();
        self.inner.closed.store(true, Ordering::SeqCst);
        self.inner.execution.close();
        self.inner.work.close();
        self.inner.work.wait().await;
        let inner = Arc::clone(&self.inner);
        // Closing drains admitted workers first, then releases the file.
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(mut db) = inner.db.lock() {
                db.take();
            }
        })
        .await;
    }

    /// Download, sample and store the puzzle database, reporting `puzzledb:progress`.
    /// Cancellation resolves with the current status: callers distinguish cancel from
    /// completion via the `puzzledb:progress` `cancelled`/`done` phase, not the result.
    pub async fn install(&self, url: String) -> Result<PuzzleDbStatus> {
        let download = {
            let mut running = self
                .inner
                .running
                .lock()
                .map_err(|_| CoreError::new("Puzzle service stopped. Retry the operation."))?;
            if running.is_some() {
                let error = CoreError::new("The puzzle database is already downloading.");
                self.inner.failed("install", &error);
                return Err(error);
            }
            let download = Arc::new(Download {
                cancel: CancellationToken::new(),
            });
            *running = Some(Arc::clone(&download));
            download
        };
        let mut progress = Progress {
            host: Arc::clone(&self.inner.host),
            received: 0,
            total: None,
            last: None,
        };
        let result = self.download(&url, &download, &mut progress).await;
        let aborted = download.cancel.is_cancelled();
        let outcome = match result {
            Ok(()) => Ok(()),
            Err(error) => {
                progress.finished(
                    if aborted {
                        Phase::Cancelled
                    } else {
                        Phase::Failed
                    },
                    (!aborted).then_some(error.message.as_str()),
                );
                if aborted { Ok(()) } else { Err(error) }
            }
        };
        self.inner.usage(0, progress.received);
        if let Ok(mut running) = self.inner.running.lock() {
            running.take();
        }
        let result = match outcome {
            Ok(()) => {
                self.inner
                    .with_db(|db| Ok(queries::read_status(db, false)?))
                    .await
            }
            Err(error) => Err(error),
        };
        self.logged("install", result).await
    }

    async fn download(
        &self,
        url: &str,
        download: &Arc<Download>,
        progress: &mut Progress,
    ) -> Result<()> {
        let cancelled = || CoreError::aborted("Cancelled");
        let cancel = download.cancel.clone();
        let response = tokio::select! {
            response = reqwest::get(url) => response.map_err(|e| CoreError::new(fetch_failed(&e))),
            _ = cancel.cancelled() => Err(cancelled()),
        }?;
        self.inner.usage(1, 0);
        if !response.status().is_success() {
            return Err(CoreError::new(format!(
                "Lichess answered {} for the puzzle database.",
                response.status().as_u16()
            )));
        }
        progress.total = response.content_length().filter(|&n| n > 0);
        // Decompression and sampling run on a blocking thread, fed chunk by chunk.
        let (send, receive) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
        let counted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = Arc::clone(&counted);
        let sampling =
            tokio::task::spawn_blocking(move || -> Result<(Vec<queries::DbPuzzle>, usize)> {
                // The decompressed CSV goes straight into the sampler.
                let mut sampler = Sampler::new(Random::new(seed()));
                zst::decode(zst::ChannelReader::new(receive), |chunk| {
                    sampler
                        .push(chunk)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                    count.store(sampler.count(), Ordering::Relaxed);
                    Ok(())
                })
                .map_err(|e| CoreError::new(e.to_string()))?;
                sampler.finish().map_err(CoreError::new)?;
                let rows = sampler
                    .kept()
                    .into_iter()
                    .map(|row| queries::DbPuzzle {
                        id: row.id.clone(),
                        fen: row.fen.clone(),
                        moves: row.moves.clone(),
                        rating: row.rating,
                        plays: Some(row.plays),
                        themes: row.themes.clone(),
                        openings: row.openings.clone(),
                        game_id: row.game_id.clone(),
                    })
                    .collect();
                Ok((rows, sampler.count()))
            });
        let mut stream = response.bytes_stream();
        let cancel = download.cancel.clone();
        let fed: Result<()> = async {
            loop {
                let chunk = tokio::select! {
                    chunk = stream.next() => chunk,
                    _ = cancel.cancelled() => return Err(cancelled()),
                };
                let Some(chunk) = chunk else { return Ok(()) };
                let chunk = chunk.map_err(|e| CoreError::new(fetch_failed(&e)))?;
                if cancel.is_cancelled() {
                    return Err(cancelled());
                }
                progress.received += chunk.len() as u64;
                progress.report(Phase::Downloading, counted.load(Ordering::Relaxed), false);
                if send.send(chunk.to_vec()).await.is_err() {
                    // The sampler stopped; its error is reported below.
                    return Ok(());
                }
            }
        }
        .await;
        drop(send);
        let sampled = sampling
            .await
            .map_err(|_| CoreError::new("Puzzle service stopped. Retry the operation."))?;
        fed?;
        let (rows, kept) = sampled?;
        if download.cancel.is_cancelled() {
            return Err(cancelled());
        }
        progress.report(Phase::Importing, kept, true);
        let cancel = download.cancel.clone();
        self.inner
            .with_db(move |db| Ok(queries::store_sample(db, &rows, &|| cancel.is_cancelled())?))
            .await?;
        progress.report(Phase::Done, kept, true);
        Ok(())
    }
}

/// `fetch` failures as Node words them are not reproducible; keep reqwest's text short.
fn fetch_failed(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "The puzzle download timed out.".to_string()
    } else {
        format!("fetch failed: {error}")
    }
}

struct Progress {
    host: Arc<dyn Host>,
    received: u64,
    total: Option<u64>,
    last: Option<Instant>,
}

impl Progress {
    /// At most four reports a second unless forced.
    fn report(&mut self, phase: Phase, kept: usize, force: bool) {
        if !force
            && self
                .last
                .is_some_and(|t| t.elapsed() < Duration::from_millis(250))
        {
            return;
        }
        self.last = Some(Instant::now());
        self.send(phase, kept, None);
    }

    fn finished(&mut self, phase: Phase, message: Option<&str>) {
        self.send(phase, 0, message);
    }

    fn send(&self, phase: Phase, kept: usize, message: Option<&str>) {
        let mut payload = json!({ "phase": phase.name(), "received": self.received, "kept": kept });
        if let Some(total) = self.total {
            payload["total"] = json!(total);
        }
        if let Some(message) = message {
            payload["message"] = json!(message);
        }
        self.host.emit("puzzledb:progress", payload);
    }
}
