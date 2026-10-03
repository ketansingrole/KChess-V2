import { createHash, randomBytes } from 'node:crypto'
import { createServer } from 'node:http'
import { setTimeout as sleep } from 'node:timers/promises'
import { shell } from 'electron'
import { LRUCache } from 'lru-cache'
import createClient from 'openapi-fetch'
import { INITIAL_FEN } from 'chessops/fen'
import type { components, paths } from '@lichess-org/types'
import type {
  AppData,
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
  PresenceReport,
  Puzzle,
  PuzzleActivityEntry,
  PuzzleDashboard,
  PuzzleDraw,
  PuzzleRequest,
  PuzzleSolveRequest,
  PuzzleSolveResult,
  StormDashboard,
  Judgment,
  ReviewEval,
  StoredReview,
} from '../shared/types'
import { puzzleFromApi, type ApiPuzzle } from '../shared/puzzle'
import { lichessLine, replay, reviewKey, sanToUci } from '../shared/review'
import { markChecked, writeReview } from './reviewStore'
import { LichessError, throwLichessErrors } from '../shared/lichessError'
import { isGameInProgress } from '../shared/gameStatus'
import { lichessFetch } from './requestPolicy'
import { validateOnlineEvent } from '../shared/onlineEvent'
import { readLines } from './ndjson'
import { attributeTo, withUsage } from './usage'
import { canBoardSeek, canDirectChallenge, perfFor } from '../shared/timeControl'
import {
  MAX_GAMES,
  addAccount,
  dismissedFriends,
  getToken,
  loadData,
  pendingGameIds,
  readApiCache,
  saveGames,
  saveGamesPage,
  saveToken,
  writeApiCache,
} from './store'

const BASE = 'https://lichess.org'
const client = createClient<paths>({ baseUrl: BASE, fetch: lichessFetch })
client.use(throwLichessErrors)
const OAUTH_CLIENT_ID = 'kchess-desktop'

type StreamCall = { data?: ReadableStream<Uint8Array> | null; response: Response }

/** Failed responses already threw in the middleware; this guards against an empty success body. */
async function unwrap<T>(call: Promise<{ data?: T | null; response: Response }>): Promise<T> {
  const { data, response } = await call
  if (data === undefined || data === null)
    throw new LichessError(response.status, new URL(response.url).pathname, 'empty response')
  return data
}

const asError = (cause: unknown): Error =>
  cause instanceof Error ? cause : new Error(String(cause))

const authorize = (token: string): Record<string, string> => ({ Authorization: `Bearer ${token}` })

const urlencoded = (body: unknown): URLSearchParams => {
  const params = new URLSearchParams()
  if (body)
    for (const [key, value] of Object.entries(body as Record<string, unknown>))
      if (value !== undefined) params.set(key, String(value))
  return params
}

/** Profile data changes slowly; a few minutes of reuse saves a round trip per page visit. */
const PROFILE_TTL_MS = 5 * 60_000

const profileKey = (username: string, kind: 'profile' | 'rating'): string =>
  `${username.toLowerCase()}:${kind}`

/** TTL memory cache; `fetch` also makes concurrent callers for a key share one request. */
const profileCache = new LRUCache<string, object, () => Promise<unknown>>({
  max: 100,
  ttl: PROFILE_TTL_MS,
  fetchMethod: async (key, _stale, { context: load }) => {
    const value = (await load()) as object
    await writeApiCache(key, value).catch(() => undefined)
    return value
  },
})

/**
 * Memory first, then the network. The result is also persisted, and if
 * Lichess is unreachable the last persisted copy is served instead of an error.
 */
async function cachedFetch<T extends object>(key: string, load: () => Promise<T>): Promise<T> {
  try {
    return (await profileCache.fetch(key, { context: load })) as T
  } catch (cause) {
    if (cause instanceof LichessError && cause.status === 404) throw cause
    const stale = await readApiCache<T>(key).catch(() => null)
    if (stale) return stale.value
    throw cause
  }
}

/** Drop the in-memory copies so the next read refetches (after a sync changes ratings and counts). */
export function forgetProfile(username: string): void {
  profileCache.delete(profileKey(username, 'profile'))
  profileCache.delete(profileKey(username, 'rating'))
}

/** Last persisted profile data, read locally with no network; lets the dashboard paint at once. */
export async function cachedProfile(username: string): Promise<{
  profile: LichessUser | null
  ratingHistory: LichessRatingHistory | null
  /** When the saved profile was fetched, so callers can skip refreshing a recent one. */
  profileFetchedAt?: number
}> {
  const [profileHit, ratingHit] = await Promise.all([
    readApiCache<LichessUser>(profileKey(username, 'profile')).catch(() => null),
    readApiCache<LichessRatingHistory>(profileKey(username, 'rating')).catch(() => null),
  ])
  return {
    profile: profileHit?.value ?? null,
    ratingHistory: ratingHit?.value ?? null,
    profileFetchedAt: profileHit?.fetchedAt,
  }
}

/** Full profiles from the last "who do I follow" fetch, kept so adding a friend needs no further request. */
const lastFollowing = new Map<string, LichessUser>()

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
  lastFollowing.clear()
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
        lastFollowing.set(key, user)
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
      problems.push({
        account: account.username,
        message: denied
          ? 'Lichess needs your permission to read who this account follows.'
          : asError(cause).message,
        needsReconnect: denied,
      })
    }
  }
  const users = [...byName.values()]
    .sort((a, b) => a.username.localeCompare(b.username))
    .slice(0, 1000)
  return { users, problems }
}

/** Save the profiles fetched by `followedUsers` for the players just added, sparing a request each. */
export async function primeProfiles(usernames: string[]): Promise<void> {
  for (const name of usernames) {
    const user = lastFollowing.get(name.toLowerCase())
    if (user) await writeApiCache(profileKey(name, 'profile'), user).catch(() => undefined)
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

export async function ratingHistory(username: string): Promise<LichessRatingHistory> {
  return cachedFetch(profileKey(username, 'rating'), () =>
    withUsage(username, 'profile', () =>
      unwrap(
        client.GET('/api/user/{username}/rating-history', { params: { path: { username } } }),
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
  const { fen, moves } = lichessLine({ moves: raw.moves ?? '', initialFen: raw.initialFen })
  if (!moves.length) return undefined
  const positions = replay(fen, moves)
  const evals: (ReviewEval | null)[] = Array.from({ length: moves.length + 1 }, () => null)
  // Lichess scores the standard start as +0.15 when it works out accuracy.
  if (fen === INITIAL_FEN) evals[0] = { cp: 15 }
  const judgments: (Judgment | null)[] = moves.map(() => null)
  raw.analysis.slice(0, moves.length).forEach((entry, i) => {
    // Entry i scores the position after move i+1; its `best` is what should have been played.
    const score =
      entry.mate !== undefined
        ? { mate: entry.mate }
        : entry.eval !== undefined
          ? { cp: entry.eval }
          : null
    if (score) evals[i + 1] = { ...evals[i + 1], ...score }
    if (entry.best) {
      const pv = entry.variation
        ? sanToUci(positions[i]?.fen ?? fen, entry.variation.split(/\s+/))
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
async function asOwner<T>(
  account: string,
  call: (auth: Record<string, string>) => Promise<T>,
): Promise<T> {
  const token = await getToken(account).catch(() => null)
  if (!token) return call({})
  try {
    return await call(authorize(token))
  } catch (cause) {
    if (cause instanceof LichessError && cause.status === 401) return call({})
    throw cause
  }
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
  await readLines(stream, (line) => {
    const raw = JSON.parse(line) as components['schemas']['GameJson']
    page.games.push(normalizeGame(raw, account))
    const review = reviewFromLichess(raw)
    if (review) page.reviews.push(review)
    if (
      !isGameInProgress(raw.status) &&
      (review || Date.now() - raw.lastMoveAt > ANALYSIS_SETTLED_MS)
    )
      page.settled.push(raw.id)
  })
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
  const found: StoredReview[] = []
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
    await readLines(stream, (line) => {
      const review = reviewFromLichess(JSON.parse(line) as components['schemas']['GameJson'])
      if (review) found.push(writeReview(review).review)
    })
  })
  markChecked(ids.slice(0, 300))
  return found
}

/** Re-fetch unfinished games by ID, even when their creation predates the next sync window. */
async function refreshPendingGames(account: string): Promise<void> {
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
    await readLines(stream, (line) => {
      const raw = JSON.parse(line) as components['schemas']['GameJson']
      if (!requested.has(raw.id)) return
      games.push(normalizeGame(raw, account))
      const review = reviewFromLichess(raw)
      if (review) reviews.push(review)
    })
    // Missing IDs remain pending; a failed request never advances the sync cursor.
    await saveGamesPage(account, games, reviews)
  }
}

const syncs = new Map<string, Promise<void>>()

/** One sync per account at a time: a second request joins the running one. */
function syncAccount(account: LichessAccount): Promise<void> {
  const key = account.username.toLowerCase()
  const running = syncs.get(key)
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
      if (games.length) await saveGamesPage(account.username, games, reviews)
      markChecked(settled)
      fetched += games.length
      if (games.length < SYNC_PAGE) break
      const oldest = games.reduce((min, g) => Math.min(min, g.createdAt), Infinity)
      if (until !== undefined && oldest >= until) break
      until = oldest
    }
    await saveGames(account.username, [], startedAt)
    forgetProfile(account.username)
  }).finally(() => syncs.delete(key))
  syncs.set(key, run)
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
      firstError ??= cause
    }
  }
  if (firstError) throw firstError
  return loadData()
}

export async function connectLichess(): Promise<{ data: AppData; username: string }> {
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
        Connection: 'close',
      })
      response.end(
        authorizationCode
          ? '<h2>KChess connected to Lichess</h2><p>You can return to the app.</p>'
          : '<h2>Lichess login was not completed</h2><p>You can close this tab and try again in KChess.</p>',
        () =>
          authorizationCode
            ? finish(undefined, { code: authorizationCode, redirect })
            : finish(new Error('Lichess login was not completed.')),
      )
    })
    const timeout = setTimeout(() => finish(new Error('Lichess login timed out.')), 300_000)
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
        scope: 'board:play challenge:write follow:read puzzle:read puzzle:write',
        code_challenge_method: 'S256',
        code_challenge: challenge,
        state,
      }))
        url.searchParams.set(key, value)
      shell.openExternal(url.toString()).catch((cause) => finish(asError(cause)))
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
  await saveToken(account.username, token.access_token)
  return { data: await addAccount(account.username, true), username: account.username }
}

/**
 * Run a puzzle request as `account`. A missing login, or one Lichess refuses (it was connected
 * before puzzles were requested, or was revoked), answers "reconnect" instead of failing.
 */
async function asAccount<T>(
  account: string,
  work: (token: string) => Promise<T>,
): Promise<T | NeedsReconnect> {
  const token = await getToken(account)
  if (!token) return { needsReconnect: true }
  try {
    return await withUsage(account, 'puzzles', () => work(token))
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
}

export class OnlineSession {
  private controller: AbortController | null = null
  private gameController: AbortController | null = null
  /** The game whose stream is open, while it lasts. */
  private liveGame = ''
  private protectedGame = ''
  private currentAccount = ''
  private pendingId = ''
  private epoch = 0
  private resumeAttempt = 0
  private presenceCache = new LRUCache<string, Promise<PresenceReport>>({ max: 100, ttl: 4000 })
  constructor(
    private emit: (event: OnlineEvent) => void,
    private error: (error: string) => void,
    private state: (state: OnlineConnection) => void = () => {},
  ) {}

  private report(cause: unknown): void {
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
      if (!signal.aborted && session === this.epoch)
        this.state({ session, account, gameId, lane: options.lane ?? 'events', phase, message })
    }
    state('connecting')
    while (!signal.aborted) {
      let received = false
      try {
        const stream = await unwrap(open())
        signal.throwIfAborted()
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
        const permanent =
          cause instanceof LichessError && cause.status < 500 && cause.status !== 429
        if (!options.reconnect || permanent) {
          state(
            cause instanceof LichessError && [401, 403].includes(cause.status)
              ? 'auth-required'
              : 'disconnected',
            asError(cause).message,
          )
          this.report(cause)
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
        state('disconnected', 'Lost connection to Lichess. Reconnect to recover the game.')
        this.error('Lost connection to Lichess. Check your network and reopen the game.')
        return
      }
      state('reconnecting', 'Connection interrupted. Reconnecting…')
      // Resolves early (rejects) if the session is cancelled while backing off; the loop then exits.
      await sleep(Math.min(15_000, 500 * 2 ** failures), undefined, { signal }).catch(
        () => undefined,
      )
    }
  }

  private eventStream(
    token: string,
    signal: AbortSignal,
    emit: (event: OnlineEvent) => void,
  ): Promise<void> {
    return this.listen(
      () =>
        client.GET('/api/stream/event', { headers: authorize(token), parseAs: 'stream', signal }),
      signal,
      emit,
      { reconnect: true, lane: 'events' },
    )
  }

  /** Reattach to a game in progress on any connected account. */
  async resume(): Promise<{ id: string; account: string } | null> {
    const attempt = ++this.resumeAttempt
    const accounts = (await loadData()).accounts.filter((a) => a.connected)
    let firstError: unknown
    for (const account of accounts) {
      try {
        const token = await getToken(account.username)
        if (!token) continue
        const playing = await withUsage(account.username, 'play', () =>
          unwrap(client.GET('/api/account/playing', { headers: authorize(token) })),
        )
        const id = playing.nowPlaying?.[0]?.gameId
        if (attempt !== this.resumeAttempt) return null
        if (!id) continue
        this.cancel()
        this.currentAccount = account.username
        attributeTo(account.username, 'play')
        this.controller = new AbortController()
        void this.eventStream(token, this.controller.signal, (event) => this.emit(event))
        void this.openGame(id, token)
        return { id, account: account.username }
      } catch (cause) {
        // One account with a revoked login must not hide a game on another.
        firstError ??= cause
      }
    }
    if (firstError) throw firstError
    this.protectedGame = ''
    this.state({
      session: this.epoch,
      account: this.currentAccount,
      gameId: '',
      lane: 'game',
      phase: 'idle',
    })
    return null
  }

  async start(options: OnlineOptions): Promise<{ id?: string; url?: string; seeking?: boolean }> {
    if (this.playing) throw new Error('Reconnect to your current game before finding another.')
    this.cancel()
    const target = options.target?.trim()
    const perf = perfFor(options.minutes, options.increment)
    const label = `${options.minutes}+${options.increment}`
    // Validate before opening any stream so a rejected request leaves nothing running.
    if (target && !canDirectChallenge(options.minutes, options.increment))
      throw new Error(
        `Lichess blocks ${perf} (${label}) even for direct challenges. The Board API allows Blitz and slower — pick at least 3 minutes of estimated play (time + 40 × increment).`,
      )
    if (!target && !canBoardSeek(options.minutes, options.increment))
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
      let gameStarted = false
      void this.eventStream(token, controller.signal, (event) => {
        if (controller.signal.aborted) return
        if (event.type === 'gameStart') {
          gameStarted = true
          this.pendingId = ''
          void this.openGame(event.game.gameId, token)
        }
        this.emit(event)
      })
      if (target) {
        try {
          const challenge = await unwrap(
            client.POST('/api/challenge/{username}', {
              params: { path: { username: target } },
              body: {
                'clock.limit': options.minutes * 60,
                'clock.increment': options.increment,
                rated: options.rated,
                color: options.color,
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
          if (!gameStarted) this.pendingId = challenge.id
          return { id: challenge.id, url: challenge.url }
        } catch (cause) {
          if (cause instanceof LichessError && cause.status === 400)
            throw new Error(
              `Lichess rejected this ${perf} (${label}) challenge. The Board API allows Blitz and slower for direct challenges — pick a longer control.`,
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
              color: options.color,
              rated: options.rated,
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
      if (this.controller === controller) this.controller = null
      throw cause
    }
  }

  private async cancelChallenge(id: string, token: string): Promise<void> {
    await unwrap(
      client.POST('/api/challenge/{challengeId}/cancel', {
        params: { path: { challengeId: id } },
        headers: authorize(token),
      }),
    ).catch(() => undefined)
  }

  private async openGame(id: string, token: string): Promise<void> {
    this.gameController?.abort()
    const controller = new AbortController()
    this.gameController = controller
    let finished = false
    this.liveGame = id
    this.protectedGame = id
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
        const raw = event as { type: string; status?: string; state?: { status?: string } }
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

  async move(id: string, uci: string): Promise<void> {
    if (id !== this.liveGame) throw new Error('Reconnect to this game before sending a move.')
    const token = await getToken(this.currentAccount)
    if (!token) throw new Error('Lichess login unavailable.')
    await withUsage(this.currentAccount, 'play', () =>
      unwrap(
        client.POST('/api/board/game/{gameId}/move/{move}', {
          params: { path: { gameId: id, move: uci } },
          headers: authorize(token),
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
    void request.catch(() => this.presenceCache.delete(key))
    return request
  }
  private async fetchPresence(usernames: string[], account: string): Promise<PresenceReport> {
    const token = account ? await getToken(account) : null
    const started = performance.now()
    const rows = await withUsage(account, 'presence', () =>
      unwrap(
        client.GET('/api/users/status', {
          params: { query: { ids: usernames.join(','), withSignal: true } },
          headers: token ? authorize(token) : {},
        }),
      ),
    )
    const latencyMs = Math.round(performance.now() - started)
    const users: PresenceReport['users'] = {}
    // The generated type predates `withSignal`, so `signal` is read off the raw row.
    for (const row of rows as ((typeof rows)[number] & { signal?: number })[])
      users[row.id.toLowerCase()] = {
        online: Boolean(row.online),
        playing: Boolean(row.playing),
        signal: row.signal,
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
    if (action === 'resign')
      await unwrap(client.POST('/api/board/game/{gameId}/resign', { params: { path }, headers }))
    else if (action === 'abort')
      await unwrap(client.POST('/api/board/game/{gameId}/abort', { params: { path }, headers }))
    else
      await unwrap(
        client.POST('/api/board/game/{gameId}/takeback/{accept}', {
          params: { path: { ...path, accept: action === 'takeback' ? 'yes' : false } },
          headers,
        }),
      )
  }

  cancel(): void {
    this.resumeAttempt++
    this.presenceCache.clear()
    this.epoch++
    this.liveGame = ''
    this.controller?.abort()
    this.gameController?.abort()
    this.controller = null
    this.gameController = null
    if (this.protectedGame)
      this.state({
        session: this.epoch,
        account: this.currentAccount,
        gameId: this.protectedGame,
        lane: 'game',
        phase: 'disconnected',
        message: 'Reconnect to verify the current game before using engine assistance.',
      })
    if (this.pendingId) {
      const id = this.pendingId
      this.pendingId = ''
      void getToken(this.currentAccount)
        .then((token) => {
          if (!token) return undefined
          return this.cancelChallenge(id, token)
        })
        .catch(() => undefined)
    }
  }
}
