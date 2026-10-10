import { scopedState, platform } from './platform'
import { registerDiagnosticSecret } from './diagnosticLog'
import { logDebug } from './logger'
import { nativeCallSync } from './nativeCore'
import type {
  AppData,
  GamePageQuery,
  GamePage,
  GameLibraryOverview,
  LichessRatingHistory,
  LichessGame,
  Settings,
  StoredReview,
} from '../contracts/types'
import { assertGamePageQuery, assertUsername } from '../domain/validate'

/**
 * Settings, accounts, the game library, the Lichess API cache and the account lifecycle. The
 * storage is the Rust core's (`crates/kchess-core/src/store/games.rs`, one `kchess.db`); this module
 * keeps the calls, the credential encryption (the operating system's store) and the one-time
 * migration's token step, which needs that store.
 */

/** Games kept per account. */
export const MAX_GAMES = 5000

const encryptionAvailable = (): boolean => platform().secrets.available()

/** Concurrent callers share one migration run; a failed run can be retried. */
function ensureMigrated(): Promise<void> {
  serviceState.migration ??= runMigration().catch((cause) => {
    serviceState.migration = null
    throw cause
  })
  return serviceState.migration
}

/**
 * The one-time import of an empty profile (`store.games.migrate`). The earlier database's
 * plaintext tokens come back here to be encrypted with the operating system's store, as saved logins are.
 */
async function runMigration(): Promise<void> {
  // Tokens are read only when encryption is available; a NULL token then rolls the import back.
  const { legacyTokens } = nativeCallSync<{
    legacyTokens: { account: string | null; token: string }[]
  }>('store.games.migrate', encryptionAvailable())
  if (!encryptionAvailable()) return
  for (const { account, token } of legacyTokens) {
    try {
      nativeCallSync(
        'store.games.importTokenCiphertext',
        account,
        platform().secrets.encrypt(token),
      )
    } catch (cause) {
      logDebug('store', 'Legacy migration failed:', cause)
    }
  }
}

export async function getSettings(): Promise<Settings> {
  await ensureMigrated()
  return nativeCallSync<Settings>('store.games.getSettings')
}

/** Settings, accounts and the library size. */
export async function loadData(): Promise<AppData> {
  await ensureMigrated()
  return nativeCallSync<AppData>('store.games.loadData')
}

/** Bounded list rows; full PGNs are fetched separately when a game is opened. */
export async function gamePage(input: GamePageQuery): Promise<GamePage> {
  const query = assertGamePageQuery(input)
  await ensureMigrated()
  return nativeCallSync<GamePage>('store.games.gamePage', query)
}

export async function gameLibraryOverview(): Promise<GameLibraryOverview> {
  await ensureMigrated()
  return nativeCallSync<GameLibraryOverview>('store.games.gameLibraryOverview')
}

export async function gameRatingHistory(account: string): Promise<LichessRatingHistory> {
  await ensureMigrated()
  return nativeCallSync<LichessRatingHistory>('store.games.gameRatingHistory', account)
}

export async function pendingGameIds(account: string): Promise<string[]> {
  await ensureMigrated()
  return nativeCallSync<string[]>('store.games.pendingGameIds', account)
}

export async function gamePgn(account: string, id: string): Promise<string | null> {
  await ensureMigrated()
  return nativeCallSync<string | null>('store.games.gamePgn', account, id)
}

/** Small JSON documents fetched from Lichess, kept so a cold start can show them at once. */
export async function readApiCache<T>(
  key: string,
): Promise<{ value: T; fetchedAt: number } | null> {
  await ensureMigrated()
  return nativeCallSync<{ value: T; fetchedAt: number } | null>('store.games.readApiCache', key)
}

export async function writeApiCache(
  key: string,
  value: unknown,
  stillCurrent: () => boolean = () => true,
): Promise<void> {
  await ensureMigrated()
  // Checked after the last await, so a logout that ran meanwhile cannot have its data written back.
  nativeCallSync('store.games.writeApiCache', key, value, stillCurrent())
}

export async function saveSettings(settings: Settings): Promise<Settings> {
  await ensureMigrated()
  return nativeCallSync<Settings>('store.games.saveSettings', settings)
}

export async function addAccount(username: string, connected = false): Promise<AppData> {
  const normalized = assertUsername(username)
  await ensureMigrated()
  return nativeCallSync<AppData>('store.games.addAccount', normalized, connected)
}

/** Add many followed players at once, without touching their games (syncing is a separate, per-friend choice). */
export async function addFriends(usernames: string[]): Promise<AppData> {
  await ensureMigrated()
  return nativeCallSync<AppData>(
    'store.games.addFriends',
    usernames.map((raw) => assertUsername(raw)),
  )
}

/** Lower-cased names of friends the user removed; an import will not suggest them again unless asked. */
export async function dismissedFriends(): Promise<Set<string>> {
  await ensureMigrated()
  return new Set(nativeCallSync<string[]>('store.games.dismissedFriends'))
}

/** Disconnect one owned account, or all owned accounts when explicitly requested. */
export async function logoutAccounts(username?: string): Promise<AppData> {
  await ensureMigrated()
  return nativeCallSync<AppData>('store.games.logoutAccounts', username ?? null)
}

export async function removeAccount(username: string): Promise<AppData> {
  await ensureMigrated()
  return nativeCallSync<AppData>('store.games.removeAccount', username)
}

/**
 * Delete what was downloaded for one account (games and cached profile) but keep the account itself.
 * The sync cursor is reset so the next sync fetches everything again rather than only what is new.
 */
export async function clearAccountData(username: string): Promise<AppData> {
  await ensureMigrated()
  return nativeCallSync<AppData>('store.games.clearAccountData', username)
}

/**
 * Persist a page of a sync in progress without moving `lastSyncedAt`, so an
 * interrupted sync keeps what it fetched and the next one still starts from
 * the old cursor.
 */
export async function saveGamesPage(
  username: string,
  games: LichessGame[],
  reviews: readonly StoredReview[] = [],
  stillCurrent: () => boolean = () => true,
): Promise<void> {
  await ensureMigrated()
  // Checked after the last await: a logout may have run while migrations were awaited.
  nativeCallSync('store.games.saveGamesPage', username, games, reviews, stillCurrent())
}

export async function saveGames(
  username: string,
  games: LichessGame[],
  syncedAt = Date.now(),
  stillCurrent: () => boolean = () => true,
): Promise<AppData> {
  await ensureMigrated()
  return nativeCallSync<AppData>('store.games.saveGames', username, games, syncedAt, stillCurrent())
}

/** Stores a new login's token and connected account together, unless a logout has run since. */
export async function saveLogin(
  username: string,
  token: string,
  stillCurrent: () => boolean,
): Promise<AppData> {
  registerDiagnosticSecret(token)
  if (!encryptionAvailable()) throw new Error('OS credential encryption is unavailable.')
  const name = assertUsername(username.trim())
  await ensureMigrated()
  if (!stillCurrent()) throw new Error('Login was cancelled by logout.')
  const encrypted = platform().secrets.encrypt(token)
  return nativeCallSync<AppData>('store.games.saveTokenCiphertext', name, encrypted)
}

export async function getToken(username: string): Promise<string | null> {
  await ensureMigrated()
  const encrypted = nativeCallSync<string | null>('store.games.tokenCiphertext', username)
  if (encrypted === null || !encryptionAvailable()) return null
  try {
    const token = platform().secrets.decrypt(encrypted)
    registerDiagnosticSecret(token)
    return token
  } catch (cause) {
    logDebug('store', 'Stored login is unavailable:', username, cause)
    return null
  }
}

/** Drop profile-scoped memory only after the service has drained its requests. */
export function resetStore(): void {
  serviceState.migration = null
}

const serviceState = scopedState(() => ({
  migration: null as Promise<void> | null,
}))
