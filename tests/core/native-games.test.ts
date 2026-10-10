import { afterEach, describe, it, vi } from 'vitest'
import { goldenFile } from './golden.ts'
import '@kchess/native/rules'
import {
  ComputerGame,
  computerState,
  LocalGame,
  localState,
  type ComputerHost,
} from '@kchess/rules/gameSession'
import { archiveState, GameArchive, type ArchiveHost } from '@kchess/rules/gameArchive'
import { setupPositionAfter } from '@kchess/rules/chess'
import { INITIAL_FEN, Position, type LegalMove } from '@kchess/rules/position'
import {
  chess960Fen,
  defaultFen,
  STANDARD_SETUP,
  VARIANTS,
  type GameSetup,
} from '@kchess/rules/variant'
import { ENGINE_LEVELS } from '@kchess/contracts/types'
import type {
  ArchivedGame,
  ComputerSession,
  GameSnapshot,
  LocalSession,
} from '@kchess/rules/library'

/**
 * The game session classes (`crates/kchess-wasm/js/gameSession.ts`, `gameArchive.ts`) driven by seeded
 * scripts with scripted hosts: a controllable clock, availability and readiness toggles, engine
 * replies resolved, rejected or left pending in seeded order, stop requests, disposal, takebacks,
 * resignations, clocks that expire, chess960 and non-standard starts. Each case records every
 * observable step: the return value, the session state, the host reads (in order), the host
 * effects (in order) and the getters. The digests (`golden/games.json`) were recorded from the
 * TypeScript classes before they moved to Rust (`KCHESS_WRITE_GOLDEN=1`); the same scripts must
 * produce the same records from the Rust-backed classes.
 */

const golden = goldenFile('games')

/** Seeded generator, as in `native-rules.test.ts`. */
function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
type Rand = () => number

function pick<T>(r: Rand, items: readonly T[]): T {
  return items[Math.floor(r() * items.length)]!
}

/** A weighted choice from `[item, weight]` pairs. */
function weighted<T>(r: Rand, table: readonly [T, number][]): T {
  const total = table.reduce((sum, [, weight]) => sum + weight, 0)
  let at = r() * total
  for (const [item, weight] of table) {
    at -= weight
    if (at < 0) return item
  }
  return table[table.length - 1]![0]
}

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

const PROMOTION: Record<string, string> = {
  knight: 'n',
  bishop: 'b',
  rook: 'r',
  queen: 'q',
  king: 'k',
  pawn: 'p',
}
const uciOf = (move: LegalMove): string =>
  `${move.from}${move.to}${move.promotion ? PROMOTION[move.promotion] : ''}`

/** `count` random legal moves from `setup`, as the setup that results. */
function playRandom(setup: GameSetup, r: Rand, count: number): GameSetup {
  let position = Position.from(setup)
  if (!position) return setup
  for (let i = 0; i < count; i++) {
    const legal = position.legalMoves()
    if (!legal.length) break
    const next = position.play(uciOf(pick(r, legal)))
    if (!next) break
    position = next.position
  }
  return { variant: setup.variant, fen: position.fen }
}

/** A start: standard, another variant, Chess960, a played-on or illegal position. */
function randomSetup(r: Rand): GameSetup {
  const kind = r()
  if (kind < 0.35) return { variant: 'standard', fen: INITIAL_FEN }
  if (kind < 0.5) {
    const variant = pick(r, VARIANTS)
    return { variant, fen: defaultFen(variant) }
  }
  if (kind < 0.6) return { variant: 'chess960', fen: chess960Fen(Math.floor(r() * 960)) }
  if (kind < 0.67) return { variant: 'standard', fen: '8/8/8/8/8/8/8/8 w - - 0 1' }
  const variant = pick(r, VARIANTS)
  return playRandom({ variant, fen: defaultFen(variant) }, r, 2 + Math.floor(r() * 30))
}

const MINUTES = [0.05, 0.05, 0.2, 1, 3, 10]
const INCREMENTS = [0, 1, 3]

function randomClock(r: Rand): { minutes: number; increment: number } | null {
  if (r() < 0.4) return null
  return { minutes: pick(r, MINUTES), increment: pick(r, INCREMENTS) }
}

function randomLocalClock(r: Rand): LocalSession['clock'] {
  if (r() < 0.4) return null
  return {
    white: randomClock(r) ?? { minutes: 3, increment: 2 },
    black: randomClock(r) ?? { minutes: 3, increment: 0 },
  }
}

/** The legal move sequence of a seeded game, as a saved computer session. */
function randomSavedComputer(r: Rand): ComputerSession {
  const setup = randomSetup(r)
  const played = playRandom(setup, r, Math.floor(r() * 25))
  const moves: string[] = []
  let position = Position.from(played)
  for (let i = 0; position && i < 25 && r() < 0.9; i++) {
    const legal = position.legalMoves()
    if (!legal.length) break
    const uci = uciOf(pick(r, legal))
    const next = position.play(uci)
    if (!next) break
    moves.push(uci)
    position = next.position
  }
  return {
    moves,
    ply: moves.length,
    level: pick(r, ENGINE_LEVELS),
    color: r() < 0.5 ? 'white' : 'black',
    resigned: r() < 0.1,
    setup: played,
    clock: randomClock(r),
    times: r() < 0.5 ? { white: Math.floor(r() * 900000), black: Math.floor(r() * 900000) } : null,
    flagged: r() < 0.1 ? (r() < 0.5 ? 'white' : 'black') : null,
  }
}

/** Each host call in order: reads and effects are kept apart, each in its own order. */
class Log {
  private reads: unknown[] = []
  private effects: unknown[] = []
  read(entry: unknown[]): void {
    this.reads.push(entry)
  }
  effect(entry: unknown[]): void {
    this.effects.push(entry)
  }
  takeReads(): unknown[] {
    return this.reads.splice(0)
  }
  takeEffects(): unknown[] {
    return this.effects.splice(0)
  }
}

/** Runs one action; an error is recorded as the step's outcome, not a test failure. */
function attempt(run: () => unknown): { ret: unknown } | { error: string } {
  try {
    return { ret: run() }
  } catch (cause) {
    return { error: cause instanceof Error ? cause.message : String(cause) }
  }
}

/** Getters of a computer game, read the way the desktop reads them (no clock reads). */
function computerViews(game: ComputerGame) {
  return {
    over: game.over,
    winner: game.winner ?? null,
    result: game.result,
    running: game.running,
    draw: game.draw ?? null,
    fen: game.position.fen,
    turn: game.position.turn,
  }
}

/** One scripted run of a computer game against scripted engine replies and stop requests. */
async function computerRun(seed: number): Promise<unknown> {
  const r = rng(seed)
  const log = new Log()
  let now = Math.floor(r() * 500)
  let allowed = r() < 0.85
  let ready = r() < 0.9
  const pending: { moves: string[]; resolve(move: string): void; reject(cause: unknown): void }[] =
    []
  const stops: { resolve(): void; reject(cause: unknown): void }[] = []
  const host: ComputerHost = {
    bestMove(moves, level, options) {
      log.effect(['bestMove', moves, level, options])
      return new Promise<string>((resolve, reject) => {
        pending.push({ moves: [...moves], resolve, reject })
      })
    },
    stopEngine() {
      log.effect(['stopEngine'])
      return new Promise<void>((resolve, reject) => {
        stops.push({ resolve, reject })
      })
    },
    ready() {
      log.read(['ready', ready])
      return ready
    },
    allowed() {
      log.read(['allowed', allowed])
      return allowed
    },
    now() {
      log.read(['now', now])
      return now
    },
    moved(san, computer) {
      log.effect(['moved', san, computer])
    },
    flagged() {
      log.effect(['flagged'])
    },
    failed(cause) {
      log.effect(['failed', cause instanceof Error ? cause.message : String(cause)])
    },
  }
  const saved = r() < 0.5 ? undefined : randomSavedComputer(r)
  const state = computerState(saved)
  const game = new ComputerGame(state, host)
  const steps: unknown[] = []
  const actions: [string, number][] = [
    ['move', 22],
    ['answer', 14],
    ['flush', 8],
    ['advance', 12],
    ['flagTime', 6],
    ['toggleAllowed', 1],
    ['toggleReady', 2],
    ['availability', 2],
    ['computerTurn', 3],
    ['start', 1],
    ['takeback', 2],
    ['resign', 1],
    ['expire', 3],
    ['expireAt', 2],
    ['answerStop', 3],
    ['snapshot', 3],
    ['views', 4],
    ['remaining', 2],
  ]
  for (let i = 0; i < 70; i++) {
    const action = weighted(r, actions)
    let outcome: unknown
    switch (action) {
      case 'move': {
        const legal = game.position.legalMoves()
        const uci =
          r() < 0.8 && legal.length
            ? uciOf(pick(r, legal))
            : pick(r, ['e2e4', 'e7e5', 'a1a1', 'g1f3', 'zz9'])
        outcome = attempt(() => game.move(uci))
        break
      }
      case 'answer': {
        if (!pending.length) break
        const index = Math.floor(r() * pending.length)
        const request = pending[index]!
        const kind = r()
        if (kind < 0.7) {
          const legal = setupPositionAfter(state.setup, request.moves).legalMoves()
          request.resolve(legal.length ? uciOf(pick(r, legal)) : 'a1a1')
          pending.splice(index, 1)
        } else if (kind < 0.8) {
          request.resolve('a1a1')
          pending.splice(index, 1)
        } else if (kind < 0.9) {
          request.reject(new Error('engine crashed'))
          pending.splice(index, 1)
        }
        outcome = { pending: pending.length }
        break
      }
      case 'flush':
        await tick()
        break
      case 'flagTime':
        // A long pause: whichever side is on the clock runs out if the game is running.
        now += 600000
        break
      case 'advance': {
        now += pick(r, [0, 1, 50, 500, 3000, 30000, 120000])
        break
      }
      case 'toggleAllowed':
        allowed = !allowed
        break
      case 'toggleReady':
        ready = !ready
        break
      case 'availability':
        outcome = attempt(() => game.availabilityChanged())
        break
      case 'computerTurn':
        outcome = attempt(() => {
          void game.computerTurn()
          return null
        })
        break
      case 'start': {
        const setup = r() < 0.2 ? undefined : randomSetup(r)
        const clock =
          r() < 0.15 ? null : { minutes: pick(r, MINUTES), increment: pick(r, INCREMENTS) }
        outcome = attempt(() => game.start(setup, clock))
        if (clock && r() < 0.5) outcome = attempt(() => game.expire(now + 60000))
        break
      }
      case 'takeback':
        outcome = attempt(() => game.takeback())
        break
      case 'resign':
        outcome = attempt(() => game.resign())
        break
      case 'expire':
        outcome = attempt(() => game.expire())
        break
      case 'expireAt': {
        const at = now - Math.floor(r() * 4000)
        outcome = attempt(() => game.expire(at))
        break
      }
      case 'answerStop': {
        if (!stops.length) break
        const index = Math.floor(r() * stops.length)
        const stop = stops[index]!
        if (r() < 0.7) stop.resolve()
        else stop.reject(new Error('stop failed'))
        stops.splice(index, 1)
        break
      }
      case 'snapshot':
        outcome = attempt(() => game.snapshot())
        break
      case 'views':
        outcome = attempt(() => ({
          ...computerViews(game),
          archive: game.archiveSnapshot(),
        }))
        break
      case 'remaining':
        outcome = attempt(() => [game.remaining('white'), game.remaining('black', now + 7)])
        break
    }
    steps.push({
      action,
      outcome: outcome ?? null,
      state: clone(state),
      reads: log.takeReads(),
      effects: log.takeEffects(),
      pending: pending.length,
      stops: stops.length,
    })
    if (r() < 0.05) {
      // Availability changes sometimes happen the way the desktop reports them (toggle, then notify).
      allowed = !allowed
      outcome = attempt(() => game.availabilityChanged())
      steps.push({
        action: 'toggleAndNotify',
        outcome,
        state: clone(state),
        reads: log.takeReads(),
        effects: log.takeEffects(),
      })
    }
  }
  if (r() < 0.3) {
    game.dispose()
  }
  await tick()
  steps.push({
    action: 'final',
    state: clone(state),
    reads: log.takeReads(),
    effects: log.takeEffects(),
    views: attempt(() => computerViews(game)),
    snapshot: attempt(() => game.snapshot()),
  })
  return steps
}

/** One scripted run of a local game: clocks, pauses, takebacks, resignations and draws. */
function localRun(seed: number): unknown {
  const r = rng(seed)
  const log = new Log()
  let now = Math.floor(r() * 500)
  const state = localState(r() < 0.5 ? undefined : randomSavedLocal(r))
  const game = new LocalGame(
    state,
    () => {
      log.read(['now', now])
      return now
    },
    () => log.effect(['flagged']),
  )
  const steps: unknown[] = []
  const actions: [string, number][] = [
    ['start', 2],
    ['move', 16],
    ['advance', 8],
    ['togglePause', 5],
    ['takeback', 4],
    ['resign', 2],
    ['draw', 1],
    ['expire', 3],
    ['expireAt', 2],
    ['snapshot', 3],
    ['views', 4],
    ['remaining', 2],
  ]
  for (let i = 0; i < 70; i++) {
    const action = weighted(r, actions)
    let outcome: unknown
    switch (action) {
      case 'start':
        outcome = attempt(() =>
          game.start(
            r() < 0.2 ? undefined : randomSetup(r),
            r() < 0.2 ? undefined : randomLocalClock(r),
          ),
        )
        break
      case 'move': {
        const legal = game.position.legalMoves()
        const uci =
          r() < 0.8 && legal.length
            ? uciOf(pick(r, legal))
            : pick(r, ['e2e4', 'e7e5', 'a1a1', 'g1f3', 'zz9'])
        outcome = attempt(() => game.move(uci))
        break
      }
      case 'advance':
        now += pick(r, [0, 1, 50, 500, 3000, 30000])
        break
      case 'togglePause':
        outcome = attempt(() => game.togglePause())
        break
      case 'takeback':
        outcome = attempt(() => game.takeback())
        break
      case 'resign':
        outcome = attempt(() => game.resign(r() < 0.5 ? 'white' : 'black'))
        break
      case 'draw':
        outcome = attempt(() => game.agreeDraw())
        break
      case 'expire':
        outcome = attempt(() => game.expire())
        break
      case 'expireAt': {
        const at = now - Math.floor(r() * 4000)
        outcome = attempt(() => game.expire(at))
        break
      }
      case 'snapshot':
        outcome = attempt(() => game.snapshot())
        break
      case 'views':
        outcome = attempt(() => ({
          over: game.over,
          running: game.running,
          result: game.result,
          fen: game.position.fen,
          archive: game.archiveSnapshot(),
        }))
        break
      case 'remaining':
        outcome = attempt(() => [game.remaining('white'), game.remaining('black', now + 7)])
        break
    }
    steps.push({
      action,
      outcome: outcome ?? null,
      state: clone(state),
      reads: log.takeReads(),
      effects: log.takeEffects(),
    })
  }
  steps.push({
    action: 'final',
    state: clone(state),
    reads: log.takeReads(),
    effects: log.takeEffects(),
    views: attempt(() => ({
      over: game.over,
      result: game.result,
      archive: game.archiveSnapshot(),
    })),
  })
  return steps
}

function randomSavedLocal(r: Rand): LocalSession {
  const setup = randomSetup(r)
  const played = playRandom(setup, r, Math.floor(r() * 20))
  const moves: string[] = []
  let position = Position.from(played)
  for (let i = 0; position && i < 20 && r() < 0.9; i++) {
    const legal = position.legalMoves()
    if (!legal.length) break
    const uci = uciOf(pick(r, legal))
    const next = position.play(uci)
    if (!next) break
    moves.push(uci)
    position = next.position
  }
  return {
    setup: played,
    moves,
    clock: randomLocalClock(r),
    times: r() < 0.5 ? { white: Math.floor(r() * 900000), black: Math.floor(r() * 900000) } : null,
    result:
      r() < 0.2
        ? { winner: r() < 0.5 ? 'white' : 'black', reason: 'Time out' }
        : r() < 0.1
          ? { reason: 'Draw agreed' }
          : null,
  }
}

/** A fake host for the archive: a store of games, counters for ids, and a controllable clock. */
function archiveRun(seed: number): unknown {
  const r = rng(seed)
  const log = new Log()
  let now = Math.floor(r() * 1000)
  let ids = 0
  const store: ArchivedGame[] = []
  const game = new LocalGame(localState(), () => now)
  const hasPlay = () => game.state.moves.length > 0
  const host: ArchiveHost = {
    source() {
      log.read(['source'])
      return clone(game.archiveSnapshot()) as GameSnapshot
    },
    hasPlay() {
      const value = hasPlay()
      log.read(['hasPlay', value])
      return value
    },
    games() {
      log.read(['games', store.length])
      return clone(store)
    },
    save(saved) {
      log.effect(['save', clone(saved)])
      const at = store.findIndex((entry) => entry.id === saved.id)
      if (at >= 0) store[at] = clone(saved)
      else store.push(clone(saved))
    },
    remove(id) {
      log.effect(['remove', id])
      const at = store.findIndex((entry) => entry.id === id)
      if (at >= 0) store.splice(at, 1)
    },
    now() {
      log.read(['now', now])
      return now
    },
    id() {
      ids++
      const value = `game-${seed}-${ids}`
      log.read(['id', value])
      return value
    },
  }
  let archiveData = archiveState(undefined, host)
  let session = new GameArchive(archiveData, host, r() < 0.5)
  const steps: unknown[] = []
  const actions: [string, number][] = [
    ['move', 14],
    ['advance', 6],
    ['takeback', 4],
    ['newGame', 3],
    ['save', 8],
    ['reset', 6],
    ['newSession', 3],
    ['newState', 2],
    ['clockGame', 2],
  ]
  for (let i = 0; i < 40; i++) {
    const action = weighted(r, actions)
    let outcome: unknown
    switch (action) {
      case 'move': {
        const legal = game.position.legalMoves()
        const uci =
          r() < 0.8 && legal.length
            ? uciOf(pick(r, legal))
            : pick(r, ['e2e4', 'e7e5', 'g1f3', 'zz9'])
        outcome = attempt(() => game.move(uci))
        break
      }
      case 'advance':
        now += pick(r, [1, 500, 60000])
        break
      case 'takeback':
        outcome = attempt(() => game.takeback())
        break
      case 'newGame':
        outcome = attempt(() => game.start(randomSetup(r), null))
        break
      case 'clockGame':
        outcome = attempt(() => game.start(STANDARD_SETUP, randomLocalClock(r)))
        break
      case 'save': {
        const finished = r() < 0.5
        outcome = attempt(() => session.save(finished))
        break
      }
      case 'reset':
        outcome = attempt(() => session.reset())
        break
      case 'newSession': {
        const resume = r() < 0.5
        outcome = attempt(() => {
          session = new GameArchive(archiveData, host, resume)
          return null
        })
        break
      }
      case 'newState': {
        const identity =
          store.length && r() < 0.6
            ? { id: pick(r, store).id, startedAt: now - Math.floor(r() * 1000) }
            : undefined
        const resume = r() < 0.5
        outcome = attempt(() => {
          archiveData = archiveState(identity, host)
          session = new GameArchive(archiveData, host, resume)
          return archiveData
        })
        break
      }
    }
    steps.push({
      action,
      outcome: outcome ?? null,
      state: clone(archiveData),
      games: clone(store),
      reads: log.takeReads(),
      effects: log.takeEffects(),
    })
  }
  return steps
}

/** Saved sessions and archive identities normalized by the constructors. */
function statesRun(seed: number): unknown {
  const r = rng(seed)
  const log = new Log()
  const computerSaved = r() < 0.2 ? undefined : randomSavedComputer(r)
  const localSaved = r() < 0.2 ? undefined : randomSavedLocal(r)
  const identityHost = {
    now: () => {
      log.read(['now'])
      return 1234
    },
    id: () => {
      log.read(['id'])
      return 'new-id'
    },
  }
  return {
    computer: clone(computerState(computerSaved)),
    local: clone(localState(localSaved)),
    archive: clone(
      archiveState(
        r() < 0.5 ? { id: `saved-${seed}`, startedAt: Math.floor(r() * 9999) } : undefined,
        identityHost,
      ),
    ),
    reads: log.takeReads(),
  }
}

describe('game sessions (Rust-backed rules)', () => {
  afterEach(() => vi.restoreAllMocks())

  it('computer games: scripted engine replies, stops, clocks, takebacks and disposal', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.spyOn(console, 'debug').mockImplementation(() => {})
    for (let i = 0; i < 200; i++) golden.check('computer', i, await computerRun(7000 + i))
  })

  it('local games: clocks, pauses, takebacks, resignations, draws and expiry', () => {
    for (let i = 0; i < 80; i++) golden.check('local', i, localRun(8000 + i))
  })

  it('archive: identity, save, reset and takeback of the stored game', () => {
    for (let i = 0; i < 60; i++) golden.check('archive', i, archiveRun(9000 + i))
  })

  it('constructors normalize saved sessions and identities', () => {
    for (let i = 0; i < 80; i++) golden.check('states', i, statesRun(10000 + i))
  })
})
