import { createHash } from 'node:crypto'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterAll, describe, expect, it } from 'vitest'
import { Chess } from 'chessops/chess'
import { makeFen, parseFen } from 'chessops/fen'
import { makeSanAndPlay } from 'chessops/san'
import { makeUci } from 'chessops/util'
import { defaultPosition, setupPosition } from 'chessops/variant'
import type { Position } from 'chessops/chess'
import type { Role } from 'chessops/types'
import {
  addMove,
  newTree,
  pvSan,
  setMoveGlyph,
  treeFromPgn,
  treeToPgn,
  type TreeNode,
} from '../../src/domain/analysisTree'
import {
  drawReason,
  pgnFromMoves,
  pgnFromSan,
  pgnFromUci,
  sanHistory,
  setupDrawReason,
  setupPgn,
  setupSanHistory,
} from '../../src/domain/chess'
import { wasmBinding, type RulesBinding } from '../../src/domain/engine'
import { studyDocumentPgn } from '../../src/domain/library'
import { analyseReview, replay } from '../../src/domain/review'
import type { StoredReview } from '../../src/contracts/types'
import {
  analyseStoredReview,
  createPuzzleSampler,
  lichessGameLine,
  studyMatchesCloud,
  summarizeReview,
} from '../../src/services/rules'
import { chess960Fen, replaySetup, rulesOf, VARIANTS, type Variant } from '../../src/domain/variant'
import { nativeRules } from '../../src/services/native'

const loaded = nativeRules()
if (!loaded) throw new Error('The native rules are not built (pnpm run build:native).')
const native = loaded

/**
 * The Rust rules must be indistinguishable from the TypeScript ones: rules the renderer still
 * shares run both and compare; Rust-only rules compare with outputs recorded from the TypeScript
 * rules they replaced (`native-golden.json`). Cases are generated from fixed seeds; a failure names its seed and
 * index so the exact input can be replayed.
 */

/** `KCHESS_FUZZ_SCALE=20` multiplies every generated case count for a deeper local search. */
const SCALE = Math.max(1, Number(process.env.KCHESS_FUZZ_SCALE) || 1)

/**
 * Golden outputs for rules that exist only in Rust: hashes of what the TypeScript rules
 * returned for each seeded case before they were removed. `KCHESS_WRITE_GOLDEN=1` rewrites
 * them from the current outputs; do that only for an intended behaviour change.
 */
// A file path: the test environment's global URL resolves against the renderer origin.
const GOLDEN_PATH = join(dirname(fileURLToPath(import.meta.url)), 'native-golden.json')
const WRITE_GOLDEN = process.env.KCHESS_WRITE_GOLDEN === '1'
const goldenFile: Record<string, string[]> = existsSync(GOLDEN_PATH)
  ? JSON.parse(readFileSync(GOLDEN_PATH, 'utf8'))
  : {}
const recorded: Record<string, string[]> = {}
/**
 * Sorted keys, and fractions to 9 significant digits: V8's Math.exp, and so review figures,
 * differ in the last bit between CPUs. Bit-exactness on each CPU is checked against Math.exp
 * itself (seeds 7105 and 7205); recorded outputs are shared by every platform.
 */
const canonical = (value: unknown): string =>
  JSON.stringify(value, (_key, inner: unknown) =>
    typeof inner === 'number' && !Number.isInteger(inner)
      ? Number(inner.toPrecision(9))
      : inner && typeof inner === 'object' && !Array.isArray(inner)
        ? Object.fromEntries(Object.entries(inner).sort(([a], [b]) => a.localeCompare(b)))
        : inner,
  ) ?? 'undefined'
const digest = (value: unknown): string =>
  createHash('sha256').update(canonical(value)).digest('hex').slice(0, 16)
function golden(suite: string, index: number, actual: unknown, reference?: unknown): void {
  if (WRITE_GOLDEN) {
    ;(recorded[suite] ??= [])[index] = digest(reference === undefined ? actual : reference)
    return
  }
  const expected = goldenFile[suite]?.[index]
  if (expected === undefined) return // cases beyond the recorded scale
  expect(digest(actual), `golden ${suite} #${index}: ${canonical(actual).slice(0, 300)}`).toBe(
    expected,
  )
}
afterAll(() => {
  if (WRITE_GOLDEN && Object.keys(recorded).length)
    writeFileSync(GOLDEN_PATH, JSON.stringify({ ...goldenFile, ...recorded }, null, 1) + '\n')
})

// Coverage instrumentation in CI slows the TypeScript side roughly tenfold (seed 7003 took
// 21.5 s there against 1.6 s locally), so the suites get generous limits.
const TIMEOUT = 120_000 * SCALE

function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const parse = <T>(json: string | null): T | undefined =>
  json === null ? undefined : JSON.parse(json)
/** Compare as values cross the boundary: `undefined` properties are simply absent. */
const plain = <T>(value: T): T => (value === undefined ? value : JSON.parse(JSON.stringify(value)))

/** A random legal line, with castling written both ways and some promotions under-promoted. */
function randomLine(random: () => number, start: Position, plies: number): string[] {
  const pos = start.clone()
  const moves: string[] = []
  for (let i = 0; i < plies && !pos.isEnd(); i++) {
    const legal = [...pos.allDests()].flatMap(([from, dests]) =>
      [...dests].map((to) => ({ from, to })),
    )
    if (!legal.length) break
    const move = legal[Math.floor(random() * legal.length)]!
    const piece = pos.board.get(move.from)
    const roles: Role[] = ['queen', 'rook', 'bishop', 'knight']
    if (pos.rules === 'antichess') roles.push('king')
    const promotion =
      piece?.role === 'pawn' && (move.to >> 3 === 7 || move.to >> 3 === 0)
        ? roles[Math.floor(random() * roles.length)]
        : undefined
    const full = promotion ? { ...move, promotion } : move
    let uci = makeUci(full)
    // chessops writes castling king-to-rook; Lichess also sends king-two-squares.
    if (piece?.role === 'king' && pos.board[pos.turn].has(move.to) && random() < 0.5) {
      const kingTo = (move.from & ~7) + (move.to > move.from ? 6 : 2)
      if (Math.abs(kingTo - move.from) === 2) uci = makeUci({ from: move.from, to: kingTo })
    }
    moves.push(uci)
    pos.play(full)
  }
  return moves
}

/** Damage a line now and then so the illegal-move cut-off is compared too. */
function damage(random: () => number, moves: string[]): string[] {
  const out = [...moves]
  const roll = random()
  if (roll < 0.1 && out.length) out[Math.floor(random() * out.length)] = 'e2e5'
  else if (roll < 0.15 && out.length) out.splice(Math.floor(random() * out.length), 1)
  else if (roll < 0.2) out.push(' e2e4 ', 'a7a8x', '', 'Q@e4')
  return out
}

const CHESS960 = Array.from({ length: 24 }, (_, i) => chess960Fen(i * 40 + 3))

describe('native rules match the TypeScript rules', { timeout: TIMEOUT }, () => {
  it('replays every variant identically (seed 7001)', () => {
    const random = rng(7001)
    for (let i = 0; i < 600 * SCALE; i++) {
      const variant = VARIANTS[i % VARIANTS.length] as Variant
      const start =
        variant === 'chess960'
          ? setupPosition('chess', parseFen(CHESS960[i % CHESS960.length]!).unwrap()).unwrap()
          : defaultPosition(rulesOf(variant))
      const fen = makeFen(start.toSetup())
      const moves = damage(random, randomLine(random, start, 40 + Math.floor(random() * 160)))
      const ts = replaySetup({ variant, fen }, moves)
      const rust = parse<{ played: unknown[]; fen: string }>(
        native.replaySetup(variant, fen, moves),
      )
      const context = `seed 7001 #${i} ${variant} ${fen} ${moves.join(' ')}`
      expect(rust?.played, context).toEqual(ts?.played)
      expect(rust?.fen, context).toBe(ts && makeFen(ts.position.toSetup()))
    }
  })

  it('replays positions for reviews identically (seed 7002)', () => {
    const random = rng(7002)
    for (let i = 0; i < 400 * SCALE; i++) {
      const line = randomLine(random, Chess.default(), 30 + Math.floor(random() * 120))
      const cut = Math.floor(random() * line.length)
      // Start mid-game so castling rights, en passant and counters vary.
      const at = replay(makeFen(Chess.default().toSetup()), line.slice(0, cut)).at(-1)!.fen
      const moves = damage(random, line.slice(cut))
      expect(JSON.parse(native.replayPositions(at, moves)), `seed 7002 #${i}`).toEqual(
        plain(replay(at, moves)),
      )
    }
  })

  it('accepts and rejects the same FENs', () => {
    const fens = [
      ...CHESS960,
      ...VARIANTS.map((v) => makeFen(defaultPosition(rulesOf(v)).toSetup())),
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1 ',
      ' rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR_w_KQkq_-_0_1',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w HAha - 0 1',
      'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1',
      'rnbqkbnr/pppp1ppp/8/8/4Pp2/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 3',
      'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e6 0 1',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 0',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 10000 1',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 3+3 0 1',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1 +1+2',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1 4+0',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR/Qq w KQkq - 0 1',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQ~KBNR w KQkq - 0 1',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBN~R w KQkq - 0 1',
      '8/8/8/8/8/8/8/8 w - - 0 1',
      '4k3/8/8/8/8/8/8/4K2R w K - 0 1',
      '4k3/8/8/8/8/8/8/R3K3 w KQ - 0 1',
      '4k3/8/8/8/8/8/8/4K3 w K - 0 1',
      'k7/8/8/8/8/8/8/K6Q b - - 0 1',
      'k7/Q7/8/8/8/8/8/K7 w - - 0 1',
      'k6P/8/8/8/8/8/8/K7 w - - 0 1',
      'kk6/8/8/8/8/8/8/K7 w - - 0 1',
      '4k3/8/8/8/8/8/8/4K3 x - - 0 1',
      '4k3/8/8/8/8/8/8/4K3 w - z9 0 1',
      '4k3/8/8/8/8/8/8/4K3 w - - a 1',
      '9/8/8/8/8/8/8/8 w - - 0 1',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkqA - 0 1',
      '1r2k1r1/8/8/8/8/8/8/1R2K1R1 w GBgb - 0 1',
      'r1k1r3/8/8/8/8/8/8/R1K1R3 w AEae - 0 1',
      '',
    ]
    for (const variant of VARIANTS)
      for (const fen of fens) {
        const ts = replaySetup({ variant, fen }, ['e2e4', 'e7e5'])
        const rust = parse<{ played: unknown[]; fen: string }>(
          native.replaySetup(variant, fen, ['e2e4', 'e7e5']),
        )
        expect(rust === undefined, `${variant} ${JSON.stringify(fen)}`).toBe(ts === undefined)
        if (ts) {
          expect(rust?.fen, `${variant} ${fen}`).toBe(makeFen(ts.position.toSetup()))
          expect(rust?.played, `${variant} ${fen}`).toEqual(ts.played)
        }
        expect(JSON.parse(native.replayPositions(fen, ['e2e4'])), fen).toEqual(
          plain(replay(fen, ['e2e4'])),
        )
      }
  })

  it('accepts and rejects the same generated FENs (seed 7006)', () => {
    const random = rng(7006)
    const pick = <T>(items: readonly T[]): T => items[Math.floor(random() * items.length)]!
    for (let i = 0; i < 400 * SCALE; i++) {
      const variant = pick(VARIANTS)
      const start =
        variant === 'chess960'
          ? setupPosition(
              'chess',
              parseFen(chess960Fen(Math.floor(random() * 960))).unwrap(),
            ).unwrap()
          : defaultPosition(rulesOf(variant))
      const line = randomLine(random, start, Math.floor(random() * 80))
      const reached = replaySetup({ variant, fen: makeFen(start.toSetup()) }, line)!.position
      const fields = makeFen(reached.toSetup()).split(' ')
      // Rewrite one field with something plausible but possibly wrong.
      const field = Math.floor(random() * 6)
      if (field === 1) fields[1] = pick(['w', 'b'])
      if (field === 2)
        fields[2] = pick(['-', 'KQkq', 'Kk', 'Qq', 'HAha', 'Bb', 'K', 'q', 'AHah', 'KQkqK'])
      if (field === 3) fields[3] = pick(['-', 'e3', 'e6', 'd6', 'a3', 'h6', 'c4'])
      if (field === 4) fields[4] = pick(['0', '99', '150', '151', '9999'])
      if (field === 5) fields[5] = pick(['1', '0', '9999', '120'])
      const fen = fields.join(' ')
      // The one deliberate difference: chessops keeps an en passant square that a piece
      // occupies (impossible after a double push) and would capture two pieces there.
      // The Rust rules drop such a square. Typed FENs are the only way to reach it.
      const ep = parseFen(fen)
      if (
        ep.isOk &&
        ep.value.epSquare !== undefined &&
        ep.value.board.occupied.has(ep.value.epSquare)
      )
        continue
      const replayed = ['e2e4', 'e7e5', 'g1f3', 'e1g1', 'e8c8'].concat(line.slice(0, 4))
      const ts = replaySetup({ variant, fen }, replayed)
      const rust = parse<{ played: unknown[]; fen: string }>(
        native.replaySetup(variant, fen, replayed),
      )
      const context = `seed 7006 #${i} ${variant} ${fen}`
      expect(rust === undefined, context).toBe(ts === undefined)
      if (ts) {
        expect(rust?.fen, context).toBe(makeFen(ts.position.toSetup()))
        expect(rust?.played, context).toEqual(ts.played)
      }
    }
  })

  it('builds identical analysis trees from generated studies (seed 7003)', () => {
    const random = rng(7003)
    for (let i = 0; i < 150 * SCALE; i++) {
      const pgn = randomStudy(random, 30 + Math.floor(random() * 80))
      expect(parse<TreeNode>(native.treeFromPgn(pgn)), `seed 7003 #${i}\n${pgn}`).toEqual(
        plain(treeFromPgn(pgn)),
      )
    }
  })

  it('reads hand-written PGN the same way', () => {
    for (const pgn of PGN_CASES) {
      const ts = plain(treeFromPgn(pgn))
      expect(parse<TreeNode>(native.treeFromPgn(pgn)), JSON.stringify(pgn)).toEqual(ts)
      expect(native.validPgn(pgn), JSON.stringify(pgn)).toBe(ts !== undefined)
    }
  })

  it('decodes library documents identically, including damaged ones (seed 7004)', () => {
    const random = rng(7004)
    for (let i = 0; i < 300 * SCALE; i++) {
      const variant = VARIANTS[i % VARIANTS.length] as Variant
      const start = defaultPosition(rulesOf(variant))
      const game: Record<string, unknown> = {
        id: `g${i}`,
        source: 'board',
        startedAt: i,
        updatedAt: i + 0.5,
        white: 'W',
        black: 'B',
        result: '*',
        reason: 'r',
        finished: random() < 0.5,
        setup: { variant, fen: makeFen(start.toSetup()) },
        moves: randomLine(random, start, 20 + Math.floor(random() * 60)),
        timeControl: '3+2',
        ...(random() < 0.3 ? { clockSummary: '1:00 – 0:59' } : {}),
      }
      const json = JSON.stringify(mutate(random, game))
      golden('archivedGame', i, parse(native.decodeArchivedGame(json)))
    }
  })

  it('decodes whole archives and single studies identically (seed 7008)', () => {
    const random = rng(7008)
    for (let i = 0; i < 60; i++) {
      const games = Array.from({ length: Math.floor(random() * 6) }, (_, g) => {
        const start = defaultPosition(rulesOf(VARIANTS[g % VARIANTS.length] as Variant))
        return mutate(random, {
          id: random() < 0.1 ? 'same' : `g${i}-${g}`,
          source: 'computer',
          startedAt: g,
          updatedAt: g,
          white: 'W',
          black: 'B',
          result: '*',
          reason: '',
          finished: true,
          setup: { variant: VARIANTS[g % VARIANTS.length], fen: makeFen(start.toSetup()) },
          moves: randomLine(random, start, 10 + Math.floor(random() * 30)),
          timeControl: '-',
        })
      })
      const archive = JSON.stringify(mutate(random, { version: random() < 0.9 ? 1 : '1', games }))
      golden('archive', i, parse(native.decodeArchive(archive)))
      const study = JSON.stringify(
        mutate(random, {
          id: `s${i}`,
          name: 'S',
          pgn: randomStudy(random, 10),
          updatedAt: i,
          ...(random() < 0.5
            ? { chapters: [{ id: 'c', name: 'C', pgn: random() < 0.2 ? '1. e5' : '1. d4 *' }] }
            : {}),
        }),
      )
      golden('study', i, parse(native.decodeStudy(study)))
    }
  })

  it('decodes studies and mistakes identically (seed 7005)', () => {
    const random = rng(7005)
    for (let i = 0; i < 40 * SCALE; i++) {
      const items = Array.from({ length: 1 + Math.floor(random() * 4) }, (_, s) => {
        const chapters = Array.from({ length: 1 + Math.floor(random() * 3) }, (_, c) => ({
          id: `c${c}`,
          name: `Chapter ${c}`,
          pgn:
            random() < 0.1
              ? '1. e4 e5 2. Ke3 *'
              : randomStudy(random, 10 + Math.floor(random() * 30)),
        }))
        return mutate(random, {
          id: `s${s}`,
          name: 'Study',
          pgn: chapters[0]!.pgn,
          ...(random() < 0.8 ? { chapters } : {}),
          updatedAt: s,
          ...(random() < 0.3
            ? {
                cloud: {
                  account: 'me',
                  id: random() < 0.8 ? 'AbCd1234' : 'bad',
                  downloadedPgn: '',
                },
              }
            : {}),
        })
      })
      const doc = JSON.stringify(mutate(random, { version: random() < 0.5 ? 2 : '1', items }))
      golden('studies', i, parse(native.decodeStudies(doc)))
      const mistakes = Array.from({ length: 1 + Math.floor(random() * 5) }, (_, m) => {
        const line = randomLine(random, Chess.default(), 12)
        const fen = replay(makeFen(Chess.default().toSetup()), line.slice(0, 8)).at(-1)!.fen
        return mutate(random, {
          id: `m${m}`,
          fen,
          solution: damage(random, line.slice(8)),
          judgment: random() < 0.5 ? 'Mistake' : 2.5,
          dueAt: m,
          streak: random() < 0.8 ? 1 : 1.5,
          attempts: 2,
        })
      })
      const mistakeDoc = JSON.stringify({ version: 1, items: mistakes })
      golden('mistakes', i, parse(native.decodeMistakes(mistakeDoc)))
    }
    for (const doc of ['null', '[]', '{}', '{"version":1}', '{"version":3,"items":[]}', 'not json'])
      for (const decode of ['decodeStudies', 'decodeMistakes', 'decodeArchivedGame'] as const)
        expect(native![decode](doc), `${decode} ${doc}`).toBeNull()
  })
})

describe('native phase 2 rules match the TypeScript rules', { timeout: TIMEOUT }, () => {
  it('analyses and summarizes reviews identically (seed 7101)', () => {
    const random = rng(7101)
    for (let i = 0; i < 300 * SCALE; i++) {
      const review = randomReview(random, i)
      const json = JSON.stringify(review)
      const context = `seed 7101 #${i} ${json.slice(0, 300)}`
      expect(parse(native.analyseReview(json)), context).toEqual(plain(analyseReview(review)))
      golden('summary', i, parse(native.summarizeReview(json)))
    }
  })

  it('computes Math.exp to the bit, as the review figures need (seed 7105)', () => {
    // V8's fdlibm exp fuses multiply-adds on arm64; a strict port differed in 572 of 1e6
    // arguments and changed one move's accuracy in its last digit (seed 7101 #475 at scale 20).
    const random = rng(7105)
    let mismatches = 0
    for (let i = 0; i < 200_000; i++) {
      const x =
        i % 3 === 0
          ? -0.00368208 * Math.round((random() - 0.5) * 2400)
          : i % 3 === 1
            ? -0.04354415386753951 * random() * 100
            : (random() - 0.5) * 1500
      if (!Object.is(native.jsExp(x), Math.exp(x))) mismatches++
    }
    expect(mismatches).toBe(0)
    for (const x of [0, -0, 1, -1, 0.5, 709.78, 709.79, -745.2, -745.1, Infinity, -Infinity, 1e-30])
      expect(Object.is(native.jsExp(x), Math.exp(x)), String(x)).toBe(true)
  })

  it('gives the same review figures through the core entry points (seed 7106)', () => {
    // services/rules.ts sends only scores across; engine lines must not matter.
    const random = rng(7106)
    for (let i = 0; i < 200 * SCALE; i++) {
      const review = randomReview(random, i)
      golden('summaryFacade', i, plain(summarizeReview(review)))
      expect(plain(analyseStoredReview(review)), `seed 7106 #${i}`).toEqual(
        plain(analyseReview(review)),
      )
    }
  })

  it('compares studies with their cloud copy identically (seed 7107)', () => {
    const random = rng(7107)
    for (let i = 0; i < 60 * SCALE; i++) {
      const chapters = Array.from({ length: 1 + Math.floor(random() * 3) }, (_, c) => ({
        id: `c${c}`,
        name: `Chapter ${c}`,
        pgn: randomStudy(random, 10 + Math.floor(random() * 40)),
      }))
      const document = studyDocumentPgn({ chapters })
      const roll = random()
      const downloaded =
        roll < 0.3
          ? document.replace(/\[ChapterName "[^"]*"\]\n/g, '[Site "https://lichess.org/x"]\n')
          : roll < 0.5
            ? document.replace(/ \$\d+/, '')
            : roll < 0.6
              ? ''
              : document
      const matches = studyMatchesCloud({ chapters }, downloaded)
      golden('cloudMatch', i, matches)
      expect(
        native.studyMatchesCloud(
          chapters.map((c) => [c.name, c.pgn]),
          downloaded,
        ),
        `seed 7107 #${i}`,
      ).toBe(matches)
    }
  })

  it('declines reviews whose start cannot be replayed', () => {
    // TypeScript throws here; the native rules decline so the caller gets the same error.
    const broken = { ...randomReview(rng(1), 0), fen: 'not a fen', evals: [{ cp: 10 }] }
    expect(() => analyseReview(broken)).toThrow()
    expect(native.analyseReview(JSON.stringify(broken))).toBeNull()
    const unscored = { ...broken, evals: [] }
    expect(parse(native.analyseReview(JSON.stringify(unscored)))).toEqual(
      plain(analyseReview(unscored)),
    )
  })

  it('turns synced Lichess games into the same lines (seed 7102)', () => {
    const random = rng(7102)
    for (let i = 0; i < 300 * SCALE; i++) {
      const start = random() < 0.2 ? CHESS960[i % CHESS960.length]! : undefined
      const pos = start
        ? setupPosition('chess', parseFen(start).unwrap()).unwrap()
        : Chess.default()
      const fen = makeFen(pos.toSetup())
      const line = randomLine(random, pos, 20 + Math.floor(random() * 100))
      const sans = replaySetup({ variant: 'standard', fen }, line)!.played.map((m) => m.san)
      if (random() < 0.15) sans.splice(Math.floor(random() * sans.length), 0, 'Qxz9')
      const moves = sans.join(random() < 0.2 ? '  \n ' : ' ')
      const roll = random()
      const game =
        roll < 0.3
          ? { moves, pgn: `[Event "x"]\n[FEN "${fen}"]\n\n${moves} *` }
          : roll < 0.5
            ? { moves, initialFen: fen }
            : roll < 0.6
              ? { moves, pgn: '[FEN ""]\n[FEN "8/8/8/8/8/8/8/8 w - - 0 1"]', initialFen: '' }
              : { moves, pgn: random() < 0.5 ? null : '' }
      const context = `seed 7102 #${i} ${JSON.stringify(game)}`
      const synced = native.lichessLine(game.moves, game.pgn ?? null, game.initialFen ?? null)
      golden('lichessLine', i, JSON.parse(synced))
      golden('sanToUci', i, native.sanToUci(fen, sans))
      expect(lichessGameLine(game), context).toEqual(JSON.parse(synced))
    }
  })

  it('writes study PGN identically (seed 7103)', () => {
    const random = rng(7103)
    for (let i = 0; i < 100 * SCALE; i++) {
      const chapters = Array.from({ length: 1 + Math.floor(random() * 4) }, (_, c) => ({
        id: `c${c}`,
        name: random() < 0.2 ? `Quote " and \\ ${c}` : `Chapter ${c}`,
        pgn: randomStudy(random, 10 + Math.floor(random() * 60)),
      }))
      const document = studyDocumentPgn({ chapters })
      expect(native.studyDocumentPgn(chapters.map((c) => [c.name, c.pgn])), `seed 7103 #${i}`).toBe(
        document,
      )
      golden('studyContent', i, native.studyContent(document))
    }
    for (const [index, pgn] of PGN_CASES.entries()) {
      golden('studyContentCases', index, native.studyContent(pgn))
      const games = pgn.trim() ? 1 : 0
      if (games && treeFromPgn(pgn))
        expect(native.studyDocumentPgn([['Name', pgn]]), JSON.stringify(pgn)).toBe(
          studyDocumentPgn({ chapters: [{ id: 'c', name: 'Name', pgn }] }),
        )
    }
    expect(native.studyDocumentPgn([['Empty', '']])).toBeNull()
  })

  it('samples the puzzle CSV identically when seeded alike (seed 7104)', () => {
    const random = rng(7104)
    for (let run = 0; run < 6 * SCALE; run++) {
      const csv = randomPuzzleCsv(random, 4000 + Math.floor(random() * 6000))
      const seed = Math.floor(random() * 2 ** 32)
      const rust = new native.PuzzleSampler(seed)
      for (let at = 0; at < csv.length;) {
        const size = 1 + Math.floor(random() * 9000)
        rust.push(csv.subarray(at, at + size))
        at += size
      }
      rust.finish()
      golden('sampler', run, {
        lines: rust.lines,
        count: rust.count,
        kept: JSON.parse(rust.kept()),
      })
    }
    // Regression (seed 7104 run 8 at scale 5): a theme listed twice on one line must see its
    // own first placement, so the 400th row fills the reservoir and the next one draws.
    const repeated = Buffer.from(
      Array.from(
        { length: 1200 },
        (_, i) => `r${i},8/8/8/8/8/8/8/K6k w - - 0 1,a1a2,1500,100,80,200,fork fork pin`,
      ).join('\n'),
    )
    const rustRepeat = new native.PuzzleSampler(42)
    rustRepeat.push(repeated)
    rustRepeat.finish()
    golden('samplerRepeat', 0, JSON.parse(rustRepeat.kept()))
    // The worker reports the kept count after storing the sample, which kept() frees.
    const facade = createPuzzleSampler()
    facade.push(repeated)
    facade.finish()
    const count = facade.count
    expect(facade.kept().length).toBeGreaterThan(0)
    expect(facade.count).toBe(count)
    const long = Buffer.from(`abc,${'x'.repeat(17_000)}`)
    expect(() => new native.PuzzleSampler(1).push(long)).toThrow('oversized CSV line')
  })
})

/** The renderer's WebAssembly module, run here in Node. */
const WASM_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  '../../../apps/desktop/app/assets/rules/kchess.wasm',
)
const wasm: RulesBinding = wasmBinding(
  new WebAssembly.Instance(new WebAssembly.Module(readFileSync(WASM_PATH))).exports,
)
/** One rules call through both runtimes, which must agree; returns the parsed result. */
function both(method: string, ...args: unknown[]): unknown {
  const json = JSON.stringify(args)
  const fromNode = native.invoke(method, json)
  expect(wasm.invoke(method, json), `${method} in WebAssembly`).toBe(fromNode)
  return JSON.parse(fromNode)
}
const orNull = <T>(value: T | undefined): T | null => (value === undefined ? null : value)

describe('the renderer rules match recorded outputs in both runtimes', { timeout: TIMEOUT }, () => {
  it('builds, edits and writes analysis trees identically (seed 7201)', () => {
    const random = rng(7201)
    for (let i = 0; i < 80; i++) {
      const pgn =
        random() < 0.15
          ? PGN_CASES[i % PGN_CASES.length]!
          : randomStudy(random, 20 + Math.floor(random() * 60))
      const tree = treeFromPgn(pgn)
      expect(both('treeFromPgn', pgn), `#${i}`).toEqual(orNull(plain(tree)))
      golden('treeFromPgn', i, both('treeFromPgn', pgn))
      if (!tree) continue
      // Edit the tree the way the analysis board does, then write it out.
      let path = ''
      for (let step = 0; step < 6; step++) {
        const node = path ? tree.children[0] : tree
        const fen = (node ?? tree).fen
        const pos = setupPosition('chess', parseFen(fen).unwrap()).unwrap()
        const [uci] = randomLine(random, pos, 1)
        if (!uci) break
        const played = addMove(tree, path, random() < 0.1 ? 'e2e5' : uci)
        if (played) {
          path = played
          if (random() < 0.3) setMoveGlyph(tree.children[0] ?? tree, 1 + Math.floor(random() * 6))
        }
      }
      golden('addMove', i, plain(tree))
      const headers =
        random() < 0.3 ? { Event: 'Edited "study"', Custom: 'x\\y' } : (tree.headers ?? {})
      expect(both('treeToPgn', tree, headers), `#${i}`).toBe(treeToPgn(tree, headers))
      golden('treeToPgn', i, both('treeToPgn', tree, headers))
    }
  })

  it('starts and extends trees from any position (seed 7202)', () => {
    const random = rng(7202)
    for (let i = 0; i < 300; i++) {
      const start =
        random() < 0.2
          ? setupPosition('chess', parseFen(CHESS960[i % CHESS960.length]!).unwrap()).unwrap()
          : Chess.default()
      const line = randomLine(random, start, Math.floor(random() * 60))
      const fen =
        random() < 0.05
          ? 'not a fen'
          : (replay(makeFen(start.toSetup()), line).at(-1)?.fen ?? makeFen(start.toSetup()))
      const root = newTree(fen)
      expect(both('startNode', fen), `#${i} ${fen}`).toEqual({ fen: root.fen, ply: root.ply })
      golden('startNode', i, both('startNode', fen))
      const pos = setupPosition('chess', parseFen(root.fen).unwrap()).unwrap()
      const next = randomLine(random, pos, 1)[0] ?? 'e2e4'
      const tries = [next, 'e1g1', 'e1h1', 'a7a8q', 'e2e5', next.toUpperCase()]
      for (const [k, uci] of tries.entries()) {
        const tree = newTree(fen)
        const path = addMove(tree, '', uci)
        const child = path === undefined ? null : tree.children[0]
        const expected = child ? { uci: child.uci, san: child.san, fen: child.fen } : null
        expect(both('playMove', root.fen, uci), `#${i} ${root.fen} ${uci}`).toEqual(expected)
        golden('playMove', i * 6 + k, both('playMove', root.fen, uci))
      }
      const pv = damage(random, randomLine(random, pos, 1 + Math.floor(random() * 16)))
      const max = random() < 0.5 ? 12 : 1 + Math.floor(random() * 20)
      expect(both('pvSan', root.fen, pv, max), `#${i}`).toEqual(pvSan(root.fen, pv, max))
      golden('pvSan', i, both('pvSan', root.fen, pv, max))
    }
  })

  it('writes move lists, PGN and draw claims identically (seed 7203)', () => {
    const random = rng(7203)
    for (let i = 0; i < 300; i++) {
      const variant = VARIANTS[i % VARIANTS.length] as Variant
      const start =
        variant === 'chess960'
          ? setupPosition('chess', parseFen(CHESS960[i % CHESS960.length]!).unwrap()).unwrap()
          : defaultPosition(rulesOf(variant))
      const setup = { variant, fen: makeFen(start.toSetup()) }
      // Shuffle knights back and forth now and then, so repetitions happen.
      const line =
        random() < 0.2
          ? ['g1f3', 'g8f6', 'f3g1', 'f6g8', 'g1f3', 'g8f6', 'f3g1', 'f6g8', 'b1c3']
          : damage(random, randomLine(random, start, Math.floor(random() * 200)))
      const standard = random() < 0.5 ? line : randomLine(random, Chess.default(), 120)
      const headers: Record<string, string> =
        random() < 0.5 ? {} : { White: 'Me', Result: random() < 0.5 ? '1-0' : '*', Site: '?' }
      const context = `seed 7203 #${i} ${variant}`
      expect(both('sanHistory', standard), context).toEqual(sanHistory(standard))
      // replaySetup keeps a TypeScript twin (its positions are chessops objects).
      const ts = replaySetup(setup, line)
      expect(both('setupReplay', setup, line), context).toEqual(
        ts ? plain({ played: ts.played, fen: makeFen(ts.position.toSetup()) }) : null,
      )
      expect(setupSanHistory(setup, line), context).toEqual(ts?.played.map((m) => m.san) ?? [])
      const sans = sanHistory(standard)
      if (random() < 0.2) sans.push('Zz9', 'O-O-O-O-O-O-O-O-O')
      expect(both('pgnFromSan', sans), context).toBe(pgnFromSan(sans))
      expect(both('pgnFromUci', standard), context).toBe(pgnFromUci(standard))
      const tokens = random() < 0.5 ? [...standard, ' 1-0 ', ''] : [...sans, '*']
      if (random() < 0.1) tokens.push('E2E4')
      expect(both('pgnFromMoves', tokens), context).toBe(pgnFromMoves(tokens))
      expect(both('setupPgn', setup, line, headers), context).toBe(setupPgn(setup, line, headers))
      expect(both('drawReason', standard), context).toEqual(orNull(drawReason(standard)))
      const padded = random() < 0.1 ? line.map((m, k) => (k === 3 ? ` ${m} ` : m)) : line
      expect(both('setupDrawReason', setup, padded), context).toEqual(
        orNull(setupDrawReason(setup, padded)),
      )
      for (const [suite, value] of [
        ['sanHistory', both('sanHistory', standard)],
        ['setupReplay', both('setupReplay', setup, line)],
        ['pgnFromSan', both('pgnFromSan', sans)],
        ['pgnFromUci', both('pgnFromUci', standard)],
        ['pgnFromMoves', both('pgnFromMoves', tokens)],
        ['setupPgn', both('setupPgn', setup, line, headers)],
        ['drawReason', both('drawReason', standard)],
        ['setupDrawReason', both('setupDrawReason', setup, padded)],
      ] as const)
        golden(suite, i, value)
    }
  })

  it('has an exp in WebAssembly that matches Math.exp to the bit on this CPU (seed 7205)', () => {
    // The loader keeps whichever variant agrees with Math.exp; one of them must agree always.
    const exports = new WebAssembly.Instance(new WebAssembly.Module(readFileSync(WASM_PATH)))
      .exports as unknown as { kc_exp(x: number, fused: number): number }
    const random = rng(7205)
    let fused = 0
    let plain = 0
    const total = 200_000
    for (let i = 0; i < total; i++) {
      const x = i % 2 ? -0.00368208 * Math.round((random() - 0.5) * 2400) : -4.354 * random()
      if (Object.is(exports.kc_exp(x, 1), Math.exp(x))) fused++
      if (Object.is(exports.kc_exp(x, 0), Math.exp(x))) plain++
    }
    expect(Math.max(fused, plain)).toBe(total)
  })

  it('replays, reviews and exports studies identically in both runtimes (seed 7204)', () => {
    const random = rng(7204)
    for (let i = 0; i < 150; i++) {
      const review = randomReview(random, i)
      expect(both('replay', review.fen, review.moves), `#${i}`).toEqual(
        plain(replay(review.fen, review.moves)),
      )
      golden('replay', i, both('replay', review.fen, review.moves))
      expect(both('analyseReview', review), `#${i}`).toEqual(plain(analyseReview(review)))
      golden('analyseReview', i, both('analyseReview', review))
      const chapters = Array.from({ length: 1 + Math.floor(random() * 3) }, (_, c) => ({
        id: `c${c}`,
        name: `Chapter ${c}`,
        pgn: randomStudy(random, 10 + Math.floor(random() * 30)),
      }))
      expect(both('studyDocumentPgn', chapters), `#${i}`).toBe(studyDocumentPgn({ chapters }))
      golden('studyDocumentPgn', i, both('studyDocumentPgn', chapters))
    }
    expect(() => native.invoke('replay', '[1]')).toThrow('fen must be a string')
    expect(() => wasm.invoke('replay', '[1]')).toThrow('fen must be a string')
    expect(() => wasm.invoke('nope', '[]')).toThrow('unknown rules method')
  })
})

function randomReview(random: () => number, i: number): StoredReview {
  const fen =
    random() < 0.15
      ? CHESS960[i % CHESS960.length]!
      : 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
  const pos = setupPosition('chess', parseFen(fen).unwrap()).unwrap()
  const moves = damage(random, randomLine(random, pos, Math.floor(random() * 140)))
  let cp = Math.round((random() - 0.5) * 100)
  const evals = Array.from({ length: moves.length + 1 - Math.floor(random() * 3) }, () => {
    const roll = random()
    if (roll < 0.05) return null
    if (roll < 0.08) return {}
    if (roll < 0.13) return { mate: Math.round((random() - 0.5) * 30), depth: 20 }
    if (roll < 0.15) return { mate: 0 }
    cp += Math.round((random() - 0.5) * (random() < 0.1 ? 3000 : 300))
    return random() < 0.1 ? { cp: cp + 0.5 } : { cp, best: 'e2e4', depth: 18 }
  })
  const lichess = random() < 0.3
  return {
    key: `k${i}`,
    fen,
    moves,
    source: lichess ? 'lichess' : 'local',
    evals,
    ...(lichess && random() < 0.8
      ? {
          judgments: moves.map(
            () => [null, null, 'inaccuracy', 'mistake', 'blunder'][Math.floor(random() * 5)],
          ) as StoredReview['judgments'],
          accuracy: random() < 0.5 ? { white: random() * 100 } : { white: 81.5, black: 62.5 },
        }
      : {}),
    depth: 18,
    complete: random() < 0.7,
    updatedAt: i,
  }
}

function randomPuzzleCsv(random: () => number, lines: number): Buffer {
  const themes = ['fork', 'pin', 'mate', 'mateIn2', 'endgame', 'short', 'crushing', 'x'.repeat(41)]
  const rows = ['PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl']
  for (let i = 0; i < lines; i++) {
    const roll = random()
    const id = roll < 0.01 ? 'bad-id' : (i * 104729).toString(36)
    const rating = roll < 0.02 ? 'x' : roll < 0.03 ? '' : String(Math.round(random() * 3200))
    const tagCount = Math.floor(random() * 4)
    const tags = Array.from(
      { length: tagCount },
      () => themes[Math.floor(random() * themes.length)],
    )
    if (random() < 0.02) tags.push(`t${Math.floor(random() * 400)}`)
    rows.push(
      [
        id,
        '8/8/8/8/8/8/8/K6k w - - 0 1',
        'a1a2 h1h2',
        rating,
        String(Math.round(40 + random() * 100)),
        String(Math.round(50 + random() * 51)),
        roll < 0.04 ? ' 0x200 ' : String(Math.round(random() * 2000)),
        tags.join(' '),
        roll > 0.98 ? 'naïve,é' : 'https://lichess.org/x',
      ].join(','),
    )
  }
  return Buffer.from(rows.join(random() < 0.5 ? '\n' : '\r\n'))
}

/** Replace, remove or retype one field now and then. */
function mutate<T extends Record<string, unknown>>(random: () => number, value: T): T {
  if (random() > 0.35) return value
  const out: Record<string, unknown> = { ...value }
  const keys = Object.keys(out)
  const key = keys[Math.floor(random() * keys.length)]!
  const choices: unknown[] = [undefined, null, 0, 1, '', 'x'.repeat(250), true, [], {}, '2', 1.5]
  const replacement = choices[Math.floor(random() * choices.length)]
  if (replacement === undefined) delete out[key]
  else out[key] = replacement
  return out as T
}

function randomStudy(random: () => number, plies: number): string {
  const root: TreeNode = {
    uci: '',
    san: '',
    fen: makeFen(Chess.default().toSetup()),
    ply: 0,
    children: [],
  }
  const grow = (node: TreeNode, pos: Position, depth: number, length: number): void => {
    let current = node
    for (let i = 0; i < length && !pos.isEnd(); i++) {
      const legal = [...pos.allDests()].flatMap(([from, dests]) =>
        [...dests].map((to) => ({ from, to })),
      )
      const before = pos.clone()
      let next: { node: TreeNode; pos: Position } | undefined
      for (let b = 0; b < (depth < 3 && random() < 0.2 ? 3 : 1); b++) {
        const position = before.clone()
        const move = legal[Math.floor(random() * legal.length)]!
        const piece = position.board.get(move.from)
        const full =
          piece?.role === 'pawn' && (move.to >> 3 === 7 || move.to >> 3 === 0)
            ? { ...move, promotion: random() < 0.7 ? ('queen' as const) : ('knight' as const) }
            : move
        const uci = makeUci(full)
        if (current.children.some((c) => c.uci === uci)) continue
        const san = makeSanAndPlay(position, full)
        const child: TreeNode = {
          uci,
          san,
          fen: makeFen(position.toSetup()),
          ply: current.ply + 1,
          children: [],
          ...(random() < 0.2 ? { comments: [`Note ${i}: ${san} {braces} [%eval 0.3]`] } : {}),
          ...(random() < 0.05 ? { startingComments: ['Before'] } : {}),
          ...(random() < 0.15 ? { nags: [1 + Math.floor(random() * 20)] } : {}),
        }
        current.children.push(child)
        if (b === 0) next = { node: child, pos: position }
        else grow(child, position, depth + 1, 3 + Math.floor(random() * 10))
      }
      if (!next) break
      current = next.node
      pos = next.pos
    }
  }
  grow(root, Chess.default(), 0, plies)
  return treeToPgn(root, { Event: 'Fuzz', Annotator: 'Seed "quoted" \\ backslash' })
}

const PGN_CASES: string[] = [
  '',
  '*',
  '1. e4 e5 2. Nf3 Nc6 *',
  '﻿[Event "BOM"]\n\n1. e4 *',
  '% escape line\n[Event "x"]\n% another\n1. d4 d5 *',
  '[Event "a"][Site "b"] 1. e4 *',
  '[Event "Quote \\" and \\\\ slash"]\n\n1. e4 *',
  '1. e4 { multi\nline\n comment } e5 { } 2. Nf3 {x}{y} Nc6 *',
  '{ before } ( 1. d4 ) 1. e4 *',
  '1. e4 ( { start } 1. d4 d5 ( 1... Nf6 2. c4 ) 2. c4 ) 1... e5 *',
  '1. e4 e5 ; rest of line ignored 2. Nf3\n2. Nc3 *',
  '1. e4! e5? 2. Nf3!! Nc6?? 3. Bb5!? a6?! $10 $255 $1000 $99999 *',
  '1. e4 e5 2. Nf3 Nc6 3. Bc4 Nf6 4. 0-0 Be7 5. Re1 O-O *',
  '1. e4 e5 2. Nf3 Nc6 3. Bc4 Nf6 4. o-o *',
  '1. e4 e5 2. Nf3 Nc6 3. Bc4 Nf6 4. O–O *',
  '1. d4 d5 2. Nc3 Nc6 3. Bf4 Bf5 4. Qd2 Qd7 5. O-O-O+ *',
  '1. Ng1f3 d5 2. N1d2 *',
  '1. e4 d5 2. exd5 Nf6 3. d4 Nxd5 4. Ngf3 *',
  '1. e4 d5 2. e4xd5 *',
  '1. e4 d5 2. e5 f5 3. exf6 *',
  '1. e4 d5 2. e5 f5 3. exf6e.p. *',
  '[FEN "4k3/1P6/8/8/8/8/8/4K3 w - - 0 1"]\n\n1. b8=Q+ *',
  '[FEN "4k3/1P6/8/8/8/8/8/4K3 w - - 0 1"]\n\n1. b8Q *',
  '[FEN "4k3/1P6/8/8/8/8/8/4K3 w - - 0 1"]\n\n1. b8=n *',
  '[FEN "4k3/1P6/8/8/8/8/8/4K3 w - - 0 1"]\n\n1. b8 *',
  '[FEN "4k3/8/8/8/8/8/8/4K3 b - - 5 40"]\n\n40... Kd7 41. Kd2 *',
  '[FEN "bad fen"]\n\n1. e4 *',
  '[FEN ""]\n\n1. e4 *',
  '[Variant "Chess960"]\n[FEN "bqnb1rkr/pp3ppp/3ppn2/2p5/5P2/P2P4/NPP1P1PP/BQ1BNRKR w HFhf - 2 9"]\n\n9. g3 Bc7 10. O-O *',
  '[Variant "King of the Hill"]\n\n1. e4 e5 2. Ke2 Ke7 3. Kd3 Kd6 4. Kd4+ *',
  '[Variant "Three-check"]\n\n1. e4 e5 2. Bc4 Nc6 3. Bxf7+ Kxf7 4. Qh5+ g6 5. Qxg6+ *',
  '[Variant "Antichess"]\n\n1. e3 b5 2. Bxb5 *',
  '[Variant "Atomic"]\n\n1. Nf3 d5 2. Ng5 h6 3. Nxf7 *',
  '[Variant "Horde"]\n\n1. e5 e6 *',
  '[Variant "Racing Kings"]\n\n1. Kh3 Ka3 *',
  '[Variant "Crazyhouse"]\n\n1. e4 d5 2. exd5 Qxd5 3. Nc3 Qd8 4. P@e4 N@f6 5. Q@h5 *',
  '[Variant "Crazyhouse"]\n[FEN "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[Pp] w KQkq - 0 1"]\n\n1. P@e4 *',
  '[Variant "Suicide"]\n\n1. e4 *',
  '[Variant "Anti"]\n\n1. e3 b5 2. Bxb5 *',
  '1. e4 -- 2. d4 *',
  '1. e4 Z0 *',
  '1. e4 e5 1-0',
  '1. e4 e5 1–0 2. Nf3',
  '[Result "0-1"]\n\n1. e4 1/2-1/2',
  '[Result "garbage"]\n\n1. e4 *',
  '1. e4 e5\n\n1. d4 d5',
  '1. e4 e5\n   \n',
  '1. e4\r\ne5\r\n2. Nf3 *\r\n',
  '1. e4 ) ) ( e5 *',
  '1. e4 ( e5 *',
  '1. e4 { unterminated comment',
  '1. e4 {a}\n{b} e5 *',
  '1. e9 *',
  '1. Qh5 *',
  '1. Nc3 Nc6 2. Nb5 Nb4 3. Nd4 Nd5 4. Nf3 Nf6 5. Nd4 *',
  '[FEN "4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1"]\n\n1. Rd1 *',
  '[FEN "4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1"]\n\n1. Rad1 *',
  '[FEN "1k6/8/8/8/8/2Q1Q3/8/2Q1K3 w - - 0 1"]\n\n1. Qd2 *',
  '[FEN "1k6/8/8/8/8/2Q1Q3/8/2Q1K3 w - - 0 1"]\n\n1. Qc3d2 *',
  '[FEN "1k6/8/8/8/8/2Q1Q3/8/2Q1K3 w - - 0 1"]\n\n1. Qc1d2 *',
  '1.e4 e5 2.Nf3 Nc6 3.Bb5 a6 *',
  '1... e5 *',
  '[Event "Only headers"]',
  '[Event "x"]\n[Site "y"]\n[Date "2024.01.01"]\n[Round "1"]\n[White "W"]\n[Black "B"]\n[Result "1-0"]\n[WhiteElo "2000"]\n\n1. e4 1-0',
  '[Event "Unicode ♞ héllo 😀"]\n\n1. e4 { ünïcode ✓ 😀 } *',
  '1. e4 $0 $5 e5 *',
  '1. e4 e5 2. Ke2 Ke7 3. Ke3 Ke6 4. Ke2 Ke7 5. Ke3 Ke6 6. Ke2 *',
]
