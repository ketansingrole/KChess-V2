//! KChess's main database (`kchess.db`), the storage the services share. The Rust core is its only owner: one connection, opened
//! and migrated on first use, with every storage method dispatched from here.

pub mod ai;
pub mod chesscom;
pub mod debug;
pub mod documents;
pub mod games;
pub mod import;
pub mod insights;
pub mod json_codec;
pub mod library;
pub mod library_import;
pub mod lookups;
pub mod migrations;
pub mod reviews;
pub mod runs;
pub mod studies;
pub mod usage;
pub mod view_preferences;
pub mod voice_log;

use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::error::{CoreError, Result};
use crate::host::{Config, Host, Level};
use crate::tasks::lock;

/// Filtered page counts, the library overview and the storage the games take, valid until a call
/// that can change the stored games runs. The count is a full scan of the filter's index, while
/// the page rows themselves are a short index range; the storage reads every game's row.
///
/// And the library snapshot, which decodes every study and replays every played game: it is kept
/// with the connection's count of changed rows when it was read, and is stale once any write has
/// changed a row since (`library_import::cached_library`).
#[derive(Default)]
pub struct StoreCache {
    counts: Mutex<HashMap<String, i64>>,
    overview: Mutex<Option<Value>>,
    storage: Mutex<Option<Vec<(String, i64, i64)>>>,
    library: Mutex<Option<(u64, Value)>>,
}

impl StoreCache {
    pub fn count(&self, key: &str) -> Option<i64> {
        lock(&self.counts).get(key).copied()
    }

    pub fn remember_count(&self, key: String, total: i64) {
        let mut counts = lock(&self.counts);
        if counts.len() >= 128 {
            counts.clear();
        }
        counts.insert(key, total);
    }

    pub fn overview(&self) -> Option<Value> {
        lock(&self.overview).clone()
    }

    pub fn remember_overview(&self, value: &Value) {
        *lock(&self.overview) = Some(value.clone());
    }

    /// Each account's games and the bytes they take (`usageReport`'s storage).
    pub fn storage(&self) -> Option<Vec<(String, i64, i64)>> {
        lock(&self.storage).clone()
    }

    pub fn remember_storage(&self, rows: &[(String, i64, i64)]) {
        *lock(&self.storage) = Some(rows.to_vec());
    }

    /// The library snapshot read when the connection had changed `changes` rows, if it is kept.
    pub fn library(&self, changes: u64) -> Option<Value> {
        let library = lock(&self.library);
        let (read_at, snapshot) = library.as_ref()?;
        (*read_at == changes).then(|| snapshot.clone())
    }

    pub fn remember_library(&self, changes: u64, snapshot: &Value) {
        *lock(&self.library) = Some((changes, snapshot.clone()));
    }

    /// A connection opened afresh counts its changes from zero again.
    pub fn forget_library(&self) {
        *lock(&self.library) = None;
    }

    pub fn clear(&self) {
        self.forget_library();
        lock(&self.counts).clear();
        *lock(&self.overview) = None;
        *lock(&self.storage) = None;
    }
}

/// Whether `method` can change the games or accounts a count or the overview is computed from.
/// Every games method except the known reads clears the cache, so a new mutation (or one that
/// only touches `accounts`, such as a login) can never leave a stale total behind.
fn invalidates_counts(method: &str) -> bool {
    if let Some(name) = method.strip_prefix("store.games.") {
        return !matches!(
            name,
            "getSettings"
                | "loadData"
                | "gamePage"
                | "gameLibraryOverview"
                | "gameRatingHistory"
                | "pendingGameIds"
                | "gamePgn"
                | "readApiCache"
                | "writeApiCache"
                | "saveSettings"
                | "dismissedFriends"
                | "legacyImportPending"
                | "tokenCiphertext"
        );
    }
    if let Some(name) = method.strip_prefix("store.chesscom.") {
        return !matches!(name, "accounts" | "archives");
    }
    method.starts_with("store.import.") || method.starts_with("store.debug.")
}

/// `kchess.db` as the core shares it: one connection, opened and migrated on first use, behind a
/// lock. The synchronous storage calls and the asynchronous services (the review queue) both
/// reach it through `call`, so there is never a second copy of the database. Callers must not
/// hold the lock across another `call`.
pub struct Database {
    connection: Mutex<Option<Connection>>,
    execution: Arc<tokio::sync::Semaphore>,
    work: tokio_util::task::TaskTracker,
    host: Arc<dyn Host>,
    config: Config,
    cache: StoreCache,
}

impl Database {
    pub fn new(config: Config, host: Arc<dyn Host>) -> Database {
        Database {
            connection: Mutex::new(None),
            execution: Arc::new(tokio::sync::Semaphore::new(1)),
            work: tokio_util::task::TaskTracker::new(),
            host,
            config,
            cache: StoreCache::default(),
        }
    }

    /// Serializes async callers before occupying a blocking worker. The worker owns its permit
    /// and tracking token, so dropping the awaiting future cannot let close race an active write.
    pub async fn call_async(self: &Arc<Self>, method: &str, args: &[Value]) -> Result<Value> {
        self.call_async_with(method, || Ok(args.to_vec())).await
    }

    /// Builds arguments and rechecks mutation ownership after waiting for database admission.
    pub async fn call_async_with(
        self: &Arc<Self>,
        method: &str,
        args: impl FnOnce() -> Result<Vec<Value>>,
    ) -> Result<Value> {
        let permit = Arc::clone(&self.execution)
            .acquire_owned()
            .await
            .map_err(|_| CoreError::new("The database is closed."))?;
        let tracked = self.work.token();
        let database = Arc::clone(self);
        let method = method.to_string();
        let args = args()?;
        let started = std::time::Instant::now();
        let logged_method = method.clone();
        let result = tokio::task::spawn_blocking(move || {
            let (_permit, _tracked) = (permit, tracked);
            database.call_inner(&method, &args, true)
        })
        .await
        .map_err(|cause| CoreError::wrap("Database worker failed", cause))?;
        // Structural head-of-line signal: a single connection serializes all storage,
        // so a slow call blocks UI reads. Log slow calls instead of silently queueing.
        let elapsed = started.elapsed();
        if elapsed.as_millis() > 500 {
            self.host.log(
                Level::Warn,
                "db",
                &format!(
                    "Slow storage call: method={logged_method} durationMs={}",
                    elapsed.as_millis()
                ),
            );
        }
        result
    }

    /// Occupies the async execution lane until the permit drops (`Core::hold_storage_for_tests`).
    pub(crate) async fn hold_execution(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.execution)
            .acquire_owned()
            .await
            .map_err(|_| CoreError::new("The database is closed."))
    }

    /// Refuses queued/new async work, drains accepted operations, then releases the file.
    pub async fn close_async(self: &Arc<Self>) {
        self.execution.close();
        self.work.close();
        self.work.wait().await;
        let database = Arc::clone(self);
        if let Err(cause) = tokio::task::spawn_blocking(move || database.close()).await {
            self.host.log(
                Level::Warn,
                "db",
                &format!("Database close failed: {cause}"),
            );
        }
    }

    /// Runs one storage method (`store.<module>.<name>`) against the database, opening it first.
    ///
    /// Blocking: runs rusqlite synchronously and holds a `std::sync::Mutex` while it does, so
    /// it must never run on an async executor thread. Async callers use `call_async`.
    #[track_caller]
    pub fn call(&self, method: &str, args: &[Value]) -> Result<Value> {
        self.call_inner(method, args, false)
    }

    fn call_inner(&self, method: &str, args: &[Value], admitted: bool) -> Result<Value> {
        let mut db = usable_connection(&self.connection, self.host.as_ref(), "db");
        if !admitted && self.execution.is_closed() {
            return Err(CoreError::new("The database is closed."));
        }
        if method == "store.debug.historical" && debug::enabled() {
            // Test-only: an earlier release's database, written before the store opens the file.
            if db.is_some() {
                return Err(CoreError::new("The database is already open."));
            }
            let path = self.config.data_dir.join("kchess.db");
            return debug::historical(&path, args);
        }
        if db.is_none() {
            let path = self.config.data_dir.join("kchess.db");
            *db = Some(open(&path, self.host.as_ref())?);
            self.cache.forget_library();
        }
        if invalidates_counts(method) {
            self.cache.clear();
        }
        match db.as_ref() {
            Some(connection) => {
                let ctx = StoreContext {
                    db: connection,
                    host: self.host.as_ref(),
                    config: &self.config,
                    cache: &self.cache,
                };
                call(&ctx, method, args)
            }
            None => Err(CoreError::new("The database is unavailable.")),
        }
    }

    /// Releases the file; the next `call` opens it again.
    pub fn close(&self) {
        self.cache.clear();
        lock(&self.connection).take();
    }
}

/// The connection behind `slot`, as a call may use it. A call that panicked holding the lock may
/// have left its transaction open, and a ROLLBACK that failed leaves one open too: either way the
/// connection is dropped (SQLite rolls the transaction back) and the next call opens the file
/// again, rather than every later call failing or joining a transaction nobody will commit.
pub(crate) fn usable_connection<'a>(
    slot: &'a std::sync::Mutex<Option<Connection>>,
    host: &dyn Host,
    scope: &str,
) -> std::sync::MutexGuard<'a, Option<Connection>> {
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

/// What a storage method may use: the open database, the host (for logs) and the core's
/// configuration (the data directory and an earlier release's database, for the one-time imports).
pub struct StoreContext<'a> {
    pub db: &'a Connection,
    pub host: &'a dyn Host,
    pub config: &'a Config,
    pub cache: &'a StoreCache,
}

/// Open `kchess.db`: WAL, foreign keys, NORMAL sync, memory temp store, a 20 MB
/// cache, migrations, then 0600 permissions.
pub fn open(path: &Path, host: &dyn Host) -> Result<Connection> {
    let db =
        Connection::open(path).map_err(|e| CoreError::wrap("Could not open the database", e))?;
    db.execute_batch(
        "PRAGMA busy_timeout = 5000;
         PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -20000;",
    )
    .map_err(|e| CoreError::new(e.to_string()))?;
    migrations::migrate(&db)?;
    restrict(path, host);
    Ok(db)
}

fn restrict(path: &Path, host: &dyn Host) {
    restrict_database(path, host, "db");
}

/// 0600 for a database and its `-wal` and `-shm` files. Opening in WAL mode creates both before
/// the database is restricted, and SQLite gives them the database's mode as it was then.
#[cfg(unix)]
pub(crate) fn restrict_database(path: &Path, host: &dyn Host, tag: &str) {
    use std::os::unix::fs::PermissionsExt;
    let mut files = vec![path.to_path_buf()];
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        files.push(name.into());
    }
    for (index, file) in files.iter().enumerate() {
        match std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)) {
            Err(cause) if index == 0 || cause.kind() != std::io::ErrorKind::NotFound => {
                host.log(Level::Debug, tag, &format!("chmod failed: {cause}"));
            }
            _ => {}
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn restrict_database(_path: &Path, _host: &dyn Host, _tag: &str) {}

type Dispatch = fn(&StoreContext, &str, &[Value]) -> Option<Result<Value>>;

/// Every storage method, `store.<module>.<name>`: each module dispatches its own.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Result<Value> {
    match method {
        // The database is opened and migrated before any call reaches this dispatch.
        "store.open" => return Ok(Value::Null),
        "store.debug.tables" if debug::enabled() => return debug::tables(ctx.db, args),
        "store.debug.exec" if debug::enabled() => return debug::exec(ctx.db, args),
        "store.debug.query" if debug::enabled() => return debug::query(ctx.db, args),
        _ => {}
    }
    let modules: [Dispatch; 12] = [
        migrations::call,
        games::call,
        chesscom::call,
        library::call,
        reviews::call,
        runs::call,
        insights::call,
        usage::call,
        voice_log::call,
        lookups::call,
        view_preferences::call,
        ai::call,
    ];
    modules
        .iter()
        .find_map(|call| call(ctx, method, args))
        .unwrap_or_else(|| Err(CoreError::new(format!("Unknown core method {method}."))))
}

#[cfg(test)]
mod scheduling_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    struct Quiet;
    impl Host for Quiet {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, _: &str, _: Value) {}
    }

    fn database() -> (tempfile::TempDir, Arc<Database>) {
        let dir = tempfile::tempdir().expect("profile");
        let database = Arc::new(Database::new(
            Config::new(dir.path().to_path_buf()),
            Arc::new(Quiet),
        ));
        (dir, database)
    }

    /// A connection opened again counts its changed rows from zero: the library read on the
    /// one before must not stand for what the file holds now (here, a game another core saved
    /// while this one had let the file go).
    #[test]
    fn a_reopened_connection_reads_the_library_afresh() {
        let (dir, database) = database();
        let games = |database: &Database| {
            database
                .call("store.library.library", &[serde_json::json!(0)])
                .expect("library")["games"]
                .as_array()
                .expect("games")
                .len()
        };
        assert_eq!(games(&database), 0);
        // As a failed call leaves it: the connection dropped, the cache not cleared.
        database.connection.lock().expect("lock").take();
        let other = Database::new(Config::new(dir.path().to_path_buf()), Arc::new(Quiet));
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        other
            .call(
                "store.library.saveArchivedGame",
                &[serde_json::json!({
                    "id": "g", "source": "board", "startedAt": 1, "updatedAt": 2, "white": "A",
                    "black": "B", "result": "*", "reason": "-", "timeControl": "-",
                    "finished": false, "setup": { "variant": "standard", "fen": start },
                    "moves": ["e2e4"],
                })],
            )
            .expect("save");
        other.close();
        database.call("store.open", &[]).expect("open");
        assert!(
            database.cache.library.lock().expect("cache").is_none(),
            "opening the file forgets the snapshot"
        );
        assert_eq!(games(&database), 1);
    }

    /// A storage call that panicked with the lock held used to leave it poisoned, so every later
    /// call failed with "The database is unavailable." until the app restarted, and its open
    /// transaction stayed open.
    #[test]
    fn a_call_that_panicked_holding_the_database_does_not_disable_storage() {
        let (_dir, database) = database();
        database.call("store.open", &[]).expect("open");
        let held = Arc::clone(&database);
        let panicked = std::thread::spawn(move || {
            let db = held.connection.lock().expect("lock");
            if let Some(connection) = db.as_ref() {
                connection.execute_batch("BEGIN IMMEDIATE").expect("begin");
            }
            panic!("a storage method panicked");
        })
        .join();
        assert!(panicked.is_err());
        assert!(database.connection.is_poisoned());
        database
            .call("store.open", &[])
            .expect("storage works after the panic");
        assert!(!database.connection.is_poisoned());
        let db = database.connection.lock().expect("lock");
        assert!(db.as_ref().is_some_and(Connection::is_autocommit));
    }

    /// A connection a failed ROLLBACK left inside its transaction is opened again, so later
    /// writes are not lost inside a transaction nobody commits.
    #[test]
    fn a_connection_left_inside_a_transaction_is_opened_again() {
        let slot = Mutex::new(Some(Connection::open_in_memory().expect("open")));
        if let Some(connection) = slot.lock().expect("lock").as_ref() {
            connection.execute_batch("BEGIN").expect("begin");
        }
        assert!(usable_connection(&slot, &Quiet, "db").is_none());
        let slot = Mutex::new(Some(Connection::open_in_memory().expect("open")));
        assert!(usable_connection(&slot, &Quiet, "db").is_some());
    }

    #[tokio::test]
    async fn a_contended_database_keeps_the_executor_free_and_close_drains_a_detached_worker() {
        let (_dir, database) = database();
        // Opened first, so the detached worker's call below is cheap: a first open creates the
        // files and runs every migration, which took over 10 s on a loaded Windows PC whose
        // virus scanner reads each new file.
        database
            .call_async("store.open", &[])
            .await
            .expect("first open");
        let (held, held_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let blocking_db = Arc::clone(&database);
        let holder = std::thread::spawn(move || {
            let _lock = blocking_db.connection.lock().expect("database lock");
            held.send(()).expect("notify");
            release_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("release lock");
        });
        held_rx.await.expect("held");
        let task_db = Arc::clone(&database);
        let caller = tokio::spawn(async move { task_db.call_async("store.open", &[]).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while database.execution.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("executor progresses while storage is blocked");
        caller.abort();
        let _ = caller.await;
        let close_db = Arc::clone(&database);
        let close = tokio::spawn(async move { close_db.close_async().await });
        tokio::task::yield_now().await;
        assert!(
            !close.is_finished(),
            "close waits for the detached database worker"
        );
        release.send(()).expect("release");
        tokio::time::timeout(Duration::from_secs(10), close)
            .await
            .expect("close completes")
            .expect("close task");
        holder.join().expect("holder");
        assert!(database.connection.lock().expect("connection").is_none());
        assert!(database.call_async("store.open", &[]).await.is_err());
        assert!(database.call("store.open", &[]).is_err());
    }

    #[tokio::test]
    async fn cancelled_waiters_do_not_write_and_mutation_ownership_is_rechecked_after_admission() {
        let (_dir, database) = database();
        let permit = Arc::clone(&database.execution)
            .acquire_owned()
            .await
            .expect("hold lane");
        let started = Arc::new(AtomicBool::new(false));
        let queued_db = Arc::clone(&database);
        let queued_started = Arc::clone(&started);
        let queued = tokio::spawn(async move {
            queued_db
                .call_async_with("store.open", || {
                    queued_started.store(true, Ordering::SeqCst);
                    Ok(vec![])
                })
                .await
        });
        tokio::task::yield_now().await;
        queued.abort();
        let _ = queued.await;
        drop(permit);
        database
            .call_async("store.open", &[])
            .await
            .expect("next call");
        assert!(
            !started.load(Ordering::SeqCst),
            "cancelled queued work never starts"
        );

        let permit = Arc::clone(&database.execution)
            .acquire_owned()
            .await
            .expect("hold lane");
        let current = AtomicBool::new(true);
        let guarded = database.call_async_with("store.open", || {
            if !current.load(Ordering::SeqCst) {
                return Err(CoreError::new("Cancelled"));
            }
            Ok(vec![])
        });
        tokio::pin!(guarded);
        assert!(futures_util::poll!(&mut guarded).is_pending());
        current.store(false, Ordering::SeqCst);
        drop(permit);
        assert_eq!(
            guarded.await.expect_err("stale mutation").message,
            "Cancelled"
        );
        database.close_async().await;
    }

    #[tokio::test]
    async fn admitted_writes_keep_their_order() {
        let (_dir, database) = database();
        let first_args = [serde_json::json!("Alice"), serde_json::json!(false)];
        let first = database.call_async("store.games.addAccount", &first_args);
        let args = [serde_json::json!("Alice"), serde_json::json!(true)];
        let second = database.call_async("store.games.addAccount", &args);
        let (one, two) = tokio::join!(first, second);
        one.expect("first write");
        two.expect("second write");
        let data = database
            .call_async("store.games.loadData", &[])
            .await
            .expect("data");
        assert_eq!(data["accounts"][0]["connected"], serde_json::json!(true));
        database.close_async().await;
    }

    #[tokio::test]
    async fn a_cache_write_queued_before_logout_does_not_restore_the_account_cache() {
        use crate::lichess::accounts::LichessStore;
        use crate::lichess::adapters::DbLichessStore;

        let (_dir, database) = database();
        let store = DbLichessStore::new(Arc::clone(&database));
        let permit = Arc::clone(&database.execution)
            .acquire_owned()
            .await
            .expect("hold lane");
        let signed_in = AtomicBool::new(true);
        let current = || signed_in.load(Ordering::SeqCst);
        let profile = serde_json::json!({ "username": "Alice" });
        let write = store.write_api_cache("alice:profile", &profile, &current);
        tokio::pin!(write);
        assert!(futures_util::poll!(&mut write).is_pending());
        signed_in.store(false, Ordering::SeqCst);
        drop(permit);
        write.await.expect("stale writes are skipped");
        assert!(
            store
                .read_api_cache("alice:profile")
                .await
                .expect("cache")
                .is_none()
        );
        database.close_async().await;
    }
}
