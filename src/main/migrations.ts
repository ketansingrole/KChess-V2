import type { DatabaseSync } from 'node:sqlite'

/**
 * Ordered schema migrations. `PRAGMA user_version` records how many have run,
 * so a change to the schema is a new entry at the end — never an edit to an
 * earlier one. The first entry is idempotent: databases created before
 * versioning existed already have these tables and start at version 0.
 */
export const MIGRATIONS: readonly string[] = [
  `
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
  `,
  `
  ALTER TABLE settings ADD COLUMN premove INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN promotion TEXT NOT NULL DEFAULT 'ask';
  ALTER TABLE settings ADD COLUMN showLegalMoves INTEGER NOT NULL DEFAULT 1;
  `,
  `
  CREATE TABLE IF NOT EXISTS usage (
    account TEXT NOT NULL COLLATE NOCASE,
    kind TEXT NOT NULL,
    requests INTEGER NOT NULL DEFAULT 0,
    bytesIn INTEGER NOT NULL DEFAULT 0,
    since INTEGER NOT NULL,
    PRIMARY KEY (account, kind)
  );
  `,
  `
  CREATE TABLE IF NOT EXISTS dismissed_friends (
    username TEXT PRIMARY KEY COLLATE NOCASE,
    dismissedAt INTEGER NOT NULL
  );
  `,
  `
  ALTER TABLE settings ADD COLUMN pieceSet TEXT NOT NULL DEFAULT 'cburnett';
  ALTER TABLE settings ADD COLUMN pieceAnimation TEXT NOT NULL DEFAULT 'normal';
  ALTER TABLE settings ADD COLUMN notificationsEnabled INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifyWhen TEXT NOT NULL DEFAULT 'away';
  ALTER TABLE settings ADD COLUMN notifyOpponentMove INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifyLowTime INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifyGameEvents INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifyComputerMove INTEGER NOT NULL DEFAULT 1;
  ALTER TABLE settings ADD COLUMN notifySound INTEGER NOT NULL DEFAULT 0;
  `,
  // "Only in the background / always" became two independent switches. `notifyWhen` stays in old
  // databases, unused.
  `
  ALTER TABLE settings ADD COLUMN notifyActive INTEGER NOT NULL DEFAULT 0;
  ALTER TABLE settings ADD COLUMN notifyBackground INTEGER NOT NULL DEFAULT 1;
  UPDATE settings SET notifyActive = (notifyWhen = 'always');
  `,
  `
  ALTER TABLE settings ADD COLUMN colorTheme TEXT NOT NULL DEFAULT 'kchess';
  `,
  // One theme per appearance: light and dark are chosen separately. `colorTheme` stays in old
  // databases, unused; its value seeds both.
  `
  ALTER TABLE settings ADD COLUMN lightTheme TEXT NOT NULL DEFAULT 'kchess';
  ALTER TABLE settings ADD COLUMN darkTheme TEXT NOT NULL DEFAULT 'kchess';
  UPDATE settings SET lightTheme = colorTheme, darkTheme = colorTheme;
  `,
  // Local puzzle database (a sample of Lichess's public one) and local scores for Storm, Streak,
  // Rush and the practice drills. Neither is ever sent anywhere.
  `
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
  `,
]

/** Apply every migration newer than the database's `user_version`, each in its own transaction. */
export function migrate(database: DatabaseSync): void {
  const { user_version: current } = database.prepare('PRAGMA user_version').get() as unknown as {
    user_version: number
  }
  for (let version = current; version < MIGRATIONS.length; version++) {
    database.exec('BEGIN IMMEDIATE')
    try {
      database.exec(MIGRATIONS[version]!)
      // PRAGMA does not accept bound parameters; `version` is a loop counter, not user input.
      database.exec(`PRAGMA user_version = ${version + 1}`)
      database.exec('COMMIT')
    } catch (cause) {
      database.exec('ROLLBACK')
      throw cause
    }
  }
}
