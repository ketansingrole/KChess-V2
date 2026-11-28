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
pub mod notifications;
pub mod own_games;
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::host::{Config, Host};
use crate::tasks::lock;
use kchess_contracts::error::Result;
use kchess_contracts::types::GameLibraryOverview;
use sql::{Lane, LaneMessages, Sql as _};

/// Reads kept until the tables they were read from change: the synced games' filtered page counts,
/// overview and storage (a count is a full scan of the filter's index, the storage reads every
/// game's row), and the library snapshot, which decodes every study and replays every played game
/// (`library_import::cached_library`).
///
/// The connection tells [`StoreCache::watch`] of every row it changes, so a write to the settings
/// keeps the library, and a played game's move keeps the synced games' totals. A change it is not
/// told of (SQLite deletes a whole table without naming its rows) still counts in the
/// connection's `total_changes`, and that makes every kept read stale; so does any change on a
/// connection nobody watches.
#[derive(Default)]
pub struct StoreCache {
    changes: Arc<Changes>,
    games: Mutex<Option<(Stamp, GameTotals)>>,
    library: Mutex<Option<(Stamp, Library)>>,
}

/// What the synced games' reads keep.
#[derive(Default)]
struct GameTotals {
    counts: HashMap<String, i64>,
    overview: Option<GameLibraryOverview>,
    storage: Option<Vec<(String, i64, i64)>>,
}

/// The tables a kept read comes from.
#[derive(Clone, Copy)]
enum Area {
    /// `games` and `accounts`.
    Games,
    /// `archived_games`, `studies` and every document but the view preferences.
    Library,
}

/// The changes the connection has told of since it opened.
#[derive(Default)]
struct Changes {
    /// Every row told of: whatever `total_changes` counts beyond them, nobody was told of.
    told: AtomicU64,
    games: AtomicU64,
    library: AtomicU64,
    /// The documents written since the library last looked: only some are the library's.
    documents: Mutex<Vec<i64>>,
}

/// When a kept read was made: its area's changes then, and the changes nobody was told of.
type Stamp = (u64, u64);

/// The most written documents remembered before the library counts as changed.
const DOCUMENTS_KEPT: usize = 64;

impl Changes {
    fn told(&self, action: rusqlite::hooks::Action, table: &str, rowid: i64) {
        self.told.fetch_add(1, Ordering::Relaxed);
        match table {
            "games" | "accounts" => {
                self.games.fetch_add(1, Ordering::Relaxed);
            }
            "documents" if action != rusqlite::hooks::Action::SQLITE_DELETE => {
                let mut documents = lock(&self.documents);
                if documents.len() < DOCUMENTS_KEPT {
                    documents.push(rowid);
                } else {
                    self.library.fetch_add(1, Ordering::Relaxed);
                }
            }
            "documents" | "archived_games" | "studies" => {
                self.library.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    fn stamp(&self, db: &Connection, area: Area) -> Stamp {
        let changes = match area {
            Area::Games => &self.games,
            Area::Library => {
                self.settle_documents(db);
                &self.library
            }
        };
        let untold = db
            .total_changes()
            .wrapping_sub(self.told.load(Ordering::Relaxed));
        (changes.load(Ordering::Relaxed), untold)
    }

    /// Counts the library as changed when a document written since is any but the view
    /// preferences, or can no longer be read.
    fn settle_documents(&self, db: &Connection) {
        let written = std::mem::take(&mut *lock(&self.documents));
        let library = written.iter().any(|rowid| {
            db.query_row(
                "SELECT key = ?2 FROM documents WHERE rowid = ?1",
                rusqlite::params![rowid, view_preferences::VIEW_PREFERENCES],
                |row| row.get::<_, bool>(0),
            ) != Ok(true)
        });
        if library {
            self.library.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl StoreCache {
    /// Tells this cache of every row `db` changes from now on.
    pub fn watch(&self, db: &mut Connection) -> Result<()> {
        let changes = Arc::clone(&self.changes);
        db.update_hook(Some(move |action, _: &str, table: &str, rowid| {
            changes.told(action, table, rowid);
        }))
        .sql()
    }

    fn games<T>(&self, db: &Connection, work: impl FnOnce(&mut GameTotals) -> T) -> T {
        let stamp = self.changes.stamp(db, Area::Games);
        let mut games = lock(&self.games);
        let kept = games.get_or_insert_with(|| (stamp, GameTotals::default()));
        if kept.0 != stamp {
            *kept = (stamp, GameTotals::default());
        }
        work(&mut kept.1)
    }

    pub fn count(&self, db: &Connection, key: &str) -> Option<i64> {
        self.games(db, |games| games.counts.get(key).copied())
    }

    pub fn remember_count(&self, db: &Connection, key: String, total: i64) {
        self.games(db, |games| {
            if games.counts.len() >= 128 {
                games.counts.clear();
            }
            games.counts.insert(key, total);
        });
    }

    pub fn overview(&self, db: &Connection) -> Option<GameLibraryOverview> {
        self.games(db, |games| games.overview.clone())
    }

    pub fn remember_overview(&self, db: &Connection, overview: &GameLibraryOverview) {
        self.games(db, |games| games.overview = Some(overview.clone()));
    }

    /// Each account's games and the bytes they take.
    pub fn storage(&self, db: &Connection) -> Option<Vec<(String, i64, i64)>> {
        self.games(db, |games| games.storage.clone())
    }

    pub fn remember_storage(&self, db: &Connection, rows: &[(String, i64, i64)]) {
        self.games(db, |games| games.storage = Some(rows.to_vec()));
    }

    /// The library snapshot, if none of its tables changed since it was read.
    pub fn library(&self, db: &Connection) -> Option<Library> {
        let stamp = self.changes.stamp(db, Area::Library);
        let library = lock(&self.library);
        let (read_at, snapshot) = library.as_ref()?;
        (*read_at == stamp).then(|| snapshot.clone())
    }

    pub fn remember_library(&self, db: &Connection, snapshot: &Library) {
        let stamp = self.changes.stamp(db, Area::Library);
        *lock(&self.library) = Some((stamp, snapshot.clone()));
    }

    /// The kept count, as it was kept, for the tests that close the database.
    #[cfg(test)]
    fn kept_count(&self, key: &str) -> Option<i64> {
        lock(&self.games).as_ref()?.1.counts.get(key).copied()
    }

    /// Forgets every kept read: a connection opened afresh counts its changes from zero again.
    pub fn clear(&self) {
        *lock(&self.library) = None;
        *lock(&self.games) = None;
    }
}

/// `kchess.db` as the core shares it: one connection on one [`Lane`], so there is never a second
/// copy of the database. Its caches keep each read until a table it came from changes
/// ([`StoreCache`]), whichever call changed it. A write that changed the player's own games says so to whoever
/// follows them (`own_games_changes`).
pub struct Database {
    lane: Arc<Lane>,
    host: Arc<dyn Host>,
    config: Config,
    cache: Arc<StoreCache>,
    own_games: tokio::sync::watch::Sender<u64>,
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
                let mut db = open(&path, host)?;
                opened.clear();
                opened.watch(&mut db)?;
                Ok(db)
            },
        );
        Database {
            lane: Arc::new(lane),
            host,
            config,
            cache,
            own_games: tokio::sync::watch::channel(0).0,
        }
    }

    /// Counts the writes that changed the player's own games (`own_games`): synced games, or a
    /// finished game of Game history.
    pub fn own_games_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.own_games.subscribe()
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
            match own_games::take_changes(db) {
                Ok(true) => self.own_games.send_modify(|count| *count += 1),
                Ok(false) => {}
                Err(cause) => self.host.log(
                    crate::host::Level::Warn,
                    "db",
                    &format!("Changes to your games not counted: {}", cause.message),
                ),
            }
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
    own_games::watch(&db)?;
    restrict_database(path, host, "db");
    Ok(db)
}

/// 0600 for a database and its `-wal` and `-shm` files. Opening in WAL mode creates both before
/// the database is restricted, and SQLite gives them the database's mode as it was then.
#[cfg(unix)]
pub(crate) fn restrict_database(path: &Path, host: &dyn Host, tag: &str) {
    use crate::host::Level;
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

    /// A kept read lasts until a table it came from changes, whichever call changed it: a played
    /// game's move keeps the synced games' totals, a setting keeps the library. A change SQLite
    /// does not name row by row (a whole table emptied at once) drops every kept read.
    #[test]
    fn a_kept_read_lasts_until_a_table_it_came_from_changes() {
        type Change = fn(&StoreContext) -> Result<()>;
        let changes: [(&str, Change, (bool, bool)); 7] = [
            (
                "nothing",
                |ctx| games::load_app_data(ctx.db).map(drop),
                (true, true),
            ),
            (
                "the settings",
                |ctx| games::save_settings(ctx.db, &games::default_settings()).map(drop),
                (true, true),
            ),
            (
                "a view preference",
                |ctx| view_preferences::set(ctx, "kchess:board", &json!(1)),
                (true, true),
            ),
            (
                "an account",
                |ctx| games::add_account(ctx.db, "Alice", false).map(drop),
                (false, true),
            ),
            (
                "a session",
                |ctx| {
                    library::save_session(
                        ctx.db,
                        kchess_contracts::types::SessionKind::Local,
                        json!({}),
                    )
                },
                (true, false),
            ),
            (
                "a played game",
                |ctx| {
                    let game = json!({
                        "id": "one", "source": "board", "startedAt": 1, "updatedAt": 2,
                        "white": "A", "black": "B", "result": "1-0", "reason": "Checkmate",
                        "timeControl": "-", "finished": true,
                        "moves": ["e2e4", "e7e5", "d1h5", "b8c6", "f1c4", "g8f6", "h5f7"],
                        "setup": { "variant": "standard", "fen": kchess_domain::rules::INITIAL_FEN },
                    });
                    let game = kchess_domain::library::decode_archived_game(&game)
                        .ok_or_else(|| kchess_contracts::error::CoreError::new("not a game"))?;
                    library::write_game(ctx.db, &game)
                },
                (true, false),
            ),
            (
                "a whole table",
                |ctx| {
                    ctx.db
                        .execute_batch(
                            "CREATE TEMP TABLE IF NOT EXISTS scratch (x);
                             INSERT INTO scratch VALUES (1);
                             DELETE FROM scratch;",
                        )
                        .sql()
                },
                (false, false),
            ),
        ];
        let (_dir, database) = database();
        for (change, write, expected) in changes {
            database
                .read(|ctx| {
                    ctx.cache.remember_count(ctx.db, "all".into(), 3);
                    library::cached_library(ctx, 0.0).map(drop)
                })
                .expect("read");
            database.write(write).expect(change);
            let kept = database
                .read(|ctx| {
                    Ok((
                        ctx.cache.count(ctx.db, "all").is_some(),
                        ctx.cache.library(ctx.db).is_some(),
                    ))
                })
                .expect("read");
            assert_eq!(kept, expected, "after a change to {change}");
        }
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

    /// `close` lets the file go and forgets the caches: the next call opens what is on disk now
    /// (here, nothing). `close_async` refuses every later call.
    #[tokio::test]
    async fn closing_lets_the_file_go_and_forgets_the_caches() {
        let (dir, database) = database();
        database
            .write(|ctx| games::add_account(ctx.db, "Alice", false))
            .expect("write");
        database
            .read(|ctx| {
                ctx.cache.remember_count(ctx.db, "all".into(), 3);
                Ok(())
            })
            .expect("read");
        database.close();
        assert_eq!(database.cache.kept_count("all"), None);
        for suffix in ["", "-wal", "-shm"] {
            let file = dir.path().join(format!("kchess.db{suffix}"));
            if file.exists() {
                std::fs::remove_file(file).expect("the released file goes");
            }
        }
        let data = database
            .read(|ctx| games::load_app_data(ctx.db))
            .expect("a fresh file");
        assert!(data.accounts.is_empty(), "{:?}", data.accounts);

        database
            .read(|ctx| {
                ctx.cache.remember_count(ctx.db, "all".into(), 3);
                Ok(())
            })
            .expect("read");
        database.close_async().await;
        assert_eq!(database.cache.kept_count("all"), None);
        let refused = database
            .read_async(|ctx| games::load_app_data(ctx.db))
            .await;
        assert_eq!(
            refused.map(drop).map_err(|cause| cause.message),
            Err("The database is closed.".to_string())
        );
    }
}
