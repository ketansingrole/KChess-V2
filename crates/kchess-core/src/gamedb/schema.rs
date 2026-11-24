//! One database's file: its schema, kept by append-only migrations as `kchess.db`'s are
//! (`store/migrations.rs`), and how it is opened to read or to write.
//!
//! Games name their players and event by id (`players`, `events`), so a name search reads the
//! few hundred thousand names rather than every game, and each name is stored once. A game's
//! moves are `kchess_domain::gamedb`'s one byte a move; its text is kept only when the columns
//! cannot write it again. `tree` is the opening tree: one row a position and the move played from
//! it, up to `meta.index_depth` plies, with the best-rated and the latest game through it.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension as _};

use crate::store::sql::{Sql as _, transaction};
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{DatabaseInfo, DatabaseSource};

/// A database file's name: its id and this extension.
pub const EXTENSION: &str = "kdb";

/// The plies from the start the opening tree covers by default.
pub const INDEX_DEPTH: u32 = 40;

const MIGRATIONS: &[&str] = &[r#"CREATE TABLE meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    name TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    index_depth INTEGER NOT NULL,
    attribution TEXT
  );
  CREATE TABLE imports (
    id INTEGER PRIMARY KEY,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    games INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    file TEXT
  );
  CREATE TABLE players (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    norm TEXT NOT NULL
  );
  CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    norm TEXT NOT NULL
  );
  CREATE TABLE games (
    id INTEGER PRIMARY KEY,
    import_id INTEGER NOT NULL,
    white INTEGER NOT NULL,
    black INTEGER NOT NULL,
    event INTEGER,
    site TEXT,
    date TEXT,
    date_key INTEGER NOT NULL,
    year INTEGER,
    round TEXT,
    white_elo INTEGER,
    black_elo INTEGER,
    rating INTEGER,
    low_elo INTEGER,
    result INTEGER,
    eco TEXT,
    opening TEXT,
    fen TEXT,
    plies INTEGER NOT NULL,
    moves BLOB NOT NULL,
    extra BLOB
  );
  CREATE TABLE tree (
    key INTEGER NOT NULL,
    mv INTEGER NOT NULL,
    white INTEGER NOT NULL,
    draws INTEGER NOT NULL,
    black INTEGER NOT NULL,
    elo_sum INTEGER NOT NULL,
    elo_n INTEGER NOT NULL,
    best_game INTEGER NOT NULL,
    best_elo INTEGER NOT NULL,
    last_game INTEGER NOT NULL,
    last_date INTEGER NOT NULL,
    PRIMARY KEY (key, mv)
  ) WITHOUT ROWID;"#];

/// The games' indexes, built after a new database's games are in: building them once is much
/// faster than keeping them up to date row by row.
pub const INDEXES: &str = "
  CREATE INDEX IF NOT EXISTS games_white ON games (white, date_key);
  CREATE INDEX IF NOT EXISTS games_black ON games (black, date_key);
  CREATE INDEX IF NOT EXISTS games_event ON games (event, date_key);
  CREATE INDEX IF NOT EXISTS games_date ON games (date_key);
  CREATE INDEX IF NOT EXISTS games_rating ON games (rating);
  CREATE INDEX IF NOT EXISTS games_low_elo ON games (low_elo);
  CREATE INDEX IF NOT EXISTS games_eco ON games (eco);
  CREATE INDEX IF NOT EXISTS games_import ON games (import_id);";

/// Apply every migration newer than the file's `user_version`, each in its own transaction. A
/// file a newer release wrote is refused rather than changed.
pub fn migrate(db: &Connection) -> Result<()> {
    let current: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .sql()?;
    let start = usize::try_from(current).unwrap_or(0);
    if start > MIGRATIONS.len() {
        return Err(CoreError::new(
            "This database was written by a newer KChess. Update KChess to open it.",
        ));
    }
    for (index, step) in MIGRATIONS.iter().enumerate().skip(start) {
        transaction(db, || {
            db.execute_batch(step).sql()?;
            db.execute_batch(&format!("PRAGMA user_version = {}", index + 1))
                .sql()
        })?;
    }
    Ok(())
}

fn opened(path: &Path, flags: OpenFlags) -> Result<Connection> {
    let db = Connection::open_with_flags(path, flags)
        .map_err(|e| CoreError::wrap("Could not open the game database", e))?;
    db.execute_batch("PRAGMA busy_timeout = 5000;").sql()?;
    Ok(db)
}

/// The file to search: read-only, so a search can never change it.
pub fn open_read(path: &Path) -> Result<Connection> {
    let db = opened(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .sql()?;
    if usize::try_from(version).unwrap_or(usize::MAX) > MIGRATIONS.len() {
        return Err(CoreError::new(
            "This database was written by a newer KChess. Update KChess to open it.",
        ));
    }
    Ok(db)
}

/// An existing file to add games to, migrated first; WAL, so searches go on meanwhile.
pub fn open_write(path: &Path) -> Result<Connection> {
    let db = opened(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
        .sql()?;
    migrate(&db)?;
    Ok(db)
}

/// A new file, written alone until it is renamed into place: nothing reads it, and a failure
/// throws it away, so it needs no journal.
pub fn create(path: &Path, name: &str, source: DatabaseSource, now: i64) -> Result<Connection> {
    let db = opened(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.execute_batch(
        "PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF; PRAGMA page_size = 8192;",
    )
    .sql()?;
    migrate(&db)?;
    db.execute(
        "INSERT INTO meta (id, name, source, created_at, updated_at, index_depth, attribution)
         VALUES (1, ?1, ?2, ?3, ?3, ?4, NULL)",
        rusqlite::params![name, source_text(source), now, INDEX_DEPTH],
    )
    .sql()?;
    Ok(db)
}

/// A finished new file made ready to be read: indexed, its journal WAL as every reader expects.
pub fn finish(db: &Connection) -> Result<()> {
    db.execute_batch(INDEXES).sql()?;
    db.execute_batch("ANALYZE; PRAGMA journal_mode = WAL;")
        .sql()
}

pub fn source_text(source: DatabaseSource) -> &'static str {
    match source {
        DatabaseSource::Imported => "imported",
        DatabaseSource::Own => "own",
        DatabaseSource::Master => "master",
    }
}

fn source_of(text: &str) -> DatabaseSource {
    match text {
        "own" => DatabaseSource::Own,
        "master" => DatabaseSource::Master,
        _ => DatabaseSource::Imported,
    }
}

/// The database `id` as `gameDbStatus` lists it; None when the file holds no database.
pub fn info(db: &Connection, id: &str, bytes: u64) -> Result<Option<DatabaseInfo>> {
    let meta = db
        .query_row(
            "SELECT name, source, updated_at, index_depth, attribution FROM meta WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .sql()?;
    let Some((name, source, updated_at, index_depth, attribution)) = meta else {
        return Ok(None);
    };
    // Each finished import records how many games it kept; counting the games table would read
    // millions of rows.
    let games: i64 = db
        .query_row(
            "SELECT COALESCE(SUM(games), 0) FROM imports WHERE finished_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .sql()?;
    let games = u64::try_from(games).unwrap_or(0);
    Ok(Some(DatabaseInfo {
        id: id.to_string(),
        name,
        source: source_of(&source),
        games,
        bytes,
        updated_at,
        index_depth,
        attribution,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_is_migrated_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.kdb");
        let db = create(&path, "Mine", DatabaseSource::Imported, 5).unwrap();
        finish(&db).unwrap();
        drop(db);
        let db = open_read(&path).unwrap();
        let info = info(&db, "a", 10).unwrap().unwrap();
        assert_eq!(
            (info.name.as_str(), info.games, info.index_depth),
            ("Mine", 0, INDEX_DEPTH)
        );
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
    }

    #[test]
    fn a_file_from_a_newer_release_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.kdb");
        create(&path, "Mine", DatabaseSource::Imported, 5).unwrap();
        Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA user_version = 99")
            .unwrap();
        assert!(
            open_read(&path)
                .unwrap_err()
                .message
                .contains("newer KChess")
        );
        assert!(
            open_write(&path)
                .unwrap_err()
                .message
                .contains("newer KChess")
        );
    }
}
