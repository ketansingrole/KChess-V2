//! What every SQLite file of the core shares: the lane a connection is used on, transactions and
//! the conversion of SQLite's errors. `kchess.db` (`super::Database`) and `puzzles.db`
//! (`crate::puzzles`) each own one lane.

use std::future::Future;
use std::panic::Location;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::task::TaskTracker;

use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

/// Carries SQLite's failures as the core's errors with `?`.
pub trait Sql<T> {
    fn sql(self) -> Result<T>;
}

impl<T> Sql<T> for rusqlite::Result<T> {
    fn sql(self) -> Result<T> {
        self.map_err(|cause| CoreError::new(cause.to_string()))
    }
}

/// Runs `work` inside `BEGIN IMMEDIATE`, committing on success and rolling back on any failure.
/// The caller sees the failure that ended the work; a ROLLBACK that fails too is appended to it.
pub fn transaction<T>(db: &Connection, work: impl FnOnce() -> Result<T>) -> Result<T> {
    db.execute_batch("BEGIN IMMEDIATE").sql()?;
    match work().and_then(|value| db.execute_batch("COMMIT").sql().map(|()| value)) {
        Ok(value) => Ok(value),
        Err(error) => Err(match db.execute_batch("ROLLBACK") {
            Ok(()) => error,
            Err(rollback) => error.with_rollback_failure(rollback),
        }),
    }
}

/// A call slower than this is logged: one connection serializes all of a file's work, so a slow
/// call holds up every read behind it.
const SLOW_CALL: Duration = Duration::from_millis(500);

/// How a lane opens its file.
type Opener = Box<dyn Fn(&dyn Host) -> Result<Connection> + Send + Sync>;

/// The messages a lane's callers see.
pub struct LaneMessages {
    /// After `close`: the file is released and no more work is admitted.
    pub closed: &'static str,
    /// The worker running a call failed (it panicked).
    pub stopped: &'static str,
}

/// One SQLite file's connection, opened on first use and used by one call at a time. Async calls
/// queue for admission and then run on a blocking worker, which owns its admission, so dropping
/// the awaiting future can neither cancel a write half way nor let `close` release the file under
/// it. `close` refuses new work, waits for the admitted work, then releases the file.
pub struct Lane {
    scope: &'static str,
    messages: LaneMessages,
    host: Arc<dyn Host>,
    open: Opener,
    connection: Mutex<Option<Connection>>,
    execution: Arc<Semaphore>,
    work: TaskTracker,
    closed: AtomicBool,
}

impl Lane {
    pub fn new(
        scope: &'static str,
        messages: LaneMessages,
        host: Arc<dyn Host>,
        open: impl Fn(&dyn Host) -> Result<Connection> + Send + Sync + 'static,
    ) -> Lane {
        Lane {
            scope,
            messages,
            host,
            open: Box::new(open),
            connection: Mutex::new(None),
            execution: Arc::new(Semaphore::new(1)),
            work: TaskTracker::new(),
            closed: AtomicBool::new(false),
        }
    }

    /// Runs `work` on this thread, opening the file first. Blocking: it must never run on an async
    /// executor thread.
    pub fn run_here<T>(&self, work: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut db = usable_connection(&self.connection, self.host.as_ref(), self.scope);
        if self.closed.load(Ordering::SeqCst) {
            return Err(CoreError::new(self.messages.closed));
        }
        if db.is_none() {
            let connection = (self.open)(self.host.as_ref())?;
            // A close that began while the file opened keeps it released.
            if self.closed.load(Ordering::SeqCst) {
                return Err(CoreError::new(self.messages.closed));
            }
            *db = Some(connection);
        }
        match db.as_ref() {
            Some(connection) => work(connection),
            None => Err(CoreError::new(self.messages.stopped)),
        }
    }

    /// Runs `work` on a blocking worker once the lane admits it.
    #[track_caller]
    pub fn run<T, F>(self: &Arc<Self>, work: F) -> impl Future<Output = Result<T>> + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        self.run_admitted(move || Ok(work))
    }

    /// Runs the work `prepare` returns once the lane admits it. `prepare` runs at admission, on
    /// the caller's task: a write that is only wanted while its caller is still current checks
    /// that here, after waiting its turn, and fails rather than writing.
    #[track_caller]
    pub fn run_admitted<T, W, P>(
        self: &Arc<Self>,
        prepare: P,
    ) -> impl Future<Output = Result<T>> + use<T, W, P>
    where
        T: Send + 'static,
        W: FnOnce(&Connection) -> Result<T> + Send + 'static,
        P: FnOnce() -> Result<W>,
    {
        let caller = Location::caller();
        let lane = Arc::clone(self);
        async move {
            let permit = lane.hold().await?;
            let tracked = lane.work.token();
            let work = prepare()?;
            let started = Instant::now();
            let worker = Arc::clone(&lane);
            let result = tokio::task::spawn_blocking(move || {
                let (_permit, _tracked) = (permit, tracked);
                worker.run_here(work)
            })
            .await
            .map_err(|_| CoreError::new(lane.messages.stopped))?;
            let elapsed = started.elapsed();
            if elapsed > SLOW_CALL {
                lane.host.log(
                    Level::Warn,
                    lane.scope,
                    &format!(
                        "Slow storage call: caller={}:{} durationMs={}",
                        caller.file(),
                        caller.line(),
                        elapsed.as_millis()
                    ),
                );
            }
            result
        }
    }

    /// The lane's admission, held until the permit drops: no other async call runs meanwhile.
    pub async fn hold(&self) -> Result<OwnedSemaphorePermit> {
        Arc::clone(&self.execution)
            .acquire_owned()
            .await
            .map_err(|_| CoreError::new(self.messages.closed))
    }

    /// Refuses new work, waits for the admitted work to end, then releases the file.
    pub async fn close(self: &Arc<Self>) {
        self.closed.store(true, Ordering::SeqCst);
        self.execution.close();
        self.work.close();
        self.work.wait().await;
        let lane = Arc::clone(self);
        if let Err(cause) = tokio::task::spawn_blocking(move || lane.release()).await {
            self.host.log(
                Level::Warn,
                self.scope,
                &format!("Database close failed: {cause}"),
            );
        }
    }

    /// Releases the file; the next call opens it again (unless the lane is closed).
    pub fn release(&self) {
        usable_connection(&self.connection, self.host.as_ref(), self.scope).take();
    }

    /// Whether a connection is open now.
    pub fn is_open(&self) -> bool {
        usable_connection(&self.connection, self.host.as_ref(), self.scope).is_some()
    }
}

/// The connection behind `slot`, as a call may use it. A call that panicked holding the lock may
/// have left its transaction open, and a ROLLBACK that failed leaves one open too: either way the
/// connection is dropped (SQLite rolls the transaction back) and the next call opens the file
/// again, rather than every later call failing or joining a transaction nobody will commit.
fn usable_connection<'a>(
    slot: &'a Mutex<Option<Connection>>,
    host: &dyn Host,
    scope: &str,
) -> MutexGuard<'a, Option<Connection>> {
    let mut db = match slot.lock() {
        Ok(db) => db,
        Err(poisoned) => {
            slot.clear_poison();
            let mut db = poisoned.into_inner();
            if db.take().is_some() {
                host.log(
                    Level::Warn,
                    scope,
                    "A storage call failed while it held the database; it is opened again.",
                );
            }
            db
        }
    };
    if db
        .as_ref()
        .is_some_and(|connection| !connection.is_autocommit())
    {
        host.log(
            Level::Warn,
            scope,
            "The database was left inside a transaction; it is opened again.",
        );
        db.take();
    }
    db
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_host::TestHost;
    use std::sync::atomic::AtomicUsize;

    const MESSAGES: LaneMessages = LaneMessages {
        closed: "closed",
        stopped: "stopped",
    };

    fn memory_lane(opens: Arc<AtomicUsize>) -> Arc<Lane> {
        Arc::new(Lane::new("test", MESSAGES, TestHost::quiet(), move |_| {
            opens.fetch_add(1, Ordering::SeqCst);
            Connection::open_in_memory().sql()
        }))
    }

    /// A ROLLBACK that fails must not hide why the work failed: the cause comes first.
    #[test]
    fn a_failed_rollback_is_added_to_the_cause_not_put_in_its_place() {
        let db = Connection::open_in_memory().expect("open");
        let failed = transaction(&db, || -> Result<()> {
            // Ends the transaction, so the helper's own ROLLBACK fails.
            db.execute_batch("COMMIT").sql()?;
            Err(CoreError::new("disk failure"))
        })
        .expect_err("the work failed");
        assert!(
            failed.message.starts_with("disk failure; rollback failed:"),
            "{}",
            failed.message
        );
        let failed = transaction(&db, || -> Result<()> { Err(CoreError::new("refused")) })
            .expect_err("the work failed");
        assert_eq!(failed.message, "refused");
        assert!(db.is_autocommit());
    }

    #[test]
    fn a_committed_transaction_keeps_its_writes() {
        let db = Connection::open_in_memory().expect("open");
        db.execute_batch("CREATE TABLE t (x)").expect("table");
        transaction(&db, || db.execute("INSERT INTO t VALUES (1)", []).sql()).expect("committed");
        let _ = transaction(&db, || -> Result<()> {
            db.execute("INSERT INTO t VALUES (2)", []).sql()?;
            Err(CoreError::new("undo"))
        });
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 1);
    }

    /// A call that panicked holding the connection used to leave the lock poisoned, so every
    /// later call failed until the app restarted, and its open transaction stayed open.
    #[test]
    fn a_call_that_panicked_holding_the_connection_does_not_disable_the_lane() {
        let opens = Arc::new(AtomicUsize::new(0));
        let lane = memory_lane(Arc::clone(&opens));
        lane.run_here(|_| Ok(())).expect("open");
        let held = Arc::clone(&lane);
        let panicked = std::thread::spawn(move || {
            held.run_here(|db| -> Result<()> {
                db.execute_batch("BEGIN IMMEDIATE").sql()?;
                panic!("a storage call panicked");
            })
        })
        .join();
        assert!(panicked.is_err());
        lane.run_here(|db| {
            assert!(db.is_autocommit());
            Ok(())
        })
        .expect("the lane works after the panic");
        assert_eq!(opens.load(Ordering::SeqCst), 2, "opened afresh");
    }

    /// A connection a failed ROLLBACK left inside its transaction is opened again, so later
    /// writes are not lost inside a transaction nobody commits.
    #[test]
    fn a_connection_left_inside_a_transaction_is_opened_again() {
        let opens = Arc::new(AtomicUsize::new(0));
        let lane = memory_lane(Arc::clone(&opens));
        lane.run_here(|db| db.execute_batch("BEGIN").sql())
            .expect("left open");
        lane.run_here(|db| {
            assert!(db.is_autocommit());
            Ok(())
        })
        .expect("reopened");
        assert_eq!(opens.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_contended_lane_keeps_the_executor_free_and_close_drains_a_detached_worker() {
        let lane = memory_lane(Arc::new(AtomicUsize::new(0)));
        lane.run(|_| Ok(())).await.expect("first open");
        let (held, held_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::channel::<()>();
        let blocked = Arc::clone(&lane);
        let caller = tokio::spawn(blocked.run(move |_| {
            held.send(()).expect("notify");
            release_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("release");
            Ok(())
        }));
        held_rx.await.expect("the worker runs");
        // The caller goes away; its worker keeps running to the end.
        caller.abort();
        let _ = caller.await;
        let closing = Arc::clone(&lane);
        let close = tokio::spawn(async move { closing.close().await });
        tokio::task::yield_now().await;
        assert!(!close.is_finished(), "close waits for the detached worker");
        release.send(()).expect("release");
        tokio::time::timeout(Duration::from_secs(10), close)
            .await
            .expect("close completes")
            .expect("close task");
        assert!(!lane.is_open());
        assert_eq!(
            lane.run(|_| Ok(())).await.expect_err("closed").message,
            "closed"
        );
        assert_eq!(
            lane.run_here(|_| Ok(())).expect_err("closed").message,
            "closed"
        );
    }

    #[tokio::test]
    async fn cancelled_waiters_never_run_and_admission_is_checked_after_the_wait() {
        let lane = memory_lane(Arc::new(AtomicUsize::new(0)));
        let permit = lane.hold().await.expect("hold the lane");
        let started = Arc::new(AtomicBool::new(false));
        let queued_started = Arc::clone(&started);
        let queued_lane = Arc::clone(&lane);
        let queued = tokio::spawn(async move {
            queued_lane
                .run_admitted(|| {
                    queued_started.store(true, Ordering::SeqCst);
                    Ok(|_: &Connection| Ok(()))
                })
                .await
        });
        tokio::task::yield_now().await;
        queued.abort();
        let _ = queued.await;
        drop(permit);
        lane.run(|_| Ok(())).await.expect("next call");
        assert!(
            !started.load(Ordering::SeqCst),
            "cancelled work never starts"
        );

        let permit = lane.hold().await.expect("hold the lane");
        let current = AtomicBool::new(true);
        let guarded = lane.run_admitted(|| {
            if !current.load(Ordering::SeqCst) {
                return Err(CoreError::new("Cancelled"));
            }
            Ok(|_: &Connection| Ok(()))
        });
        tokio::pin!(guarded);
        assert!(futures_util::poll!(&mut guarded).is_pending());
        current.store(false, Ordering::SeqCst);
        drop(permit);
        assert_eq!(guarded.await.expect_err("stale").message, "Cancelled");
    }

    #[tokio::test]
    async fn admitted_work_keeps_its_order() {
        let lane = memory_lane(Arc::new(AtomicUsize::new(0)));
        lane.run(|db| db.execute_batch("CREATE TABLE t (x)").sql())
            .await
            .expect("table");
        let first = lane.run(|db| db.execute("INSERT INTO t VALUES (1)", []).sql());
        let second = lane.run(|db| db.execute("UPDATE t SET x = 2", []).sql());
        let (one, two) = tokio::join!(first, second);
        one.expect("first");
        two.expect("second");
        let x: i64 = lane
            .run(|db| db.query_row("SELECT x FROM t", [], |row| row.get(0)).sql())
            .await
            .expect("read");
        assert_eq!(x, 2);
    }
}
