import { scopedState, coreSignal, platform } from './platform'
import { DEFAULT_OAUTH_LOOK } from '../domain/oauthLook'
import { oauthPage } from './oauthPage'
import { errorSummary, logDebug, logInfo, logWarn } from './logger'
import { lichessGameLine, replayPositions, sanLineToUci } from './rules'
import { createHash, randomBytes } from 'node:crypto'
import { createServer } from 'node:http'
import { setTimeout as sleep } from 'node:timers/promises'
import { LRUCache } from 'lru-cache'
import createClient from 'openapi-fetch'
import { INITIAL_FEN } from 'chessops/fen'
import type { components, paths } from '@lichess-org/types'
import type {
  AppData,
  ChallengeInfo,
  ChatLine,
  ChatRoom,
  DeclineReason,
  OngoingGame,
  LichessAccount,
  LichessGame,
  LichessRatingHistory,
  LichessUser,
  NeedsReconnect,
  OnlineAction,
  OnlineEvent,
  OnlineConnection,
  FollowedUser,
  FollowingProblem,
  FollowingReport,
  OnlineOptions,
  Crosstable,
  OAuthPageLook,
  PerfStats,
  PerfType,
  PresenceReport,
  Puzzle,
  PuzzleActivityEntry,
  PuzzleDashboard,
  PuzzleDraw,
  PuzzleRequest,
  PuzzleSolveRequest,
  PuzzleSolveResult,
  StormDashboard,
  UsageKind,
  Judgment,
  ReviewEval,
  StoredReview,
} from '../contracts/types'
import { puzzleFromApi, type ApiPuzzle } from '../domain/puzzle'
import { reviewKey } from '../domain/review'
import { markChecked, writeReview } from './reviewStore'
import { LichessError, throwLichessErrors } from '../domain/lichessError'
import { isGameInProgress } from '../domain/gameStatus'
import { lichessFetch } from './requestPolicy'
import { validateOnlineEvent } from '../domain/onlineEvent'
import { readLines } from './ndjson'
import { RequestScope } from '../domain/requestScope'
import { attributeTo, withUsage } from './usage'
import { canBoardSeek, canDirectChallenge, perfFor } from '../domain/timeControl'
import { GAME_ID } from '../domain/patterns'
import {
  MAX_GAMES,
  dismissedFriends,
  getToken,
  loadData,
  pendingGameIds,
  readApiCache,
  saveGames,
  saveGamesPage,
  saveLogin,
  writeApiCache,
} from './store'

const BASE = 'https://lichess.org'
export const client = createClient<paths>({ baseUrl: BASE, fetch: lichessFetch })
client.use(throwLichessErrors)
const OAUTH_CLIENT_ID = 'kchess-desktop'
/**
 * Permissions asked for at login. Studies, tournaments, team Swiss events and messages need
 * their own; accounts connected earlier are asked to reconnect the first time one is used.
 * Lichess rejects the whole request if any scope is reserved for its own apps (`web:*`).
 */
export const OAUTH_SCOPES = [
  'board:play',
  'challenge:read',
  'challenge:write',
  'follow:read',
  'msg:write',
  'puzzle:read',
  'puzzle:write',
  'study:read',
  'study:write',
  'tournament:write',
  'team:read',
] as const

export type StreamCall = { data?: ReadableStream<Uint8Array> | null; response: Response }

/** Failed responses already threw in the middleware; this guards against an empty success body. */
export async function unwrap<T>(
  call: Promise<{ data?: T | null; response: Response }>,
): Promise<T> {
  const { data, response } = await call
  if (data === undefined || data === null)
    throw new LichessError(response.status, new URL(response.url).pathname, 'empty response')
  return data
}

const asError = (cause: unknown): Error =>
  cause instanceof Error ? cause : new Error(String(cause))

export const authorize = (token: string): Record<string, string> => ({
  Authorization: `Bearer ${token}`,
})

export const urlencoded = (body: unknown): URLSearchParams => {
  const params = new URLSearchParams()
  if (body)
    for (const [key, value] of Object.entries(body as Record<string, unknown>))
      if (value !== undefined) params.set(key, String(value))
  return params
}

/** Profile data changes slowly; a few minutes of reuse saves a round trip per page visit. */

const accountEpoch = (name: string): number =>
  serviceState.accountEpochs.get(name.toLowerCase()) ?? 0
export function invalidateLogin(accounts: string[]): void {
  serviceState.loginEpoch++
  // Who the signed-in accounts follow is theirs; it must not outlive the login.
  serviceState.lastFollowing.clear()
  cancelAccountSyncs(accounts)
}
/** Stops running syncs for these accounts before they save anything else, without ending a login. */
export function cancelAccountSyncs(accounts: string[]): void {
  for (const name of accounts) {
    serviceState.accountEpochs.set(name.toLowerCase(), accountEpoch(name) + 1)
    forgetProfile(name)
    serviceState.syncs.delete(name.toLowerCase())
  }
}

const PROFILE_TTL_MS = 5 * 60_000

const profileKey = (username: string, kind: 'profile' | 'rating'): string =>
  `${username.toLowerCase()}:${kind}`

/** TTL memory cache; `fetch` also makes concurrent callers for a key share one request. */

/**
 * Memory first, then the network. The result is also persisted, and if
 * Lichess is unreachable the last persisted copy is served instead of an error.
 */
async function cachedFetch<T extends object>(key: string, load: () => Promise<T>): Promise<T> {
  try {
    return (await serviceState.profileCache.fetch(key, { context: load })) as T
  } catch (cause) {
    if (cause instanceof LichessError && cause.status === 404) throw cause
    const stale = await readApiCache<T>(key).catch((error: unknown) => {
      logDebug('lichess', 'API cache read failed:', key, error)
      return null
    })
    if (stale) return stale.value
    throw cause
  }
}

/** Drop the in-memory copies so the next read refetches (after a sync changes ratings and counts). */
export function forgetProfile(username: string): void {
  serviceState.profileCache.delete(profileKey(username, 'profile'))
  serviceState.profileCache.delete(profileKey(username, 'rating'))
}

/** Last persisted profile data, read locally with no network; lets the dashboard paint at once. */
export async function cachedProfile(username: string): Promise<{
  profile: LichessUser | null
  ratingHistory: LichessRatingHistory | null
  /** When the saved profile was fetched, so callers can skip refreshing a recent one. */
  profileFetchedAt?: number
}> {
  const [profileHit, ratingHit] = await Promise.all([
    readApiCache<LichessUser>(profileKey(username, 'profile')).catch((error: unknown) => {
      logDebug('lichess', 'Cached profile read failed:', username, error)
      return null
    }),
    readApiCache<LichessRatingHistory>(profileKey(username, 'rating')).catch((error: unknown) => {
      logDebug('lichess', 'Cached rating read failed:', username, error)
      return null
    }),
  ])
  return {
    profile: profileHit?.value ?? null,
    ratingHistory: ratingHit?.value ?? null,
    profileFetchedAt: profileHit?.fetchedAt,
  }
}

/** Full profiles from the last "who do I follow" fetch, kept so adding a friend needs no further request. */

/**
 * Everyone the connected accounts follow on Lichess. Needs the `follow:read` permission, which
 * accounts connected before it was requested lack: those come back as a problem to reconnect.
 */
export async function followedUsers(): Promise<FollowingReport> {
  const data = await loadData()
  const added = new Set(data.accounts.map((account) => account.username.toLowerCase()))
  const dismissed = await dismissedFriends()
  const byName = new Map<string, FollowedUser>()
  const problems: FollowingProblem[] = []
  const epoch = serviceState.loginEpoch
  const following = new Map<string, LichessUser>()
  for (const account of data.accounts.filter((entry) => entry.connected)) {
    try {
      const token = await getToken(account.username)
      if (!token) {
        problems.push({
          account: account.username,
          message: 'Its Lichess login is unavailable.',
          needsReconnect: true,
        })
        continue
      }
      const stream = await withUsage(account.username, 'profile', () =>
        unwrap(
          client.GET('/api/rel/following', {
            headers: { ...authorize(token), Accept: 'application/x-ndjson' },
            parseAs: 'stream',
          }),
        ),
      )
      await readLines(stream, (line) => {
        const user = JSON.parse(line) as LichessUser
        const key = user.username.toLowerCase()
        following.set(key, user)
        const entry = byName.get(key) ?? {
          username: user.username,
          title: user.title ?? undefined,
          ratings: {
            bullet: user.perfs?.bullet?.rating,
            blitz: user.perfs?.blitz?.rating,
            rapid: user.perfs?.rapid?.rating,
            classical: user.perfs?.classical?.rating,
          },
          followedBy: [],
          alreadyAdded: added.has(key),
          dismissed: dismissed.has(key),
        }
        entry.followedBy.push(account.username)
        byName.set(key, entry)
      })
    } catch (cause) {
      const denied = cause instanceof LichessError && (cause.status === 401 || cause.status === 403)
      logWarn('lichess', 'Could not load followed users:', account.username, cause)
      problems.push({
        account: account.username,
        message: denied
          ? 'Lichess needs your permission to read who this account follows.'
          : asError(cause).message,
        needsReconnect: denied,
      })
    }
  }
  // A logout during the download: this list belonged to an account that is gone.
  if (epoch !== serviceState.loginEpoch) throw new Error('Login was cancelled by logout.')
  serviceState.lastFollowing.clear()
  for (const [key, user] of following) serviceState.lastFollowing.set(key, user)
  const users = [...byName.values()]
    .sort((a, b) => a.username.localeCompare(b.username))
    .slice(0, 1000)
  return { users, problems }
}

/** Save the profiles fetched by `followedUsers` for the players just added, sparing a request each. */
export async function primeProfiles(usernames: string[]): Promise<void> {
  for (const name of usernames) {
    const user = serviceState.lastFollowing.get(name.toLowerCase())
    if (user)
      await writeApiCache(profileKey(name, 'profile'), user).catch((error: unknown) => {
        logDebug('lichess', 'Profile prime write failed:', name, error)
        return undefined
      })
  }
}

/** Turn Lichess's generic 404 into a message that names the user. */
const noSuchUser =
  (username: string) =>
  (cause: unknown): never => {
    if (cause instanceof LichessError && cause.status === 404)
      throw new LichessError(404, cause.endpoint, `no such Lichess user "${username}"`)
    throw cause
  }

export async function profile(username: string): Promise<LichessUser> {
  return cachedFetch(profileKey(username, 'profile'), () =>
    withUsage(username, 'profile', () =>
      unwrap(client.GET('/api/user/{username}', { params: { path: { username } } })).catch(
        noSuchUser(username),
      ),
    ),
  )
}

/** One player's record in one rating category (public; no login needed). */
export async function playerPerf(username: string, perf: PerfType): Promise<PerfStats> {
  const raw = await withUsage('', 'profile', () =>
    unwrap(
      client.GET('/api/user/{username}/perf/{perf}', { params: { path: { username, perf } } }),
    ).catch(noSuchUser(username)),
  )
  const stat = raw.stat
  const results = (
    list: { opRating: number; opId: { name: string }; at: string; gameId: string }[] = [],
  ) =>
    list.slice(0, 5).map((entry) => ({
      opponent: entry.opId?.name ?? '?',
      opponentRating: entry.opRating,
      at: entry.at,
      gameId: entry.gameId,
    }))
  const point = (value?: { int: number; at: string; gameId: string }) =>
    value ? { rating: value.int, at: value.at, gameId: value.gameId } : undefined
  return {
    perf,
    rating: raw.perf.glicko?.rating,
    deviation: raw.perf.glicko?.deviation,
    provisional: raw.perf.glicko?.provisional,
    rank: raw.rank ?? undefined,
    percentile: raw.percentile,
    progress: raw.perf.progress,
    count: stat.count,
    highest: point(stat.highest),
    lowest: point(stat.lowest),
    bestWins: results(stat.bestWins?.results),
    worstLosses: results(stat.worstLosses?.results),
    winStreak: {
      current: stat.resultStreak?.win.cur.v ?? 0,
      best: stat.resultStreak?.win.max.v ?? 0,
    },
    lossStreak: {
      current: stat.resultStreak?.loss.cur.v ?? 0,
      best: stat.resultStreak?.loss.max.v ?? 0,
    },
  }
}

/** Any finished Lichess game as PGN (with clocks), for the analysis board. */
export async function exportGame(id: string): Promise<string> {
  const pgn = await withUsage('', 'games', () =>
    unwrap(
      client.GET('/game/export/{gameId}', {
        params: { path: { gameId: id }, query: { clocks: true, evals: false, opening: true } },
        headers: { Accept: 'application/x-chess-pgn' },
        parseAs: 'text',
      }) as Promise<{ data?: string | null; response: Response }>,
    ),
  )
  if (pgn.length > 200_000) throw new Error('That game is too long to open.')
  return pgn
}

/**
 * A player's latest public games, newest first, seen from their side. `rated` asks for more of
 * their rated games instead: enough to chart their ratings when Lichess sends no history.
 */
export async function recentGames(username: string, rated = false): Promise<LichessGame[]> {
  const max = rated ? RATED_GAMES : RECENT_GAMES
  return withUsage('', 'profile', async () => {
    const stream = await asAnyAccount((auth) =>
      unwrap(
        client.GET('/api/games/user/{username}', {
          params: {
            path: { username },
            query: {
              max,
              moves: false,
              opening: !rated,
              ongoing: false,
              ...(rated ? { rated: true } : {}),
            },
          },
          headers: { ...auth, Accept: 'application/x-ndjson' },
          parseAs: 'stream',
        }),
      ),
    )
    const games: LichessGame[] = []
    await readLines(stream, (line) => {
      if (games.length < max)
        games.push(normalizeGame(JSON.parse(line) as components['schemas']['GameJson'], username))
    })
    return games
  })
}
const RECENT_GAMES = 10
const RATED_GAMES = 200

/**
 * A private message from `account`. Logins made before messaging was requested lack the
 * permission, and answer "reconnect" rather than failing.
 */
export async function sendMessage(
  account: string,
  username: string,
  text: string,
): Promise<{ sent: true } | NeedsReconnect> {
  return asAccount(
    account,
    async (token) => {
      await unwrap(
        client.POST('/inbox/{username}', {
          params: { path: { username } },
          body: { text },
          bodySerializer: urlencoded,
          headers: { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' },
        }),
      )
      return { sent: true as const }
    },
    'profile',
  )
}

/** Lifetime score between two players, and their current matchup when they are playing. */
export async function crosstable(a: string, b: string): Promise<Crosstable> {
  const raw = (await withUsage('', 'profile', () =>
    unwrap(
      client.GET('/api/crosstable/{user1}/{user2}', {
        params: { path: { user1: a, user2: b }, query: { matchup: true } },
      }),
    ),
  )) as { users: Record<string, number>; nbGames: number; matchup?: Crosstable['matchup'] }
  const lower = (users: Record<string, number> = {}) =>
    Object.fromEntries(Object.entries(users).map(([name, score]) => [name.toLowerCase(), score]))
  return {
    users: lower(raw.users),
    nbGames: raw.nbGames ?? 0,
    matchup: raw.matchup
      ? { users: lower(raw.matchup.users), nbGames: raw.matchup.nbGames ?? 0 }
      : undefined,
  }
}

export async function ratingHistory(username: string): Promise<LichessRatingHistory> {
  return cachedFetch(profileKey(username, 'rating'), () =>
    withUsage(username, 'profile', () =>
      asAnyAccount(
        (auth) =>
          unwrap(
            client.GET('/api/user/{username}/rating-history', {
              params: { path: { username } },
              headers: auth,
            }),
          ),
        username,
      ).catch(noSuchUser(username)),
    ),
  )
}

function playerName(
  player: components['schemas']['GamePlayerUser'] | components['schemas']['GamePlayerAi'],
): string {
  if ('user' in player) return player.user.name ?? player.name ?? 'Anonymous'
  if ('aiLevel' in player && typeof player.aiLevel === 'number')
    return `Stockfish level ${player.aiLevel}`
  return 'Anonymous'
}

function normalizeGame(raw: components['schemas']['GameJson'], account: string): LichessGame {
  const white = playerName(raw.players.white)
  const black = playerName(raw.players.black)
  const color: 'white' | 'black' = white.toLowerCase() === account.toLowerCase() ? 'white' : 'black'
  const player = raw.players[color] as components['schemas']['GamePlayerUser']
  const opponent = raw.players[color === 'white' ? 'black' : 'white']
  return {
    id: raw.id,
    account,
    createdAt: raw.createdAt,
    lastMoveAt: raw.lastMoveAt,
    rated: raw.rated,
    speed: raw.speed,
    perf: raw.perf,
    status: raw.status,
    winner: raw.winner,
    color,
    opponent: color === 'white' ? black : white,
    opponentRating: 'rating' in opponent ? opponent.rating : undefined,
    playerRating: player.rating,
    ratingDiff: player.ratingDiff,
    opening: raw.opening?.name,
    moves: raw.moves ?? '',
    pgn: raw.pgn,
  }
}

/**
 * Lichess's computer analysis of a game, as a review (scores from White's side, its labels and
 * accuracy); undefined when Lichess has not analysed it.
 */
export function reviewFromLichess(
  raw: components['schemas']['GameJson'],
): StoredReview | undefined {
  if (isGameInProgress(raw.status) || !raw.analysis?.length) return undefined
  if (raw.variant !== 'standard' && raw.variant !== 'fromPosition') return undefined
  const { fen, moves } = lichessGameLine({ moves: raw.moves ?? '', initialFen: raw.initialFen })
  if (!moves.length) return undefined
  const positions = replayPositions(fen, moves)
  const evals: (ReviewEval | null)[] = Array.from({ length: moves.length + 1 }, () => null)
  // Lichess scores the standard start as +0.15 when it works out accuracy.
  if (fen === INITIAL_FEN) evals[0] = { cp: 15 }
  const judgments: (Judgment | null)[] = moves.map(() => null)
  raw.analysis.slice(0, moves.length).forEach((entry, i) => {
    // Entry i scores the position after move i+1; its `best` is what should have been played.
    // Only numbers are scores: a review is stored as Lichess sent it, and read by the rules.
    const score =
      typeof entry.mate === 'number'
        ? { mate: entry.mate }
        : typeof entry.eval === 'number'
          ? { cp: entry.eval }
          : null
    if (score) evals[i + 1] = { ...evals[i + 1], ...score }
    if (entry.best) {
      const pv = entry.variation
        ? sanLineToUci(positions[i]?.fen ?? fen, entry.variation.split(/\s+/))
        : [entry.best]
      evals[i] = { ...(evals[i] ?? {}), best: entry.best, pv: pv.length ? pv : [entry.best] }
    }
    const name = entry.judgment?.name?.toLowerCase()
    if (name === 'inaccuracy' || name === 'mistake' || name === 'blunder') judgments[i] = name
  })
  const accuracyOf = (color: 'white' | 'black'): number | undefined => {
    const player = raw.players[color]
    return 'analysis' in player ? player.analysis?.accuracy : undefined
  }
  return {
    key: reviewKey(fen, moves),
    fen,
    moves,
    source: 'lichess',
    evals,
    judgments,
    accuracy: { white: accuracyOf('white'), black: accuracyOf('black') },
    depth: 0,
    complete: true,
    updatedAt: Date.now(),
    gameId: raw.id,
  }
}

const SYNC_PAGE = 1000

/**
 * Run an export with the account's own token when it has one: Lichess streams
 * a user's own games at 60/s authenticated versus 20/s anonymously. A rejected
 * token falls back to the anonymous request, so sync is never worse than before.
 */
export async function asOwner<T>(
  account: string,
  call: (auth: Record<string, string>) => Promise<T>,
): Promise<T> {
  const token = await getToken(account).catch((error: unknown) => {
    logDebug('lichess', 'Stored login is unavailable:', account, error)
    return null
  })
  if (!token) return call({})
  try {
    return await call(authorize(token))
  } catch (cause) {
    if (cause instanceof LichessError && cause.status === 401) return call({})
    throw cause
  }
}

/**
 * Lichess answers some public lookups (a player's games, rating history and teams) only to
 * signed-in apps, and otherwise sends nothing or "not found". Use a connected login when there
 * is one, preferring `account`; a refused login falls back to asking anonymously.
 */
export async function asAnyAccount<T>(
  call: (auth: Record<string, string>) => Promise<T>,
  account?: string,
): Promise<T> {
  const connected = (
    (
      await loadData().catch((error: unknown) => {
        logDebug('lichess', 'Stored accounts are unavailable:', error)
        return null
      })
    )?.accounts ?? []
  )
    .filter((entry) => entry.connected)
    .map((entry) => entry.username)
  const owner =
    connected.find((name) => name.toLowerCase() === account?.toLowerCase()) ?? connected[0]
  return owner ? asOwner(owner, call) : call({})
}

/** Games that ended this long ago have had their chance to be analysed on Lichess. */
const ANALYSIS_SETTLED_MS = 86_400_000

interface GamesPage {
  games: LichessGame[]
  /** Lichess's own analysis of these games, saved with them. */
  reviews: StoredReview[]
  /** Games Lichess was effectively asked about; the background review need not ask again. */
  settled: string[]
}

async function fetchGamesPage(
  account: string,
  since: number | undefined,
  until: number | undefined,
): Promise<GamesPage> {
  const stream = await asOwner(account, (auth) =>
    unwrap(
      client.GET('/api/games/user/{username}', {
        params: {
          path: { username: account },
          query: {
            max: SYNC_PAGE,
            ongoing: true,
            since,
            until,
            opening: true,
            moves: true,
            pgnInJson: true,
            clocks: true,
            evals: true,
            accuracy: true,
          },
        },
        headers: { ...auth, Accept: 'application/x-ndjson' },
        parseAs: 'stream',
      }),
    ),
  )
  const page: GamesPage = { games: [], reviews: [], settled: [] }
  // Drain the stream fast (releasing the bulk lane sooner), then do the chess work in batches
  // with yields so a 1,000-game page does not block the event loop for seconds.
  const lines: string[] = []
  await readLines(stream, (line) => {
    lines.push(line)
  })
  const now = Date.now()
  for (let i = 0; i < lines.length; i++) {
    const raw = JSON.parse(lines[i]!) as components['schemas']['GameJson']
    page.games.push(normalizeGame(raw, account))
    const review = reviewFromLichess(raw)
    if (review) page.reviews.push(review)
    if (!isGameInProgress(raw.status) && (review || now - raw.lastMoveAt > ANALYSIS_SETTLED_MS))
      page.settled.push(raw.id)
    if (i % 200 === 199) await sleep(0)
  }
  return page
}

/**
 * Ask Lichess for its analysis of games already synced (up to 300 per request), saving each one
 * it has; every game asked about is remembered so it is not asked again.
 */
export async function fetchLichessReviews(
  account: string,
  ids: readonly string[],
): Promise<StoredReview[]> {
  if (!ids.length) return []
  const epoch = accountEpoch(account)
  const parsed: StoredReview[] = []
  await withUsage(account, 'games', async () => {
    const stream = await asOwner(account, (auth) =>
      unwrap(
        client.POST('/api/games/export/_ids', {
          params: { query: { moves: true, evals: true, accuracy: true } },
          body: ids.slice(0, 300).join(','),
          bodySerializer: (body: string) => body,
          headers: { ...auth, Accept: 'application/x-ndjson', 'Content-Type': 'text/plain' },
          parseAs: 'stream',
        }) as Promise<StreamCall>,
      ),
    )
    const lines: string[] = []
    await readLines(stream, (line) => {
      lines.push(line)
    })
    for (let i = 0; i < lines.length; i++) {
      const review = reviewFromLichess(JSON.parse(lines[i]!) as components['schemas']['GameJson'])
      if (review) parsed.push(review)
      if (i % 100 === 99) await sleep(0)
    }
  })
  // Nothing is written until the download is over: a logout meanwhile keeps it all off disk.
  if (epoch !== accountEpoch(account)) throw new Error('Sync was cancelled.')
  const found = parsed.map((review) => writeReview(review).review)
  markChecked(ids.slice(0, 300))
  return found
}

/** Re-fetch unfinished games by ID, even when their creation predates the next sync window. */
async function refreshPendingGames(account: string): Promise<void> {
  const epoch = accountEpoch(account)
  const ids = await pendingGameIds(account)
  for (let offset = 0; offset < ids.length; offset += 300) {
    const stream = await asOwner(account, (auth) =>
      unwrap(
        client.POST('/api/games/export/_ids', {
          params: {
            query: {
              moves: true,
              pgnInJson: true,
              clocks: true,
              opening: true,
              evals: true,
              accuracy: true,
            },
          },
          body: ids.slice(offset, offset + 300).join(','),
          bodySerializer: (body: string) => body,
          headers: { ...auth, Accept: 'application/x-ndjson', 'Content-Type': 'text/plain' },
          parseAs: 'stream',
        }) as Promise<StreamCall>,
      ),
    )
    const requested = new Set(ids.slice(offset, offset + 300))
    const games: LichessGame[] = []
    const reviews: StoredReview[] = []
    const lines: string[] = []
    await readLines(stream, (line) => {
      lines.push(line)
    })
    for (let i = 0; i < lines.length; i++) {
      const raw = JSON.parse(lines[i]!) as components['schemas']['GameJson']
      if (!requested.has(raw.id)) continue
      games.push(normalizeGame(raw, account))
      const review = reviewFromLichess(raw)
      if (review) reviews.push(review)
      if (i % 100 === 99) await sleep(0)
    }
    // Missing IDs remain pending; a failed request never advances the sync cursor.
    await saveGamesPage(account, games, reviews, () => epoch === accountEpoch(account))
  }
}

/** One sync per account at a time: a second request joins the running one. */
function syncAccount(account: LichessAccount): Promise<number> {
  const epoch = accountEpoch(account.username)
  const key = account.username.toLowerCase()
  const running = serviceState.syncs.get(key)
  if (running) return running
  const run = withUsage(account.username, 'games', async () => {
    const startedAt = Date.now()
    await refreshPendingGames(account.username)
    // Incremental sync: Lichess supports `since` (ms). Overlap by 60s for
    // clock skew. Unfinished games are tracked separately because Lichess filters by creation time.
    const since = account.lastSyncedAt ? account.lastSyncedAt - 60_000 : undefined
    // Lichess returns newest first and caps each response, so page backwards
    // with `until`; otherwise a long gap between syncs would silently lose games.
    let fetched = 0
    let until: number | undefined
    while (fetched < MAX_GAMES) {
      const { games, reviews, settled } = await fetchGamesPage(account.username, since, until)
      // Save as we go: an interrupted sync keeps its pages (the cursor only moves once all are in).
      // Reviews go in the page's transaction; games are marked checked only once they are saved.
      const current = (): boolean => epoch === accountEpoch(account.username)
      if (!current()) throw new Error('Sync was cancelled.')
      if (games.length) await saveGamesPage(account.username, games, reviews, current)
      if (!current()) throw new Error('Sync was cancelled.')
      markChecked(settled)
      fetched += games.length
      if (games.length < SYNC_PAGE) break
      const oldest = games.reduce((min, g) => Math.min(min, g.createdAt), Infinity)
      if (until !== undefined && oldest >= until) break
      until = oldest
    }
    await saveGames(account.username, [], startedAt, () => epoch === accountEpoch(account.username))
    forgetProfile(account.username)
    return fetched
  })
    .then((fetched) => {
      logInfo('lichess', 'Game sync completed:', account.username, `fetched=${fetched}`)
      return fetched
    })
    .finally(() => {
      if (serviceState.syncs.get(key) === run) serviceState.syncs.delete(key)
    })
  serviceState.syncs.set(key, run)
  return run
}

export async function syncGames(username?: string): Promise<AppData> {
  const data = await loadData()
  const accounts = data.accounts.filter(
    (a) => !username || a.username.toLowerCase() === username.toLowerCase(),
  )
  // Lichess asks for one request at a time, so accounts sync in turn. Every
  // account still syncs when one fails; the first failure is reported afterwards.
  let firstError: unknown
  for (const account of accounts) {
    try {
      await syncAccount(account)
    } catch (cause) {
      logWarn('lichess', 'Game sync failed:', account.username, errorSummary(cause))
      firstError ??= cause
    }
  }
  if (firstError) throw firstError
  return loadData()
}

/** `onReturn` runs as soon as the browser hands the login back, before the token exchange. */
export async function connectLichess(
  look: OAuthPageLook = DEFAULT_OAUTH_LOOK,
  onReturn?: () => void,
): Promise<{ data: AppData; username: string }> {
  const epoch = serviceState.loginEpoch
  const verifier = randomBytes(32).toString('base64url')
  const challenge = createHash('sha256').update(verifier).digest('base64url')
  const state = randomBytes(24).toString('base64url')
  const code = await new Promise<{ code: string; redirect: string }>((resolve, reject) => {
    let redirect = ''
    let done = false
    const finish = (error?: Error, result?: { code: string; redirect: string }): void => {
      if (done) return
      done = true
      clearTimeout(timeout)
      signal?.removeEventListener('abort', abort)
      server.close()
      server.closeAllConnections()
      if (result) resolve(result)
      else reject(error ?? new Error('Lichess login was not completed.'))
    }
    const server = createServer((request, response) => {
      const callback = new URL(request.url ?? '/', 'http://127.0.0.1')
      // Only the real redirect (right path, right `state`) may end the login;
      // favicon requests, port probes and forged callbacks are ignored.
      if (callback.pathname !== '/callback' || callback.searchParams.get('state') !== state) {
        response.writeHead(404, { Connection: 'close' }).end()
        return
      }
      const authorizationCode = callback.searchParams.get('code')
      response.writeHead(authorizationCode ? 200 : 400, {
        'Content-Type': 'text/html; charset=utf-8',
        'Cache-Control': 'no-store',
        'Referrer-Policy': 'no-referrer',
        'Content-Security-Policy':
          "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'",
        'X-Content-Type-Options': 'nosniff',
        Connection: 'close',
      })
      onReturn?.()
      response.end(oauthPage(Boolean(authorizationCode), look), () =>
        authorizationCode
          ? finish(undefined, { code: authorizationCode, redirect })
          : finish(new Error('Lichess login was not completed.')),
      )
    })
    const signal = coreSignal()
    const abort = (): void => finish(new DOMException('Core closed.', 'AbortError'))
    const timeout = setTimeout(() => finish(new Error('Lichess login timed out.')), 300_000)
    signal?.addEventListener('abort', abort, { once: true })
    if (signal?.aborted) {
      abort()
      return
    }
    server.on('error', (cause) => finish(cause))
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      if (!address || typeof address === 'string') {
        finish(new Error('Could not open OAuth callback.'))
        return
      }
      redirect = `http://127.0.0.1:${address.port}/callback`
      const url = new URL(`${BASE}/oauth`)
      for (const [key, value] of Object.entries({
        response_type: 'code',
        client_id: OAUTH_CLIENT_ID,
        redirect_uri: redirect,
        scope: OAUTH_SCOPES.join(' '),
        code_challenge_method: 'S256',
        code_challenge: challenge,
        state,
      }))
        url.searchParams.set(key, value)
      platform()
        .openExternal(url.toString())
        .catch((cause: unknown) => {
          logWarn('lichess', 'Could not open Lichess login page:', cause)
          finish(asError(cause))
        })
    })
  })
  const token = await unwrap(
    client.POST('/api/token', {
      body: {
        grant_type: 'authorization_code',
        code: code.code,
        redirect_uri: code.redirect,
        client_id: OAUTH_CLIENT_ID,
        code_verifier: verifier,
      },
      bodySerializer: urlencoded,
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    }),
  )
  const account = await unwrap(
    client.GET('/api/account', { headers: authorize(token.access_token) }),
  )
  const data = await saveLogin(
    account.username,
    token.access_token,
    () => epoch === serviceState.loginEpoch,
  )
  return { data, username: account.username }
}

/**
 * Run a puzzle request as `account`. A missing login, or one Lichess refuses (it was connected
 * before puzzles were requested, or was revoked), answers "reconnect" instead of failing.
 */
export async function asAccount<T>(
  account: string,
  work: (token: string) => Promise<T>,
  kind: UsageKind = 'puzzles',
): Promise<T | NeedsReconnect> {
  const token = await getToken(account)
  if (!token) return { needsReconnect: true }
  try {
    return await withUsage(account, kind, () => work(token))
  } catch (cause) {
    if (cause instanceof LichessError && (cause.status === 401 || cause.status === 403))
      return { needsReconnect: true }
    throw cause
  }
}

function readablePuzzle(raw: ApiPuzzle): Puzzle {
  const puzzle = puzzleFromApi(raw)
  if (!puzzle) throw new Error('Lichess sent a puzzle KChess could not read.')
  return puzzle
}

/** The next puzzle. With an account it is chosen for its rating and one it has not seen; without, it is random. */
export async function puzzleNext(request: PuzzleRequest): Promise<PuzzleDraw | NeedsReconnect> {
  const { angle, difficulty, color } = request
  if (!request.account) {
    const raw = await withUsage('', 'puzzles', () =>
      unwrap(client.GET('/api/puzzle/next', { params: { query: { angle, difficulty, color } } })),
    )
    return { puzzle: readablePuzzle(raw as unknown as ApiPuzzle) }
  }
  return asAccount(request.account, async (token) => {
    const batch = await unwrap(
      client.GET('/api/puzzle/batch/{angle}', {
        params: { path: { angle }, query: { nb: 1, difficulty, color } },
        headers: authorize(token),
      }),
    )
    const first = batch.puzzles?.[0]
    if (!first) throw new Error('Lichess has no more puzzles for this theme and difficulty.')
    return { puzzle: readablePuzzle(first as unknown as ApiPuzzle), glicko: batch.glicko }
  })
}

/** Tell Lichess how a puzzle went. Rated results move the account's Lichess puzzle rating. */
export async function puzzleSolve(
  request: PuzzleSolveRequest,
): Promise<PuzzleSolveResult | NeedsReconnect> {
  return asAccount(request.account, async (token) => {
    const result = await unwrap(
      client.POST('/api/puzzle/batch/{angle}', {
        params: { path: { angle: request.angle } },
        body: { solutions: [{ id: request.id, win: request.win, rated: request.rated }] },
        headers: authorize(token),
      }),
    )
    const round = result.rounds?.find((entry) => entry.id === request.id)
    return { ratingDiff: request.rated ? round?.ratingDiff : undefined }
  })
}

export async function puzzleDaily(): Promise<Puzzle> {
  const raw = await withUsage('', 'puzzles', () => unwrap(client.GET('/api/puzzle/daily')))
  return readablePuzzle(raw as unknown as ApiPuzzle)
}

export async function puzzleDashboard(
  account: string,
  days: number,
): Promise<PuzzleDashboard | NeedsReconnect> {
  return asAccount(account, (token) =>
    unwrap(
      client.GET('/api/puzzle/dashboard/{days}', {
        params: { path: { days } },
        headers: authorize(token),
      }),
    ),
  )
}

export async function puzzleActivity(
  account: string,
  max: number,
): Promise<PuzzleActivityEntry[] | NeedsReconnect> {
  return asAccount(account, async (token) => {
    const stream = await unwrap(
      client.GET('/api/puzzle/activity', {
        params: { query: { max } },
        headers: { ...authorize(token), Accept: 'application/x-ndjson' },
        parseAs: 'stream',
      }),
    )
    const entries: PuzzleActivityEntry[] = []
    await readLines(stream, (line) => {
      const raw = JSON.parse(line) as components['schemas']['PuzzleActivity']
      const puzzle = puzzleFromApi({
        game: {},
        puzzle: { ...raw.puzzle, initialPly: 0 },
      })
      if (puzzle) entries.push({ date: raw.date, win: raw.win, puzzle })
    })
    return entries
  })
}

/** Anyone's public Storm results; needs no login. */
export async function stormDashboard(username: string, days: number): Promise<StormDashboard> {
  return withUsage('', 'puzzles', () =>
    unwrap(
      client.GET('/api/storm/dashboard/{username}', {
        params: { path: { username }, query: { days } },
      }),
    ).catch(noSuchUser(username)),
  )
}

const MAX_RECONNECTS = 6

interface ListenOptions {
  lane?: OnlineConnection['lane']
  /** Re-open the stream when it drops (event and game streams, not one-shot seeks). */
  reconnect?: boolean
  /** Checked after a clean end of stream; true stops reconnecting (the game is over). */
  finished?: () => boolean
  /** Report the stream's health here instead of as the game connection (the idle event stream). */
  quiet?: (phase: OnlineConnection['phase'], message?: string) => void
}

/** Hooks for what the event stream carries besides the game being played. */
export interface OnlineHooks {
  /** A challenge event for `account` (from whichever event stream is open). */
  challenge?: (account: string, event: OnlineEvent) => void
  /** A game started or finished that is not shown on the board (correspondence, other account). */
  ongoingChanged?: () => void
  /** Health of the idle event stream that waits for challenges and pairings. */
  lobbyState?: (state: {
    account: string
    phase: OnlineConnection['phase']
    message?: string
  }) => void
}

type ChallengeEventType = 'challenge' | 'challengeCanceled' | 'challengeDeclined'
const isChallengeEvent = (event: OnlineEvent): boolean =>
  (['challenge', 'challengeCanceled', 'challengeDeclined'] as ChallengeEventType[]).includes(
    event.type as ChallengeEventType,
  )

/** Board API body for a seek or challenge. */
function gameBody(options: OnlineOptions): Record<string, unknown> {
  const variant = options.fen && options.variant !== 'chess960' ? 'fromPosition' : options.variant
  return {
    rated: options.rated,
    color: options.color,
    variant: variant && variant !== 'standard' ? variant : undefined,
    fen: options.fen,
  }
}

export class OnlineSession {
  private controller: AbortController | null = null
  private gameController: AbortController | null = null
  /** The game whose stream is open, while it lasts. */
  private liveGame = ''
  /** Whether the open game is a correspondence game (another may replace it). */
  private liveCorrespondence = false
  private protectedGame = ''
  private protectedAccount = ''
  private recoveryUnverified = true
  private currentAccount = ''
  private pendingId = ''
  private epoch = 0
  private resumeAttempt = 0
  private readonly attachments = new RequestScope()
  private attachingAccount = ''
  /** Games whose start has been announced to the window, so a reconnect does not repeat it. */
  private announced = new Set<string>()
  private presenceCache = new LRUCache<string, Promise<PresenceReport>>({ max: 100, ttl: 4000 })
  /** The account whose event stream should stay open while no game is being played. */
  private lobbyAccount = ''
  private lobby: { account: string; controller: AbortController } | null = null
  constructor(
    private emit: (event: OnlineEvent) => void,
    private error: (error: string) => void,
    private state: (state: OnlineConnection) => void = () => {},
    private hooks: OnlineHooks = {},
  ) {}

  private report(cause: unknown): void {
    if (cause instanceof DOMException && cause.name === 'AbortError') return
    if (
      cause instanceof LichessError &&
      cause.status === 400 &&
      cause.endpoint === 'POST /api/board/seek'
    )
      this.error(
        'Lichess rejected this public seek. The Board API allows Rapid and slower for matchmaking (Blitz and faster need a direct challenge). Pick a longer control or add a username.',
      )
    else this.error(asError(cause).message)
  }

  private async listen(
    open: () => Promise<StreamCall>,
    signal: AbortSignal,
    emit: (event: OnlineEvent) => void,
    options: ListenOptions = {},
  ): Promise<void> {
    let failures = 0
    const session = this.epoch
    const account = this.currentAccount
    const gameId = options.lane === 'game' ? this.liveGame : ''
    const state = (phase: OnlineConnection['phase'], message?: string): void => {
      if (signal.aborted) return
      if (options.quiet) options.quiet(phase, message)
      else if (session === this.epoch)
        this.state({ session, account, gameId, lane: options.lane ?? 'events', phase, message })
    }
    state('connecting')
    while (!signal.aborted) {
      let received = false
      try {
        const stream = await unwrap(open())
        signal.throwIfAborted()
        // The idle stream may stay silent for hours; an open connection is a healthy one.
        if (options.quiet) state('connected')
        await readLines(
          stream,
          (line) => {
            if (signal.aborted) return
            received = true
            const event = validateOnlineEvent(JSON.parse(line))
            if (event) {
              state('connected')
              emit(event)
            }
          },
          { signal },
        )
      } catch (cause) {
        if (signal.aborted) return
        logDebug('lichess', 'Stream failed:', account, gameId, cause)
        const permanent =
          cause instanceof LichessError && cause.status < 500 && cause.status !== 429
        if (!options.reconnect || permanent) {
          state(
            cause instanceof LichessError && [401, 403].includes(cause.status)
              ? 'auth-required'
              : 'disconnected',
            asError(cause).message,
          )
          if (!options.quiet) this.report(cause)
          return
        }
      }
      if (signal.aborted || options.finished?.()) return
      if (!options.reconnect) {
        state('disconnected', 'The seek ended. Try finding a game again.')
        return
      }
      failures = received ? 1 : failures + 1
      if (failures > MAX_RECONNECTS) {
        logWarn(
          'lichess',
          'Lichess stream gave up reconnecting:',
          account,
          gameId,
          `failures=${failures}`,
        )
        state('disconnected', 'Lost connection to Lichess. Reconnect to recover the game.')
        if (!options.quiet)
          this.error('Lost connection to Lichess. Check your network and reopen the game.')
        return
      }
      state('reconnecting', 'Connection interrupted. Reconnecting…')
      // Resolves early (rejects) if the session is cancelled while backing off; the loop then exits.
      await sleep(Math.min(15_000, 500 * 2 ** failures), undefined, { signal }).catch(
        (cause: unknown) => {
          logDebug('lichess', 'Reconnect wait was cancelled:', account, gameId, cause)
          return undefined
        },
      )
    }
  }

  private openEvents(token: string, signal: AbortSignal, options: ListenOptions = {}) {
    return (emit: (event: OnlineEvent) => void): Promise<void> =>
      this.listen(
        () =>
          client.GET('/api/stream/event', { headers: authorize(token), parseAs: 'stream', signal }),
        signal,
        emit,
        { reconnect: true, lane: 'events', ...options },
      )
  }

  /**
   * The event stream of the account being played: challenges go to the inbox, a real-time game
   * starting opens it on the board (a seek, an accepted challenge or rematch, a tournament pairing).
   */
  private eventStream(token: string, signal: AbortSignal): Promise<void> {
    const account = this.currentAccount
    return this.openEvents(
      token,
      signal,
    )((event) => {
      if (signal.aborted) return
      if (isChallengeEvent(event)) {
        this.hooks.challenge?.(account, event)
        return
      }
      if (event.type === 'gameStart') {
        const game = event.game
        const correspondence = game.speed === 'correspondence'
        if (game.gameId !== this.liveGame) {
          // Another game is on the board; correspondence games open only when asked for.
          if (correspondence || (this.liveGame && !this.liveCorrespondence)) {
            this.hooks.ongoingChanged?.()
            return
          }
          this.pendingId = ''
          void this.openGame(game.gameId, token)
        }
        this.announce(event)
        return
      }
      if (event.type === 'gameFinish') {
        this.hooks.ongoingChanged?.()
        if (event.game.gameId !== this.liveGame && event.game.gameId !== this.protectedGame) return
      }
      this.emit(event)
    })
  }

  private announce(event: OnlineEvent & { type: 'gameStart' }): void {
    if (this.announced.has(event.game.gameId)) return
    this.announced.add(event.game.gameId)
    if (this.announced.size > 200) this.announced.delete(this.announced.values().next().value!)
    this.emit(event)
  }

  /* ── The idle event stream ─────────────────────────────────────────── */

  /** Keep `account`'s event stream open while nothing else is (empty: stop). */
  stayConnected(account: string): void {
    this.lobbyAccount = account
    if (!account) this.stopLobby()
    else this.startLobby()
  }

  private stopLobby(): void {
    this.lobby?.controller.abort()
    this.lobby = null
  }

  private startLobby(): void {
    const account = this.lobbyAccount
    if (!account || this.controller) return
    if (this.lobby && this.lobby.account === account && !this.lobby.controller.signal.aborted)
      return
    this.stopLobby()
    const controller = new AbortController()
    this.lobby = { account, controller }
    const quiet = (phase: OnlineConnection['phase'], message?: string): void =>
      this.hooks.lobbyState?.({ account, phase, message })
    void (async () => {
      const token = await getToken(account).catch((error: unknown) => {
        logDebug('lichess', 'Stored login is unavailable:', account, error)
        return null
      })
      if (controller.signal.aborted) return
      if (!token) {
        quiet('auth-required', 'Its Lichess login is unavailable. Reconnect it in Settings.')
        return
      }
      await withUsage(account, 'play', () =>
        this.openEvents(token, controller.signal, { quiet })((event) => {
          if (controller.signal.aborted) return
          if (isChallengeEvent(event)) this.hooks.challenge?.(account, event)
          else if (event.type === 'gameStart' && event.game.speed !== 'correspondence')
            void this.attach(account, event.game.gameId).catch((cause: unknown) => {
              logWarn('lichess', 'Could not open game:', account, event.game.gameId, cause)
              this.report(cause)
            })
          else if (event.type === 'gameStart' || event.type === 'gameFinish')
            this.hooks.ongoingChanged?.()
        }),
      )
    })()
  }

  /* ── Sessions ─────────────────────────────────────────────────────── */

  /** Open a known game of `account` on the board: its event stream, then its game stream. */
  async attach(account: string, id: string): Promise<void> {
    if (this.liveGame === id) return
    const request = this.attachments.next()
    this.attachingAccount = account
    try {
      const token = await getToken(account)
      if (!request.current()) throw new DOMException('Attachment cancelled.', 'AbortError')
      if (!token)
        throw new Error(`@${account}'s Lichess login is unavailable. Reconnect it in Settings.`)
      // Another caller may have attached this game while credentials were loading.
      if (this.liveGame === id) return
      this.cancel()
      this.currentAccount = account
      attributeTo(account, 'play')
      this.controller = new AbortController()
      void this.eventStream(token, this.controller.signal)
      void this.openGame(id, token)
    } catch (cause) {
      if (!request.current()) throw new DOMException('Attachment cancelled.', 'AbortError')
      throw cause
    } finally {
      if (request.current()) this.attachingAccount = ''
    }
  }

  /** Reattach to a real-time game in progress on any connected account. */
  async resume(): Promise<{ id: string; account: string } | null> {
    this.attachments.invalidate()
    this.attachingAccount = ''
    const attempt = ++this.resumeAttempt
    this.recoveryUnverified = true
    let authRequired = false
    let checkingAccount = this.currentAccount
    this.state({
      session: this.epoch,
      account: this.currentAccount,
      gameId: this.protectedGame,
      lane: 'game',
      phase: 'checking',
      message: 'Checking Lichess for an ongoing game…',
    })
    try {
      const accounts = (await loadData()).accounts.filter((a) => a.connected)
      if (attempt !== this.resumeAttempt)
        throw new DOMException('Recovery cancelled.', 'AbortError')
      const protectedAccount = this.protectedGame ? this.protectedAccount : ''
      if (protectedAccount) {
        const at = accounts.findIndex(
          (a) => a.username.toLowerCase() === protectedAccount.toLowerCase(),
        )
        if (at >= 0) accounts.splice(at, 1)
        accounts.unshift({ username: protectedAccount, connected: true })
      }
      let firstError: unknown
      for (const account of accounts) {
        checkingAccount = account.username
        try {
          const token = await getToken(account.username)
          if (attempt !== this.resumeAttempt)
            throw new DOMException('Recovery cancelled.', 'AbortError')
          if (!token) {
            authRequired = true
            this.state({
              session: this.epoch,
              account: account.username,
              gameId: this.protectedGame,
              lane: 'game',
              phase: 'auth-required',
              message: 'Lichess login is unavailable. Reconnect in Settings.',
            })
            throw new Error(
              `@${account.username}'s Lichess login is unavailable. Reconnect in Settings.`,
            )
          }
          const playing = await withUsage(account.username, 'play', () =>
            unwrap(client.GET('/api/account/playing', { headers: authorize(token) })),
          )
          if (!Array.isArray(playing.nowPlaying))
            throw new Error('Lichess returned an invalid ongoing-game response.')
          // Correspondence games last for days: they open only when the player picks one.
          const id = playing.nowPlaying?.find((game) => game.speed !== 'correspondence')?.gameId
          if (attempt !== this.resumeAttempt)
            throw new DOMException('Recovery cancelled.', 'AbortError')
          if (!id) continue
          this.recoveryUnverified = false
          this.cancel()
          this.currentAccount = account.username
          attributeTo(account.username, 'play')
          this.controller = new AbortController()
          void this.eventStream(token, this.controller.signal)
          void this.openGame(id, token)
          return { id, account: account.username }
        } catch (cause) {
          // One account with a revoked login must not hide a game on another.
          if (attempt !== this.resumeAttempt)
            throw new DOMException('Recovery cancelled.', 'AbortError')
          if (protectedAccount && account.username.toLowerCase() === protectedAccount.toLowerCase())
            throw cause
          logWarn('lichess', 'Game recovery check failed:', account.username, cause)
          firstError ??= cause
        }
      }
      if (attempt !== this.resumeAttempt)
        throw new DOMException('Recovery cancelled.', 'AbortError')
      if (firstError) throw firstError
      this.recoveryUnverified = false
      this.cancel()
      this.protectedGame = ''
      this.state({
        session: this.epoch,
        account: this.currentAccount,
        gameId: '',
        lane: 'game',
        phase: 'idle',
      })
      this.startLobby()
      return null
    } catch (cause) {
      if (attempt === this.resumeAttempt)
        this.state({
          session: this.epoch,
          account: this.protectedAccount || checkingAccount,
          gameId: this.protectedGame,
          lane: 'game',
          phase:
            authRequired || (cause instanceof LichessError && [401, 403].includes(cause.status))
              ? 'auth-required'
              : 'disconnected',
          message: asError(cause).message,
        })
      throw cause
    }
  }

  /** Every game the connected accounts are playing, most urgent first. */
  async ongoing(): Promise<OngoingGame[]> {
    const accounts = (await loadData()).accounts.filter((a) => a.connected)
    const games: OngoingGame[] = []
    let firstError: unknown
    for (const account of accounts) {
      try {
        const token = await getToken(account.username)
        if (!token) continue
        const playing = await withUsage(account.username, 'play', () =>
          unwrap(
            client.GET('/api/account/playing', {
              params: { query: { nb: 50 } },
              headers: authorize(token),
            }),
          ),
        )
        for (const game of playing.nowPlaying ?? []) {
          const opponent = game.opponent as
            { username?: string; rating?: number; ai?: number } | undefined
          games.push({
            gameId: game.gameId,
            account: account.username,
            opponent: {
              name: opponent?.ai
                ? `Stockfish level ${opponent.ai}`
                : (opponent?.username ?? 'Opponent'),
              rating: opponent?.rating,
            },
            color: game.color,
            fen: game.fen,
            lastMove: game.lastMove || undefined,
            isMyTurn: Boolean(game.isMyTurn),
            secondsLeft: typeof game.secondsLeft === 'number' ? game.secondsLeft : undefined,
            variant: game.variant?.key ?? 'standard',
            speed: game.speed,
            rated: game.rated,
            tournamentId: (game as { tournamentId?: string }).tournamentId,
          })
        }
      } catch (cause) {
        logWarn('lichess', 'Could not load ongoing games:', account.username, cause)
        firstError ??= cause
      }
    }
    if (!games.length && firstError) throw firstError
    return games.sort((a, b) => Number(b.isMyTurn) - Number(a.isMyTurn))
  }

  /** Open one of `ongoing()`'s games; a live real-time game on the board is never replaced. */
  async open(account: string, id: string): Promise<void> {
    if (this.liveGame === id) return
    if (this.liveGame && !this.liveCorrespondence)
      throw new Error('Finish the live game on the board before opening another.')
    await this.attach(account, id)
  }

  async start(
    options: OnlineOptions,
  ): Promise<{ id?: string; url?: string; seeking?: boolean; correspondence?: boolean }> {
    if (this.liveGame && !this.liveCorrespondence)
      throw new Error('Reconnect to your current game before finding another.')
    if (this.protectedGame && !this.liveCorrespondence)
      throw new Error('Reconnect to your current game before finding another.')
    this.cancel()
    const target = options.target?.trim()
    const correspondence = options.days !== undefined
    const perf = perfFor(options.minutes, options.increment)
    const label = `${options.minutes}+${options.increment}`
    // Validate before opening any stream so a rejected request leaves nothing running.
    if (options.fen && (!target || options.rated))
      throw new Error('A game from a set-up position must be a casual challenge to a player.')
    if (!correspondence && target && !canDirectChallenge(options.minutes, options.increment))
      throw new Error(
        `Lichess blocks ${perf} (${label}) even for direct challenges. The Board API allows Blitz and slower — pick at least 3 minutes of estimated play (time + 40 × increment).`,
      )
    if (!correspondence && !target && !canBoardSeek(options.minutes, options.increment))
      throw new Error(
        `Lichess blocks ${perf} (${label}) for public seeks. The Board API allows Rapid and slower for matchmaking — add a username to challenge directly (Blitz allowed) or pick a longer control.`,
      )
    const controller = new AbortController()
    this.controller = controller
    try {
      const connected = (await loadData()).accounts.filter((a) => a.connected)
      controller.signal.throwIfAborted()
      if (!connected.length) throw new Error('Connect your Lichess account in Settings first.')
      // An explicit account must match exactly: never play as a different one.
      const account = options.account
        ? connected.find((a) => a.username.toLowerCase() === options.account?.toLowerCase())
        : connected[0]
      if (!account) throw new Error(`@${options.account} is not connected. Connect it in Settings.`)
      this.currentAccount = account.username
      attributeTo(account.username, 'play')
      const token = await getToken(account.username)
      controller.signal.throwIfAborted()
      if (!token) throw new Error('Your Lichess login is unavailable. Reconnect in Settings.')
      if (correspondence) {
        // Correspondence games run for days: nothing waits here; the idle stream reports acceptance.
        const created = await this.createCorrespondence(options, token, controller.signal, target)
        controller.abort()
        if (this.controller === controller) this.controller = null
        this.startLobby()
        return { ...created, correspondence: true }
      }
      void this.eventStream(token, controller.signal)
      if (target) {
        try {
          const challenge = await unwrap(
            client.POST('/api/challenge/{username}', {
              params: { path: { username: target } },
              body: {
                'clock.limit': options.minutes * 60,
                'clock.increment': options.increment,
                ...gameBody(options),
              },
              bodySerializer: urlencoded,
              headers: { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' },
              signal: controller.signal,
            }),
          )
          // A server may finish creating the challenge just as cancellation arrives.
          if (controller.signal.aborted) {
            await this.cancelChallenge(challenge.id, token)
            controller.signal.throwIfAborted()
          }
          if (!this.liveGame) this.pendingId = challenge.id
          return { id: challenge.id, url: challenge.url }
        } catch (cause) {
          if (cause instanceof LichessError && cause.status === 400)
            throw new Error(
              `Lichess rejected this ${perf} (${label}) challenge: ${cause.message}. The Board API allows Blitz and slower for direct challenges.`,
              { cause },
            )
          throw cause
        }
      }
      const seekController = new AbortController()
      this.gameController = seekController
      void this.listen(
        () =>
          client.POST('/api/board/seek', {
            body: {
              time: options.minutes,
              increment: options.increment,
              ...gameBody(options),
            },
            bodySerializer: urlencoded,
            headers: { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' },
            parseAs: 'stream',
            signal: seekController.signal,
          }),
        seekController.signal,
        (event) => this.emit(event),
        { lane: 'seek' },
      )
      return { seeking: true }
    } catch (cause) {
      // Don't leave the event stream running for a game the UI never started.
      controller.abort()
      if (this.controller === controller) {
        this.controller = null
        this.startLobby()
      }
      throw cause
    }
  }

  private async createCorrespondence(
    options: OnlineOptions,
    token: string,
    signal: AbortSignal,
    target?: string,
  ): Promise<{ id?: string; url?: string }> {
    const headers = { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' }
    if (target) {
      const challenge = await unwrap(
        client.POST('/api/challenge/{username}', {
          params: { path: { username: target } },
          body: { days: options.days, ...gameBody(options) } as never,
          bodySerializer: urlencoded,
          headers,
          signal,
        }),
      )
      return { id: challenge.id, url: challenge.url }
    }
    const seek = await unwrap(
      client.POST('/api/board/seek', {
        body: { days: options.days, ...gameBody(options) } as never,
        bodySerializer: urlencoded,
        headers,
        signal,
      }) as Promise<{ data?: { id?: string } | null; response: Response }>,
    )
    return { id: seek.id }
  }

  private async cancelChallenge(id: string, token: string): Promise<void> {
    await unwrap(
      client.POST('/api/challenge/{challengeId}/cancel', {
        params: { path: { challengeId: id } },
        headers: authorize(token),
      }),
    ).catch((cause: unknown) => {
      logDebug('lichess', 'Challenge cancel failed:', id, cause)
      return undefined
    })
  }

  /** Accept an incoming challenge as the account it was sent to; a real-time game opens at once. */
  async acceptChallenge(challenge: ChallengeInfo): Promise<void> {
    const live = challenge.timeControl.type === 'clock'
    if (live && this.liveGame && !this.liveCorrespondence)
      throw new Error('Finish the game on the board before accepting another.')
    const token = await getToken(challenge.account)
    if (!token)
      throw new Error(
        `@${challenge.account}'s Lichess login is unavailable. Reconnect in Settings.`,
      )
    await withUsage(challenge.account, 'play', () =>
      unwrap(
        client.POST('/api/challenge/{challengeId}/accept', {
          params: { path: { challengeId: challenge.id } },
          headers: authorize(token),
        }),
      ),
    )
    // The game has the challenge's id. Open it now rather than wait for the event stream,
    // which may belong to another connected account.
    if (live) await this.attach(challenge.account, challenge.id)
    else this.hooks.ongoingChanged?.()
  }

  async declineChallenge(challenge: ChallengeInfo, reason: DeclineReason): Promise<void> {
    const token = await getToken(challenge.account)
    if (!token)
      throw new Error(
        `@${challenge.account}'s Lichess login is unavailable. Reconnect in Settings.`,
      )
    await withUsage(challenge.account, 'play', () =>
      unwrap(
        client.POST('/api/challenge/{challengeId}/decline', {
          params: { path: { challengeId: challenge.id } },
          body: { reason } as never,
          bodySerializer: urlencoded,
          headers: { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' },
        }),
      ),
    )
  }

  async withdrawChallenge(challenge: ChallengeInfo): Promise<void> {
    const token = await getToken(challenge.account)
    if (!token)
      throw new Error(
        `@${challenge.account}'s Lichess login is unavailable. Reconnect in Settings.`,
      )
    if (this.pendingId === challenge.id) this.pendingId = ''
    await withUsage(challenge.account, 'play', () =>
      unwrap(
        client.POST('/api/challenge/{challengeId}/cancel', {
          params: { path: { challengeId: challenge.id } },
          headers: authorize(token),
        }),
      ),
    )
  }

  private async openGame(id: string, token: string): Promise<void> {
    this.gameController?.abort()
    const controller = new AbortController()
    this.gameController = controller
    let finished = false
    this.liveGame = id
    this.liveCorrespondence = false
    this.protectedGame = id
    this.protectedAccount = this.currentAccount
    await this.listen(
      () =>
        client.GET('/api/board/game/stream/{gameId}', {
          params: { path: { gameId: id } },
          headers: authorize(token),
          parseAs: 'stream',
          signal: controller.signal,
        }),
      controller.signal,
      (event) => {
        // Lichess closes the game stream when the game ends; don't reconnect then.
        const raw = event as {
          type: string
          status?: string
          speed?: string
          state?: { status?: string }
        }
        if (raw.type === 'gameFull') this.liveCorrespondence = raw.speed === 'correspondence'
        const status =
          raw.type === 'gameFull'
            ? raw.state?.status
            : raw.type === 'gameState'
              ? raw.status
              : undefined
        if (status && !isGameInProgress(status)) {
          finished = true
          this.liveGame = ''
          if (this.protectedGame === id) this.protectedGame = ''
        }
        this.emit({ ...event, id } as OnlineEvent)
      },
      { reconnect: true, finished: () => finished, lane: 'game' },
    ).finally(() => {
      if (finished && this.gameController === controller && this.liveGame === id) this.liveGame = ''
    })
  }

  /** An online game is being played: background work should keep out of its way. */
  get playing(): boolean {
    return this.liveGame !== '' || this.protectedGame !== ''
  }

  /** Unknown startup/recovery state also blocks assistance, even before a game ID is known. */
  get assistanceBlocked(): boolean {
    return this.recoveryUnverified || this.playing
  }

  private async liveToken(id: string, what: string): Promise<string> {
    if (id !== this.liveGame) throw new Error(`Reconnect to this game before ${what}.`)
    const token = await getToken(this.currentAccount)
    if (!token) throw new Error('Lichess login unavailable.')
    return token
  }

  async move(id: string, uci: string): Promise<void> {
    const token = await this.liveToken(id, 'sending a move')
    await withUsage(this.currentAccount, 'play', () =>
      unwrap(
        client.POST('/api/board/game/{gameId}/move/{move}', {
          params: { path: { gameId: id, move: uci } },
          headers: authorize(token),
        }),
      ),
    )
  }

  /** The private chat between the two players so far. */
  async chat(id: string): Promise<ChatLine[]> {
    const token = await this.liveToken(id, 'reading the chat')
    const lines = await withUsage(this.currentAccount, 'play', () =>
      unwrap(
        client.GET('/api/board/game/{gameId}/chat', {
          params: { path: { gameId: id } },
          headers: authorize(token),
        }),
      ),
    )
    return (Array.isArray(lines) ? lines : [])
      .slice(-200)
      .filter((line) => typeof line?.text === 'string' && typeof line.user === 'string')
      .map((line) => ({
        user: line.user.slice(0, 40),
        text: line.text.slice(0, 400),
        room: 'player' as const,
      }))
  }

  async sendChat(id: string, room: ChatRoom, text: string): Promise<void> {
    const token = await this.liveToken(id, 'chatting')
    await withUsage(this.currentAccount, 'play', () =>
      unwrap(
        client.POST('/api/board/game/{gameId}/chat', {
          params: { path: { gameId: id } },
          body: { room, text },
          bodySerializer: urlencoded,
          headers: { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' },
        }),
      ),
    )
  }

  /**
   * Lichess's online/playing/signal flags for some users, plus how long the request took.
   * Works with no game in progress too (the Friends page), just without a login.
   */
  async presence(usernames: string[]): Promise<PresenceReport> {
    const account = this.currentAccount
    const ids = [...new Set(usernames.map((name) => name.toLowerCase()))].sort()
    const key = `${account}|${ids.join(',')}`
    const existing = this.presenceCache.get(key)
    if (existing) return existing
    const request = this.fetchPresence(ids, account)
    this.presenceCache.set(key, request)
    void request.catch((cause: unknown) => {
      logDebug('lichess', 'Presence check failed:', cause)
      this.presenceCache.delete(key)
    })
    return request
  }
  private async fetchPresence(usernames: string[], account: string): Promise<PresenceReport> {
    const token = account ? await getToken(account) : null
    const started = performance.now()
    const rows = await withUsage(account, 'presence', () =>
      unwrap(
        client.GET('/api/users/status', {
          params: { query: { ids: usernames.join(','), withSignal: true, withGameIds: true } },
          headers: token ? authorize(token) : {},
        }),
      ),
    )
    const latencyMs = Math.round(performance.now() - started)
    const users: PresenceReport['users'] = {}
    // The generated type predates `withSignal`, so `signal` is read off the raw row.
    for (const row of rows as ((typeof rows)[number] & { signal?: number; playingId?: string })[])
      users[row.id.toLowerCase()] = {
        online: Boolean(row.online),
        playing: Boolean(row.playing),
        signal: row.signal,
        playingId:
          typeof row.playingId === 'string' && GAME_ID.test(row.playingId)
            ? row.playingId
            : undefined,
      }
    return { users, latencyMs }
  }

  async action(id: string, action: OnlineAction): Promise<void> {
    if (id !== this.liveGame) throw new Error('Reconnect to this game before sending an action.')
    attributeTo(this.currentAccount, 'play')
    const token = await getToken(this.currentAccount)
    if (!token) throw new Error('Lichess login unavailable.')
    const headers = authorize(token)
    const path = { gameId: id }
    switch (action) {
      case 'resign':
        await unwrap(client.POST('/api/board/game/{gameId}/resign', { params: { path }, headers }))
        return
      case 'abort':
        await unwrap(client.POST('/api/board/game/{gameId}/abort', { params: { path }, headers }))
        return
      case 'offerDraw':
      case 'acceptDraw':
      case 'declineDraw':
        await unwrap(
          client.POST('/api/board/game/{gameId}/draw/{accept}', {
            params: { path: { ...path, accept: action === 'declineDraw' ? false : 'yes' } },
            headers,
          }),
        )
        return
      case 'claimVictory':
        await unwrap(
          client.POST('/api/board/game/{gameId}/claim-victory', { params: { path }, headers }),
        )
        return
      case 'claimDraw':
        await unwrap(
          client.POST('/api/board/game/{gameId}/claim-draw', { params: { path }, headers }),
        )
        return
      case 'berserk':
        await unwrap(client.POST('/api/board/game/{gameId}/berserk', { params: { path }, headers }))
        return
      default:
        await unwrap(
          client.POST('/api/board/game/{gameId}/takeback/{accept}', {
            params: { path: { ...path, accept: action === 'takeback' ? 'yes' : false } },
            headers,
          }),
        )
    }
  }

  cancel(): void {
    this.attachments.invalidate()
    this.attachingAccount = ''
    // A correspondence game has no clock to protect: closing it frees the board at once.
    const leavingCorrespondence =
      this.liveCorrespondence && this.liveGame !== '' && this.protectedGame === this.liveGame
    if (leavingCorrespondence) this.protectedGame = ''
    this.resumeAttempt++
    this.presenceCache.clear()
    this.epoch++
    this.liveGame = ''
    this.liveCorrespondence = false
    this.stopLobby()
    this.controller?.abort()
    this.gameController?.abort()
    this.controller = null
    this.gameController = null
    if (this.protectedGame || this.recoveryUnverified)
      this.state({
        session: this.epoch,
        account: this.currentAccount,
        gameId: this.protectedGame,
        lane: 'game',
        phase: 'disconnected',
        message: 'Reconnect to verify the current game before using engine assistance.',
      })
    else if (leavingCorrespondence)
      this.state({
        session: this.epoch,
        account: this.currentAccount,
        gameId: '',
        lane: 'game',
        phase: 'idle',
      })
    if (this.pendingId) {
      const id = this.pendingId
      this.pendingId = ''
      void getToken(this.currentAccount)
        .then((token) => {
          if (!token) return undefined
          return this.cancelChallenge(id, token)
        })
        .catch((cause: unknown) => {
          logDebug('lichess', 'Pending challenge cancel failed:', id, cause)
          return undefined
        })
    }
    // Whatever replaces this session (start, attach) claims the event stream first; otherwise
    // the idle stream resumes so challenges keep arriving.
    queueMicrotask(() => this.startLobby())
  }

  /** Forget the session after explicit logout, including recovery and game identity. */
  logout(accounts?: string[]): void {
    if (accounts) {
      const names = new Set(accounts.map((name) => name.toLowerCase()))
      if (
        !names.has(this.attachingAccount.toLowerCase()) &&
        !names.has(this.currentAccount.toLowerCase()) &&
        !names.has(this.protectedAccount.toLowerCase()) &&
        !(this.recoveryUnverified && !this.currentAccount && !this.protectedAccount)
      ) {
        if (names.has(this.lobbyAccount.toLowerCase())) this.stayConnected('')
        return
      }
    }
    this.close()
    this.protectedGame = ''
    this.protectedAccount = ''
    this.currentAccount = ''
    this.recoveryUnverified = false
    this.state({ session: this.epoch, account: '', gameId: '', lane: 'game', phase: 'idle' })
  }

  /** Stop everything for good (the app is quitting). */
  close(): void {
    this.lobbyAccount = ''
    this.cancel()
    this.stopLobby()
  }
}

export function resetLichess(): void {
  serviceState.loginEpoch++
  for (const name of serviceState.syncs.keys())
    serviceState.accountEpochs.set(name, accountEpoch(name) + 1)
  serviceState.syncs.clear()
  serviceState.profileCache.clear()
  serviceState.lastFollowing.clear()
}

const serviceState = scopedState(() => ({
  loginEpoch: 0,
  accountEpochs: new Map<string, number>(),
  profileCache: new LRUCache<string, object, () => Promise<unknown>>({
    max: 100,
    ttl: PROFILE_TTL_MS,
    fetchMethod: async (key, _stale, { context: load }) => {
      const account = key.split(':')[0]!
      const epoch = accountEpoch(account)
      const value = (await load()) as object
      const current = (): boolean => epoch === accountEpoch(account)
      if (!current()) throw new Error('Account was logged out.')
      await writeApiCache(key, value, current).catch((error: unknown) =>
        logWarn('lichess-cache', 'Profile cache write failed:', key, error),
      )
      return value
    },
  }),
  lastFollowing: new Map<string, LichessUser>(),
  syncs: new Map<string, Promise<number>>(),
}))
