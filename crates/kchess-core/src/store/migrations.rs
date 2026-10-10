//! `crates/kchess-node/js/migrations.ts`: the ordered, append-only schema migrations and `migrate`.
//! The SQL below is the TypeScript list, byte for byte (generated from `MIGRATIONS` and checked by
//! `tests/core/native-store-games.test.ts`); `PRAGMA user_version` counts how many have run.

use rusqlite::Connection;
use serde_json::Value;

use super::StoreContext;
use crate::error::{CoreError, Result};

/// Ordered schema migrations. Append only: an earlier entry is never edited.
pub const MIGRATIONS: &[&str] = &[
    r#"
  CREATE TABLE IF NOT EXISTS settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    appearance TEXT NOT NULL,
    boardTheme TEXT NOT NULL,
    coordinates TEXT NOT NULL,
    soundEnabled INTEGER NOT NULL,
    soundVolume REAL NOT NULL,
    enginePath TEXT NOT NULL
  );
  CREATE TABLE IF NOT EXISTS accounts (
    username TEXT PRIMARY KEY COLLATE NOCASE,
    connected INTEGER NOT NULL,
    lastSyncedAt INTEGER
  );
  CREATE TABLE IF NOT EXISTS games (
    account TEXT NOT NULL COLLATE NOCASE,
    id TEXT NOT NULL,
    createdAt INTEGER NOT NULL,
    lastMoveAt INTEGER NOT NULL,
    rated INTEGER NOT NULL,
    speed TEXT NOT NULL,
    perf TEXT NOT NULL,
    status TEXT NOT NULL,
    winner TEXT,
    color TEXT NOT NULL,
    opponent TEXT NOT NULL,
    opponentRating INTEGER,
    playerRating INTEGER,
    ratingDiff INTEGER,
    opening TEXT,
    moves TEXT NOT NULL,
    pgn TEXT,
    PRIMARY KEY (account, id)
  );
  CREATE INDEX IF NOT EXISTS idx_games_account_created ON games (account, createdAt DESC);
  CREATE INDEX IF NOT EXISTS idx_games_created ON games (createdAt DESC);
  CREATE TABLE IF NOT EXISTS tokens (
    username TEXT PRIMARY KEY COLLATE NOCASE,
    encrypted TEXT NOT NULL
  );
  CREATE TABLE IF NOT EXISTS api_cache (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    fetchedAt INTEGER NOT NULL
  );
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN premove INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN promotion TEXT NOT NULL DEFAULT 'ask';
  ALTER TABLE settings ADD COLUMN showLegalMoves INTEGER NOT NULL DEFAULT 1;
  "#,
    r#"
  CREATE TABLE IF NOT EXISTS usage (
    account TEXT NOT NULL COLLATE NOCASE,
    kind TEXT NOT NULL,
    requests INTEGER NOT NULL DEFAULT 0,
    bytesIn INTEGER NOT NULL DEFAULT 0,
    since INTEGER NOT NULL,
    PRIMARY KEY (account, kind)
  );
  "#,
    r#"
  CREATE TABLE IF NOT EXISTS dismissed_friends (
    username TEXT PRIMARY KEY COLLATE NOCASE,
    dismissedAt INTEGER NOT NULL
  );
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN pieceSet TEXT NOT NULL DEFAULT 'cburnett';
  ALTER TABLE settings ADD COLUMN pieceAnimation TEXT NOT NULL DEFAULT 'normal';
  ALTER TABLE settings ADD COLUMN notificationsEnabled INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifyWhen TEXT NOT NULL DEFAULT 'away';
  ALTER TABLE settings ADD COLUMN notifyOpponentMove INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifyLowTime INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifyGameEvents INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifyComputerMove INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifySound INTEGER NOT NULL DEFAULT 0;
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN notifyActive INTEGER NOT NULL DEFAULT 0;
  ALTER TABLE settings ADD COLUMN notifyBackground INTEGER NOT NULL DEFAULT 1;
  UPDATE settings SET notifyActive = (notifyWhen = 'always');
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN colorTheme TEXT NOT NULL DEFAULT 'kchess';
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN lightTheme TEXT NOT NULL DEFAULT 'kchess';
  ALTER TABLE settings ADD COLUMN darkTheme TEXT NOT NULL DEFAULT 'kchess';
  UPDATE settings SET lightTheme = colorTheme, darkTheme = colorTheme;
  "#,
    r#"
  CREATE TABLE IF NOT EXISTS puzzles (
    id TEXT PRIMARY KEY,
    fen TEXT NOT NULL,
    moves TEXT NOT NULL,
    rating INTEGER NOT NULL,
    plays INTEGER NOT NULL DEFAULT 0,
    themes TEXT NOT NULL
  );
  CREATE INDEX IF NOT EXISTS idx_puzzles_rating ON puzzles (rating);
  CREATE TABLE IF NOT EXISTS puzzle_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    importedAt INTEGER NOT NULL,
    count INTEGER NOT NULL
  );
  CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    variant TEXT NOT NULL DEFAULT '',
    score REAL NOT NULL,
    detail TEXT NOT NULL DEFAULT '{}',
    playedAt INTEGER NOT NULL
  );
  CREATE INDEX IF NOT EXISTS idx_runs_kind ON runs (kind, variant, playedAt DESC);
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN voicePushToTalk INTEGER NOT NULL DEFAULT 0;
  ALTER TABLE settings ADD COLUMN voiceConfirmMoves INTEGER NOT NULL DEFAULT 1;
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN voiceHistory INTEGER NOT NULL DEFAULT 1;
  CREATE TABLE IF NOT EXISTS voice_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    source TEXT NOT NULL,
    heard TEXT NOT NULL,
    confidence REAL NOT NULL,
    words TEXT NOT NULL DEFAULT '[]',
    outcome TEXT NOT NULL,
    parsed TEXT,
    expected TEXT,
    fen TEXT,
    retryOf INTEGER
  );
  CREATE INDEX IF NOT EXISTS idx_voice_log_at ON voice_log (at DESC);
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN updateAutoCheck INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN updateAutoDownload INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN updateInstallOnQuit INTEGER NOT NULL DEFAULT 1;
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN engineLevels TEXT NOT NULL DEFAULT 'beginner,club,expert,fm,im,gm,max';
  "#,
    r#"
  ALTER TABLE settings ADD COLUMN reviewAuto TEXT NOT NULL DEFAULT 'recent';
  ALTER TABLE settings ADD COLUMN reviewOnBattery INTEGER NOT NULL DEFAULT 0;
  CREATE TABLE IF NOT EXISTS reviews (
    key TEXT PRIMARY KEY,
    gameId TEXT,
    source TEXT NOT NULL,
    complete INTEGER NOT NULL,
    depth INTEGER NOT NULL,
    data TEXT NOT NULL,
    summary TEXT NOT NULL,
    updatedAt INTEGER NOT NULL
  );
  CREATE INDEX IF NOT EXISTS idx_reviews_game ON reviews (gameId);
  CREATE TABLE IF NOT EXISTS lichess_review_checks (
    id TEXT PRIMARY KEY,
    checkedAt INTEGER NOT NULL
  );
  "#,
    r#"
  ALTER TABLE puzzle_meta ADD COLUMN bytes INTEGER NOT NULL DEFAULT 0;
  UPDATE puzzle_meta SET bytes = COALESCE((SELECT SUM(LENGTH(id) + LENGTH(fen) + LENGTH(moves) + LENGTH(themes) + 24) FROM puzzles), 0);
  "#,
    r#"CREATE TABLE IF NOT EXISTS position_lookups (
    key TEXT PRIMARY KEY, data TEXT NOT NULL, fetchedAt INTEGER NOT NULL
  );"#,
    r#"CREATE TABLE pending_game_sync (
    account TEXT NOT NULL COLLATE NOCASE REFERENCES accounts(username) ON DELETE CASCADE,
    id TEXT NOT NULL,
    PRIMARY KEY (account, id)
  );
  INSERT INTO pending_game_sync (account, id)
    SELECT account, id FROM games WHERE status IN ('created', 'started')
      AND account IN (SELECT username FROM accounts);
  DELETE FROM games WHERE status IN ('created', 'started');
  UPDATE accounts SET lastSyncedAt = NULL;
  CREATE INDEX idx_games_page ON games (createdAt DESC, account, id);
  CREATE INDEX idx_games_account_page ON games (account COLLATE NOCASE, createdAt DESC, id);"#,
    r#"CREATE INDEX idx_games_result_page ON games (
    (CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END),
    createdAt DESC, account, id);
  CREATE INDEX idx_games_account_result_page ON games (account COLLATE NOCASE,
    (CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END),
    createdAt DESC, id);"#,
    r#"ALTER TABLE settings ADD COLUMN receiveChallenges INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifyChallenges INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN onlineChat INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN correspondencePoll INTEGER NOT NULL DEFAULT 5;
  ALTER TABLE settings ADD COLUMN zenMode INTEGER NOT NULL DEFAULT 0;
  ALTER TABLE settings ADD COLUMN blindfold INTEGER NOT NULL DEFAULT 0;
  ALTER TABLE settings ADD COLUMN cloudEval INTEGER NOT NULL DEFAULT 0;
  ALTER TABLE settings ADD COLUMN showOpeningName INTEGER NOT NULL DEFAULT 1;"#,
    r#"CREATE TABLE game_reviews (
    gameId TEXT NOT NULL,
    reviewKey TEXT NOT NULL REFERENCES reviews(key) ON DELETE CASCADE,
    PRIMARY KEY (gameId, reviewKey)
  );
  CREATE INDEX idx_game_reviews_key ON game_reviews(reviewKey);
  INSERT INTO game_reviews SELECT gameId, key FROM reviews WHERE gameId IS NOT NULL;"#,
    r#"ALTER TABLE settings ADD COLUMN swipeNavigation INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN swipeIndicator INTEGER NOT NULL DEFAULT 1;"#,
    r#"CREATE TABLE documents (
    key TEXT PRIMARY KEY,
    body TEXT NOT NULL,
    updatedAt INTEGER NOT NULL
  );
  CREATE TABLE archived_games (
    id TEXT PRIMARY KEY,
    body TEXT NOT NULL,
    seq INTEGER NOT NULL
  );
  CREATE INDEX idx_archived_games_seq ON archived_games(seq);"#,
];

fn storage_error(cause: rusqlite::Error) -> CoreError {
    CoreError::new(cause.to_string())
}

/// Migrations expose no storage methods; None for every method.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let db = ctx.db;
    let _ = (db, method, args);
    None
}

/// Apply every migration newer than the database's `user_version`, each in its own transaction.
pub fn migrate(db: &Connection) -> Result<()> {
    migrate_to(db, MIGRATIONS.len())
}

/// Apply the migrations up to `steps` (the first `steps` of the list), as an earlier release
/// would have left the database. Test-only callers pass fewer than all of them.
pub fn migrate_to(db: &Connection, steps: usize) -> Result<()> {
    let current: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(storage_error)?;
    let start = usize::try_from(current).unwrap_or(0);
    for (index, sql) in MIGRATIONS.iter().enumerate().take(steps).skip(start) {
        db.execute_batch("BEGIN IMMEDIATE").map_err(storage_error)?;
        // `index` is a loop counter, not user input: PRAGMA takes no bound parameters.
        let applied = db
            .execute_batch(sql)
            .and_then(|()| db.execute_batch(&format!("PRAGMA user_version = {}", index + 1)))
            .and_then(|()| db.execute_batch("COMMIT"));
        if let Err(cause) = applied {
            let _ = db.execute_batch("ROLLBACK");
            return Err(storage_error(cause));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(db: &Connection) -> i64 {
        db.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_fresh_database_reaches_the_latest_version_and_migrating_again_changes_nothing() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        assert_eq!(version(&db), MIGRATIONS.len() as i64);
        let schema: String = db
            .query_row(
                "SELECT group_concat(sql, ';') FROM sqlite_master ORDER BY name",
                [],
                |row| row.get(0),
            )
            .unwrap();
        migrate(&db).unwrap();
        let again: String = db
            .query_row(
                "SELECT group_concat(sql, ';') FROM sqlite_master ORDER BY name",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(schema, again);
    }

    /// `migrations.test.ts`: the one-time backfill keeps completed games and resets cursors once.
    #[test]
    fn appends_a_one_time_backfill_migration_without_losing_completed_games() {
        let db = Connection::open_in_memory().unwrap();
        let backfill = MIGRATIONS
            .iter()
            .position(|sql| sql.contains("CREATE TABLE pending_game_sync"))
            .unwrap();
        for sql in &MIGRATIONS[..backfill] {
            db.execute_batch(sql).unwrap();
        }
        db.execute_batch(&format!("PRAGMA user_version = {backfill}"))
            .unwrap();
        db.execute_batch(
            "INSERT INTO accounts (username, connected, lastSyncedAt) VALUES ('Alice', 1, 1000);",
        )
        .unwrap();
        for (id, status) in [("Pending1", "started"), ("Finished", "mate")] {
            db.execute(
                "INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status, color, opponent, moves) VALUES ('Alice', ?1, 1, 1, 1, 'blitz', 'blitz', ?2, 'white', 'Bob', 'e4')",
                [id, status],
            )
            .unwrap();
        }
        migrate(&db).unwrap();
        let synced: Option<i64> = db
            .query_row("SELECT lastSyncedAt FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(synced, None);
        let pending: String = db
            .query_row("SELECT id FROM pending_game_sync", [], |row| row.get(0))
            .unwrap();
        assert_eq!(pending, "Pending1");
        let finished: String = db
            .query_row("SELECT id FROM games", [], |row| row.get(0))
            .unwrap();
        assert_eq!(finished, "Finished");
        db.execute_batch("UPDATE accounts SET lastSyncedAt = 2000")
            .unwrap();
        migrate(&db).unwrap();
        let synced: Option<i64> = db
            .query_row("SELECT lastSyncedAt FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(synced, Some(2000));
    }
}
