//! KChess's main database (`kchess.db`), the storage the TypeScript services used to own
//! (`crates/kchess-node/js/{db,migrations,store,library,reviewStore,runs,insights,usage,voiceLog,
//! setupPositionLookup}.ts`, now removed). The Rust core is its only owner: one connection, opened
//! and migrated on first use, with every storage method dispatched from here.

pub mod debug;
pub mod games;
pub mod import;
pub mod insights;
pub mod library;
pub mod lookups;
pub mod migrations;
pub mod reviews;
pub mod runs;
pub mod usage;
pub mod voice_log;

use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::error::{CoreError, Result};
use crate::host::{Config, Host, Level};

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
}

impl Database {
    pub fn new(config: Config, host: Arc<dyn Host>) -> Database {
        Database {
            connection: Mutex::new(None),
            execution: Arc::new(tokio::sync::Semaphore::new(1)),
            work: tokio_util::task::TaskTracker::new(),
            host,
            config,
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
        tokio::task::spawn_blocking(move || {
            let (_permit, _tracked) = (permit, tracked);
            database.call_inner(&method, &args, true)
        })
        .await
        .map_err(|cause| CoreError::wrap("Database worker failed", cause))?
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
        let mut db = self
            .connection
            .lock()
            .map_err(|_| CoreError::new("The database is unavailable."))?;
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
        }
        match db.as_ref() {
            Some(connection) => {
                let ctx = StoreContext {
                    db: connection,
                    host: self.host.as_ref(),
                    config: &self.config,
                };
                call(&ctx, method, args)
            }
            None => Err(CoreError::new("The database is unavailable.")),
        }
    }

    /// Releases the file; the next `call` opens it again.
    pub fn close(&self) {
        if let Ok(mut db) = self.connection.lock() {
            db.take();
        }
    }
}

/// What a storage method may use: the open database, the host (for logs) and the core's
/// configuration (the data directory and an earlier release's database, for the one-time imports).
pub struct StoreContext<'a> {
    pub db: &'a Connection,
    pub host: &'a dyn Host,
    pub config: &'a Config,
}

/// Open `kchess.db` as `db.ts` did: WAL, foreign keys, NORMAL sync, memory temp store, a 20 MB
/// cache, migrations, then 0600 permissions.
pub fn open(path: &Path, host: &dyn Host) -> Result<Connection> {
    let db =
        Connection::open(path).map_err(|e| CoreError::wrap("Could not open the database", e))?;
    db.execute_batch(
        "PRAGMA journal_mode = WAL;
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

#[cfg(unix)]
fn restrict(path: &Path, host: &dyn Host) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(cause) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        host.log(Level::Debug, "db", &format!("chmod failed: {cause}"));
    }
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _host: &dyn Host) {}

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
    let modules: [Dispatch; 9] = [
        migrations::call,
        games::call,
        library::call,
        reviews::call,
        runs::call,
        insights::call,
        usage::call,
        voice_log::call,
        lookups::call,
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

    #[tokio::test]
    async fn a_contended_database_keeps_the_executor_free_and_close_drains_a_detached_worker() {
        let (_dir, database) = database();
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
        tokio::time::timeout(Duration::from_secs(1), close)
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
