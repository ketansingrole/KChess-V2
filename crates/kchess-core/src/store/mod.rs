//! KChess's main database (`kchess.db`), the storage the services share. The Rust core is its
//! only owner: one connection, opened and migrated on first use. Callers run the storage
//! functions of the modules below through [`Database::read`] and [`Database::write`].

pub mod ai;
pub mod chesscom;
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
pub mod sql;
pub mod studies;
pub mod usage;
pub mod view_preferences;
pub mod voice_log;

use kchess_contracts::types::Library;
use rusqlite::Connection;
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::host::{Config, Host, Level};
use crate::tasks::lock;
use kchess_contracts::error::Result;
use kchess_contracts::types::GameLibraryOverview;
use sql::{Lane, LaneMessages, Sql as _};

/// Filtered page counts, the library overview and the storage the games take, valid until a
/// write runs. The count is a full scan of the filter's index, while the page rows themselves are
/// a short index range; the storage reads every game's row.
///
/// And the library snapshot, which decodes every study and replays every played game: it is kept
/// with the connection's count of changed rows when it was read, and is stale once any write has
/// changed a row since (`library_import::cached_library`).
#[derive(Default)]
pub struct StoreCache {
    counts: Mutex<HashMap<String, i64>>,
    overview: Mutex<Option<GameLibraryOverview>>,
    storage: Mutex<Option<Vec<(String, i64, i64)>>>,
    library: Mutex<Option<(u64, Library)>>,
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

    pub fn overview(&self) -> Option<GameLibraryOverview> {
        lock(&self.overview).clone()
    }

    pub fn remember_overview(&self, overview: &GameLibraryOverview) {
        *lock(&self.overview) = Some(overview.clone());
    }

    /// Each account's games and the bytes they take.
    pub fn storage(&self) -> Option<Vec<(String, i64, i64)>> {
        lock(&self.storage).clone()
    }

    pub fn remember_storage(&self, rows: &[(String, i64, i64)]) {
        *lock(&self.storage) = Some(rows.to_vec());
    }

    /// The library snapshot read when the connection had changed `changes` rows, if it is kept.
    pub fn library(&self, changes: u64) -> Option<Library> {
        let library = lock(&self.library);
        let (read_at, snapshot) = library.as_ref()?;
        (*read_at == changes).then(|| snapshot.clone())
    }

    pub fn remember_library(&self, changes: u64, snapshot: &Library) {
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

/// `kchess.db` as the core shares it: one connection on one [`Lane`], so there is never a second
/// copy of the database. A read leaves the caches alone; a write clears them, so no write can
/// leave a stale total behind.
pub struct Database {
    lane: Arc<Lane>,
    host: Arc<dyn Host>,
    config: Config,
    cache: Arc<StoreCache>,
}

/// Whether a call may change what the caches were computed from.
#[derive(Clone, Copy)]
enum Access {
    Read,
    Write,
}

impl Database {
    pub fn new(config: Config, host: Arc<dyn Host>) -> Database {
        let cache = Arc::new(StoreCache::default());
        let path = config.data_dir.join("kchess.db");
        let opened = Arc::clone(&cache);
        let lane = Lane::new(
            "db",
            LaneMessages {
                closed: "The database is closed.",
                stopped: "Database worker failed.",
            },
            Arc::clone(&host),
            move |host| {
                let db = open(&path, host)?;
                opened.forget_library();
                Ok(db)
            },
        );
        Database {
            lane: Arc::new(lane),
            host,
            config,
            cache,
        }
    }

    /// Runs a read on this thread. Blocking: never on an async executor thread.
    pub fn read<T>(&self, work: impl FnOnce(&StoreContext) -> Result<T>) -> Result<T> {
        self.here(Access::Read, work)
    }

    /// Runs a write on this thread. Blocking: never on an async executor thread.
    pub fn write<T>(&self, work: impl FnOnce(&StoreContext) -> Result<T>) -> Result<T> {
        self.here(Access::Write, work)
    }

    /// Runs a read on a blocking worker, in turn with every other call.
    #[track_caller]
    pub fn read_async<T, F>(
        self: &Arc<Self>,
        work: F,
    ) -> impl Future<Output = Result<T>> + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&StoreContext) -> Result<T> + Send + 'static,
    {
        self.admitted(Access::Read, move || Ok(work))
    }

    /// Runs a write on a blocking worker, in turn with every other call.
    #[track_caller]
    pub fn write_async<T, F>(
        self: &Arc<Self>,
        work: F,
    ) -> impl Future<Output = Result<T>> + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&StoreContext) -> Result<T> + Send + 'static,
    {
        self.admitted(Access::Write, move || Ok(work))
    }

    /// Runs the write `prepare` returns once the database admits it. `prepare` runs at admission:
    /// a write wanted only while its caller is still current (a sync a logout ended) checks that
    /// there, after waiting its turn, and fails or skips rather than writing.
    #[track_caller]
    pub fn write_admitted<T, W, P>(
        self: &Arc<Self>,
        prepare: P,
    ) -> impl Future<Output = Result<T>> + use<T, W, P>
    where
        T: Send + 'static,
        W: FnOnce(&StoreContext) -> Result<T> + Send + 'static,
        P: FnOnce() -> Result<W>,
    {
        self.admitted(Access::Write, prepare)
    }

    #[track_caller]
    fn admitted<T, W, P>(
        self: &Arc<Self>,
        access: Access,
        prepare: P,
    ) -> impl Future<Output = Result<T>> + use<T, W, P>
    where
        T: Send + 'static,
        W: FnOnce(&StoreContext) -> Result<T> + Send + 'static,
        P: FnOnce() -> Result<W>,
    {
        let database = Arc::clone(self);
        self.lane.run_admitted(move || {
            let work = prepare()?;
            Ok(move |db: &Connection| database.with_context(access, db, work))
        })
    }

    fn here<T>(&self, access: Access, work: impl FnOnce(&StoreContext) -> Result<T>) -> Result<T> {
        self.lane.run_here(|db| self.with_context(access, db, work))
    }

    fn with_context<T>(
        &self,
        access: Access,
        db: &Connection,
        work: impl FnOnce(&StoreContext) -> Result<T>,
    ) -> Result<T> {
        let ctx = StoreContext {
            db,
            host: self.host.as_ref(),
            config: &self.config,
            cache: &self.cache,
        };
        let result = work(&ctx);
        if matches!(access, Access::Write) {
            self.cache.clear();
        }
        result
    }

    /// Occupies the async execution lane until the permit drops: a test holds the store busy.
    pub async fn hold_execution(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        self.lane.hold().await
    }

    /// Refuses queued/new work, drains accepted operations, then releases the file.
    pub async fn close_async(&self) {
        self.lane.close().await;
        self.cache.clear();
    }

    /// Releases the file; the next call opens it again.
    pub fn close(&self) {
        self.cache.clear();
        self.lane.release();
    }
}

/// What a storage function may use: the open database, the host (for logs), the core's
/// configuration (the data directory and an earlier release's database, for the one-time imports)
/// and the caches.
pub struct StoreContext<'a> {
    pub db: &'a Connection,
    pub host: &'a dyn Host,
    pub config: &'a Config,
    pub cache: &'a StoreCache,
}

/// Runs `work` over a fresh, migrated database in memory.
#[cfg(test)]
pub(crate) fn in_memory<T>(work: impl FnOnce(&StoreContext) -> T) -> T {
    let db = Connection::open_in_memory().expect("open");
    migrations::migrate(&db).expect("migrate");
    let config = Config::new(std::path::PathBuf::new());
    let cache = StoreCache::default();
    work(&StoreContext {
        db: &db,
        host: &crate::test_host::QUIET,
        config: &config,
        cache: &cache,
    })
}

/// Open `kchess.db`: WAL, foreign keys, NORMAL sync, memory temp store, a 20 MB
/// cache, migrations, then 0600 permissions.
pub fn open(path: &Path, host: &dyn Host) -> Result<Connection> {
    let db = Connection::open(path)
        .map_err(|e| kchess_contracts::error::CoreError::wrap("Could not open the database", e))?;
    db.execute_batch(
        "PRAGMA busy_timeout = 5000;
         PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -20000;",
    )
    .sql()?;
    migrations::migrate(&db)?;
    restrict_database(path, host, "db");
    Ok(db)
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

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn database() -> (tempfile::TempDir, Arc<Database>) {
        let dir = tempfile::tempdir().expect("profile");
        let database = Arc::new(Database::new(
            Config::new(dir.path().to_path_buf()),
            crate::test_host::TestHost::quiet(),
        ));
        (dir, database)
    }

    fn played_games(database: &Database) -> usize {
        database
            .read(|ctx| library::cached_library(ctx, 0.0))
            .expect("library")
            .games
            .len()
    }

    /// A connection opened again counts its changed rows from zero: the library read on the
    /// one before must not stand for what the file holds now (here, a game another core saved
    /// while this one had let the file go).
    #[test]
    fn a_reopened_connection_reads_the_library_afresh() {
        let (dir, database) = database();
        assert_eq!(played_games(&database), 0);
        database.lane.release();
        let other = Database::new(
            Config::new(dir.path().to_path_buf()),
            crate::test_host::TestHost::quiet(),
        );
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let game = json!({
            "id": "g", "source": "board", "startedAt": 1, "updatedAt": 2, "white": "A",
            "black": "B", "result": "*", "reason": "-", "timeControl": "-",
            "finished": false, "setup": { "variant": "standard", "fen": start },
            "moves": ["e2e4"],
        });
        other
            .write(|ctx| {
                library::write_game(
                    ctx.db,
                    &kchess_domain::library::decode_archived_game(&game).expect("a played game"),
                )
            })
            .expect("save");
        other.close();
        assert_eq!(played_games(&database), 1);
    }

    /// A write clears the cached totals, whichever table it changed.
    #[test]
    fn a_write_clears_the_cached_totals_and_a_read_keeps_them() {
        let (_dir, database) = database();
        database
            .read(|ctx| {
                ctx.cache.remember_count("all".into(), 3);
                Ok(())
            })
            .expect("read");
        assert_eq!(database.cache.count("all"), Some(3));
        database
            .read(|ctx| games::load_app_data(ctx.db))
            .expect("read");
        assert_eq!(database.cache.count("all"), Some(3));
        database
            .write(|ctx| games::add_account(ctx.db, "Alice", false))
            .expect("write");
        assert_eq!(database.cache.count("all"), None);
    }

    #[tokio::test]
    async fn admitted_writes_keep_their_order() {
        let (_dir, database) = database();
        let first = database.write_async(|ctx| games::add_account(ctx.db, "Alice", false));
        let second = database.write_async(|ctx| games::add_account(ctx.db, "Alice", true));
        let (one, two) = tokio::join!(first, second);
        one.expect("first write");
        two.expect("second write");
        let data = database
            .read_async(|ctx| games::load_app_data(ctx.db))
            .await
            .expect("data");
        assert!(data.accounts[0].connected);
        database.close_async().await;
    }

    #[tokio::test]
    async fn a_cache_write_queued_before_logout_does_not_restore_the_account_cache() {
        use crate::lichess::accounts::LichessStore;
        use crate::lichess::adapters::DbLichessStore;

        let (_dir, database) = database();
        let store = DbLichessStore::new(Arc::clone(&database));
        let permit = database.hold_execution().await.expect("hold lane");
        let signed_in = AtomicBool::new(true);
        let current = || signed_in.load(Ordering::SeqCst);
        let profile = json!({ "username": "Alice" });
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
