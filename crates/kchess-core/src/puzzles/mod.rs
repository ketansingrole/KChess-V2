//! The offline puzzle database (`puzzles.db`), which this service alone owns.
//! Lichess publishes every rated puzzle as one
//! CC0 file; the service streams it once, keeps a sample (`kchess_domain::puzzle::Sampler`) and
//! stores it, so Storm, Streak, Rush and offline practice work without the original.

pub mod attempts;
pub mod queries;
mod zst;

use kchess_contracts::CoreEvent;
use kchess_contracts::types::{PuzzleDbPhase as Phase, PuzzleDbProgress};
use rusqlite::Connection;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::host::{Config, Host, Level};
use crate::store::sql::{Lane, LaneMessages, Sql as _, transaction};
use crate::tasks::lock;
use crate::usage::Meter;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    LocalLadderQuery, LocalPuzzleQuery, Puzzle, PuzzleAttemptInput, PuzzleDbStatus, PuzzleProgress,
    PuzzleProgressQuery,
};
use kchess_domain::puzzle::{Random, Sampler};

/// How long the puzzle database download may go without a byte before it fails.
const DOWNLOAD_SILENCE: std::time::Duration = std::time::Duration::from_secs(60);
/// The most the download may bring: the compressed database is some 300 MB (2026).
const MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;

/// One running download: cancelled through the token.
struct Download {
    cancel: CancellationToken,
}

/// The running download: cancelled, and no longer running, when dropped. Dropped before it
/// reported its end (`ended`), as when its caller went away mid-download, it reports it
/// cancelled: the other windows once kept showing it downloading, with a Cancel that did nothing.
struct Installing<'a> {
    inner: &'a Inner,
    download: Arc<Download>,
    /// How the download ended, reported as its slot is freed; a cancellation when it never got
    /// that far (its caller was dropped).
    end: Option<PuzzleDbProgress>,
}

impl Drop for Installing<'_> {
    fn drop(&mut self) {
        self.download.cancel.cancel();
        let _ending = lock(&self.inner.ending);
        let ours = {
            let mut running = lock(&self.inner.running);
            let ours = running
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.download));
            if ours {
                running.take();
            }
            ours
        };
        if !ours {
            return;
        }
        let end = self.end.take().unwrap_or(PuzzleDbProgress {
            phase: Phase::Cancelled,
            received: 0,
            kept: 0,
            total: None,
            message: None,
        });
        self.inner.host.emit(CoreEvent::PuzzleDbProgress(end));
    }
}

struct Inner {
    host: Arc<dyn Host>,
    meter: Arc<dyn Meter>,
    lane: Arc<Lane>,
    running: Mutex<Option<Arc<Download>>>,
    /// Held while a download's slot is freed and its end reported, and by the next download as
    /// it claims the slot: the end of one always reaches the windows before the next one's first
    /// progress. Reported after the slot was freed, an end could land after a download another
    /// window started meanwhile, and hide its progress and Cancel.
    ending: Mutex<()>,
}

pub struct PuzzleService {
    inner: Arc<Inner>,
}

/// Write `rows` as the downloaded puzzle database in `data_dir`, as a finished download leaves it:
/// for test profiles and fixtures, which cannot reach Lichess's database.
pub fn write_database(data_dir: &Path, rows: &[queries::DbPuzzle]) -> Result<usize> {
    let db = Connection::open(data_dir.join("puzzles.db"))
        .map_err(|e| CoreError::wrap("Could not open the puzzle database", e))?;
    queries::create_schema(&db)?;
    attempts::create_schema(&db)?;
    let count = queries::store_sample(&db, rows, &|| false)?;
    db.execute_batch("INSERT OR IGNORE INTO worker_meta VALUES (1)")
        .sql()?;
    Ok(count)
}

/// The schema, permissions and one-time import from the old main database.
fn open(config: &Config, host: &dyn Host) -> Result<Connection> {
    let path = config.data_dir.join("puzzles.db");
    let db = Connection::open(&path)
        .map_err(|e| CoreError::wrap("Could not open the puzzle database", e))?;
    db.execute_batch("PRAGMA busy_timeout = 5000;").sql()?;
    queries::create_schema(&db)?;
    attempts::create_schema(&db)?;
    restrict(&path, host);
    let migrated = db
        .query_row("SELECT id FROM worker_meta WHERE id = 1", [], |_| Ok(()))
        .is_ok();
    // Earlier releases kept the puzzles in `kchess.db`, the main database. It stays an intact
    // fallback; all later puzzle writes have one owner.
    let legacy = config.data_dir.join("kchess.db");
    let mut deferred = false;
    if !migrated && legacy.exists() {
        db.execute(
            "ATTACH DATABASE ?1 AS legacy",
            [legacy.to_string_lossy().as_ref()],
        )
        .sql()?;
        // A main database the store has not migrated yet has no puzzle tables to read; leave the
        // import for a later open rather than marking it done.
        let has_tables: bool = db
            .query_row(
                "SELECT COUNT(*) = 2 FROM legacy.sqlite_master
                 WHERE type = 'table' AND name IN ('puzzles', 'puzzle_meta')",
                [],
                |row| row.get(0),
            )
            .sql()?;
        if has_tables {
            transaction(&db, || {
                db.execute_batch(
                    "INSERT OR IGNORE INTO puzzles (id, fen, moves, rating, plays, themes)
                        SELECT id, fen, moves, rating, plays, themes FROM legacy.puzzles;
                    INSERT OR IGNORE INTO puzzle_meta SELECT id, importedAt, count, bytes FROM legacy.puzzle_meta;
                    INSERT OR REPLACE INTO worker_meta VALUES (1);",
                )
                .sql()
            })?;
        }
        deferred = !has_tables;
        db.execute_batch("DETACH DATABASE legacy").sql()?;
    }
    if !deferred {
        db.execute_batch("INSERT OR IGNORE INTO worker_meta VALUES (1)")
            .sql()?;
    }
    Ok(db)
}

fn restrict(path: &Path, host: &dyn Host) {
    crate::store::restrict_database(path, host, "puzzles");
}

/// A random sampler seed.
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

/// Uniform numbers in [0, 1) for the queries' shuffles.
fn random() -> impl FnMut() -> f64 {
    let mut generator = Random::new(seed());
    move || generator.next_f64()
}

impl Inner {
    /// Runs `work` on the database off the async threads, opening it on first use.
    async fn with_db<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.lane.run(work).await
    }

    /// The download in progress. A download whose task panicked holding the lock is still the
    /// one running: reporting none would let `delete` clear the database under its writes.
    fn running(&self) -> Option<Arc<Download>> {
        lock(&self.running).clone()
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
    pub fn new(config: Config, host: Arc<dyn Host>, meter: Arc<dyn Meter>) -> PuzzleService {
        let lane = Lane::new(
            "puzzles",
            LaneMessages {
                closed: "Puzzle service closed.",
                stopped: "Puzzle service stopped. Retry the operation.",
            },
            Arc::clone(&host),
            move |host| open(&config, host),
        );
        PuzzleService {
            inner: Arc::new(Inner {
                host,
                meter,
                lane: Arc::new(lane),
                running: Mutex::new(None),
                ending: Mutex::new(()),
            }),
        }
    }

    fn logged<T>(&self, method: &str, result: Result<T>) -> Result<T> {
        if let Err(error) = &result {
            self.inner.failed(method, error);
        }
        result
    }

    pub async fn status(&self) -> Result<PuzzleDbStatus> {
        let running = self.inner.running().is_some();
        let result = self
            .inner
            .with_db(move |db| queries::read_status(db, running))
            .await;
        self.logged("status", result)
    }

    pub async fn query(&self, query: LocalPuzzleQuery) -> Result<Vec<Puzzle>> {
        let result = self
            .inner
            .with_db(move |db| queries::query_puzzles(db, &query, &mut random()))
            .await;
        self.logged("query", result)
    }

    pub async fn ladder(&self, query: LocalLadderQuery) -> Result<Vec<Puzzle>> {
        let result = self
            .inner
            .with_db(move |db| queries::query_ladder(db, &query, &mut random()))
            .await;
        self.logged("ladder", result)
    }

    /// Keep a training attempt; the progress it leaves, as of the attempt's own day.
    pub async fn record_attempt(&self, attempt: PuzzleAttemptInput) -> Result<PuzzleProgress> {
        let result = self
            .inner
            .with_db(move |db| {
                attempts::record(db, &attempt, crate::time::now_ms())?;
                let query = PuzzleProgressQuery {
                    today: attempt.day,
                    limit: attempts::RECENT,
                };
                attempts::progress(db, &query)
            })
            .await;
        self.logged("recordAttempt", result)
    }

    pub async fn progress(&self, query: PuzzleProgressQuery) -> Result<PuzzleProgress> {
        let result = self
            .inner
            .with_db(move |db| attempts::progress(db, &query))
            .await;
        self.logged("progress", result)
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
                    let compacted = queries::compact(db);
                    Ok((queries::read_status(db, false)?, compacted))
                })
                .await
                .map(|(status, compacted)| {
                    // The puzzles are gone either way: a compaction that failed is only logged.
                    if let Err(error) = &compacted {
                        self.inner.failed("delete (compacting)", error);
                    }
                    status
                })
        };
        self.logged("delete", result)
    }

    /// Stop a running download; it resolves with the current status.
    pub fn cancel(&self) {
        if let Some(download) = self.inner.running() {
            download.cancel.cancel();
        }
    }

    pub async fn close(&self) {
        self.cancel();
        self.inner.lane.close().await;
    }

    /// Download, sample and store the puzzle database, reporting `puzzledb:progress`.
    /// Cancellation resolves with the current status: callers distinguish cancel from
    /// completion via the `puzzledb:progress` `cancelled`/`done` phase, not the result.
    pub async fn install(&self, url: String) -> Result<PuzzleDbStatus> {
        let download = {
            let _ending = lock(&self.inner.ending);
            let mut running = lock(&self.inner.running);
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
        // Ends this download however `install` ends, including when its caller is dropped.
        let mut installing = Installing {
            inner: &self.inner,
            download: Arc::clone(&download),
            end: None,
        };
        let mut progress = Progress {
            host: Arc::clone(&self.inner.host),
            received: 0,
            total: None,
            last: None,
        };
        let result = self.download(&url, &download, &mut progress).await;
        let aborted = download.cancel.is_cancelled();
        // No longer running before its end is reported: a window that read the status on hearing
        // a failure once found it still running, and nothing told it otherwise afterwards. Both
        // happen under `ending` (`Installing::drop`).
        let (end, outcome) = match result {
            Ok(kept) => (progress.event(Phase::Done, kept, None), Ok(())),
            Err(error) => {
                let phase = if aborted {
                    Phase::Cancelled
                } else {
                    Phase::Failed
                };
                let end = progress.event(phase, 0, (!aborted).then_some(error.message.as_str()));
                (end, if aborted { Ok(()) } else { Err(error) })
            }
        };
        installing.end = Some(end);
        drop(installing);
        let result = match outcome {
            Ok(()) => {
                self.inner
                    .with_db(|db| queries::read_status(db, false))
                    .await
            }
            Err(error) => Err(error),
        };
        self.logged("install", result)
    }

    async fn download(
        &self,
        url: &str,
        download: &Arc<Download>,
        progress: &mut Progress,
    ) -> Result<usize> {
        let cancelled = || CoreError::aborted(crate::download::ABORTED);
        // A server that stops sending ends the download instead of holding the puzzle database's
        // "downloading" state until the user cancels.
        let client = crate::download::client(Some(DOWNLOAD_SILENCE))?;
        let limits = crate::download::Limits {
            max_bytes: MAX_DOWNLOAD,
            deadline: None,
        };
        let usage = crate::download::Usage {
            meter: &*self.inner.meter,
            kind: kchess_contracts::types::UsageKind::Database,
        };
        let mut body = crate::download::open(
            &client,
            || client.get(url),
            limits,
            &download.cancel,
            usage,
            &WORDING,
        )
        .await?;
        progress.total = body.total();
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
        let fed: Result<()> = async {
            while let Some(chunk) = body.next().await? {
                progress.received = body.received();
                progress.report(Phase::Downloading, counted.load(Ordering::Relaxed), false);
                if send.send(chunk.to_vec()).await.is_err() {
                    // The sampler stopped; its error is reported below.
                    return Ok(());
                }
            }
            Ok(())
        }
        .await;
        drop(body);
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
            .with_db(move |db| queries::store_sample(db, &rows, &|| cancel.is_cancelled()))
            .await?;
        Ok(kept)
    }
}

/// How the database download words its failures.
const WORDING: crate::download::Wording = crate::download::Wording {
    network: |error| {
        if error.is_timeout() {
            "The puzzle download timed out.".to_string()
        } else {
            format!("fetch failed: {error}")
        }
    },
    status: |status| format!("Lichess answered {status} for the puzzle database."),
    too_large: "The puzzle database download is unexpectedly large.",
};

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

    fn send(&self, phase: Phase, kept: usize, message: Option<&str>) {
        let progress = self.event(phase, kept, message);
        self.host.emit(CoreEvent::PuzzleDbProgress(progress));
    }

    fn event(&self, phase: Phase, kept: usize, message: Option<&str>) -> PuzzleDbProgress {
        PuzzleDbProgress {
            phase,
            received: self.received,
            kept: kept as u64,
            total: self.total,
            message: message.map(str::to_string),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_host::TestHost;
    use serde_json::json;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn csv(rows: &[(&str, u32, &str)]) -> Vec<u8> {
        let mut text = String::from(
            "PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl,OpeningTags\n",
        );
        for (id, rating, themes) in rows {
            text.push_str(&format!(
                "{id},{START},e2e4 e7e5 g1f3,{rating},75,95,1000,{themes},https://lichess.org/abcdefgh#1,\n"
            ));
        }
        ruzstd::encoding::compress_to_vec(
            text.as_bytes(),
            ruzstd::encoding::CompressionLevel::Fastest,
        )
    }

    /// Answers every request with `status` and `body`, or holds it open when `body` is None.
    async fn serve(status: u16, body: Option<Vec<u8>>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/puzzles.csv.zst", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut request = [0u8; 4096];
                    let _ = socket.read(&mut request).await;
                    let Some(body) = body else {
                        // Headers, then a body that never comes.
                        let _ = socket
                            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000\r\n\r\n")
                            .await;
                        std::future::pending::<()>().await;
                        return;
                    };
                    let head = format!(
                        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    crate::loopback::close_after_client(&mut socket).await;
                });
            }
        });
        url
    }

    /// Offers byte ranges; the first answer stops halfway, a range request gets the rest.
    async fn serve_dropping_once(body: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/puzzles.csv.zst", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0u8; 4096];
                let read = socket.read(&mut request).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]).to_lowercase();
                let from: Option<usize> = request
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="))
                    .and_then(|range| range.trim_end_matches('-').parse().ok());
                let answer = match from {
                    None => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\naccept-ranges: bytes\r\netag: \"db\"\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        );
                        [head.as_bytes(), &body[..body.len() / 2]].concat()
                    }
                    Some(from) => {
                        let head = format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {from}-{}/{}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len() - 1,
                            body.len(),
                            body.len() - from
                        );
                        [head.as_bytes(), &body[from..]].concat()
                    }
                };
                let _ = socket.write_all(&answer).await;
                let _ = socket.shutdown().await;
            }
        });
        url
    }

    #[tokio::test]
    async fn a_download_cut_off_halfway_resumes_and_installs() {
        let dir = tempfile::tempdir().unwrap();
        let (puzzles, host, usage) = service(dir.path());
        let body = csv(&[
            ("aaa", 1200, "fork short"),
            ("bbb", 1500, "pin middlegame"),
            ("ccc", 1800, "fork long"),
        ]);
        let length = body.len() as u64;
        let status = puzzles
            .install(serve_dropping_once(body).await)
            .await
            .unwrap();
        assert_eq!(status.count, 3);
        let done = host.named("puzzledb:progress").pop().unwrap();
        // Each byte arrived once: the resumed request asked only for the rest.
        assert_eq!(done["received"], json!(length));
        // The request and the one that resumed it, with every byte counted once.
        let usage = usage.counted();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0]["requests"], json!(2));
        assert_eq!(usage[0]["bytes"], json!(length));
    }

    /// The service, its host and the usage it counts.
    fn service(dir: &Path) -> (PuzzleService, Arc<TestHost>, Arc<crate::usage::UsageLog>) {
        let host = TestHost::new();
        let usage = Arc::new(crate::usage::UsageLog::default());
        let config = Config::new(dir.to_path_buf());
        (
            PuzzleService::new(
                config,
                Arc::clone(&host) as Arc<dyn Host>,
                Arc::clone(&usage) as Arc<dyn Meter>,
            ),
            host,
            usage,
        )
    }

    /// The phases the progress events reported, in order.
    fn phases(host: &TestHost) -> Vec<String> {
        host.named("puzzledb:progress")
            .iter()
            .map(|payload| payload["phase"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn installs_a_download_then_answers_queries_until_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let (puzzles, host, usage) = service(dir.path());
        assert!(!puzzles.status().await.unwrap().installed);
        let url = serve(
            200,
            Some(csv(&[
                ("aaa", 1200, "fork short"),
                ("bbb", 1500, "pin middlegame"),
                ("ccc", 1800, "fork long"),
            ])),
        )
        .await;
        let status = puzzles.install(url).await.unwrap();
        assert!(status.installed);
        assert_eq!(status.count, 3);
        assert!(!status.busy);
        assert!(status.imported_at.is_some());
        let phases = phases(&host);
        assert_eq!(phases.first().map(String::as_str), Some("downloading"));
        assert_eq!(&phases[phases.len() - 2..], ["importing", "done"]);
        let done = host.named("puzzledb:progress").pop().unwrap();
        assert_eq!(done["kept"], json!(3));
        assert!(done["total"].as_u64().is_some_and(|total| total > 0));
        // One request and the bytes it brought.
        let usage = usage.counted();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0]["requests"], json!(1));
        assert_eq!(usage[0]["bytes"], done["received"]);

        let forks = puzzles
            .query(LocalPuzzleQuery {
                theme: Some("fork".into()),
                min_rating: None,
                max_rating: Some(1600),
                count: 10,
            })
            .await
            .unwrap();
        assert_eq!(forks.len(), 1);
        assert_eq!(forks[0].id, "aaa");
        assert_eq!(forks[0].last_move.as_deref(), Some("e2e4"));
        assert_eq!(forks[0].solution, ["e7e5", "g1f3"]);
        let ladder = puzzles
            .ladder(LocalLadderQuery {
                from: 1000,
                to: 2000,
                count: 3,
            })
            .await
            .unwrap();
        assert_eq!(ladder.len(), 3);

        let deleted = puzzles.delete().await.unwrap();
        assert!(!deleted.installed);
        assert_eq!(deleted.count, 0);
        puzzles.close().await;
        let closed = puzzles.status().await.unwrap_err();
        assert_eq!(closed.message, "Puzzle service closed.");
    }

    #[tokio::test]
    async fn a_refused_download_fails_and_keeps_the_stored_puzzles() {
        let dir = tempfile::tempdir().unwrap();
        let (puzzles, host, _usage) = service(dir.path());
        let good = serve(200, Some(csv(&[("aaa", 1200, "fork")]))).await;
        puzzles.install(good).await.unwrap();
        let refused = serve(404, Some(b"gone".to_vec())).await;
        let error = puzzles.install(refused).await.unwrap_err();
        assert_eq!(
            error.message,
            "Lichess answered 404 for the puzzle database."
        );
        let failed = host.named("puzzledb:progress").pop().unwrap();
        assert_eq!(failed["phase"], json!("failed"));
        assert_eq!(failed["message"], json!(error.message));
        // Not zstd: the sampler's error is the download's.
        let garbage = serve(200, Some(b"definitely not zstd".to_vec())).await;
        assert!(puzzles.install(garbage).await.is_err());
        assert_eq!(puzzles.status().await.unwrap().count, 1);
        assert!(host.logged("[puzzles] Puzzle operation install failed"));
        puzzles.close().await;
    }

    /// Whether the service said it was downloading as each download's end left: a window that
    /// read the status on hearing the end found it running.
    #[tokio::test]
    async fn a_download_has_stopped_running_when_its_end_is_reported() {
        use std::sync::{Mutex as StdMutex, OnceLock};
        struct Watching {
            host: Arc<TestHost>,
            inner: OnceLock<std::sync::Weak<Inner>>,
            running_at_end: StdMutex<Vec<bool>>,
        }
        impl Host for Watching {
            fn log(&self, level: crate::host::Level, scope: &str, message: &str) {
                self.host.log(level, scope, message);
            }
            fn emit(&self, event: CoreEvent) {
                if let CoreEvent::PuzzleDbProgress(progress) = &event
                    && matches!(
                        progress.phase,
                        Phase::Done | Phase::Failed | Phase::Cancelled
                    )
                    && let Some(inner) = self.inner.get().and_then(std::sync::Weak::upgrade)
                {
                    let running = inner.running().is_some();
                    self.running_at_end.lock().unwrap().push(running);
                }
                self.host.emit(event);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let watching = Arc::new(Watching {
            host: TestHost::new(),
            inner: OnceLock::new(),
            running_at_end: StdMutex::default(),
        });
        let puzzles = PuzzleService::new(
            Config::new(dir.path().to_path_buf()),
            Arc::clone(&watching) as Arc<dyn Host>,
            Arc::new(crate::usage::UsageLog::default()) as Arc<dyn Meter>,
        );
        let _ = watching.inner.set(Arc::downgrade(&puzzles.inner));
        let good = serve(200, Some(csv(&[("aaa", 1200, "fork")]))).await;
        puzzles.install(good).await.unwrap();
        let refused = serve(404, Some(b"gone".to_vec())).await;
        assert!(puzzles.install(refused).await.is_err());
        assert_eq!(*watching.running_at_end.lock().unwrap(), [false, false]);
        puzzles.close().await;
    }

    /// 2026-11-24: a download's end was reported after its slot was freed, so a download another
    /// window started in between could report its first progress before it, and the late end hid
    /// the new download's progress and Cancel. The end now always comes first.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_downloads_end_is_reported_before_the_next_one_starts() {
        struct Slow {
            host: Arc<TestHost>,
            ending: tokio::sync::Notify,
        }
        impl Host for Slow {
            fn log(&self, level: crate::host::Level, scope: &str, message: &str) {
                self.host.log(level, scope, message);
            }
            fn emit(&self, event: CoreEvent) {
                if let CoreEvent::PuzzleDbProgress(progress) = &event
                    && progress.phase == Phase::Failed
                {
                    // The worker pauses on its way to report the end; the next download starts.
                    self.ending.notify_one();
                    std::thread::sleep(Duration::from_millis(300));
                }
                self.host.emit(event);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let slow = Arc::new(Slow {
            host: TestHost::new(),
            ending: tokio::sync::Notify::new(),
        });
        let puzzles = Arc::new(PuzzleService::new(
            Config::new(dir.path().to_path_buf()),
            Arc::clone(&slow) as Arc<dyn Host>,
            Arc::new(crate::usage::UsageLog::default()) as Arc<dyn Meter>,
        ));
        // The next download reports its first bytes, then waits.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let hanging = format!("http://{}/puzzles.csv.zst", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = [0u8; 4096];
                    let _ = socket.read(&mut request).await;
                    let head = "HTTP/1.1 200 OK\r\ncontent-length: 100000\r\n\r\n";
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&[0u8; 1000]).await;
                    std::future::pending::<()>().await;
                });
            }
        });
        let next = tokio::spawn({
            let (puzzles, slow) = (Arc::clone(&puzzles), Arc::clone(&slow));
            async move {
                slow.ending.notified().await;
                puzzles.install(hanging).await
            }
        });
        let refused = serve(404, Some(b"gone".to_vec())).await;
        assert!(puzzles.install(refused).await.is_err());
        tokio::time::timeout(Duration::from_secs(30), async {
            while phases(&slow.host).len() < 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the next download reports its progress");
        let phases = phases(&slow.host);
        assert_eq!(phases[..2], ["failed", "downloading"], "{phases:?}");
        puzzles.cancel();
        next.await.unwrap().unwrap();
        puzzles.close().await;
    }

    /// Waits until the download has started; fails the test if it never does (an install that
    /// ended at once would otherwise leave the test polling forever).
    async fn downloading(puzzles: &PuzzleService) {
        tokio::time::timeout(Duration::from_secs(30), async {
            while !puzzles.status().await.unwrap().busy {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the download starts");
    }

    #[tokio::test]
    async fn a_running_download_blocks_another_and_deletion_until_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let (puzzles, host, _usage) = service(dir.path());
        let puzzles = Arc::new(puzzles);
        let stalled = serve(200, None).await;
        let first = tokio::spawn({
            let puzzles = Arc::clone(&puzzles);
            async move { puzzles.install(stalled).await }
        });
        downloading(&puzzles).await;
        let again = puzzles.install("http://127.0.0.1:9/".into()).await;
        assert_eq!(
            again.unwrap_err().message,
            "The puzzle database is already downloading."
        );
        assert_eq!(
            puzzles.delete().await.unwrap_err().message,
            "Cancel the download before deleting puzzles."
        );
        puzzles.cancel();
        // Cancelling resolves with the current status; the phase tells it from completion.
        let status = first.await.unwrap().unwrap();
        assert!(!status.installed && !status.busy);
        assert_eq!(phases(&host).last().map(String::as_str), Some("cancelled"));
        assert!(
            host.named("puzzledb:progress")
                .last()
                .unwrap()
                .get("message")
                .is_none()
        );
        puzzles.close().await;
    }

    /// A client that goes away mid-download has its call aborted: the download ends with it, and
    /// another install or a deletion is not refused forever.
    #[tokio::test]
    async fn an_aborted_install_stops_its_download() {
        let dir = tempfile::tempdir().unwrap();
        let (puzzles, host, _) = service(dir.path());
        let puzzles = Arc::new(puzzles);
        let stalled = serve(200, None).await;
        let first = tokio::spawn({
            let puzzles = Arc::clone(&puzzles);
            async move { puzzles.install(stalled).await }
        });
        downloading(&puzzles).await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        assert!(!puzzles.status().await.unwrap().busy);
        // Every window hears it end: one that only saw it downloading once kept showing Cancel.
        assert_eq!(phases(&host).last().map(String::as_str), Some("cancelled"));
        puzzles.delete().await.expect("deleting is allowed again");
        puzzles.close().await;
    }

    /// A task that panicked holding the running download's lock poisoned it, and the poisoned
    /// lock read as no download: `delete` then cleared the database under the download's writes.
    #[tokio::test]
    async fn a_download_whose_lock_was_poisoned_still_counts_as_running() {
        let dir = tempfile::tempdir().unwrap();
        let (puzzles, _host, _) = service(dir.path());
        let download = Arc::new(Download {
            cancel: CancellationToken::new(),
        });
        *lock(&puzzles.inner.running) = Some(Arc::clone(&download));
        let inner = Arc::clone(&puzzles.inner);
        std::thread::spawn(move || {
            let _held = inner.running.lock().unwrap();
            panic!("a download task failed holding the lock");
        })
        .join()
        .expect_err("the thread panicked");
        assert!(puzzles.inner.running.is_poisoned());

        assert!(puzzles.status().await.unwrap().busy);
        let refused = puzzles.delete().await.expect_err("the download still runs");
        assert_eq!(
            refused.message,
            "Cancel the download before deleting puzzles."
        );
        puzzles.cancel();
        assert!(download.cancel.is_cancelled());
        puzzles.close().await;
    }

    #[tokio::test]
    async fn the_first_open_takes_over_the_puzzles_of_the_main_database_once() {
        let dir = tempfile::tempdir().unwrap();
        {
            let main = Connection::open(dir.path().join("kchess.db")).unwrap();
            crate::store::migrations::migrate(&main).unwrap();
            main.execute_batch(&format!(
                "INSERT INTO puzzles (id, fen, moves, rating, plays, themes)
                   VALUES ('old', '{START}', 'e2e4 e7e5', 1400, 10, 'fork');
                 INSERT INTO puzzle_meta (id, importedAt, count, bytes) VALUES (1, 5, 1, 60);"
            ))
            .unwrap();
        }
        let (puzzles, ..) = service(dir.path());
        let status = puzzles.status().await.unwrap();
        assert_eq!((status.count, status.imported_at), (1, Some(5)));
        puzzles.delete().await.unwrap();
        puzzles.close().await;
        // Deleted puzzles are not taken over again from the untouched main database.
        let (puzzles, ..) = service(dir.path());
        assert!(!puzzles.status().await.unwrap().installed);
        puzzles.close().await;
    }

    #[tokio::test]
    async fn a_main_database_without_puzzle_tables_is_taken_over_later() {
        let dir = tempfile::tempdir().unwrap();
        Connection::open(dir.path().join("kchess.db"))
            .unwrap()
            .execute_batch("CREATE TABLE unrelated (id INTEGER)")
            .unwrap();
        let (puzzles, ..) = service(dir.path());
        assert!(!puzzles.status().await.unwrap().installed);
        puzzles.close().await;
        {
            let main = Connection::open(dir.path().join("kchess.db")).unwrap();
            crate::store::migrations::migrate(&main).unwrap();
            main.execute_batch(&format!(
                "INSERT INTO puzzles (id, fen, moves, rating, themes)
                   VALUES ('old', '{START}', 'e2e4 e7e5', 1400, 'fork');
                 INSERT INTO puzzle_meta (id, importedAt, count, bytes) VALUES (1, 5, 1, 60);"
            ))
            .unwrap();
        }
        let (puzzles, ..) = service(dir.path());
        assert_eq!(puzzles.status().await.unwrap().count, 1);
        puzzles.close().await;
    }
}
