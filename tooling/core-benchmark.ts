/**
 * Core rules benchmark: the CPU-bound work the core does on every library load, study save,
 * review and puzzle download, through the same entry points the services call. Fixtures come
 * from a fixed seed, so every run sees the same games; `benchmark-baseline/typescript.json`
 * holds the TypeScript rules' numbers on them. Run with `pnpm run benchmark:core`.
 */
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { treeToPgn, type TreeNode } from '../core/src/domain/analysisTree.ts'
import { replay, reviewKey } from '../core/src/domain/review.ts'
import type { ReviewSummary, StoredReview } from '../core/src/contracts/types.ts'
import {
  studyDocumentPgn,
  type ArchivedGame,
  type SavedStudy,
  type MistakeExercise,
} from '../core/src/domain/library.ts'
import { INITIAL_FEN, Position, type SquareName } from '../core/src/domain/position.ts'
import {
  defaultFen,
  replaySetup,
  type GameSetup,
  type Variant,
} from '../core/src/domain/variant.ts'
import { nativeRules, type NativeRules } from '../core/src/services/native.ts'
import {
  createPuzzleSampler,
  decodeArchivedGameText,
  decodeMistakesText,
  decodeStudiesText,
  lichessGameLine,
  replayPositions,
  studyMatchesCloud,
  summarizeReview,
} from '../core/src/services/rules.ts'

/* ── Seeded fixtures ── */

let seed = 0x6b636865
const random = (): number => {
  seed = (seed + 0x6d2b79f5) | 0
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296
}
const pick = <T>(items: readonly T[]): T => items[Math.floor(random() * items.length)]!

/** A UCI move from `pos`, promoting to a queen when a pawn reaches the last rank. */
function moveUci(pos: Position, from: SquareName, to: SquareName): string {
  const promotes = pos.pieceAt(from)?.role === 'pawn' && (to[1] === '1' || to[1] === '8')
  return `${from}${to}${promotes ? 'q' : ''}`
}

function randomGame(variant: Variant, plies: number): { setup: GameSetup; moves: string[] } {
  const setup = { variant, fen: defaultFen(variant) }
  let pos = Position.from(setup)!
  const moves: string[] = []
  for (let i = 0; i < plies && !pos.isEnd(); i++) {
    const legal = [...pos.dests('rules')].flatMap(([from, dests]) =>
      [...dests].map((to) => ({ from, to })),
    )
    const move = pick(legal)
    const played = pos.play(moveUci(pos, move.from, move.to))!
    moves.push(played.uci)
    pos = played.position
  }
  return { setup, moves }
}

/** A study chapter: a main line with side variations, comments and glyphs. */
function randomStudyTree(plies: number, branching = 0.12): string {
  const root: TreeNode = {
    uci: '',
    san: '',
    fen: INITIAL_FEN,
    ply: 0,
    children: [],
  }
  const grow = (node: TreeNode, pos: Position, depth: number, length: number): void => {
    let current = node
    for (let i = 0; i < length && !pos.isEnd(); i++) {
      const legal = [...pos.dests('rules')].flatMap(([from, dests]) =>
        [...dests].map((to) => ({ from, to })),
      )
      const branches = depth < 2 && random() < branching ? 2 : 1
      let next: { node: TreeNode; pos: Position } | undefined
      for (let b = 0; b < branches; b++) {
        const move = pick(legal)
        const played = pos.play(moveUci(pos, move.from, move.to))!
        const { uci, san } = played
        if (current.children.some((c) => c.uci === uci)) continue
        const child: TreeNode = {
          uci,
          san,
          fen: played.position.fen,
          ply: current.ply + 1,
          children: [],
          ...(random() < 0.15 ? { comments: ['Idea: ' + san + ' keeps the tension.'] } : {}),
          ...(random() < 0.1 ? { nags: [pick([1, 2, 3, 4, 5, 6])] } : {}),
        }
        current.children.push(child)
        if (b === 0) next = { node: child, pos: played.position }
        else grow(child, played.position, depth + 1, Math.floor(4 + random() * 8))
      }
      if (!next) break
      current = next.node
      pos = next.pos
    }
  }
  grow(root, Position.initial(), 0, plies)
  return treeToPgn(root, { Event: 'Benchmark study', White: 'Seed', Black: 'Fixture' })
}

const VARIANT_MIX: Variant[] = [
  ...Array<Variant>(14).fill('standard'),
  'chess960',
  'kingOfTheHill',
  'threeCheck',
  'antichess',
  'atomic',
  'racingKings',
]

function makeFixtures() {
  const games: ArchivedGame[] = []
  for (let i = 0; i < 500; i++) {
    const variant = pick(VARIANT_MIX)
    const { setup, moves } = randomGame(variant, 60 + Math.floor(random() * 100))
    games.push({
      id: `game-${i}`,
      source: pick(['computer', 'board', 'clock'] as const),
      startedAt: 1_700_000_000_000 + i * 60_000,
      updatedAt: 1_700_000_000_000 + i * 60_000 + 30_000,
      white: 'White player',
      black: 'Black player',
      result: '*',
      reason: '',
      finished: false,
      setup,
      moves,
      timeControl: '5+0',
    })
  }
  const studies = {
    version: 2,
    items: Array.from({ length: 50 }, (_, s) => {
      const chapters = Array.from({ length: 8 }, (_, c) => ({
        id: `s${s}c${c}`,
        name: `Chapter ${c + 1}`,
        pgn: randomStudyTree(40 + Math.floor(random() * 50)),
      }))
      return { id: `s${s}`, name: `Study ${s}`, pgn: chapters[0]!.pgn, chapters, updatedAt: s }
    }),
  }
  const mistakes: MistakeExercise[] = []
  for (let i = 0; i < 500; i++) {
    const game = randomGame('standard', 20 + Math.floor(random() * 40))
    const at = replay(game.setup.fen, game.moves)
    const start = Math.max(0, at.length - 6)
    mistakes.push({
      id: `m${i}`,
      fen: at[start]!.fen,
      solution: game.moves.slice(start, start + 4),
      judgment: 'Mistake',
      dueAt: i,
      streak: 0,
      attempts: 0,
    })
  }
  const bigPgn = randomStudyTree(400, 0.6)
  // Phase 2 fixtures come after the first ones so those stay byte-for-byte the same.
  return {
    games,
    studies,
    mistakes: { version: 1, items: mistakes },
    bigPgn,
    puzzleChunks: puzzleCsv(PUZZLE_LINES),
    ...reviewFixtures(),
    syncedStudies: studies.items.map((study) => ({
      ...study,
      cloud: { account: 'me', id: 'AbCd1234', downloadedPgn: studyDocumentPgn(study) },
    })) as SavedStudy[],
  }
}

/** Lichess's puzzle CSV (decompressed), in the 64 KiB chunks a download arrives in. */
const PUZZLE_LINES = 300_000
const THEMES = [
  'advantage',
  'crushing',
  'endgame',
  'middlegame',
  'opening',
  'short',
  'long',
  'veryLong',
  'mate',
  'mateIn1',
  'mateIn2',
  'mateIn3',
  'fork',
  'pin',
  'skewer',
  'sacrifice',
  'discoveredAttack',
  'hangingPiece',
  'kingsideAttack',
  'queensideAttack',
  'defensiveMove',
  'deflection',
  'attraction',
  'backRankMate',
  'rookEndgame',
  'pawnEndgame',
  'promotion',
  'quietMove',
  'zugzwang',
  'master',
]
function puzzleCsv(lines: number): Buffer[] {
  const rows = [
    'PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl,OpeningTags',
  ]
  for (let i = 0; i < lines; i++) {
    const id = (i * 7919 + 4096).toString(36).padStart(5, '0')
    const rating = Math.round(400 + random() * 2600 + (random() - 0.5) * 400)
    const deviation = Math.round(60 + random() * 150)
    const popularity = Math.round(60 + random() * 41)
    const plays = Math.round(random() ** 2 * 20_000)
    const themes = Array.from({ length: 1 + Math.floor(random() * 4) }, () => pick(THEMES)).join(
      ' ',
    )
    rows.push(
      `${id},r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4,` +
        `f3g5 d7d5 e4d5 f6d5,${rating},${deviation},${popularity},${plays},${themes},` +
        `https://lichess.org/abcdefgh#${i % 120},Italian_Game Italian_Game_Two_Knights_Defense`,
    )
  }
  const bytes = Buffer.from(rows.join('\n') + '\n')
  const chunks: Buffer[] = []
  for (let at = 0; at < bytes.length; at += 65_536) chunks.push(bytes.subarray(at, at + 65_536))
  return chunks
}

/** Synced Lichess games (SAN, as stored) and their reviews with a score for every position. */
function reviewFixtures() {
  const lichessGames: { moves: string; pgn: string | null; initialFen?: string }[] = []
  const reviews: StoredReview[] = []
  for (let i = 0; i < 300; i++) {
    const { setup, moves } = randomGame('standard', 40 + Math.floor(random() * 100))
    const sans = replaySetup(setup, moves)!.played.map((move) => move.san)
    lichessGames.push({
      moves: sans.join(' '),
      pgn: i % 3 === 0 ? `[Event "Rated blitz game"]\n\n${sans.join(' ')} *` : null,
    })
    let cp = 20
    const lichess = i % 4 === 0
    const evals = replay(setup.fen, moves).map((position, at) => {
      cp += Math.round((random() - 0.5) * (at > 30 ? 260 : 80))
      if (position.end) return null
      if (random() < 0.03)
        return { mate: (random() < 0.5 ? -1 : 1) * (1 + Math.floor(random() * 8)), depth: 18 }
      return { cp, best: moves[at] ?? 'e2e4', pv: moves.slice(at, at + 6), depth: 18 }
    })
    reviews.push({
      key: reviewKey(setup.fen, moves),
      fen: setup.fen,
      moves,
      source: lichess ? 'lichess' : 'local',
      evals,
      ...(lichess
        ? {
            judgments: moves.map(() =>
              pick([null, null, null, 'inaccuracy', 'mistake', 'blunder'] as const),
            ),
            accuracy: { white: 80 + random() * 15, black: 70 + random() * 25 },
          }
        : {}),
      depth: lichess ? 0 : 18,
      complete: true,
      updatedAt: i,
    })
  }
  return { lichessGames, reviews }
}

/* ── Implementations under test ── */

interface Rules {
  replayGames(games: readonly ArchivedGame[]): number
  reviewReplay(games: readonly ArchivedGame[]): number
  treeFromPgn(pgn: string): number
  loadArchive(rows: readonly { body: string }[]): number
  loadStudies(body: string): number
  loadMistakes(body: string): number
  puzzleSample(chunks: readonly Buffer[]): number
  lichessLines(games: readonly { moves: string; pgn: string | null }[]): number
  reviewSummaries(reviews: readonly StoredReview[]): number
  studySyncCheck(studies: readonly SavedStudy[]): number
}

const reviewTotals = (summary: ReviewSummary): number =>
  summary.white.inaccuracy +
  summary.white.mistake +
  summary.white.blunder +
  summary.black.inaccuracy +
  summary.black.mistake +
  summary.black.blunder +
  (summary.white.accuracy ?? 0) +
  (summary.black.accuracy ?? 0) +
  (summary.white.acpl ?? 0)

const countNodes = (node: TreeNode): number =>
  node.children.reduce((sum, child) => sum + countNodes(child), 1)

/** The Rust rules through the same entry points the core services call (`services/rules.ts`). */
function nativeImpl(native: NativeRules): Rules {
  return {
    replayGames: (games) =>
      games.reduce(
        (sum, g) =>
          sum +
          (JSON.parse(native.replaySetup(g.setup.variant, g.setup.fen, g.moves) ?? 'null')?.played
            .length ?? 0),
        0,
      ),
    reviewReplay: (games) =>
      games.reduce((sum, g) => sum + replayPositions(g.setup.fen, g.moves).length, 0),
    treeFromPgn: (pgn) => countNodes(JSON.parse(native.treeFromPgn(pgn)!)),
    loadArchive: (rows) => rows.filter((row) => decodeArchivedGameText(row.body)).length,
    loadStudies: (body) => decodeStudiesText(body)?.length ?? -1,
    loadMistakes: (body) => decodeMistakesText(body)?.length ?? -1,
    puzzleSample: (chunks) => {
      const sampler = createPuzzleSampler()
      for (const chunk of chunks) sampler.push(chunk)
      sampler.finish()
      sampler.kept()
      return sampler.lines + sampler.count
    },
    lichessLines: (games) => games.reduce((sum, g) => sum + lichessGameLine(g).moves.length, 0),
    reviewSummaries: (reviews) =>
      reviews.reduce((sum, r) => sum + reviewTotals(summarizeReview(r)), 0),
    studySyncCheck: (studies) =>
      studies.filter((study) => studyMatchesCloud(study, study.cloud!.downloadedPgn)).length,
  }
}

/* ── Timing ── */

function measure(fn: () => number, minRuns = 7, minMs = 400) {
  fn() // warm the JIT and caches
  const times: number[] = []
  let result = 0
  const start = performance.now()
  while (times.length < minRuns || performance.now() - start < minMs) {
    const t = performance.now()
    result = fn()
    times.push(performance.now() - t)
    if (times.length >= 200) break
  }
  times.sort((a, b) => a - b)
  return {
    medianMs: +times[Math.floor(times.length / 2)]!.toFixed(3),
    p95Ms: +times[Math.floor(times.length * 0.95)]!.toFixed(3),
    runs: times.length,
    result,
  }
}

function run(name: string, rules: Rules, fixtures: ReturnType<typeof makeFixtures>) {
  const db = new DatabaseSync(':memory:')
  db.exec('CREATE TABLE archived_games (id TEXT PRIMARY KEY, body TEXT, seq INTEGER)')
  db.exec('CREATE TABLE documents (key TEXT PRIMARY KEY, body TEXT)')
  const insert = db.prepare('INSERT INTO archived_games VALUES (?, ?, ?)')
  fixtures.games.forEach((g, i) => insert.run(g.id, JSON.stringify(g), i))
  const doc = db.prepare('INSERT INTO documents VALUES (?, ?)')
  doc.run('studies', JSON.stringify(fixtures.studies))
  doc.run('mistakes', JSON.stringify(fixtures.mistakes))
  const rows = () =>
    db.prepare('SELECT body FROM archived_games ORDER BY seq DESC').all() as { body: string }[]
  const body = (key: string) =>
    (db.prepare('SELECT body FROM documents WHERE key = ?').get(key) as { body: string }).body
  const libraryLoad = () =>
    rules.loadArchive(rows()) +
    rules.loadStudies(body('studies')) +
    rules.loadMistakes(body('mistakes'))

  // The first library load in a fresh process: what a user waits for at startup.
  const coldStart = performance.now()
  libraryLoad()
  const coldLibraryLoadMs = +(performance.now() - coldStart).toFixed(3)
  const results = {
    libraryLoad: measure(libraryLoad),
    loadArchive: measure(() => rules.loadArchive(rows())),
    loadStudies: measure(() => rules.loadStudies(body('studies'))),
    loadMistakes: measure(() => rules.loadMistakes(body('mistakes'))),
    replayGames: measure(() => rules.replayGames(fixtures.games)),
    reviewReplay: measure(() => rules.reviewReplay(fixtures.games)),
    treeFromPgn: measure(() => rules.treeFromPgn(fixtures.bigPgn)),
    puzzleSample: measure(() => rules.puzzleSample(fixtures.puzzleChunks), 5, 2000),
    lichessLines: measure(() => rules.lichessLines(fixtures.lichessGames)),
    reviewSummaries: measure(() => rules.reviewSummaries(fixtures.reviews)),
    studySyncCheck: measure(() => rules.studySyncCheck(fixtures.syncedStudies)),
  }
  db.close()
  const memory = process.memoryUsage()
  return {
    name,
    coldLibraryLoadMs,
    results,
    rssBytes: memory.rss,
    heapUsedBytes: memory.heapUsed,
  }
}

const args = process.argv.slice(2)
const out = args.includes('--out') ? args[args.indexOf('--out') + 1]! : undefined
const fixtures = makeFixtures()
const plies = fixtures.games.reduce((sum, g) => sum + g.moves.length, 0)
const native = nativeRules()
if (!native) throw new Error('The native rules module is not built (pnpm run build:native).')
const bigNodes = countNodes(JSON.parse(native.treeFromPgn(fixtures.bigPgn)!))
const describe = {
  games: fixtures.games.length,
  plies,
  studies: fixtures.studies.items.length,
  chapters: fixtures.studies.items.length * 8,
  mistakes: fixtures.mistakes.items.length,
  bigPgnKiB: +(fixtures.bigPgn.length / 1024).toFixed(1),
  bigPgnNodes: bigNodes,
}
console.info(`[benchmark] ${JSON.stringify(describe)}`)
if (args.includes('--fixtures')) {
  // Raw fixtures for the core API benchmark and for profiling the Rust rules on their own.
  const dir = args[args.indexOf('--fixtures') + 1]!
  mkdirSync(dir, { recursive: true })
  writeFileSync(`${dir}/studies.json`, JSON.stringify(fixtures.studies))
  writeFileSync(`${dir}/mistakes.json`, JSON.stringify(fixtures.mistakes))
  writeFileSync(`${dir}/games.json`, JSON.stringify(fixtures.games))
  writeFileSync(`${dir}/big.pgn`, fixtures.bigPgn)
  writeFileSync(`${dir}/puzzles.csv`, Buffer.concat(fixtures.puzzleChunks))
  writeFileSync(`${dir}/reviews.json`, JSON.stringify(fixtures.reviews))
  writeFileSync(`${dir}/synced-studies.json`, JSON.stringify(fixtures.syncedStudies))
  process.exit(0)
}

const report = run('native', nativeImpl(native), fixtures)

for (const [key, value] of Object.entries(report.results))
  console.info(
    `  ${key.padEnd(14)} median ${String(value.medianMs).padStart(9)} ms   p95 ${value.p95Ms} ms   (${value.result})`,
  )
console.info(`  cold library load ${report.coldLibraryLoadMs} ms`)
if (out) {
  mkdirSync(dirname(out), { recursive: true })
  writeFileSync(out, JSON.stringify({ fixtures: describe, ...report }, null, 2) + '\n')
}
