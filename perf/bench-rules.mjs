// Domain/rules microbenchmarks through the native binding, including the JSON boundary.
// Usage: node perf/bench-rules.mjs [--label=before] [--iterations=200] [--plies=120]
import { writeFile, mkdir } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { performance } from 'node:perf_hooks'

const args = Object.fromEntries(
  process.argv
    .slice(2)
    .filter((a) => a.startsWith('--'))
    .map((a) => {
      const [k, v] = a.replace(/^--/, '').split('=')
      return [k, v ?? 'true']
    }),
)
const root = resolve(args.root ?? resolve(import.meta.dirname, '..'))
const label = args.label ?? 'run'
const iterations = Number(args.iterations ?? 200)
const plies = Number(args.plies ?? 120)
const native = (
  await import(pathToFileURL(join(root, 'crates/kchess-node/dist/native.js')).href)
).nativeRules()
if (!native) throw new Error('native rules not built')

const invoke = (method, ...a) => JSON.parse(native.invoke(method, JSON.stringify(a)))

// A deterministic legal game: pick the k-th legal move from each position.
const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const moves = []
const sans = []
{
  let fen = START
  let seed = 12345
  for (let i = 0; i < plies; i++) {
    const legal = invoke('legalMoves', { variant: 'standard', fen })
    if (!legal.length) break
    seed = (seed * 1103515245 + 12345) % 2147483648
    const picked = legal[seed % legal.length]
    const promotion = picked.promotion
      ? { queen: 'q', rook: 'r', bishop: 'b', knight: 'n' }[picked.promotion]
      : ''
    const uci = `${picked.from}${picked.to}${promotion}`
    const played = invoke('play', { variant: 'standard', fen }, uci)
    moves.push(uci)
    sans.push(played.san)
    fen = played.position.fen
  }
}
const pgn = invoke('pgnFromSan', sans)
const tree = invoke('treeFromPgn', pgn)
if (!tree) throw new Error('fixture PGN did not import')

// A review of the same game with a score at every position.
const review = {
  key: invoke('reviewKey', START, moves),
  fen: START,
  moves,
  source: 'local',
  engine: 'bench',
  evals: moves.map((_, i) => ({ cp: (i % 40) * 10 - 200 })),
  judgments: null,
  accuracy: null,
  depth: 20,
  complete: true,
  updatedAt: 1,
  gameId: null,
}

const samples = {}
function time(name, work, n = iterations) {
  work()
  const values = []
  for (let i = 0; i < n; i++) {
    const start = performance.now()
    work(i)
    values.push(performance.now() - start)
  }
  values.sort((a, b) => a - b)
  samples[name] = {
    n: values.length,
    p50: +values[Math.floor(values.length / 2)].toFixed(4),
    p95: +values[Math.floor(values.length * 0.95)].toFixed(4),
  }
}

time('position_info', () => invoke('position', { variant: 'standard', fen: START }))
time('legalMoves', () => invoke('legalMoves', { variant: 'standard', fen: START }))
time('replay_120', () => invoke('replay', START, moves))
time('setupReplay_120', () => invoke('setupReplay', { variant: 'standard', fen: START }, moves))
time('sanHistory_120', () => invoke('sanHistory', moves))
time('materialDiff', () => invoke('materialDiff', tree.children[0]?.fen ?? START))
time('line_120', () => invoke('line', { variant: 'standard', fen: START }, moves, { san: true }))
time('openingAt', () =>
  invoke('openingAt', { variant: 'standard', fen: START }, moves, moves.length),
)
time('treeFromPgn', () => invoke('treeFromPgn', pgn))
time('treeToPgn', () => invoke('treeToPgn', tree))
time('analyseReview_120', () => invoke('analyseReview', review))
time('explainReviewedMove_120', () =>
  invoke('explainReviewedMove', review, Math.floor(moves.length / 2)),
)
time(
  'parseInfo',
  () =>
    invoke(
      'parseInfo',
      'info depth 20 seldepth 30 multipv 1 score cp 35 nodes 120000 nps 900000 time 130 pv e2e4 e7e5 g1f3',
    ),
  2000,
)

// Review labels: every move judged, then every other score missing (the end-of-game fallback).
const judged = {
  ...review,
  judgments: moves.map((_, i) => ['inaccuracy', 'mistake', 'blunder'][i % 3]),
}
time('analyseReview_judged', () => invoke('analyseReview', judged))
const partial = {
  ...review,
  evals: review.evals.map((value, i) => (i % 2 ? null : value)),
}
time('analyseReview_partial', () => invoke('analyseReview', partial))

// Openings page / opening name.
time('openingTree', () => invoke('openingTree'), 20)

// Puzzle move: the solution check replays the same position twice.
const puzzle = { fen: '6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1', solution: ['d1d8'] }
const puzzleState = { fen: puzzle.fen, index: 0, status: 'playing', sans: [] }
time('playerMove', () => invoke('playerMove', puzzle, puzzleState, 'd1d8'), 500)

// A study export over 20 chapters.
const chapters = Array.from({ length: 20 }, (_, i) => ({
  name: `Chapter ${i + 1}`,
  pgn: invoke('pgnFromSan', sans.slice(0, 20 + i)),
}))
time('studyDocumentPgn_20', () => invoke('studyDocumentPgn', chapters), 50)

const output = { label, root, plies: moves.length, iterations, samples }
console.log(JSON.stringify(output, null, 2))
const outDir = process.env.PERF_OUT ? resolve(process.env.PERF_OUT) : resolve(root, 'perf/results')
await mkdir(outDir, { recursive: true })
await writeFile(resolve(outDir, `rules-${label}.json`), `${JSON.stringify(output, null, 2)}\n`)
