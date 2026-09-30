import { DEFAULT_SETTINGS } from '../shared/defaultSettings'
import { registerDiagnosticSecret } from './diagnosticLog'
import { app, safeStorage } from 'electron'
import { readFile, rename } from 'node:fs/promises'
import { join } from 'node:path'
import { homedir } from 'node:os'
import { DatabaseSync } from 'node:sqlite'
import { getDb } from './db'
import { PIECE_ANIMATIONS, type AppData, type LichessGame, type Settings } from '../shared/types'
import { assertUsername } from '../shared/validate'

const defaults = DEFAULT_SETTINGS

/** Games kept per account. */
export const MAX_GAMES = 5000

/** `safeStorage` reports "available" on Linux even with its plaintext fallback backend. */
function encryptionAvailable(): boolean {
  if (!safeStorage.isEncryptionAvailable()) return false
  if (process.platform === 'linux' && safeStorage.getSelectedStorageBackend() === 'basic_text')
    return false
  return true
}

/** Every persisted setting, in column order. Adding a setting means adding it here, to `Settings` and to a migration. */
const SETTINGS_KEYS = [
  'appearance',
  'boardTheme',
  'lightTheme',
  'darkTheme',
  'pieceSet',
  'pieceAnimation',
  'coordinates',
  'soundEnabled',
  'soundVolume',
  'enginePath',
  'premove',
  'promotion',
  'showLegalMoves',
  'notificationsEnabled',
  'notifyActive',
  'notifyBackground',
  'notifyOpponentMove',
  'notifyLowTime',
  'notifyGameEvents',
  'notifyComputerMove',
  'notifySound',
] as const satisfies readonly (keyof Settings)[]

const placeholders = (count: number): string => Array(count).fill('?').join(', ')
const SETTINGS_SELECT = `SELECT ${SETTINGS_KEYS.join(', ')} FROM settings WHERE id = 1`
const SETTINGS_UPSERT = `INSERT OR REPLACE INTO settings (id, ${SETTINGS_KEYS.join(', ')}) VALUES (1, ${placeholders(SETTINGS_KEYS.length)})`

/** SQLite has neither booleans nor undefined: store 1/0 and NULL. */
const toSql = (value: unknown): string | number | null =>
  value === undefined || value === null
    ? null
    : typeof value === 'boolean'
      ? +value
      : (value as string | number)

type SettingsRow = Record<(typeof SETTINGS_KEYS)[number], string | number>

interface AccountRow {
  username: string
  connected: number
  lastSyncedAt: number | null
}

interface GameRow {
  account: string
  id: string
  createdAt: number
  lastMoveAt: number
  rated: number
  speed: string
  perf: string
  status: string
  winner: string | null
  color: string
  opponent: string
  opponentRating: number | null
  playerRating: number | null
  ratingDiff: number | null
  opening: string | null
  moves: string
  pgn: string | null
}

function normalizeSettings(stored: Partial<Settings> & { boardPreset?: string }): Settings {
  const { boardPreset, ...rest } = stored
  const settings: Settings = {
    ...defaults,
    ...rest,
    // Older builds stored a preset plus two square colors instead of a board theme.
    boardTheme: rest.boardTheme ?? (boardPreset === 'chess.com' ? 'green' : defaults.boardTheme),
    coordinates:
      rest.coordinates === 'none' || rest.coordinates === 'outside' ? rest.coordinates : 'inside',
    promotion: rest.promotion === 'queen' || rest.promotion === 'premove' ? rest.promotion : 'ask',
    pieceAnimation: PIECE_ANIMATIONS.includes(rest.pieceAnimation as never)
      ? (rest.pieceAnimation as Settings['pieceAnimation'])
      : defaults.pieceAnimation,
  }
  if (settings.soundVolume > 1) settings.soundVolume = Math.min(1, settings.soundVolume / 100)
  return settings
}

function rowToSettings(row: SettingsRow): Settings {
  return normalizeSettings({
    appearance: row.appearance as Settings['appearance'],
    boardTheme: row.boardTheme as string,
    lightTheme: row.lightTheme as string,
    darkTheme: row.darkTheme as string,
    pieceSet: row.pieceSet as string,
    pieceAnimation: row.pieceAnimation as Settings['pieceAnimation'],
    coordinates: row.coordinates as Settings['coordinates'],
    soundEnabled: row.soundEnabled === 1,
    soundVolume: row.soundVolume as number,
    enginePath: row.enginePath as string,
    premove: row.premove === 1,
    promotion: row.promotion as Settings['promotion'],
    showLegalMoves: row.showLegalMoves === 1,
    notificationsEnabled: row.notificationsEnabled === 1,
    notifyActive: row.notifyActive === 1,
    notifyBackground: row.notifyBackground === 1,
    notifyOpponentMove: row.notifyOpponentMove === 1,
    notifyLowTime: row.notifyLowTime === 1,
    notifyGameEvents: row.notifyGameEvents === 1,
    notifyComputerMove: row.notifyComputerMove === 1,
    notifySound: row.notifySound === 1,
  })
}

function readSettings(database: DatabaseSync): Settings | undefined {
  const row = database.prepare(SETTINGS_SELECT).get() as unknown as SettingsRow | undefined
  return row ? rowToSettings(row) : undefined
}

function writeSettings(database: DatabaseSync, settings: Settings): void {
  database.prepare(SETTINGS_UPSERT).run(...SETTINGS_KEYS.map((key) => toSql(settings[key])))
}

function fixTimestamp(value: number | undefined): number | undefined {
  if (value && value < 1e11) return value * 1000
  return value
}

function rowToGame(row: GameRow): LichessGame {
  return {
    id: row.id,
    account: row.account,
    createdAt: row.createdAt,
    lastMoveAt: row.lastMoveAt,
    rated: row.rated === 1,
    speed: row.speed,
    perf: row.perf,
    status: row.status,
    winner: row.winner === 'white' || row.winner === 'black' ? row.winner : undefined,
    color: row.color === 'black' ? 'black' : 'white',
    opponent: row.opponent,
    opponentRating: row.opponentRating ?? undefined,
    playerRating: row.playerRating ?? undefined,
    ratingDiff: row.ratingDiff ?? undefined,
    opening: row.opening ?? undefined,
    moves: row.moves,
    pgn: row.pgn ?? undefined,
  }
}

/** Every persisted game field, in column order. */
const GAME_KEYS = [
  'account',
  'id',
  'createdAt',
  'lastMoveAt',
  'rated',
  'speed',
  'perf',
  'status',
  'winner',
  'color',
  'opponent',
  'opponentRating',
  'playerRating',
  'ratingDiff',
  'opening',
  'moves',
  'pgn',
] as const satisfies readonly (keyof LichessGame)[]

const GAME_COLUMNS = GAME_KEYS.join(', ')
/** The list view never shows the (large, clock-annotated) PGN; it is fetched per game on review. */
const LIST_COLUMNS = GAME_KEYS.filter((key) => key !== 'pgn').join(', ')
const GAME_PLACEHOLDERS = placeholders(GAME_KEYS.length)
const gameParams = (game: LichessGame): (string | number | null)[] =>
  GAME_KEYS.map((key) => toSql(game[key]))

function runInsertGame(database: DatabaseSync, game: LichessGame): void {
  database
    .prepare(`INSERT OR REPLACE INTO games (${GAME_COLUMNS}) VALUES (${GAME_PLACEHOLDERS})`)
    .run(...gameParams(game))
}

function rollback(database: DatabaseSync): void {
  try {
    database.exec('ROLLBACK')
  } catch {
    // Ignore rollback failures; the original error is what matters.
  }
}

let migration: Promise<void> | null = null

/** Concurrent callers share one migration run; a failed run can be retried. */
function ensureMigrated(): Promise<void> {
  migration ??= runMigration().catch((cause) => {
    migration = null
    throw cause
  })
  return migration
}

async function runMigration(): Promise<void> {
  const database = getDb()
  const accountCount = (
    database.prepare('SELECT COUNT(*) AS n FROM accounts').get() as unknown as { n: number }
  ).n
  const gameCount = (
    database.prepare('SELECT COUNT(*) AS n FROM games').get() as unknown as { n: number }
  ).n
  const tokenCount = (
    database.prepare('SELECT COUNT(*) AS n FROM tokens').get() as unknown as { n: number }
  ).n
  const settingsRow = database.prepare('SELECT id FROM settings WHERE id = 1').get() as
    unknown | undefined
  if (accountCount > 0 || gameCount > 0 || tokenCount > 0 || settingsRow) return
  if (await migrateFromJson(database)) return
  await migrateLegacy(database)
}

async function migrateFromJson(database: DatabaseSync): Promise<boolean> {
  const dataPath = join(app.getPath('userData'), 'kchess-data.json')
  const tokenPath = join(app.getPath('userData'), 'lichess-tokens.json')
  interface JsonBackup {
    settings?: Partial<Settings> & { boardPreset?: string }
    accounts?: AppData['accounts']
    games?: AppData['games']
  }
  let raw: JsonBackup | null
  try {
    raw = JSON.parse(await readFile(dataPath, 'utf8')) as JsonBackup
  } catch {
    return false
  }
  if (!raw) return false
  const settings = normalizeSettings(raw.settings ?? {})
  const accounts = raw.accounts ?? []
  const games = [...(raw.games ?? [])].sort((a, b) => b.createdAt - a.createdAt).slice(0, MAX_GAMES)
  let tokens: Record<string, string>
  try {
    tokens = JSON.parse(await readFile(tokenPath, 'utf8')) as Record<string, string>
  } catch {
    tokens = {}
  }
  database.exec('BEGIN IMMEDIATE')
  try {
    writeSettings(database, settings)
    const insertAccount = database.prepare(
      'INSERT OR REPLACE INTO accounts (username, connected, lastSyncedAt) VALUES (?, ?, ?)',
    )
    for (const account of accounts)
      insertAccount.run(
        account.username,
        account.connected ? 1 : 0,
        fixTimestamp(account.lastSyncedAt) ?? null,
      )
    for (const game of games) runInsertGame(database, game)
    const insertToken = database.prepare(
      'INSERT OR REPLACE INTO tokens (username, encrypted) VALUES (?, ?)',
    )
    for (const [username, encrypted] of Object.entries(tokens)) insertToken.run(username, encrypted)
    database.exec('COMMIT')
  } catch {
    rollback(database)
    return false
  }
  try {
    await rename(dataPath, `${dataPath}.bak`)
  } catch {
    // Keep the original file if it cannot be archived.
  }
  try {
    await rename(tokenPath, `${tokenPath}.bak`)
  } catch {
    // Token file may not exist; nothing to archive.
  }
  return true
}

async function migrateLegacy(database: DatabaseSync): Promise<void> {
  if (process.platform !== 'darwin' || process.env.KCHESS_USER_DATA_DIR) return
  const legacyPath = join(homedir(), '.kchess', 'kchess.db')
  let legacy: DatabaseSync
  try {
    legacy = new DatabaseSync(legacyPath, { readOnly: true })
  } catch {
    return
  }
  try {
    let settingRows: { key: string; value: string }[] = []
    let oldAccounts: { username: string; auth_kind: string; last_synced_at: number | null }[] = []
    let oldGames: Record<string, { toString(): string } | number | null>[] = []
    let oldTokens: { username: string; token: string }[] = []
    try {
      settingRows = legacy
        .prepare('SELECT key, value FROM app_settings')
        .all() as unknown as typeof settingRows
    } catch {
      settingRows = []
    }
    try {
      oldAccounts = legacy
        .prepare('SELECT username, auth_kind, last_synced_at FROM lichess_accounts')
        .all() as unknown as typeof oldAccounts
    } catch {
      oldAccounts = []
    }
    try {
      oldGames = legacy
        .prepare(
          'SELECT game_id, account_username, played_at, rated, speed, perf, status, winner, color, opponent_name, opponent_rating, player_rating, rating_diff, opening_name, moves FROM lichess_games ORDER BY played_at DESC LIMIT 5000',
        )
        .all() as unknown as typeof oldGames
    } catch {
      oldGames = []
    }
    try {
      oldTokens = legacy
        .prepare('SELECT username, token FROM lichess_tokens')
        .all() as unknown as typeof oldTokens
    } catch {
      oldTokens = []
    }
    const get = (key: string): string | undefined =>
      settingRows.find((row) => row.key === key)?.value
    const appearance = get('app.appearance_mode')
    const settings = normalizeSettings({
      appearance: appearance === 'light' || appearance === 'dark' ? appearance : 'system',
      boardTheme: get('board.theme_preset') === 'chess.com' ? 'green' : defaults.boardTheme,
      soundEnabled: get('sound.enabled') !== 'false',
      soundVolume: Math.min(1, Number(get('sound.volume') ?? 70) / 100),
      enginePath: get('engine.custom_path') || get('engine.managed_path') || '',
    })
    const games: LichessGame[] = oldGames.map((g) => ({
      id: String(g.game_id),
      account: String(g.account_username),
      createdAt: Number(g.played_at),
      lastMoveAt: Number(g.played_at),
      rated: Boolean(g.rated),
      speed: String(g.speed),
      perf: String(g.perf),
      status: String(g.status),
      winner: g.winner as 'white' | 'black' | undefined,
      color: g.color as 'white' | 'black',
      opponent: String(g.opponent_name),
      opponentRating: g.opponent_rating == null ? undefined : Number(g.opponent_rating),
      playerRating: g.player_rating == null ? undefined : Number(g.player_rating),
      ratingDiff: g.rating_diff == null ? undefined : Number(g.rating_diff),
      opening: g.opening_name == null ? undefined : String(g.opening_name),
      moves: String(g.moves),
    }))
    database.exec('BEGIN IMMEDIATE')
    try {
      writeSettings(database, settings)
      const insertAccount = database.prepare(
        'INSERT OR REPLACE INTO accounts (username, connected, lastSyncedAt) VALUES (?, ?, ?)',
      )
      for (const account of oldAccounts) {
        insertAccount.run(
          account.username,
          account.auth_kind === 'oauth' ? 1 : 0,
          account.last_synced_at ? account.last_synced_at * 1000 : null,
        )
      }
      for (const game of games) runInsertGame(database, game)
      if (encryptionAvailable()) {
        const insertToken = database.prepare(
          'INSERT OR REPLACE INTO tokens (username, encrypted) VALUES (?, ?)',
        )
        for (const row of oldTokens)
          insertToken.run(row.username, safeStorage.encryptString(row.token).toString('base64'))
      }
      database.exec('COMMIT')
    } catch {
      rollback(database)
    }
  } finally {
    try {
      legacy.close()
    } catch {
      // Ignore close errors for the legacy read-only handle.
    }
  }
}

export async function getSettings(): Promise<Settings> {
  await ensureMigrated()
  return readSettings(getDb()) ?? { ...defaults }
}

/**
 * Everything the renderer shows, kept in memory between writes. Mutations
 * clear it, so a read after a write sees fresh rows and all other reads skip
 * SQLite and the row mapping. Callers must treat the result as read-only.
 */
let snapshot: AppData | null = null

function invalidate(): void {
  snapshot = null
}

export async function loadData(): Promise<AppData> {
  if (snapshot) return snapshot
  await ensureMigrated()
  const database = getDb()
  const accountRows = database
    .prepare('SELECT username, connected, lastSyncedAt FROM accounts ORDER BY rowid')
    .all() as unknown as AccountRow[]
  const gameRows = database
    .prepare(`SELECT ${LIST_COLUMNS} FROM games ORDER BY createdAt DESC`)
    .all() as unknown as GameRow[]
  const loaded: AppData = {
    settings: readSettings(database) ?? { ...defaults },
    accounts: accountRows.map((row) => ({
      username: row.username,
      connected: row.connected === 1,
      lastSyncedAt: row.lastSyncedAt ?? undefined,
    })),
    games: gameRows.map(rowToGame),
  }
  snapshot = loaded
  return loaded
}

export async function gamePgn(account: string, id: string): Promise<string | null> {
  await ensureMigrated()
  const row = getDb()
    .prepare('SELECT pgn FROM games WHERE account = ? COLLATE NOCASE AND id = ?')
    .get(account, id) as unknown as { pgn: string | null } | undefined
  return row?.pgn ?? null
}

/** Small JSON documents fetched from Lichess, kept so a cold start can show them at once. */
export async function readApiCache<T>(
  key: string,
): Promise<{ value: T; fetchedAt: number } | null> {
  await ensureMigrated()
  const row = getDb()
    .prepare('SELECT value, fetchedAt FROM api_cache WHERE key = ?')
    .get(key) as unknown as { value: string; fetchedAt: number } | undefined
  if (!row) return null
  try {
    return { value: JSON.parse(row.value) as T, fetchedAt: row.fetchedAt }
  } catch {
    return null
  }
}

export async function writeApiCache(key: string, value: unknown): Promise<void> {
  await ensureMigrated()
  getDb()
    .prepare('INSERT OR REPLACE INTO api_cache (key, value, fetchedAt) VALUES (?, ?, ?)')
    .run(key, JSON.stringify(value), Date.now())
}

export async function deleteApiCache(prefix: string): Promise<void> {
  await ensureMigrated()
  getDb()
    .prepare("DELETE FROM api_cache WHERE key LIKE ? ESCAPE '\\'")
    .run(`${prefix.replace(/[\\%_]/g, '\\$&')}%`)
}

export async function saveSettings(settings: Settings): Promise<Settings> {
  await ensureMigrated()
  writeSettings(getDb(), settings)
  invalidate()
  return settings
}

export async function addAccount(username: string, connected = false): Promise<AppData> {
  const normalized = assertUsername(username)
  await ensureMigrated()
  const database = getDb()
  const existing = database
    .prepare('SELECT username, connected FROM accounts WHERE username = ? COLLATE NOCASE')
    .get(normalized) as unknown as { username: string; connected: number } | undefined
  if (existing) {
    if (connected && existing.connected !== 1) {
      database
        .prepare('UPDATE accounts SET connected = 1 WHERE username = ? COLLATE NOCASE')
        .run(normalized)
    }
  } else {
    database
      .prepare('INSERT INTO accounts (username, connected) VALUES (?, ?)')
      .run(normalized, connected ? 1 : 0)
  }
  // Adding someone on purpose overrides an earlier removal.
  database
    .prepare('DELETE FROM dismissed_friends WHERE username = ? COLLATE NOCASE')
    .run(normalized)
  invalidate()
  return loadData()
}

/** Add many followed players at once, without touching their games (syncing is a separate, per-friend choice). */
export async function addFriends(usernames: string[]): Promise<AppData> {
  await ensureMigrated()
  const database = getDb()
  const exists = database.prepare('SELECT 1 FROM accounts WHERE username = ? COLLATE NOCASE')
  const insert = database.prepare('INSERT INTO accounts (username, connected) VALUES (?, 0)')
  const undismiss = database.prepare(
    'DELETE FROM dismissed_friends WHERE username = ? COLLATE NOCASE',
  )
  database.exec('BEGIN IMMEDIATE')
  try {
    for (const raw of usernames) {
      const name = assertUsername(raw)
      if (!exists.get(name)) insert.run(name)
      undismiss.run(name)
    }
    database.exec('COMMIT')
  } catch (cause) {
    rollback(database)
    throw cause
  }
  invalidate()
  return loadData()
}

/** Lower-cased names of friends the user removed; an import will not suggest them again unless asked. */
export async function dismissedFriends(): Promise<Set<string>> {
  await ensureMigrated()
  const rows = getDb().prepare('SELECT username FROM dismissed_friends').all() as unknown as {
    username: string
  }[]
  return new Set(rows.map((row) => row.username.toLowerCase()))
}

export async function removeAccount(username: string): Promise<AppData> {
  await ensureMigrated()
  const database = getDb()
  database.exec('BEGIN IMMEDIATE')
  try {
    // Only friends are remembered as removed; disconnecting one's own account is not a "no thanks".
    const existing = database
      .prepare('SELECT connected FROM accounts WHERE username = ? COLLATE NOCASE')
      .get(username) as unknown as { connected: number } | undefined
    if (existing && existing.connected === 0)
      database
        .prepare('INSERT OR REPLACE INTO dismissed_friends (username, dismissedAt) VALUES (?, ?)')
        .run(username, Date.now())
    database.prepare('DELETE FROM games WHERE account = ? COLLATE NOCASE').run(username)
    database.prepare('DELETE FROM tokens WHERE username = ? COLLATE NOCASE').run(username)
    database.prepare('DELETE FROM accounts WHERE username = ? COLLATE NOCASE').run(username)
    database.exec('COMMIT')
  } catch (cause) {
    rollback(database)
    throw cause
  }
  invalidate()
  await deleteApiCache(`${username.toLowerCase()}:`)
  return loadData()
}

/**
 * Delete what was downloaded for one account (games and cached profile) but keep the account itself.
 * The sync cursor is reset so the next sync fetches everything again rather than only what is new.
 */
export async function clearAccountData(username: string): Promise<AppData> {
  await ensureMigrated()
  const database = getDb()
  database.exec('BEGIN IMMEDIATE')
  try {
    database.prepare('DELETE FROM games WHERE account = ? COLLATE NOCASE').run(username)
    database
      .prepare('UPDATE accounts SET lastSyncedAt = NULL WHERE username = ? COLLATE NOCASE')
      .run(username)
    database.exec('COMMIT')
  } catch (cause) {
    rollback(database)
    throw cause
  }
  invalidate()
  await deleteApiCache(`${username.toLowerCase()}:`)
  return loadData()
}

const UPSERT_GAME = `INSERT INTO games (${GAME_COLUMNS}) VALUES (${GAME_PLACEHOLDERS}) ON CONFLICT(account, id) DO UPDATE SET ${GAME_KEYS.filter(
  (key) => key !== 'account' && key !== 'id',
)
  .map((key) => `${key} = excluded.${key}`)
  .join(', ')}`

/** Upsert one batch in a single transaction; `syncedAt` also marks the account as synced. */
function upsertGames(username: string, games: LichessGame[], syncedAt?: number): void {
  const database = getDb()
  const upsert = database.prepare(UPSERT_GAME)
  database.exec('BEGIN IMMEDIATE')
  try {
    for (const game of games) upsert.run(...gameParams(game))
    database
      .prepare(
        `DELETE FROM games WHERE account = ? AND rowid NOT IN (SELECT rowid FROM games WHERE account = ? ORDER BY createdAt DESC LIMIT ${MAX_GAMES})`,
      )
      .run(username, username)
    if (syncedAt !== undefined)
      database
        .prepare('UPDATE accounts SET lastSyncedAt = ? WHERE username = ? COLLATE NOCASE')
        .run(syncedAt, username)
    database.exec('COMMIT')
  } catch (cause) {
    rollback(database)
    throw cause
  } finally {
    invalidate()
  }
}

/**
 * Persist a page of a sync in progress without moving `lastSyncedAt`, so an
 * interrupted sync keeps what it fetched and the next one still starts from
 * the old cursor.
 */
export async function saveGamesPage(username: string, games: LichessGame[]): Promise<void> {
  await ensureMigrated()
  upsertGames(username, games)
}

export async function saveGames(
  username: string,
  games: LichessGame[],
  syncedAt = Date.now(),
): Promise<AppData> {
  await ensureMigrated()
  upsertGames(username, games, syncedAt)
  return loadData()
}

export async function saveToken(username: string, token: string): Promise<void> {
  registerDiagnosticSecret(token)
  if (!encryptionAvailable()) throw new Error('OS credential encryption is unavailable.')
  await ensureMigrated()
  const encrypted = safeStorage.encryptString(token).toString('base64')
  getDb()
    .prepare('INSERT OR REPLACE INTO tokens (username, encrypted) VALUES (?, ?)')
    .run(username.trim(), encrypted)
}

export async function getToken(username: string): Promise<string | null> {
  await ensureMigrated()
  const row = getDb()
    .prepare('SELECT encrypted FROM tokens WHERE username = ? COLLATE NOCASE')
    .get(username) as unknown as { encrypted: string } | undefined
  if (!row || !encryptionAvailable()) return null
  try {
    const token = safeStorage.decryptString(Buffer.from(row.encrypted, 'base64'))
    registerDiagnosticSecret(token)
    return token
  } catch {
    return null
  }
}
