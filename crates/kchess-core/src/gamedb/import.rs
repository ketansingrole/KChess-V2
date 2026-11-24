//! Reading a PGN file into a database, on one blocking thread.
//!
//! A new database is written to a `.partial` file with no journal and renamed into place once it
//! is complete, so a failure or a cancellation leaves no database behind. Games added to an
//! existing database are written in batches under their import's id, invisible to searches until
//! the import finishes (`search::visible`), and the tree rows are added in the final transaction;
//! a failure removes the import's games and leaves the tree as it was.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{Connection, OptionalExtension as _, params};

use super::schema;
use super::tree::{Stats, TreeBuilder};
use crate::store::sql::{Sql as _, transaction};
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::DatabaseSource;
use kchess_domain::gamedb::{self, Stored};
use kchess_domain::pgn::{PgnItem, PgnReader};

/// Games written to the file in one transaction.
const BATCH: u64 = 5_000;

/// Where an import writes.
pub enum Target {
    /// A new database, written to `partial` and renamed to `file` when it is complete.
    New {
        file: PathBuf,
        partial: PathBuf,
        name: String,
        source: DatabaseSource,
    },
    /// An existing database's file, added to.
    Existing { file: PathBuf },
}

/// How far an import has come.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    pub games: u64,
    pub skipped: u64,
    /// Bytes of the file read (compressed bytes for a `.zst`).
    pub bytes: u64,
    pub total_bytes: u64,
}

/// What an import is told along the way.
pub trait Watch {
    /// Games read so far; called often, so it should be cheap.
    fn reading(&mut self, count: Count);
    /// The games are in: the tree and indexes are being written.
    fn indexing(&mut self, count: Count);
    fn cancelled(&self) -> bool;
}

/// A file's bytes, counted as they are read.
struct Counted<R> {
    inner: R,
    read: Arc<AtomicU64>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(out)?;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

fn cancelled_error() -> CoreError {
    CoreError::new("Import cancelled.")
}

/// The source file, opened: a regular file outside the profile, so an import can never read the
/// profile's own files (its logins, its databases).
fn open_source(path: &Path, profile: &Path) -> Result<File> {
    let unreadable = |e| CoreError::wrap("Could not read that file", e);
    let real = std::fs::canonicalize(path).map_err(unreadable)?;
    if let Ok(profile) = std::fs::canonicalize(profile)
        && real.starts_with(&profile)
    {
        return Err(CoreError::new("Choose a file outside KChess's own folder."));
    }
    let file = File::open(&real).map_err(unreadable)?;
    if !file.metadata().map_err(unreadable)?.is_file() {
        return Err(CoreError::new("Choose a .pgn or .pgn.zst file."));
    }
    Ok(file)
}

/// Names (players, events) by id, added to their table the first time a game names them.
struct Names {
    table: &'static str,
    ids: HashMap<String, i64>,
}

impl Names {
    fn new(table: &'static str) -> Names {
        Names {
            table,
            ids: HashMap::new(),
        }
    }

    fn id(&mut self, db: &Connection, name: &str) -> Result<i64> {
        if let Some(&id) = self.ids.get(name) {
            return Ok(id);
        }
        let table = self.table;
        db.prepare_cached(&format!(
            "INSERT INTO {table} (name, norm) VALUES (?1, ?2) ON CONFLICT (name) DO NOTHING"
        ))
        .sql()?
        .execute(params![name, gamedb::fold_name(name)])
        .sql()?;
        let id: i64 = db
            .prepare_cached(&format!("SELECT id FROM {table} WHERE name = ?1"))
            .sql()?
            .query_row([name], |row| row.get(0))
            .sql()?;
        // Bounded: a file with millions of distinct names reads them from the table instead.
        if self.ids.len() < 1_000_000 {
            self.ids.insert(name.to_string(), id);
        }
        Ok(id)
    }
}

/// `YYYYMMDD` of a stored date, with 0 for what is unknown; orders games by date.
fn date_key(stored: &Stored) -> u32 {
    let day = stored
        .date
        .as_deref()
        .and_then(|date| date.split(['.', '-', '/']).nth(2))
        .and_then(|day| day.parse::<u32>().ok())
        .filter(|day| (1..=31).contains(day))
        .unwrap_or(0);
    let year = stored.year.and_then(|y| u32::try_from(y).ok()).unwrap_or(0);
    year * 10_000 + u32::from(stored.month.unwrap_or(0)) * 100 + day
}

/// The lower rating of a game both of whose players have one: "both rated at least" is one
/// indexed range on it.
fn low_elo(stored: &Stored) -> Option<u16> {
    Some(stored.white_elo?.min(stored.black_elo?))
}

/// The average of the ratings a game has.
fn rating(stored: &Stored) -> Option<u16> {
    match (stored.white_elo, stored.black_elo) {
        (Some(w), Some(b)) => Some(((u32::from(w) + u32::from(b)) / 2) as u16),
        (one, other) => one.or(other),
    }
}

fn insert_game(
    db: &Connection,
    import: i64,
    stored: &Stored,
    players: &mut Names,
    events: &mut Names,
) -> Result<i64> {
    let white = players.id(db, &stored.white)?;
    let black = players.id(db, &stored.black)?;
    let event = match &stored.event {
        Some(event) => Some(events.id(db, event)?),
        None => None,
    };
    db.prepare_cached(
        "INSERT INTO games (import_id, white, black, event, site, date, date_key, year, round,
            white_elo, black_elo, rating, low_elo, result, eco, opening, fen, plies, moves, extra)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19, ?20)",
    )
    .sql()?
    .execute(params![
        import,
        white,
        black,
        event,
        stored.site,
        stored.date,
        date_key(stored),
        stored.year,
        stored.round,
        stored.white_elo,
        stored.black_elo,
        rating(stored),
        low_elo(stored),
        stored.outcome.score(),
        stored.eco,
        stored.opening,
        stored.fen,
        stored.plies as i64,
        stored.moves,
        stored.extra,
    ])
    .sql()?;
    Ok(db.last_insert_rowid())
}

/// The tree rows of one game: each of its first `depth` positions and the move played there.
fn add_to_tree(tree: &mut TreeBuilder, id: i64, stored: &Stored, depth: u32) -> Result<()> {
    let Some(start) = gamedb::start_of(stored.fen.as_deref()) else {
        return Ok(());
    };
    let stats = Stats::of(id, stored.outcome, rating(stored), date_key(stored));
    let mut ply = 0;
    let mut failed = None;
    gamedb::walk_line(&start, &stored.moves, |pos, m| {
        if ply >= depth {
            return false;
        }
        ply += 1;
        match tree.add(gamedb::position_key(pos), gamedb::pack_move(m), &stats) {
            Ok(()) => true,
            Err(error) => {
                failed = Some(error);
                false
            }
        }
    });
    failed.map_or(Ok(()), Err)
}

/// The games of the file at `path` into `target`. `dir` holds the tree's run files; `capacity`
/// bounds the tree rows held in memory.
pub fn run(
    path: &Path,
    compressed: bool,
    profile: &Path,
    target: &Target,
    dir: &Path,
    capacity: usize,
    watch: &mut dyn Watch,
) -> Result<Count> {
    let file = open_source(path, profile)?;
    let total_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
    let read = Arc::new(AtomicU64::new(0));
    let counted = Counted {
        inner: file,
        read: Arc::clone(&read),
    };
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    let now = crate::time::now_ms();
    let (db, adding) = match target {
        Target::New {
            partial,
            name,
            source,
            ..
        } => {
            remove_files(partial);
            (schema::create(partial, name, *source, now)?, false)
        }
        Target::Existing { file } => (schema::open_write(file)?, true),
    };
    let result = (|| {
        let depth: u32 = db
            .query_row("SELECT index_depth FROM meta WHERE id = 1", [], |row| {
                row.get(0)
            })
            .sql()?;
        // An import that never finished (KChess was stopped under it) left games no search
        // sees; they go before this one's, which a later finished import would make visible.
        transaction(&db, || {
            db.execute_batch(
                "DELETE FROM games WHERE import_id IN
                   (SELECT id FROM imports WHERE finished_at IS NULL);
                 DELETE FROM imports WHERE finished_at IS NULL;",
            )
            .sql()
        })?;
        db.execute(
            "INSERT INTO imports (started_at, file) VALUES (?1, ?2)",
            params![now, file_name],
        )
        .sql()?;
        let import = db.last_insert_rowid();
        let count = if compressed {
            let (send, receive) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
            let decoder = std::thread::Builder::new()
                .name("kchess-gamedb-zstd".into())
                .spawn(move || {
                    crate::zst::decode(counted, |chunk| {
                        send.blocking_send(chunk.to_vec())
                            .map_err(|_| std::io::Error::other("import stopped"))
                    })
                })
                .map_err(|e| CoreError::wrap("Could not start the import", e))?;
            let input = BufReader::with_capacity(1 << 20, crate::zst::ChannelReader::new(receive));
            let games = read_games(
                &db,
                import,
                input,
                depth,
                dir,
                capacity,
                &read,
                total_bytes,
                watch,
            );
            // The decoder ends once the reader is dropped; its failure (a damaged file) fails
            // the import even when the games read so far looked complete.
            let decoded = decoder
                .join()
                .map_err(|_| CoreError::new("The import stopped. Try again."))?;
            match (games, decoded) {
                (Ok(games), Ok(())) => Ok(games),
                (Err(error), _) => Err(error),
                (Ok(_), Err(cause)) => Err(CoreError::wrap(
                    "That file is not a readable .pgn.zst",
                    cause,
                )),
            }?
        } else {
            let input = BufReader::with_capacity(1 << 20, counted);
            read_games(
                &db,
                import,
                input,
                depth,
                dir,
                capacity,
                &read,
                total_bytes,
                watch,
            )?
        };
        Ok::<_, CoreError>((import, count))
    })();
    let (import, (count, tree)) = match result {
        Ok(done) => done,
        Err(error) => {
            undo(db, target);
            return Err(error);
        }
    };
    watch.indexing(count);
    let finished =
        (|| {
            if watch.cancelled() {
                return Err(cancelled_error());
            }
            let finish =
                || {
                    tree.write(&db, adding, &|| watch.cancelled())?;
                    db.execute(
                "UPDATE imports SET finished_at = ?2, games = ?3, skipped = ?4 WHERE id = ?1",
                params![import, crate::time::now_ms(), count.games as i64, count.skipped as i64],
            )
            .sql()?;
                    db.execute(
                        "UPDATE meta SET updated_at = ?1 WHERE id = 1",
                        [crate::time::now_ms()],
                    )
                    .sql()?;
                    Ok(())
                };
            if adding {
                transaction(&db, finish)?;
            } else {
                finish()?;
                if watch.cancelled() {
                    return Err(cancelled_error());
                }
                schema::finish(&db)?;
            }
            Ok(())
        })();
    if let Err(error) = finished {
        undo(db, target);
        return Err(error);
    }
    drop(db);
    if let Target::New { file, partial, .. } = target {
        std::fs::rename(partial, file).map_err(|e| {
            remove_files(partial);
            CoreError::wrap("Could not save the database", e)
        })?;
    }
    Ok(count)
}

#[allow(clippy::too_many_arguments)]
fn read_games(
    db: &Connection,
    import: i64,
    input: impl std::io::BufRead,
    depth: u32,
    dir: &Path,
    capacity: usize,
    read: &AtomicU64,
    total_bytes: u64,
    watch: &mut dyn Watch,
) -> Result<(Count, TreeBuilder)> {
    let mut tree = TreeBuilder::new(dir, capacity)?;
    let mut players = Names::new("players");
    let mut events = Names::new("events");
    let mut count = Count {
        total_bytes,
        ..Count::default()
    };
    let mut open = false;
    let result = (|| {
        for item in PgnReader::new(input) {
            let item = item.map_err(|e| CoreError::wrap("Could not read that file", e))?;
            let stored = match item {
                PgnItem::Game(game) => gamedb::store(&game).ok(),
                PgnItem::TooLarge => None,
            };
            let Some(stored) = stored else {
                count.skipped += 1;
                continue;
            };
            if !open {
                db.execute_batch("BEGIN").sql()?;
                open = true;
            }
            let id = insert_game(db, import, &stored, &mut players, &mut events)?;
            add_to_tree(&mut tree, id, &stored, depth)?;
            count.games += 1;
            if (count.games + count.skipped).is_multiple_of(1_000) {
                if watch.cancelled() {
                    return Err(cancelled_error());
                }
                count.bytes = read.load(Ordering::Relaxed);
                watch.reading(count);
            }
            if count.games.is_multiple_of(BATCH) {
                db.execute_batch("COMMIT").sql()?;
                open = false;
            }
        }
        if open {
            db.execute_batch("COMMIT").sql()?;
            open = false;
        }
        Ok(())
    })();
    if let Err(error) = result {
        if open {
            db.execute_batch("ROLLBACK").ok();
        }
        return Err(error);
    }
    count.bytes = read.load(Ordering::Relaxed);
    watch.reading(count);
    Ok((count, tree))
}

/// The files of a database: the file itself and SQLite's journal beside it.
pub fn remove_files(file: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = file.as_os_str().to_owned();
        name.push(suffix);
        std::fs::remove_file(PathBuf::from(name)).ok();
    }
}

/// What a failed import leaves undone: a new database's file removed, or an existing one's games
/// of this import deleted (its tree rows were never written).
fn undo(db: Connection, target: &Target) {
    match target {
        Target::New { partial, .. } => {
            drop(db);
            remove_files(partial);
        }
        Target::Existing { .. } => {
            let unfinished: Option<i64> = db
                .query_row(
                    "SELECT id FROM imports WHERE finished_at IS NULL ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .ok()
                .flatten();
            if let Some(import) = unfinished {
                transaction(&db, || {
                    db.execute("DELETE FROM games WHERE import_id = ?1", [import])
                        .sql()?;
                    db.execute("DELETE FROM imports WHERE id = ?1", [import])
                        .sql()?;
                    Ok(())
                })
                .ok();
            }
        }
    }
}
