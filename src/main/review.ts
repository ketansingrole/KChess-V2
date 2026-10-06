import {
  endEval,
  hasScore,
  isReviewablePerf,
  lichessLine,
  replay,
  reviewKey,
  type ReplayedPosition,
} from '../shared/review'
import type {
  ReviewEval,
  ReviewRequest,
  ReviewStatus,
  ReviewUpdate,
  Settings,
  StoredReview,
} from '../shared/types'
import { parseInfo } from '../shared/uciInfo'
import { UciController, SearchCancelled } from './uci'
import { withEngineLease, searchThreads } from './engineScheduler'
import { engineIdentity, engineStatus, spawnEngine } from './engine'
import { gamesToReview, hasAccount, markChecked, readReview, writeReview } from './reviewStore'
import { logDebug, logWarn } from './logger'

/**
 * Game review: Stockfish scores every position of a game, one after another, in its own process
 * (the analysis board and the computer opponent keep theirs). Reviews someone asks for run at
 * once, first a quick pass so labels show within seconds and then a deeper one that refines them.
 * Automatic reviews of synced games run one at a time on a single thread, and only while nothing
 * else needs the computer: no online game, no engine in use, and (unless allowed) not on battery.
 * A Lichess game is first looked up on Lichess, in case it was already analysed there.
 */

/** Depth of the quick first pass, and of the review that is kept. */
const QUICK_DEPTH = 10
export const FULL_DEPTH = 18
const QUICK_MS = 400
const FULL_MS = 5_000
/** Positions between saves of a review in progress. */
const SAVE_EVERY = 6
/** How often paused automatic reviews look again whether they may run. */
const RECHECK_MS = 30_000
/** Free the engine after this long with nothing to review. */
const IDLE_MS = 60_000
const RECENT_MS = 30 * 86_400_000
const PV_LENGTH = 10

export interface ReviewHost {
  settings: () => Promise<Settings>
  /** The accounts whose games are reviewed automatically. */
  accounts: () => Promise<string[]>
  onBattery: () => boolean
  /** Something in the foreground (an online game, the analysis board) wants the computer. */
  busy: () => 'engine' | 'online' | undefined
  /** Ask Lichess for its analysis of these games of `account`; saves and returns what it has. */
  fetchLichess: (account: string, ids: string[]) => Promise<StoredReview[]>
  update: (update: ReviewUpdate) => void
  status: (status: ReviewStatus) => void
}

interface Job {
  key: string
  fen: string
  moves: string[]
  positions: ReplayedPosition[]
  gameId?: string
  account?: string
  background: boolean
  cancelled?: boolean
  discarded?: boolean
}

let host: ReviewHost | null = null
const asked: Job[] = []
let reviewEpoch = 0
let running: Job | null = null
let progress: ReviewStatus['current']
let paused: ReviewStatus['paused']
let waitingBackground = 0
let failed: ReviewStatus['failed']
/** Games that could not be reviewed this session (unreadable moves, engine failures). */
const skipped = new Set<string>()
let pumping = false
let recheck: ReturnType<typeof setTimeout> | undefined

export function setupReviews(next: ReviewHost): void {
  host = next
  scheduleRecheck(10_000)
}

/* ── The engine ───────────────────────────────────────────────────── */

interface Engine {
  key: string
  uci: UciController
}
let engine: Engine | null = null
let searchController: AbortController | undefined
let idleTimer: ReturnType<typeof setTimeout> | undefined
function write(target: Engine, command: string): void {
  target.uci.write(command)
}
async function openEngine(configured: string): Promise<Engine> {
  clearTimeout(idleTimer)
  const status = await engineStatus(configured)
  if (!status.ready) throw new Error('Stockfish could not be found. Choose an engine in Settings.')
  const key = await engineIdentity(status)
  if (engine?.key !== key || engine.uci.failed) {
    closeEngine()
    engine = { key, uci: new UciController(spawnEngine(status)) }
  }
  await engine.uci.ready
  return engine
}
async function search(
  target: Engine,
  job: Job,
  index: number,
  depth: number,
  ms: number,
): Promise<ReviewEval> {
  const controller = new AbortController()
  searchController = controller
  let score: ReviewEval = {}
  try {
    return await withEngineLease(
      1,
      () => controller.abort(),
      controller.signal,
      async () => {
        if (job.cancelled || !host || host.busy() === 'online') throw new SearchCancelled()
        write(target, `setoption name Threads value ${job.background ? 1 : searchThreads()}`)
        await target.uci.sync()
        controller.signal.throwIfAborted()
        write(
          target,
          `position fen ${job.fen}${index ? ` moves ${job.moves.slice(0, index).join(' ')}` : ''}`,
        )
        await target.uci.search(
          `go depth ${depth} movetime ${ms}`,
          (line) => {
            const parsed = parseInfo(line, job.positions[index]!.turn === 'white')
            if (!parsed || parsed.line.rank !== 1) return
            const { cp, mate, pv, depth } = parsed.line
            score = {
              ...(mate !== undefined ? { mate } : { cp }),
              best: pv[0],
              pv: pv.slice(0, PV_LENGTH),
              depth,
            }
          },
          ms + 10_000,
          controller.signal,
        )
        return score
      },
    )
  } catch (error) {
    if (controller.signal.aborted) throw new SearchCancelled()
    throw error
  } finally {
    if (searchController === controller) searchController = undefined
  }
}
function closeEngine(): void {
  clearTimeout(idleTimer)
  searchController?.abort()
  engine?.uci.close()
  engine = null
}

/* ── The queue ────────────────────────────────────────────────────── */

function jobFor(request: ReviewRequest, background: boolean): Job | null {
  const positions = replay(request.fen, request.moves)
  if (positions.length < 2) return null
  const moves = request.moves.slice(0, positions.length - 1)
  const fen = positions[0]!.fen
  return {
    key: reviewKey(fen, moves),
    fen,
    moves,
    positions,
    gameId: request.gameId,
    account: request.account,
    background,
  }
}

function emptyReview(job: Job): StoredReview {
  return {
    key: job.key,
    fen: job.fen,
    moves: job.moves,
    source: 'local',
    evals: job.positions.map(() => null),
    depth: FULL_DEPTH,
    complete: false,
    updatedAt: Date.now(),
    gameId: job.gameId,
  }
}

function sendStatus(): void {
  host?.status(reviewStatus())
}

export function reviewStatus(): ReviewStatus {
  return {
    ...(progress ? { current: progress } : {}),
    waiting: asked.length + waitingBackground,
    ...(paused ? { paused } : {}),
    ...(failed ? { failed } : {}),
  }
}

/** The stored review of these moves, if any (`fen` and moves as the caller has them). */
export function getReview(fen: string, moves: string[]): StoredReview | null {
  const positions = replay(fen, moves)
  if (!positions.length) return null
  return readReview(reviewKey(positions[0]!.fen, moves.slice(0, positions.length - 1)))
}

/**
 * Review a game now, ahead of automatic reviews. Returns what is already stored (complete or
 * not); updates arrive as the review goes on.
 */
export function requestReview(request: ReviewRequest): StoredReview | null {
  const job = jobFor(request, false)
  if (!job) return null
  // A request sent just before its account was logged out or removed must not store anything.
  if (job.account && !hasAccount(job.account)) return null
  const cached = readReview(job.key)
  // Link the requesting game even when cached output is returned or its search is already queued.
  const stored = job.gameId
    ? writeReview({ ...(cached ?? emptyReview(job)), gameId: job.gameId }).review
    : cached
  if (stored?.complete && stored.source === 'lichess') return stored
  if (running?.key === job.key) {
    running.background = false
    return stored
  }
  const queued = asked.findIndex((other) => other.key === job.key)
  if (queued >= 0) asked.splice(queued, 1)
  // The game asked for most recently is the one being looked at: it goes first.
  asked.unshift(job)
  skipped.delete(job.key)
  if (failed?.key === job.key) failed = undefined
  sendStatus()
  void pump()
  return stored
}

/** Stop reviewing these moves (what is done so far is kept). */
export function cancelReview(key: string): void {
  const queued = asked.findIndex((job) => job.key === key)
  if (queued >= 0) asked.splice(queued, 1)
  if (running?.key === key) {
    running.cancelled = true
    searchController?.abort()
  }
  sendStatus()
}

/** Logout must prevent an interrupted review from restoring deleted account data. */
export function discardAccountReviews(accounts: string[]): void {
  reviewEpoch++
  const names = new Set(accounts.map((a) => a.toLowerCase()))
  for (let i = asked.length - 1; i >= 0; i--)
    if (names.has(asked[i]!.account?.toLowerCase() ?? '')) asked.splice(i, 1)
  if (running && names.has(running.account?.toLowerCase() ?? '')) {
    running.cancelled = true
    running.discarded = true
    searchController?.abort()
  }
  sendStatus()
}

/** Settings changed or a sync finished: look again for work. */
export function reviewsChanged(): void {
  void pump()
}

/** The engine was deleted or replaced: drop it (the review in progress stops, keeping its work). */
export function restartReviewEngine(): void {
  if (running) running.cancelled = true
  closeEngine()
}

/** The app is quitting. */
export function stopReviews(): void {
  asked.length = 0
  if (running) running.cancelled = true
  clearTimeout(recheck)
  closeEngine()
  host = null
}

function scheduleRecheck(ms = RECHECK_MS): void {
  clearTimeout(recheck)
  recheck = setTimeout(() => void pump(), ms)
}

/** Why automatic reviews may not run now, if they may not. */
async function backgroundBlock(): Promise<ReviewStatus['paused']> {
  if (!host) return 'off'
  const settings = await host.settings()
  if (settings.reviewAuto === 'off') return 'off'
  const busy = host.busy()
  if (busy) return busy
  if (!settings.reviewOnBattery && host.onBattery()) return 'battery'
  return undefined
}

/** The next synced game to review automatically, looking it up on Lichess first. */
async function nextBackgroundJob(): Promise<Job | null> {
  const epoch = reviewEpoch
  if (!host) return null
  const settings = await host.settings()
  const accounts = await host.accounts()
  if (epoch !== reviewEpoch) return null
  const since = settings.reviewAuto === 'recent' ? Date.now() - RECENT_MS : 0
  const games = gamesToReview(accounts, since).filter(
    (game) => !skipped.has(game.id) && isReviewablePerf(game.perf),
  )
  waitingBackground = games.length
  // Games not yet looked up on Lichess go there first, a batch per account.
  const unchecked = games.filter((game) => !game.checked)
  if (unchecked.length) {
    const account = unchecked[0]!.account
    const ids = unchecked.filter((game) => game.account === account).map((game) => game.id)
    try {
      const found = await host.fetchLichess(account, ids)
      if (epoch !== reviewEpoch) return null
      for (const review of found) host.update(updateOf(review))
    } catch (cause) {
      logWarn('review', 'Lichess review lookup failed, reviewing locally:', account, cause)
      if (epoch !== reviewEpoch) return null
      // Offline or rate limited: review locally rather than wait.
      markChecked(ids)
    }
    return nextBackgroundJob()
  }
  for (const game of games) {
    const line = lichessLine(game)
    const job = jobFor({ ...line, gameId: game.id, account: game.account }, true)
    if (job && readReview(job.key)?.complete) {
      // Reviewed already (opened from the analysis board): just link it to the game.
      writeReview({ ...readReview(job.key)!, gameId: game.id })
      continue
    }
    if (job) return job
    skipped.add(game.id)
  }
  waitingBackground = 0
  return null
}

function updateOf(review: StoredReview): ReviewUpdate {
  const { review: saved, summary } = writeReview(review)
  return { review: saved, summary }
}

async function pump(): Promise<void> {
  if (pumping || !host) return
  pumping = true
  try {
    while (host) {
      if (host.busy() === 'online') {
        paused = 'online'
        sendStatus()
        scheduleRecheck()
        break
      }
      let job = asked.shift() ?? null
      if (!job) {
        paused = await backgroundBlock()
        if (paused) {
          sendStatus()
          scheduleRecheck()
          break
        }
        job = await nextBackgroundJob()
        if (!job) {
          sendStatus()
          scheduleRecheck(5 * 60_000)
          break
        }
      }
      paused = undefined
      await run(job)
    }
  } finally {
    pumping = false
    progress = undefined
    sendStatus()
    if (!running && !asked.length) {
      clearTimeout(idleTimer)
      idleTimer = setTimeout(closeEngine, IDLE_MS)
    }
  }
}

/** Should the job in progress give way (to a game someone asked for, or to the foreground)? */
async function shouldYield(job: Job): Promise<boolean> {
  if (job.cancelled || !host || host.busy() === 'online') return true
  // A game asked for more recently goes first; this one carries on after it.
  if (asked.length) return true
  if (!job.background) return false
  return Boolean(await backgroundBlock())
}

async function run(job: Job): Promise<void> {
  if (!host) return
  const publish = (value: StoredReview): void => {
    if (!job.discarded) host?.update(updateOf(value))
  }
  running = job
  const total = job.positions.length
  let review = readReview(job.key) ?? emptyReview(job)
  if (review.source === 'lichess' && review.complete) {
    running = null
    return
  }
  try {
    // A Lichess game someone opened: Lichess may have analysed it already.
    if (!job.background && job.gameId && job.account) {
      const found = await host.fetchLichess(job.account, [job.gameId]).catch((error: unknown) => {
        logDebug('review', 'Lichess review fetch failed:', job.gameId ?? job.key, error)
        return []
      })
      const match = found.find((r) => r.key === job.key)
      if (match) {
        publish(match)
        return
      }
    }
    review = { ...review, gameId: job.gameId ?? review.gameId, evals: [...review.evals] }
    while (review.evals.length < total) review.evals.push(null)
    const settings = await host.settings()
    const target = await openEngine(settings.enginePath)
    if (review.engine !== target.key)
      review = {
        ...review,
        engine: target.key,
        complete: false,
        evals: job.positions.map(() => null),
      }
    write(target, 'ucinewgame')
    write(target, 'setoption name Hash value 64')
    const passes: { depth: number; ms: number }[] = job.background
      ? [{ depth: FULL_DEPTH, ms: FULL_MS }]
      : [
          { depth: QUICK_DEPTH, ms: QUICK_MS },
          { depth: FULL_DEPTH, ms: FULL_MS },
        ]
    for (const pass of passes) {
      let sinceSave = 0
      let done = 0
      for (let i = 0; i < total; i++) {
        const position = job.positions[i]!
        const known = review.evals[i]
        if (position.end) review.evals[i] = endEval(position.end)
        else if (!known || !hasScore(known) || (known.depth ?? 0) < pass.depth) {
          if (await shouldYield(job)) {
            publish(review)
            if (!job.cancelled && !job.background) asked.push(job)
            return
          }
          const score = await search(target, job, i, pass.depth, pass.ms)
          if (hasScore(score)) review.evals[i] = score
          sinceSave++
        }
        done++
        progress = { key: job.key, gameId: job.gameId, done, total, background: job.background }
        sendStatus()
        // The quick pass shows as it goes; the deep one replaces it a few positions at a time.
        if (pass.depth === QUICK_DEPTH || sinceSave >= SAVE_EVERY) {
          publish(review)
          sinceSave = 0
        }
      }
    }
    review.complete = review.evals.every((score) => score && hasScore(score))
    publish(review)
    if (!review.complete) skipped.add(job.gameId ?? job.key)
  } catch (cause) {
    if (
      job.cancelled ||
      cause instanceof SearchCancelled ||
      (searchController?.signal.aborted && !engine?.uci.failed)
    ) {
      logDebug('review', 'Review interrupted:', job.key, cause)
      if (review.evals.some(Boolean)) publish(review)
      if (!job.cancelled && !job.background && host) asked.push(job)
      return
    }
    logWarn('review', 'Review failed:', job.key, job.gameId, cause)
    // The engine is missing or failed: try other games, and this one again next session.
    skipped.add(job.gameId ?? job.key)
    if (!job.background)
      failed = {
        key: job.key,
        message: cause instanceof Error ? cause.message : 'Stockfish could not review this game.',
      }
    closeEngine()
    if (review.evals.some(Boolean)) publish(review)
  } finally {
    running = null
  }
}
