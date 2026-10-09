import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { wasmBinding, type RulesBinding } from '../../src/domain/engine'
import training from '../../src/domain/data/training.json' with { type: 'json' }
import type { GameSetup, Variant } from '../../src/domain/variant'
import type { StoredReview } from '../../src/contracts/types'
import { nativeRules } from '../../src/services/native'
import { goldenFile } from './golden'

/**
 * The training rules (`crates/kchess-domain/src/training.rs`), checked against golden digests.
 * The digests were recorded from the TypeScript implementations they replaced
 * (`KCHESS_WRITE_GOLDEN=1`), before those were deleted. Inputs come from seeded generators that
 * only use the Rust rules (legal moves, play), so every run draws the same cases. Each call is
 * made in both runtimes (the Node module and the renderer's WebAssembly build), which must agree.
 */

const loaded = nativeRules()
if (!loaded) throw new Error('The native rules are not built (pnpm run build:native).')
const native = loaded
const WASM_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  '../../../apps/desktop/app/assets/rules/kchess.wasm',
)
const wasm: RulesBinding = wasmBinding(
  new WebAssembly.Instance(new WebAssembly.Module(readFileSync(WASM_PATH))).exports,
)
const golden = goldenFile('training')

const TIMEOUT = 120_000

function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
const pick = <T>(random: () => number, items: readonly T[]): T =>
  items[Math.floor(random() * items.length)]!

/** One rules call through both runtimes, which must agree; returns the parsed result. */
function rust(method: string, ...args: unknown[]) {
  const json = JSON.stringify(args)
  const fromNode = native.invoke(method, json)
  expect(wasm.invoke(method, json), `${method} in WebAssembly`).toBe(fromNode)
  return JSON.parse(fromNode)
}

/** A JavaScript number as a JSON argument: non-finite values travel as their text. */
const enc = (n: number): number | string => (Number.isFinite(n) ? n : String(n))

/** What a call returns or throws, as a comparable value. */
function attempt(call: () => unknown): unknown {
  try {
    return call()
  } catch (cause) {
    return { error: (cause as Error).message }
  }
}

/** Comparable form: `undefined` is absent (null at the top), non-finite numbers are their text. */
const norm = (value: unknown) =>
  value === undefined
    ? null
    : JSON.parse(
        JSON.stringify(value, (_key, inner: unknown) =>
          typeof inner === 'number' && !Number.isFinite(inner) ? String(inner) : inner,
        ),
      )

/** Per suite, the next golden index. */
const counters = new Map<string, number>()
function check(suite: string, value: unknown): void {
  const index = counters.get(suite) ?? 0
  counters.set(suite, index + 1)
  golden.check(suite, index, norm(value))
}

/* ── Generators (Rust rules only) ─────────────────────────────────────────────────────────── */

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const setupOf = (fen: string, variant: Variant = 'standard'): GameSetup => ({ variant, fen })
interface LegalMove {
  from: string
  to: string
  role: string
  promotion?: string
  san: string
}
const LETTER: Record<string, string> = { knight: 'n', bishop: 'b', rook: 'r', queen: 'q' }
const legal = (fen: string): LegalMove[] => rust('legalMoves', setupOf(fen))
const uciOf = (move: LegalMove): string =>
  `${move.from}${move.to}${move.promotion ? LETTER[move.promotion] : ''}`

interface Game {
  fens: string[]
  ucis: string[]
  sans: string[]
}

/** A random game from `fen`, `plies` moves at most. */
function randomGame(random: () => number, plies: number, fen = START): Game {
  const game: Game = { fens: [fen], ucis: [], sans: [] }
  for (let i = 0; i < plies; i++) {
    const moves = legal(fen)
    if (!moves.length) break
    const uci = uciOf(pick(random, moves))
    const played = rust('play', setupOf(fen), uci)
    game.ucis.push(uci)
    game.sans.push(played.san)
    fen = played.position.fen
    game.fens.push(fen)
  }
  return game
}

/** A random legal line of up to `plies` moves from `fen`, as UCI. */
function randomLine(random: () => number, fen: string, plies: number): string[] {
  const line: string[] = []
  for (let i = 0; i < plies; i++) {
    const moves = legal(fen)
    if (!moves.length) break
    const uci = uciOf(pick(random, moves))
    fen = rust('play', setupOf(fen), uci).position.fen
    line.push(uci)
  }
  return line
}

/** The PGN move text of a game: `1. e4 e5 2. Nf3`. */
const pgnOf = (sans: string[]): string =>
  sans.map((san, i) => (i % 2 === 0 ? `${i / 2 + 1}. ${san}` : san)).join(' ')

/* ── Rush ─────────────────────────────────────────────────────────────────────────────────── */

type RushOp =
  | { op: 'correct' }
  | { op: 'mistake' }
  | { op: 'skip' }
  | { op: 'solved'; rating: number }
  | { op: 'tick'; ms: number }
  | { op: 'finish'; reason: string }
  | { op: 'read' }

interface RushConfigJson {
  mode: string
  variant: string
  title: string
  rules: string
  durationMs?: number
  strikes?: number
  skips: number
  penaltyMs: number
  combo: boolean
  ladder: { from: number; to: number; count: number }
}

/** The same ops through the Rust transitions, threading the state through each one. */
function runRush(config: RushConfigJson, ops: RushOp[]): unknown[] {
  let state = rust('newRush', config)
  const trace: unknown[] = []
  for (const step of ops) {
    if (step.op === 'correct') {
      const r = rust('correctMove', state, config)
      state = r.state
      trace.push(r.result)
    } else if (step.op === 'mistake') {
      state = rust('mistake', state, config)
    } else if (step.op === 'skip') {
      const r = rust('skip', state)
      state = r.state
      trace.push(r.result)
    } else if (step.op === 'solved') {
      state = rust('puzzleSolved', state, enc(step.rating))
    } else if (step.op === 'tick') {
      state = rust('tick', state, enc(step.ms))
    } else if (step.op === 'finish') {
      state = rust('finish', state, step.reason)
    } else {
      trace.push(rust('accuracy', state))
    }
    trace.push(state)
  }
  return trace
}

function rushOps(random: () => number): RushOp[] {
  const ops: RushOp[] = []
  const length = Math.floor(random() * 30)
  for (let i = 0; i < length; i++) {
    const r = random()
    if (r < 0.4) ops.push({ op: 'correct' })
    else if (r < 0.55) ops.push({ op: 'mistake' })
    else if (r < 0.62) ops.push({ op: 'skip' })
    else if (r < 0.7) ops.push({ op: 'solved', rating: Math.floor(500 + random() * 2500) })
    else if (r < 0.9)
      ops.push({
        op: 'tick',
        ms: random() < 0.5 ? Math.floor(random() * 20_000) : random() * 60_000,
      })
    else if (r < 0.95)
      ops.push({ op: 'finish', reason: pick(random, ['time', 'strikes', 'complete', 'ended']) })
    else ops.push({ op: 'read' })
  }
  return ops
}

/** The run kinds, from the shared data (`core/src/domain/data/training.json`). */
const RUSH_CONFIGS = training.rushConfigs as Record<string, RushConfigJson>
const RUSH_KEYS = training.rushChoices

describe('rush runs', { timeout: TIMEOUT }, () => {
  it('follow the rules of every run kind', () => {
    const random = rng(7301)
    expect(rust('rushChoices')).toEqual(RUSH_KEYS)
    check('rush-constants', rust('rushConfigs'))
    for (let i = 0; i < 200; i++) {
      const config = RUSH_CONFIGS[RUSH_KEYS[i % RUSH_KEYS.length]!]!
      check('rush-trace', runRush(config, rushOps(random)))
    }
  })

  it('compute the combo and clock helpers', () => {
    const random = rng(7302)
    for (let combo = 0; combo <= 60; combo++) {
      check('rush-combo', rust('comboBonusMs', combo))
      check('rush-combo', rust('comboProgress', combo))
    }
    for (let i = 0; i < 150; i++) {
      const combo = random() < 0.5 ? Math.floor(random() * 80) : random() * 80
      check('rush-combo', rust('comboBonusMs', enc(combo)))
      check('rush-combo', rust('comboProgress', enc(combo)))
    }
    const clocks = [
      0, 1, 9_400, 9_999, 9_999.5, 10_000, 59_999.9, 60_000, 183_000, 3_599_999.9, 3_600_000, -5,
      0.4,
    ]
    for (let i = 0; i < 300; i++)
      clocks.push(random() < 0.5 ? Math.floor(random() * 400_000) : random() * 40_000 - 1000)
    for (const ms of clocks) check('rush-clock', rust('formatRunClock', enc(ms)))
    for (const ms of [Number.NaN, Number.POSITIVE_INFINITY]) {
      check('rush-clock', rust('formatRunClock', enc(ms)))
    }
  })

  it('count accuracy and skips along the run', () => {
    const random = rng(7303)
    for (let i = 0; i < 80; i++) {
      const config = RUSH_CONFIGS[RUSH_KEYS[i % RUSH_KEYS.length]!]!
      check('rush-trace', runRush(config, rushOps(random)))
    }
  })
})

/* ── Puzzles ──────────────────────────────────────────────────────────────────────────────── */

describe('puzzles', { timeout: TIMEOUT }, () => {
  it('convert Lichess API and database puzzles', () => {
    const random = rng(7311)
    const games = Array.from({ length: 30 }, () =>
      randomGame(random, 12 + Math.floor(random() * 50)),
    )
    for (const [index, game] of games.entries()) {
      for (let k = 1; k < game.fens.length - 1; k += 3) {
        const solution = randomLine(random, game.fens[k]!, 1 + Math.floor(random() * 5))
        const withFen = {
          game: { id: `g${index}`, pgn: pgnOf(game.sans) },
          puzzle: {
            id: `p${index}`,
            rating: 1200 + Math.floor(random() * 800),
            plays: 100,
            solution,
            themes: ['mateIn2', 'short'],
            fen: game.fens[k],
            lastMove: game.ucis[k - 1],
            initialPly: k - 1,
          },
        }
        check('puzzle-api', rust('puzzleFromApi', withFen))
        const fromPgn = {
          game: { pgn: pgnOf(game.sans) },
          puzzle: {
            id: `q${index}`,
            rating: 1500,
            solution,
            themes: [],
            initialPly: random() < 0.8 ? k - 1 : k + 4,
          },
        }
        check('puzzle-api', rust('puzzleFromApi', fromPgn))
        const noSolution = { ...fromPgn, puzzle: { ...fromPgn.puzzle, solution: [] } }
        check('puzzle-api', rust('puzzleFromApi', noSolution))
        const row = {
          id: `r${index}`,
          fen: game.fens[k - 1],
          moves: [game.ucis[k - 1], ...randomLine(random, game.fens[k]!, 2)].join(' '),
          rating: 1700,
          plays: 55,
          themes: 'mate  mateIn2 short',
        }
        check('puzzle-db', rust('puzzleFromDb', row))
        const bad = { ...row, moves: 'a1a8 a2e6' }
        check('puzzle-db', rust('puzzleFromDb', bad))
      }
    }
    // Fixed cases from the smoke test, and a few malformed inputs.
    const daily = {
      game: {
        id: 'qLcxVHzB',
        pgn: 'd4 Nf6 c4 d6 Nc3 g6 e4 Bg7 Be2 O-O e5 dxe5 dxe5 Qxd1+ Nxd1 Ne4 Be3 Bxe5 Bd3 Nf6',
      },
      puzzle: {
        id: '9862H',
        rating: 1850,
        plays: 53523,
        solution: ['f2f4', 'e5d6', 'c4c5', 'd6c5', 'e3c5'],
        themes: ['opening', 'advantage'],
        fen: 'rnb2rk1/ppp1pp1p/5np1/4b3/2P5/3BB3/PP3PPP/R2NK1NR w KQ - 1 1',
        lastMove: 'e4f6',
        initialPly: 19,
      },
    }
    check('puzzle-api', rust('puzzleFromApi', daily))
    const rebuilt = {
      ...daily,
      puzzle: { id: 'x', rating: 1, solution: ['f2f4'], themes: [], initialPly: 19 },
    }
    check('puzzle-api', rust('puzzleFromApi', rebuilt))
    const missingPly = { ...rebuilt, puzzle: { ...rebuilt.puzzle, initialPly: undefined } }
    check('puzzle-api', rust('puzzleFromApi', missingPly))
    const longer = { ...rebuilt, puzzle: { ...rebuilt.puzzle, initialPly: 500 } }
    check('puzzle-api', rust('puzzleFromApi', longer))
    const negative = { ...rebuilt, puzzle: { ...rebuilt.puzzle, initialPly: -3 } }
    check('puzzle-api', rust('puzzleFromApi', negative))
    const dbRow = {
      id: '00sHx',
      fen: 'q3k1nr/1pp1nQpp/3p4/1P2p3/4P3/B1PP1b2/B5PP/5K2 b k - 0 17',
      moves: 'e8d7 a2e6 d7d8 f7f8',
      rating: 1760,
      plays: 550,
      themes: 'mate mateIn2 middlegame short',
    }
    check('puzzle-db', rust('puzzleFromDb', dbRow))
    check('puzzle-db', rust('puzzleFromDb', { ...dbRow, fen: 'nonsense' }))
  })

  it('highlight squares, including castling', () => {
    const random = rng(7312)
    const castling = 'r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1'
    for (const uci of [
      'e1g1',
      'e1h1',
      'e1c1',
      'e1a1',
      'e8a8',
      'e8h8',
      'a1e1',
      'e1d1',
      'e7e8q',
      'e7e8k',
      'e7e8x',
      'x9',
      '',
    ]) {
      check('puzzle-squares', rust('moveSquares', castling, uci))
    }
    check('puzzle-squares', rust('moveSquares', 'bad', 'e2e4'))
    for (let i = 0; i < 300; i++) {
      const game = randomGame(random, 30)
      const k = Math.floor(random() * game.fens.length)
      const fen = game.fens[k]!
      const moves = legal(fen)
      const uci = moves.length && random() < 0.9 ? uciOf(pick(random, moves)) : 'a1h8'
      check('puzzle-squares', rust('moveSquares', fen, uci))
    }
  })

  it('solve puzzles move by move', () => {
    const random = rng(7313)
    const fixed = [
      {
        id: 'm1',
        fen: '6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1',
        solution: ['a1a8'],
        rating: 600,
        themes: [],
      },
      {
        id: 'm2',
        fen: 'k7/8/1K6/8/8/8/8/6R1 w - - 0 1',
        solution: ['g1a1'],
        rating: 600,
        themes: [],
      },
      {
        id: 'c1',
        fen: 'r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1',
        solution: ['e1g1', 'a8a1'],
        rating: 1000,
        themes: [],
      },
    ]
    const generated = Array.from({ length: 60 }, () => {
      const game = randomGame(random, 25)
      const k = Math.floor(random() * (game.fens.length - 1))
      const solution = randomLine(random, game.fens[k]!, 1 + Math.floor(random() * 6))
      return {
        id: 'gen',
        fen: game.fens[k]!,
        lastMove: game.ucis[k - 1],
        solution,
        rating: 1400,
        themes: ['x'],
      }
    })
    for (const puzzle of [...fixed, ...generated]) {
      const start = rust('startPuzzle', puzzle)
      check('puzzle-start', start)
      check('puzzle-start', rust('playerColor', puzzle))
      // Plan the player's moves from the Rust rules, then replay the same plan.
      const plan: string[] = []
      let state = start
      for (let step = 0; step < 6; step++) {
        const solved = puzzle.solution[state.index]
        let uci: string
        const roll = random()
        if (roll < 0.55 && solved) uci = solved
        else if (roll < 0.9) {
          const moves = legal(state.fen)
          if (!moves.length) break
          uci = uciOf(pick(random, moves))
        } else uci = 'h7h1'
        plan.push(uci)
        const result = rust('playerMove', puzzle, state, uci)
        state = result.state
        if (result.reply) state = rust('opponentReply', state, result.reply)
      }
      check('puzzle-trace', solveTrace(puzzle, plan))
    }
  })
})

/** Plays `plan` against a puzzle with the Rust rules. */
function solveTrace(puzzle: object, plan: string[]): unknown[] {
  const trace: unknown[] = []
  let state = rust('startPuzzle', puzzle)
  for (const uci of plan) {
    const result = rust('playerMove', puzzle, state, uci)
    state = result.state
    trace.push({ correct: result.correct, reply: result.reply ?? null, state })
    if (result.reply) {
      state = rust('opponentReply', state, result.reply)
      trace.push(state)
    }
    trace.push(rust('nextSolutionSquares', puzzle, state))
  }
  return trace
}

/* ── Endgame drills ───────────────────────────────────────────────────────────────────────── */

const DRILLS = [
  { id: 'kq-vs-k', fen: '4k3/8/8/8/8/8/8/3QK3 w - - 0 1', player: 'white', goal: 'win' },
  { id: 'kr-vs-k', fen: '4k3/8/8/8/8/8/8/R3K3 w - - 0 1', player: 'white', goal: 'win' },
  { id: 'kbb-vs-k', fen: '4k3/8/8/8/8/8/8/2B1KB2 w - - 0 1', player: 'white', goal: 'win' },
  { id: 'kbn-vs-k', fen: '4k3/8/8/8/8/8/8/2B1K1N1 w - - 0 1', player: 'white', goal: 'win' },
  { id: 'kp-key-squares', fen: '4k3/8/4K3/4P3/8/8/8/8 w - - 0 1', player: 'white', goal: 'win' },
  { id: 'kp-opposition', fen: '8/8/8/3k4/8/3K4/3P4/8 b - - 0 1', player: 'white', goal: 'win' },
  { id: 'kp-hold', fen: '8/8/3k4/3P4/3K4/8/8/8 b - - 0 1', player: 'black', goal: 'draw' },
  { id: 'lucena', fen: '1K1k4/1P6/8/8/8/8/r7/2R5 w - - 0 1', player: 'white', goal: 'win' },
  { id: 'philidor', fen: '4k3/8/r7/3KP3/8/8/8/7R b - - 0 1', player: 'black', goal: 'draw' },
]

describe('endgame drills', { timeout: TIMEOUT }, () => {
  it('evaluate lines and name the moves', () => {
    const random = rng(7321)
    check('endgame-constants', rust('endgameDrills'))
    const shuffle = ['g1f3', 'g8f6', 'f3g1', 'f6g8', 'g1f3', 'g8f6', 'f3g1', 'f6g8']
    for (const drill of DRILLS) {
      for (let i = 0; i < 30; i++) {
        const plies = Math.floor(random() * 40)
        const moves = randomLine(random, drill.fen, plies)
        const goal = random() < 0.5 ? drill.goal : pick(random, ['win', 'draw'] as const)
        const player = random() < 0.7 ? drill.player : pick(random, ['white', 'black'] as const)
        check(
          'endgame-evaluate',
          attempt(() => rust('evaluateEndgame', drill.fen, moves, player, goal)),
        )
        check('endgame-san', rust('sanFrom', drill.fen, moves))
      }
      check(
        'endgame-evaluate',
        rust('evaluateEndgame', drill.fen, shuffle, drill.player, drill.goal),
      )
    }
    const cases: [string, string[], string, string][] = [
      ['7k/5Q2/6K1/8/8/8/8/8 w - - 0 1', ['f7g7'], 'white', 'win'],
      ['7k/8/6K1/8/8/8/8/5Q2 w - - 0 1', ['f1f7'], 'white', 'win'],
      ['7k/8/6K1/8/8/8/8/5Q2 w - - 0 1', ['f1f7'], 'black', 'draw'],
      [
        '4k3/8/8/8/8/8/8/3QK3 w - - 0 1',
        ['d1d2', 'e8f8', 'd2d1', 'f8e8', 'd1d2', 'e8f8', 'd2d1', 'f8e8'],
        'white',
        'win',
      ],
      ['nonsense', [], 'white', 'win'],
      ['4k3/8/8/8/8/8/8/3QK3 w - - 0 1', ['bogus'], 'white', 'win'],
    ]
    for (const [fen, moves, player, goal] of cases) {
      check(
        'endgame-evaluate',
        attempt(() => rust('evaluateEndgame', fen, moves, player, goal)),
      )
      check('endgame-san', rust('sanFrom', fen, moves))
    }
  })
})

/* ── Openings ─────────────────────────────────────────────────────────────────────────────── */

describe('openings', { timeout: TIMEOUT }, () => {
  it('name the last named position', () => {
    const random = rng(7331)
    const games = Array.from({ length: 60 }, () =>
      randomGame(random, 20 + Math.floor(random() * 15)),
    )
    for (const game of games) {
      for (let k = 0; k <= game.ucis.length; k += 2) {
        const fen = game.fens[k]!
        const moves = game.ucis
        const ply = random() < 0.4 ? undefined : Math.floor(random() * (moves.length + 3))
        check(
          'openings',
          rust('openingAt', setupOf(START), moves, ply === undefined ? undefined : enc(ply)),
        )
        check('openings', rust('openingAt', setupOf(fen), moves.slice(k), undefined))
      }
    }
    const moves = ['e2e4', 'a7a6', 'h2h3', 'bogus', 'd7d5']
    for (const ply of [
      -2,
      0,
      2.5,
      Number.NaN,
      Number.POSITIVE_INFINITY,
      Number.NEGATIVE_INFINITY,
    ]) {
      check('openings', rust('openingAt', setupOf(START), moves, enc(ply)))
    }
    check('openings', rust('openingAt', setupOf(START, 'chess960'), ['e2e4'], 1))
    check('openings', rust('openingAt', setupOf('nonsense'), ['e2e4'], 1))
    check('openings', rust('openingAt', setupOf(START), ['e2e4', 'a7a6', 'h2h3'], 3))
  })
})

/* ── Clock ────────────────────────────────────────────────────────────────────────────────── */

describe('clock', { timeout: TIMEOUT }, () => {
  it('interpolate, pause and alert', () => {
    const random = rng(7341)
    for (let run = 0; run < 120; run++) {
      let now = Math.floor(random() * 10_000)
      let state = rust('clockInitial')
      const trace: unknown[] = []
      for (let step = 0; step < 25; step++) {
        now += Math.floor(random() * 3_000)
        const roll = random()
        const color = random() < 0.5 ? 'white' : 'black'
        if (roll < 0.2) {
          const seconds = pick(random, [0, 30, 60, 120, 300, 600, 1500, 3600])
          const data = {
            white: Math.floor(random() * 300_000),
            black: Math.floor(random() * 300_000) - 1000,
            ticking: pick(random, ['white', 'black', undefined] as const),
            initialSeconds: random() < 0.8 ? seconds : undefined,
            delayCentis: random() < 0.3 ? Math.floor(random() * 50) : undefined,
          }
          state = rust('clockSet', state, data, enc(now))
        } else if (roll < 0.3) {
          state = rust('clockPause', state, enc(now))
        } else if (roll < 0.6) {
          trace.push(rust('clockRemaining', state, color, enc(now)))
        } else if (roll < 0.9) {
          const r = rust('clockLowTimeAlert', state, color, enc(now))
          state = r.state
          trace.push(r.result)
        } else {
          trace.push(state.ticking ?? null)
        }
      }
      check('clock-trace', trace)
    }
    for (const ms of [
      0, 999, 1000, 9_000, 59_999, 60_000, 3_599_999, 3_600_000, 86_399_999, 86_400_000, 3e9, -5,
    ]) {
      check('clock-format', rust('formatClock', enc(ms)))
    }
    for (let i = 0; i < 200; i++)
      check('clock-format', rust('formatClock', enc(random() * 2e8 - 1e6)))
  })
})

/* ── Engine levels ────────────────────────────────────────────────────────────────────────── */

const LEVEL_IDS = [
  'beginner',
  'novice',
  'casual',
  'club',
  'strong-club',
  'expert',
  'cm',
  'fm',
  'im',
  'gm',
  'super-gm',
  'max',
]

describe('engine levels', { timeout: TIMEOUT }, () => {
  it('describe, label and substitute levels', () => {
    const random = rng(7351)
    check('engine-constants', rust('engineLadder'))
    check('engine-constants', rust('defaultEngineLevels'))
    for (const id of [...LEVEL_IDS, 'bogus']) {
      check(
        'engine-info',
        attempt(() => rust('engineLevelInfo', id)),
      )
      if (LEVEL_IDS.includes(id)) check('engine-label', rust('engineLevelLabel', id))
    }
    const junk = ['bogus', 'x', 'gm', '']
    for (let i = 0; i < 300; i++) {
      const pool = [...LEVEL_IDS, ...junk]
      const levels = Array.from({ length: Math.floor(random() * 6) }, () => pick(random, pool))
      check('engine-normalize', rust('normalizeEngineLevels', levels))
      const wanted = pick(random, pool)
      const enabled = levels.filter(() => random() < 0.8)
      check(
        'engine-nearest',
        attempt(() => rust('nearestEngineLevel', wanted, enabled)),
      )
    }
    check('engine-normalize', rust('normalizeEngineLevels', []))
  })
})

/* ── UCI info lines ───────────────────────────────────────────────────────────────────────── */

describe('uci info lines', { timeout: TIMEOUT }, () => {
  it('parse scored principal variations', () => {
    const random = rng(7361)
    const vocabulary = [
      'info',
      'info',
      'info',
      'depth',
      'depth',
      'seldepth',
      'multipv',
      'score',
      'score',
      'cp',
      'cp',
      'mate',
      'mate',
      'lowerbound',
      'upperbound',
      'nps',
      'pv',
      'pv',
      'wdl',
      'string',
      'hashfull',
      'currmove',
      'nodes',
      'time',
      '0',
      '1',
      '2',
      '5',
      '12',
      '-31',
      '40',
      '-2',
      '300',
      '1000000',
      'NaN',
      'Infinity',
      '0x10',
      '1e3',
      '2.5',
      'e2e4',
      'd2d4',
      'g1f3',
      'a7a8q',
      'e7e8k',
      'h1h9',
      'e2e4x',
      'bestmove',
      '',
      'NNUE',
    ]
    const separators = [' ', ' ', ' ', '  ', '\t', '\n', '   ', '\r']
    const lines: string[] = [
      'info depth 5 score mate 2 pv d8h4',
      'info depth 9 score cp 12 wdl 300 500 200 pv d2d4',
      'info depth 12 currmove e2e4 currmovenumber 1',
      'info depth 12 score cp 40 lowerbound nodes 1 pv e2e4',
      'info string NNUE evaluation using nn.nnue',
      'info depth 0 score mate 0',
      'bestmove e2e4',
      '  bestmove e2e4  ',
      '  info depth 5 score mate 2 pv d8h4  ',
      'info depth 20 score cp -31 nodes 123 nps 456789 multipv 2 pv e2e4 e7e5 g1f3',
      ' info depth 4 score cp 10 pv e2e4',
      '',
      '   ',
      'info',
    ]
    for (let i = 0; i < 3000; i++) {
      const length = 1 + Math.floor(random() * 22)
      let text = random() < 0.7 ? 'info' : ''
      for (let t = 0; t < length; t++) text += pick(random, separators) + pick(random, vocabulary)
      if (random() < 0.2) text = pick(random, separators) + text
      lines.push(text)
    }
    for (const text of lines) {
      for (const white of [true, false]) check('uci-info', rust('parseInfo', text, white))
    }
  })
})

/* ── Coaching ─────────────────────────────────────────────────────────────────────────────── */

describe('review coaching', { timeout: TIMEOUT }, () => {
  it('explain a reviewed move from its scores', () => {
    const random = rng(7371)
    /** A scored position: a mate or centipawns (often ties such as 25 and -75), with a best move and PV. */
    const scoreOf = (fen: string): StoredReview['evals'][number] => {
      if (random() < 0.1) return null
      const legalNow = legal(fen).map(uciOf)
      const best =
        random() < 0.8 && legalNow.length ? pick(random, legalNow) : random() < 0.5 ? '' : undefined
      const pv =
        random() < 0.7 && legalNow.length
          ? [best ?? legalNow[0]!, ...randomLine(random, fen, 3)]
          : undefined
      const mate = random() < 0.1 ? Math.floor(random() * 9) - 4 : undefined
      const cp = random() < 0.5 ? Math.floor(random() * 1200) - 600 : random() < 0.5 ? 25 : -75
      const value: Record<string, unknown> = {
        depth: random() < 0.9 ? Math.floor(random() * 30) : 0,
      }
      if (mate !== undefined && mate !== 0) value.mate = mate
      else value.cp = cp
      if (best !== undefined) value.best = best
      if (pv !== undefined) value.pv = pv
      return value as StoredReview['evals'][number]
    }
    for (let i = 0; i < 400; i++) {
      const game = randomGame(random, 4 + Math.floor(random() * 14))
      const moves = random() < 0.9 ? game.ucis : [...game.ucis, 'bogus']
      const evals: StoredReview['evals'] = []
      for (let k = 0; k <= moves.length; k++)
        evals.push(scoreOf(game.fens[Math.min(k, game.fens.length - 1)]!))
      const review = {
        key: 'k',
        fen: START,
        moves,
        source: random() < 0.5 ? 'lichess' : 'local',
        evals,
        complete: random() < 0.6,
      }
      for (let index = -1; index <= moves.length + 1; index++) {
        check('coach', rust('explainReviewedMove', review, index))
      }
    }
  })
})
