import { nativeCallSync } from '../../core/src/services/nativeCore'
import { platform } from '../../core/src/services/platform'
import { registerDiagnosticSecret } from '../../core/src/services/diagnosticLog'
import { logDebug } from '../../core/src/services/logger'
import type {
  AppData,
  GamePageQuery,
  GamePage,
  GameLibraryOverview,
  LichessGame,
  LichessRatingHistory,
  Settings,
  StoredReview,
} from '../../core/src/contracts/types'
import { assertGamePageQuery, assertUsername } from '../../core/src/domain/validate'

/**
 * Test-only calls of the Rust core's storage methods (`store.games.*`), on the current native
 * scope. They keep what the TypeScript storage wrappers did before the facade moved to Rust: the
 * one-time import of an earlier profile (its plaintext tokens encrypted with the platform's
 * secrets), the argument checks, and the credential encryption of tokens. The storage itself is
 * the native store's.
 */

/** Games kept per account. */
export const MAX_GAMES = 5000

const encryptionAvailable = (): boolean => platform().secrets.available()

/** The one-time import of an earlier profile, run before each store call as it was before. */
function migrate(): void {
  const available = encryptionAvailable()
  const { legacyTokens } = nativeCallSync<{
    legacyTokens: { account: string | null; token: string }[]
  }>('store.games.migrate', available)
  if (!available) return
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

/** A store call, after the migration. */
function storeCall<T>(method: string, ...args: unknown[]): T {
  migrate()
  return nativeCallSync<T>(method, ...args)
}

export const getSettings = async (): Promise<Settings> =>
  storeCall<Settings>('store.games.getSettings')

export const loadData = async (): Promise<AppData> => storeCall<AppData>('store.games.loadData')

export const gamePage = async (input: GamePageQuery): Promise<GamePage> =>
  storeCall<GamePage>('store.games.gamePage', assertGamePageQuery(input))

export const gameLibraryOverview = async (): Promise<GameLibraryOverview> =>
  storeCall<GameLibraryOverview>('store.games.gameLibraryOverview')

export const gameRatingHistory = async (account: string): Promise<LichessRatingHistory> =>
  storeCall<LichessRatingHistory>('store.games.gameRatingHistory', account)

export const pendingGameIds = async (account: string): Promise<string[]> =>
  storeCall<string[]>('store.games.pendingGameIds', account)

export const gamePgn = async (account: string, id: string): Promise<string | null> =>
  storeCall<string | null>('store.games.gamePgn', account, id)

export async function readApiCache<T>(
  key: string,
): Promise<{ value: T; fetchedAt: number } | null> {
  return storeCall<{ value: T; fetchedAt: number } | null>('store.games.readApiCache', key)
}

export async function writeApiCache(
  key: string,
  value: unknown,
  stillCurrent: () => boolean = () => true,
): Promise<void> {
  storeCall('store.games.writeApiCache', key, value, stillCurrent())
}

export const saveSettings = async (settings: Settings): Promise<Settings> =>
  storeCall<Settings>('store.games.saveSettings', settings)

export async function addAccount(username: string, connected = false): Promise<AppData> {
  return storeCall<AppData>('store.games.addAccount', assertUsername(username), connected)
}

export async function addFriends(usernames: string[]): Promise<AppData> {
  return storeCall<AppData>(
    'store.games.addFriends',
    usernames.map((raw) => assertUsername(raw)),
  )
}

export async function dismissedFriends(): Promise<Set<string>> {
  return new Set(storeCall<string[]>('store.games.dismissedFriends'))
}

export async function logoutAccounts(username?: string): Promise<AppData> {
  return storeCall<AppData>('store.games.logoutAccounts', username ?? null)
}

export async function removeAccount(username: string): Promise<AppData> {
  return storeCall<AppData>('store.games.removeAccount', username)
}

export async function clearAccountData(username: string): Promise<AppData> {
  return storeCall<AppData>('store.games.clearAccountData', username)
}

export async function saveGamesPage(
  username: string,
  games: LichessGame[],
  reviews: readonly StoredReview[] = [],
  stillCurrent: () => boolean = () => true,
): Promise<void> {
  storeCall('store.games.saveGamesPage', username, games, reviews, stillCurrent())
}

export async function saveGames(
  username: string,
  games: LichessGame[],
  syncedAt = Date.now(),
  stillCurrent: () => boolean = () => true,
): Promise<AppData> {
  return storeCall<AppData>('store.games.saveGames', username, games, syncedAt, stillCurrent())
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
  migrate()
  if (!stillCurrent()) throw new Error('Login was cancelled by logout.')
  const encrypted = platform().secrets.encrypt(token)
  return nativeCallSync<AppData>('store.games.saveTokenCiphertext', name, encrypted)
}

/** The stored token of an account, decrypted; null when none is stored or it cannot be read. */
export async function getToken(username: string): Promise<string | null> {
  const encrypted = storeCall<string | null>('store.games.tokenCiphertext', username)
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

/** Whether the review store has games for an account. */
export const hasAccount = (username: string): boolean =>
  nativeCallSync<boolean>('store.reviewStore.hasAccount', username)

/** Nothing to drop: the native store keeps no state of its own between calls. */
export function resetStore(): void {}
