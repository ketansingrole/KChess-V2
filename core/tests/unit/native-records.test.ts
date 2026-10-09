import { describe, expect, it } from 'vitest'
import { goldenFile } from './golden.ts'
import { rules } from '../../src/domain/engine.ts'
import '../../src/services/rules.ts'
import { Position } from '../../src/domain/position.ts'
import { defaultFen, VARIANTS, type Variant } from '../../src/domain/variant.ts'
import { LichessError } from '../../src/domain/lichessError.ts'
import { DEFAULT_OAUTH_LOOK } from '../../src/domain/oauthLook.ts'
import { RATING_DISPLAY_NAMES } from '../../src/domain/ratings.ts'

/**
 * The records rules (`crates/kchess-domain/src/records*`) pinned to outputs recorded from the
 * TypeScript they replaced (`golden/records.json`, `KCHESS_WRITE_GOLDEN=1` records them). Cases
 * come from fixed seeds, so a failure names its suite and index and can be replayed. The
 * TypeScript exports are now thin wrappers over these rules; their behaviour is tested through
 * the callers' own suites.
 */

const golden = goldenFile('records')

const counts: Record<string, number> = {}

/** A call's outcome as the rules report it: a value (undefined is null) or an error message. */
function outcome(run: () => unknown): { ok: unknown } | { error: string } {
  try {
    return { ok: plain(run() ?? null) }
  } catch (cause) {
    return { error: cause instanceof Error ? cause.message : String(cause) }
  }
}

/** Compare as values cross the boundary: `undefined` properties are simply absent. */
function plain<T>(value: T): T {
  return value === undefined ? value : (JSON.parse(JSON.stringify(value)) as T)
}

type Args = unknown[]

/** Run every case of a suite against its golden outputs, numbering them in order. */
function run(suite: keyof typeof SUITES, cases: Args[]): void {
  for (const args of cases) {
    const index = counts[suite] ?? 0
    counts[suite] = index + 1
    golden.check(
      suite,
      index,
      outcome(() => SUITES[suite](args)),
    )
  }
}

/* ── Deterministic generators ── */

function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const pick = <T>(r: () => number, items: readonly T[]): T => items[Math.floor(r() * items.length)]!
const int = (r: () => number, lo: number, hi: number): number =>
  lo + Math.floor(r() * (hi - lo + 1))
const chance = (r: () => number, p: number): boolean => r() < p
const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null

/** Strings that exercise Unicode case mapping, separators, and UTF-16 lengths. */
const ODD = [
  'a',
  'Z',
  'q',
  '1',
  '_',
  '-',
  ' ',
  '\t',
  'é',
  'İ',
  'Σ',
  '😀',
  'ß',
  ' ',
  '　',
  '﻿',
  '(',
  ')',
  ',',
  '#',
  '%',
  '|',
  '@',
  '"',
  '\\',
  '\n',
]
const text = (r: () => number, max: number): string =>
  Array.from({ length: int(r, 0, max) }, () => pick(r, ODD)).join('')

const PERFS = [
  'blitz',
  'Blitz',
  'rapid',
  'Rapid',
  'bullet',
  'ultraBullet',
  'UltraBullet',
  'classical',
  'Puzzles',
  'puzzle',
  'King of the Hill',
  'kingOfTheHill',
  'threeCheck',
  'Three-check',
  'chess960',
  'Bullet_x',
  'atomic',
  'correspondence',
  'foo',
  '',
  'Racing Kings',
  'Horde',
]
const STATUS = [
  'created',
  'started',
  'mate',
  'resign',
  'aborted',
  'timeout',
  'draw',
  'outoftime',
  'noStart',
]
const BASE_DAY = Date.UTC(2025, 0, 1)
const DAY = 86_400_000

const startOf = (variant: Variant): string => defaultFen(variant)

/** Positions reached by random legal play from `start`, with the SAN of each move. */
function playRandom(
  start: Position,
  r: () => number,
  plies: number,
): { positions: Position[]; sans: string[] } {
  const positions = [start]
  const sans: string[] = []
  let pos = start
  for (let i = 0; i < plies; i++) {
    const legal = pos.legalMoves()
    if (!legal.length) break
    const san = pick(r, legal).san
    const next = pos.playSan(san)
    if (!next) break
    sans.push(san)
    pos = next.position
    positions.push(pos)
  }
  return { positions, sans }
}

/** A PGN with headers, comments, NAGs and variations, built from random legal play. */
function randomPgn(r: () => number): string {
  const start = Position.from({ variant: 'standard', fen: startOf('standard') })!
  const { positions, sans } = playRandom(start, r, int(r, 0, 40))
  const headers: string[] = []
  if (chance(r, 0.6)) headers.push(`[White "${pick(r, ['Ana', 'Zoë', '?', '', 'Bob'])}"]`)
  if (chance(r, 0.5)) headers.push(`[Black "${pick(r, ['Cy', '?', 'Dee', ''])}"]`)
  if (chance(r, 0.5)) headers.push(`[Event "${pick(r, ['Casual', '?', 'Cup 2026', ''])}"]`)
  const tokens: string[] = []
  const number = (ply: number): string => `${Math.floor(ply / 2) + 1}${ply % 2 === 0 ? '.' : '...'}`
  sans.forEach((san, ply) => {
    tokens.push(number(ply), san)
    if (chance(r, 0.2)) tokens.push(`{ note ${int(r, 0, 99)} }`)
    if (chance(r, 0.08)) tokens.push('$1')
    if (chance(r, 0.15)) {
      const before = positions[ply]!
      const alternatives = before
        .legalMoves()
        .map((m) => m.san)
        .filter((s) => s !== san)
      if (alternatives.length) {
        const alt = pick(r, alternatives)
        const altPos = before.playSan(alt)?.position
        const more = altPos ? playRandom(altPos, r, int(r, 0, 3)).sans : []
        const line = [alt, ...more.slice(0, 2)]
        const shown = line.map((s, i) => (i === 0 ? `${number(ply)} ${s}` : s))
        tokens.push(`(${shown.join(' ')})`)
      }
    }
  })
  return [...headers, '', [...tokens, chance(r, 0.5) ? '*' : '1-0'].join(' ')].join('\n')
}

/** Lichess board-API events: well-formed, then mutated. */
const ID_CHARS = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'
const gameId = (r: () => number): string =>
  Array.from({ length: int(r, 8, 12) }, () => ID_CHARS[int(r, 0, ID_CHARS.length - 1)]).join('')
const uciMoves = (r: () => number): string =>
  Array.from({ length: int(r, 0, 6) }, () => {
    const file = () => 'abcdefgh'[int(r, 0, 7)]
    const rank = () => String(int(r, 1, 8))
    return `${file()}${rank()}${file()}${rank()}${chance(r, 0.1) ? 'q' : ''}`
  }).join(' ')
const MUTATIONS: unknown[] = [
  null,
  'x',
  5,
  -1,
  true,
  false,
  [],
  {},
  ['e2e4'],
  'e2e4',
  '',
  '   ',
  'e2e4 garbage',
  'P@e4 e2e4',
  'Nd4',
  'a'.repeat(10_001),
  'b'.repeat(41),
  'c'.repeat(401),
  'd'.repeat(9),
  'white',
  'black',
  'started',
  'nope',
  0,
  1e308,
  -0,
  'x'.repeat(31),
  'y'.repeat(121),
]
function randomEvent(r: () => number): Record<string, unknown> {
  const kind = pick(r, [
    'gameState',
    'gameFull',
    'gameStart',
    'gameFinish',
    'opponentGone',
    'chatLine',
    'challenge',
    'challengeCanceled',
    'challengeDeclined',
    'gameState',
    'gameFull',
  ])
  const state = {
    moves: chance(r, 0.7) ? uciMoves(r) : pick(r, ['', 'e2e4', 'P@e4 e2e4 xyz']),
    status: pick(r, STATUS),
    wtime: int(r, 0, 900_000),
    btime: int(r, 0, 900_000),
    winc: 0,
    binc: 2000,
    winner: pick(r, [undefined, 'white', 'black']),
    wdraw: chance(r, 0.1),
    bdraw: undefined,
    expiration: chance(r, 0.2) ? { idleMillis: 1000, millisToMove: 2000 } : undefined,
  }
  const player = { id: 'pl', name: 'Ana', rating: 1500, title: chance(r, 0.2) ? 'GM' : null }
  const base: Record<string, unknown> = {
    gameState: { type: 'gameState', ...state },
    gameFull: {
      type: 'gameFull',
      id: gameId(r),
      white: player,
      black: { name: 'Bob', aiLevel: 3 },
      variant: { key: 'standard' },
      speed: 'blitz',
      initialFen: 'startpos',
      clock: chance(r, 0.8) ? { initial: 180, increment: 2 } : null,
      daysPerTurn: 3,
      tournamentId: 'abc',
      state,
    },
    gameStart: {
      type: 'gameStart',
      game: { gameId: gameId(r), color: 'white', opponent: { username: 'Bob' } },
    },
    gameFinish: {
      type: 'gameFinish',
      game: { gameId: gameId(r), color: pick(r, ['white', 'black']) },
    },
    opponentGone: { type: 'opponentGone', gone: chance(r, 0.5), claimWinInSeconds: 30 },
    chatLine: { type: 'chatLine', username: 'Bob', text: text(r, 60), room: 'player' },
    challenge: { type: 'challenge', challenge: { id: gameId(r) } },
    challengeCanceled: { type: 'challengeCanceled', challenge: { id: gameId(r) } },
    challengeDeclined: { type: 'challengeDeclined', challenge: { id: gameId(r) } },
  }
  // Round-trip through JSON so `undefined` entries are absent, as they are on the wire.
  const event = JSON.parse(JSON.stringify(base[kind])) as Record<string, unknown>
  if (chance(r, 0.4)) {
    // One or two mutations, at the top level or one level down.
    for (let m = int(r, 1, 2); m > 0; m--) {
      const nested = Object.keys(event).filter((k) => isRecord(event[k]))
      const target =
        nested.length && chance(r, 0.5)
          ? (event[pick(r, nested)] as Record<string, unknown>)
          : event
      const key = pick(r, Object.keys(target).concat(['extra', 'type']))
      if (chance(r, 0.15)) delete target[key]
      else target[key] = pick(r, MUTATIONS)
    }
  }
  if (chance(r, 0.15)) event.unknownExtra = { deep: [1, 'two', { three: null }] }
  return event
}

function randomLook(r: () => number): unknown {
  const color = (): unknown =>
    pick(r, [
      '#fff',
      '#ffffff',
      '#ffff',
      '#ffffff80',
      '#abcdef',
      '#ABCDEF',
      '#gggggg',
      '#ff',
      '#fffffffff',
      'red',
      'color-mix(in srgb, #fff 50%, #000)',
      'COLOR-MIX(IN SRGB, red)',
      'color-mix(in srgb,)',
      'color-mix(in srgb, #fff;)',
      '#fff;',
      'url(x)',
      'color-mix(in srgb, rgb(1,2,3) 10%, #000)',
      'color-mix(in srgb, é, #000)',
      'x'.repeat(241),
      '',
      'color-mix(in srgb,#000',
      null,
      3,
    ])
  const scheme = (): unknown => {
    if (chance(r, 0.05)) return pick(r, [null, [], 'str', undefined])
    const out: Record<string, unknown> = {}
    for (const key of ['bg', 'elevated', 'text', 'textMuted', 'primary', 'border']) {
      if (!chance(r, 0.03))
        out[key] = chance(r, 0.85)
          ? pick(r, ['#0f172a', '#fff', '#ABCDEF', 'color-mix(in srgb, #fff 50%, #000)'])
          : color()
    }
    return out
  }
  if (chance(r, 0.05)) return pick(r, [undefined, null, [], 'x', 1])
  return {
    appearance: pick(r, ['system', 'light', 'dark', 'neon', undefined, 3]),
    light: scheme(),
    dark: scheme(),
  }
}

/* ── Suites: the Rust rule of each TypeScript export, by its name ── */

const STANDARD_FEN = startOf('standard')
const FEN_SAMPLES = [
  STANDARD_FEN,
  startOf('chess960'),
  ...VARIANTS.map(startOf),
  '7k/5Q2/6K1/8/8/8/8/8 b - - 0 1',
  '7k/8/8/8/8/8/3K4/8 w - - 0 1',
  '7k/8/8/8/8/8/8/KQ6 w - - 0 1',
]

const SUITES = {
  reviewKey: ([fen, moves]: Args) => rules('reviewKey', fen, moves),
  hasScore: ([score]: Args) => rules('hasScore', score),
  endEval: ([end]: Args) => rules('endEval', end),
  isReviewablePerf: ([perf]: Args) => rules('isReviewablePerf', perf),
  summarizeStudy: ([pgn]: Args) => rules('summarizeStudy', pgn),
  normalizeRatingKey: ([name]: Args) => rules('normalizeRatingKey', name),
  ratingDisplayName: ([name]: Args) => rules('ratingDisplayName', name),
  isPuzzleHistory: ([name]: Args) => rules('isPuzzleHistory', name),
  ratingHistoryFromGames: ([games]: Args) => rules('ratingHistoryFromGames', games),
  mergeRatingHistories: ([official, fromGames]: Args) =>
    rules('mergeRatingHistories', official, fromGames),
  opponent: ([color]: Args) => rules('opponent', color),
  boardResult: ([setup, variant, draw]: Args) => rules('boardResult', setup, variant, draw),
  timeoutWinner: ([setup, flagged]: Args) => rules('timeoutWinner', setup, flagged),
  pgnResult: ([over, winner]: Args) => rules('pgnResult', over, winner),
  isGameInProgress: ([status]: Args) => rules('isGameInProgress', status),
  gameResult: ([game]: Args) => rules('gameResult', game),
  totalSeconds: ([minutes, increment]: Args) => rules('totalSeconds', minutes, increment),
  perfFor: ([minutes, increment]: Args) => rules('perfFor', minutes, increment),
  canBoardSeek: ([minutes, increment]: Args) => rules('canBoardSeek', minutes, increment),
  canDirectChallenge: ([minutes, increment]: Args) =>
    rules('canDirectChallenge', minutes, increment),
  canPlayOnline: ([minutes, increment, targeted]: Args) =>
    rules('canPlayOnline', minutes, increment, targeted),
  connectedAccountIndex: ([accounts, preferred]: Args) =>
    rules('connectedAccountIndex', accounts, preferred),
  validateOnlineEvent: ([raw]: Args) => rules('validateOnlineEvent', raw),
  lichessError: ([status, error, endpoint]: Args) => {
    // The message is the one `LichessError` builds from the rules' fields.
    const fields = rules<{ status: number; endpoint: string; detail: string }>(
      'lichessError',
      { status },
      error,
      endpoint,
    )
    const err = new LichessError(fields.status, fields.endpoint, fields.detail)
    return { status: err.status, endpoint: err.endpoint, message: err.message }
  },
  // Rust declines a look it cannot trust; the TypeScript wrapper answers with the default.
  oauthLook: ([value]: Args) => rules('oauthLook', value) ?? rules('oauthLookDefault'),
  analysisContext: ([root, moves]: Args) =>
    moves === undefined ? rules('analysisContext', root) : rules('analysisContext', root, moves),
}

/* ── Cases ── */

describe('records rules: review', () => {
  it('reviewKey, hasScore, endEval and isReviewablePerf', () => {
    const r = rng(3101)
    const cases: Args[] = [
      [STANDARD_FEN, []],
      [STANDARD_FEN, ['e2e4', 'e7e5', 'g1f3']],
      ['🙂 x', ['a2a4']],
      ['', ['']],
      ['f|g', ['e2e4', '']],
      [STANDARD_FEN, ['é😀 ', 'İ']],
    ]
    for (let i = 0; i < 400; i++) {
      const fen = chance(r, 0.6) ? pick(r, FEN_SAMPLES) : text(r, 20)
      cases.push([
        fen,
        Array.from({ length: int(r, 0, 12) }, () =>
          chance(r, 0.8)
            ? `${pick(r, [...'abcdefgh'])}${int(r, 1, 8)}${pick(r, [...'abcdefgh'])}${int(r, 1, 8)}`
            : text(r, 4),
        ),
      ])
    }
    run('reviewKey', cases)
    run('hasScore', [
      [{}],
      [{ cp: 0 }],
      [{ mate: 0 }],
      [{ cp: null }],
      [{ best: 'e2e4' }],
      [{ cp: 12, mate: undefined }],
      [{ cp: undefined, mate: 2 }],
      [{ mate: -3, best: 'e7e5' }],
    ])
    run('endEval', [['checkmate'], ['draw']])
    run(
      'isReviewablePerf',
      PERFS.map((perf) => [perf]).concat([
        ['classical'],
        ['correspondence'],
        ['ultraBullet'],
        ['chess960'],
      ]),
    )
  })
})

describe('records rules: studies', () => {
  it('summarizeStudy agrees on generated and malformed PGN', () => {
    const r = rng(3202)
    const cases: Args[] = [
      ['1. e4 e5 2. Nf3 Nc6 *'],
      ['[White "A"]\n[Black "?"]\n[Event "E"]\n\n1. d4 d5 (1... Nf6 2. c4) {hi} 2. c4 *'],
      ['[FEN "7k/8/8/8/8/8/3K4/8 w - - 0 1"]\n[SetUp "1"]\n\n1. Kd3 *'],
      [''],
      ['not a pgn at all'],
      ['1. e4 (1. d4 (1. c4) d5) e5 *'],
    ]
    for (let i = 0; i < 150; i++) cases.push([randomPgn(r)])
    run('summarizeStudy', cases)
  })
})

describe('records rules: ratings', () => {
  it('normalizes keys and display names', () => {
    const r = rng(3303)
    const names: Args[] = PERFS.map((name) => [name])
    names.push(
      ['King of the Hill'],
      ['Three-check'],
      ['UltraBullet'],
      [' Blitz '],
      ['İnvalid'],
      ['ΣΑΣ'],
      [undefined],
      [''],
      [null],
      [5],
    )
    for (let i = 0; i < 300; i++) names.push([text(r, 12)])
    run('normalizeRatingKey', names)
    run('ratingDisplayName', names)
    run('isPuzzleHistory', names)
    expect(rules('ratingDisplayNames')).toEqual(RATING_DISPLAY_NAMES)
  })

  it('rebuilds histories from games and merges them with Lichess histories', () => {
    const r = rng(3404)
    const game = (): Record<string, unknown> => {
      const g: Record<string, unknown> = {
        createdAt: BASE_DAY + int(r, 0, 400) * DAY + int(r, 0, DAY - 1),
        rated: chance(r, 0.85),
        perf: pick(r, PERFS),
      }
      if (chance(r, 0.9)) g.playerRating = int(r, 1000, 2600)
      if (chance(r, 0.9)) g.ratingDiff = int(r, -40, 40)
      if (chance(r, 0.03)) delete g.perf
      return g
    }
    const cases: Args[] = [
      [[]],
      [[{ createdAt: 0, rated: true, perf: 'blitz', playerRating: 1500, ratingDiff: 8 }]],
    ]
    for (let i = 0; i < 80; i++) cases.push([Array.from({ length: int(r, 0, 40) }, game)])
    run('ratingHistoryFromGames', cases)

    const official = [
      { name: 'blitz', points: [[2026, 0, 1, 1400]] },
      { name: 'rapid', points: [] },
      { name: 'Puzzles', points: [[2026, 0, 2, 2000]] },
    ]
    const merges: Args[] = [
      [official, []],
      [official, [{ name: 'rapid', points: [[2026, 0, 3, 2]] }]],
    ]
    for (let i = 0; i < 60; i++) {
      merges.push([
        official.slice(0, int(r, 0, 3)),
        [
          {
            name: pick(r, PERFS),
            points: [[int(r, 2000, 2030), int(r, 0, 11), int(r, 1, 28), int(r, 1000, 2500)]],
          },
        ],
      ])
    }
    run('mergeRatingHistories', merges)
  })
})

describe('records rules: game results', () => {
  it('names results, timeouts and PGN tokens from generated positions', () => {
    const r = rng(3505)
    const cases: Args[] = []
    const timeouts: Args[] = []
    for (const variant of VARIANTS) {
      for (let g = 0; g < 14; g++) {
        const start = Position.from({ variant, fen: startOf(variant) })!
        const { positions } = playRandom(start, r, int(r, 0, 120))
        for (const pos of positions.filter((_, i) => i % 3 === 0 || i === positions.length - 1)) {
          const setup = pos.setup
          cases.push([
            setup,
            pick(r, [variant, 'standard', 'kingOfTheHill', 'atomic', 'racingKings']),
            pick(r, [undefined, 'Threefold repetition', 'Fifty-move rule']),
          ])
          timeouts.push([setup, pick(r, ['white', 'black'])])
        }
      }
    }
    cases.push([
      { variant: 'standard', fen: '7k/5Q2/6K1/8/8/8/8/8 b - - 0 1' },
      'standard',
      undefined,
    ])
    cases.push([
      { variant: 'kingOfTheHill', fen: '7k/8/8/8/8/8/8/KQ6 w - - 0 1' },
      'kingOfTheHill',
      undefined,
    ])
    run('boardResult', cases)
    run('timeoutWinner', timeouts)
    run('pgnResult', [
      [true, 'white'],
      [true, 'black'],
      [true, undefined],
      [false, 'white'],
      [false, undefined],
      [true, 'nobody'],
    ])
    run('opponent', [['white'], ['black'], ['none']])
  })

  it('reads game status and results', () => {
    const r = rng(3606)
    run('isGameInProgress', [...STATUS.map((s) => [s]), [undefined], [''], [null]])
    const games: Args[] = []
    for (let i = 0; i < 120; i++) {
      games.push([
        {
          color: pick(r, ['white', 'black']),
          ...(chance(r, 0.8) ? { winner: pick(r, ['white', 'black', '']) } : {}),
        },
      ])
    }
    run('gameResult', games)
  })
})

describe('records rules: time controls and accounts', () => {
  it('applies the Lichess time-control rules', () => {
    const r = rng(3707)
    const cases: Args[] = []
    for (const [m, i] of [
      [0, 0],
      [0.5, 0],
      [1, 0],
      [2, 0],
      [3, 0],
      [5, 0],
      [8, 0],
      [10, 0],
      [3, 2],
      [5, 3],
      [15, 10],
      [25, 0],
      [1, 15],
    ]) {
      cases.push([m, i, true], [m, i, false], [m, i])
    }
    for (let i = 0; i < 120; i++) {
      const minutes = pick(r, [int(r, 0, 180), r() * 60, 0.25, 90]) as number
      const increment = pick(r, [int(r, 0, 30), r() * 5, 0])
      cases.push([minutes, increment, chance(r, 0.5)])
    }
    const pairs = cases.map(([m, i]) => [m, i])
    run('totalSeconds', pairs)
    run('perfFor', pairs)
    run('canBoardSeek', pairs)
    run('canDirectChallenge', pairs)
    run('canPlayOnline', cases)
  })

  it('chooses the connected account the way the sign-in picker does', () => {
    const r = rng(3808)
    const names = ['Alice', 'alice2', 'BOB', 'carol_99', 'watched', 'Dan-x', 'é_user']
    const cases: Args[] = []
    for (let i = 0; i < 200; i++) {
      const accounts = Array.from({ length: int(r, 0, 5) }, () => ({
        username: pick(r, names),
        connected: chance(r, 0.6),
      }))
      const preferred = pick(r, [undefined, '', ...names, names[0]!.toUpperCase(), 'nobody'])
      cases.push([accounts, preferred])
    }
    run('connectedAccountIndex', cases)
  })
})

describe('records rules: online events', () => {
  it('accepts and rejects Lichess events exactly as before', () => {
    const r = rng(3909)
    const cases: Args[] = [
      [undefined],
      [null],
      [0],
      ['gameState'],
      [[]],
      [['gameState']],
      [{}],
      [{ type: 'futureEvent' }],
      [{ type: ['gameState'], moves: '', status: 'started' }],
      [{ type: 'gameState', moves: ['e2e4'], status: 'started' }],
      [{ type: 'gameState', moves: 'e2e4 garbage', status: 'started' }],
      [{ type: 'gameState', moves: 'e2e4', status: 'started', wtime: 100, btime: 100 }],
      [{ type: 'gameState', moves: '', status: 'created' }],
      [{ type: 'gameState', moves: '   ', status: 'started' }],
      [{ type: 'gameState', moves: '  e2e4\te7e5  ', status: 'started' }],
      [{ type: 'gameState', moves: 'P@e4', status: 'started' }],
      [{ type: 'gameState', moves: 'e2e4 P@e4 e7e8q', status: 'started' }],
      [{ type: 'gameState', moves: 'e2e4', status: 'started', wtime: -1 }],
      [{ type: 'gameState', moves: 'e2e4', status: 'started', wtime: 0 }],
      [{ type: 'gameState', moves: 'e2e4', status: 'started', winner: 'draw' }],
      [{ type: 'gameState', moves: 'e2e4', status: 'started', expiration: { idleMillis: 1 } }],
      [{ type: 'gameState', moves: 'e2e4', status: 'started', extra: [1, { z: 2 }] }],
      [{ type: 'gameState', moves: 'e2e4', status: 'started', winc: null }],
      [
        {
          type: 'gameFull',
          id: 'Abcdefgh1',
          white: [],
          black: { name: 'x' },
          state: { moves: 'e2e4', status: 'started' },
        },
      ],
      [
        {
          type: 'gameFull',
          id: 'Abcdefgh1',
          white: { title: null },
          black: { title: 'x'.repeat(9) },
          state: { moves: 'e2e4', status: 'started' },
        },
      ],
      [
        {
          type: 'gameFull',
          id: 'short',
          white: {},
          black: {},
          state: { moves: '', status: 'started' },
        },
      ],
      [
        {
          type: 'gameFull',
          id: 'Abcdefgh1',
          white: {},
          black: {},
          clock: null,
          state: { moves: 'e7e5', status: 'created' },
        },
      ],
      [
        {
          type: 'gameFull',
          id: 'Abcdefgh1',
          white: {},
          black: {},
          clock: null,
          state: { moves: 'e7e5 Q@a1', status: 'created' },
        },
      ],
      [
        {
          type: 'gameFull',
          id: 'Abcdefgh1',
          white: {},
          black: {},
          variant: {},
          state: { moves: '', status: 'started' },
        },
      ],
      [
        {
          type: 'gameFull',
          id: 'Abcdefgh1',
          white: {},
          black: {},
          clock: { initial: 'x' },
          state: { moves: '', status: 'started' },
        },
      ],
      [
        {
          type: 'gameFull',
          id: 'Abcdefgh1',
          white: {},
          black: {},
          state: { moves: 'e2e4', status: 'started' },
        },
      ],
      [{ type: 'gameStart', game: { gameId: 'Abcdefgh1' } }],
      [{ type: 'gameStart', game: { gameId: 'bad id' } }],
      [{ type: 'gameFinish' }],
      [{ type: 'opponentGone', gone: true }],
      [{ type: 'opponentGone', gone: 'yes' }],
      [{ type: 'chatLine', username: 'x', text: 'hi', room: 'player' }],
      [{ type: 'chatLine', username: 'x'.repeat(41), text: 'hi', room: 'player' }],
      [{ type: 'challenge', challenge: { id: 'Abcdefgh1' } }],
      [{ type: 'challengeDeclined', challenge: {} }],
    ]
    for (let i = 0; i < 400; i++) cases.push([randomEvent(r)])
    run('validateOnlineEvent', cases)
  })
})

describe('records rules: errors, looks and contexts', () => {
  it('turns failed Lichess responses into the same errors', () => {
    const r = rng(4010)
    const errors: unknown[] = [
      undefined,
      null,
      '<!DOCTYPE html><html>',
      '   <html>',
      'Not found',
      '',
      'x'.repeat(300),
      { error: 'bad' },
      { b: 1, a: 2 },
      { '2': 'x', '1': 'y', z: 3 },
      [1, 2, 3],
      [],
      1.5,
      0,
      true,
      { deep: { list: [1, 'two', { three: null }] } },
      { long: 'z'.repeat(400) },
      'ö'.repeat(50),
    ]
    const cases: Args[] = []
    for (const status of [400, 401, 403, 404, 429, 500, 503]) {
      for (const error of errors) cases.push([status, error, '/api/x'])
    }
    for (let i = 0; i < 200; i++) {
      cases.push([
        pick(r, [404, 400, 500]),
        pick(r, [text(r, 30), { note: text(r, 80), n: int(r, 0, 9) }, [text(r, 4)], undefined]),
        pick(r, ['GET /api/user/magnus', 'POST /api/y', '']),
      ])
    }
    run('lichessError', cases)
  })

  it('applies the OAuth page look and its default', () => {
    const r = rng(4111)
    const cases: Args[] = [[undefined], [null], [DEFAULT_OAUTH_LOOK], [[]], ['light']]
    for (let i = 0; i < 300; i++) cases.push([randomLook(r)])
    run('oauthLook', cases)
    expect(rules('oauthLookDefault')).toEqual(DEFAULT_OAUTH_LOOK)
  })

  it('keys analyses by root position and moves', () => {
    const r = rng(4212)
    const cases: Args[] = [[STANDARD_FEN, []], [STANDARD_FEN], [STANDARD_FEN, ['e2e4']]]
    for (let i = 0; i < 200; i++) {
      cases.push([
        chance(r, 0.5) ? pick(r, FEN_SAMPLES) : text(r, 16),
        Array.from({ length: int(r, 0, 10) }, () => (chance(r, 0.8) ? 'e2e4' : text(r, 5))),
      ])
    }
    run('analysisContext', cases)
  })
})
