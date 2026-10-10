//! The offline puzzle database (`puzzles.db`), which this service alone owns
//! (formerly `core/src/services/puzzleWorker.ts`). Lichess publishes every rated puzzle as one
//! CC0 file; the service streams it once, keeps a sample (`kchess_domain::puzzle::Sampler`) and
//! stores it, so Storm, Streak, Rush and offline practice work without the original.

pub mod queries;

use futures_util::StreamExt;
use rusqlite::Connection;
use serde_json::json;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

use crate::error::{CoreError, Result};
use crate::host::{Config, Host, Level};
use kchess_domain::puzzle::{Random, Sampler};
use queries::{LocalLadderQuery, LocalPuzzleQuery, Puzzle, PuzzleDbStatus};

pub const PUZZLE_DB_URL: &str = "https://database.lichess.org/lichess_db_puzzle.csv.zst";

/// One running download: cancelled through the flag, woken through `wake`.
struct Download {
    cancelled: AtomicBool,
    wake: Notify,
}

struct Inner {
    config: Config,
    host: Arc<dyn Host>,
    db: Mutex<Option<Connection>>,
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
    let db = Connection::open(&path).map_err(|e| CoreError::new(e.to_string()))?;
    queries::create_schema(&db)?;
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
                "BEGIN;
                INSERT OR IGNORE INTO puzzles SELECT * FROM legacy.puzzles;
                INSERT OR IGNORE INTO puzzle_meta SELECT id, importedAt, count, bytes FROM legacy.puzzle_meta;
                INSERT OR REPLACE INTO worker_meta VALUES (1);
                COMMIT;",
            )
            .map_err(|e| {
                let _ = db.execute_batch("ROLLBACK");
                CoreError::new(e.to_string())
            })?;
        }
        db.execute_batch("DETACH DATABASE legacy")
            .map_err(|e| CoreError::new(e.to_string()))?;
    }
    db.execute_batch("INSERT OR IGNORE INTO worker_meta VALUES (1)")
        .map_err(|e| CoreError::new(e.to_string()))?;
    Ok(db)
}

#[cfg(unix)]
fn restrict(path: &Path, host: &dyn Host) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(cause) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        host.log(
            Level::Debug,
            "puzzles",
            &format!("Could not restrict database file permissions {cause}"),
        );
    }
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _host: &dyn Host) {}

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

/// The decompressed CSV goes straight into the sampler.
struct SamplerSink(Sampler);

impl Write for SamplerSink {
    fn write(&mut self, chunk: &[u8]) -> std::io::Result<usize> {
        self.0
            .push(chunk)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(chunk.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Inner {
    /// Run `work` on the database off the async threads, opening it on first use.
    async fn with_db<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let inner = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            if inner.closed.load(Ordering::SeqCst) {
                return Err(CoreError::new("Puzzle service closed."));
            }
            let mut db = inner
                .db
                .lock()
                .map_err(|_| CoreError::new("Puzzle service stopped. Retry the operation."))?;
            if db.is_none() {
                *db = Some(open(&inner.config, inner.host.as_ref())?);
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
            download.cancelled.store(true, Ordering::SeqCst);
            download.wake.notify_one();
        }
    }

    pub async fn close(&self) {
        self.cancel();
        self.inner.closed.store(true, Ordering::SeqCst);
        let inner = Arc::clone(&self.inner);
        // Closing waits for an import that holds the database to see the cancellation.
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(mut db) = inner.db.lock() {
                db.take();
            }
        })
        .await;
    }

    /// Download, sample and store the puzzle database, reporting `puzzledb:progress`.
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
                cancelled: AtomicBool::new(false),
                wake: Notify::new(),
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
        let aborted = download.cancelled.load(Ordering::SeqCst);
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
        let response = tokio::select! {
            response = reqwest::get(url) => response.map_err(|e| CoreError::new(fetch_failed(&e))),
            _ = download.wake.notified() => Err(cancelled()),
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
        let (send, mut receive) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
        let counted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = Arc::clone(&counted);
        let sampling =
            tokio::task::spawn_blocking(move || -> Result<(Vec<queries::DbPuzzle>, usize)> {
                let mut decoder = zstd::stream::write::Decoder::new(SamplerSink(Sampler::new(
                    Random::new(seed()),
                )))
                .map_err(|e| CoreError::new(e.to_string()))?;
                while let Some(chunk) = receive.blocking_recv() {
                    decoder
                        .write_all(&chunk)
                        .map_err(|e| CoreError::new(e.to_string()))?;
                    count.store(decoder.get_ref().0.count(), Ordering::Relaxed);
                }
                decoder.flush().map_err(|e| CoreError::new(e.to_string()))?;
                let mut sampler = decoder.into_inner().0;
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
                    })
                    .collect();
                Ok((rows, sampler.count()))
            });
        let mut stream = response.bytes_stream();
        let fed: Result<()> = async {
            loop {
                let chunk = tokio::select! {
                    chunk = stream.next() => chunk,
                    _ = download.wake.notified() => return Err(cancelled()),
                };
                let Some(chunk) = chunk else { return Ok(()) };
                let chunk = chunk.map_err(|e| CoreError::new(fetch_failed(&e)))?;
                if download.cancelled.load(Ordering::SeqCst) {
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
        if download.cancelled.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        progress.report(Phase::Importing, kept, true);
        let flag = Arc::clone(download);
        self.inner
            .with_db(move |db| {
                Ok(queries::store_sample(db, &rows, &|| {
                    flag.cancelled.load(Ordering::SeqCst)
                })?)
            })
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
