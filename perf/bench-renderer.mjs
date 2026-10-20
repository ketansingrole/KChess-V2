// Renderer/helper microbenchmarks, run against a checkout root so before/after are the same code.
// Usage: node perf/bench-renderer.mjs --root=/path/to/repo --label=before
import { performance } from 'node:perf_hooks'
import { writeFile, mkdir } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'

const args = Object.fromEntries(
  process.argv
    .slice(2)
    .filter((a) => a.startsWith('--'))
    .map((a) => {
      const [k, v] = a.replace(/^--/, '').split('=')
      return [k, v ?? 'true']
    }),
)
const root = resolve(args.root ?? process.cwd())
const label = args.label ?? 'run'
const url = (path) => pathToFileURL(join(root, path)).href

const native = (await import(url('crates/kchess-node/dist/native.js'))).nativeRules()
if (!native) throw new Error('native rules not built in ' + root)
const { setRulesBinding } = await import(url('crates/kchess-wasm/js/engine.ts'))
setRulesBinding({ invoke: (method, input) => native.invoke(method, input) })

const analysisTree = await import(url('crates/kchess-wasm/js/analysisTree.ts'))
const chess = await import(url('crates/kchess-wasm/js/chess.ts'))
const variant = await import(url('crates/kchess-wasm/js/variant.ts'))
const games = await import(url('apps/desktop/app/utils/games.ts'))
const gameStatus = await import(url('crates/kchess-wasm/js/gameStatus.ts'))
const clock = await import(url('crates/kchess-wasm/js/clock.ts'))

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const invoke = (method, ...a) => JSON.parse(native.invoke(method, JSON.stringify(a)))

// A legal 120-ply game.
const moves = []
let lastFen = START
{
  let fen = START
  let seed = 4242
  for (let i = 0; i < 120; i++) {
    const legal = invoke('legalMoves', { variant: 'standard', fen })
    if (!legal.length) break
    seed = (seed * 1103515245 + 12345) % 2147483648
    const p = legal[seed % legal.length]
    const promotion = p.promotion
      ? { queen: 'q', rook: 'r', bishop: 'b', knight: 'n' }[p.promotion]
      : ''
    const uci = `${p.from}${p.to}${promotion}`
    const played = invoke('play', { variant: 'standard', fen }, uci)
    moves.push(uci)
    fen = played.position.fen
    lastFen = fen
  }
}
const setup = { variant: 'standard', fen: START }
const pgn = invoke('pgnFromSan', invoke('sanHistory', moves))
const tree = invoke('treeFromPgn', pgn)

const samples = {}
function time(name, work, n = 200) {
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

// formatGameDate: one Intl.DateTimeFormat per call in the old implementation.
const timestamp = 1_700_000_000_000
time(
  'formatGameDate_x100',
  () => {
    for (let i = 0; i < 100; i++) games.formatGameDate(timestamp + i)
  },
  100,
)

// A session-shaped JSON clone, like the old plain() on every save.
const session = {
  setup,
  moves,
  clock: null,
  times: null,
  paused: false,
  orientation: 'white',
  savedAt: timestamp,
}
const sessionBytes = JSON.stringify(session).length
time('session_json_clone', () => JSON.parse(JSON.stringify(session)), 100)

// A growing local game: position then SAN history for the same array (what the stores ask).
time(
  'position_plus_history_last_ply',
  () => {
    const position = chess.setupPositionAfter(setup, moves)
    const history = chess.setupSanHistory(setup, moves)
    return position.fen + history.length
  },
  100,
)
time(
  'position_plus_history_per_move',
  () => {
    const growing = []
    for (let n = 1; n <= 40; n++) {
      growing.push(moves[n - 1])
      chess.setupPositionAfter(setup, growing)
      chess.setupSanHistory(setup, growing)
    }
  },
  10,
)
time('replaySetup_full', () => variant.replaySetup(setup, moves), 200)
time(
  'replaySetup_per_move',
  () => {
    const growing = []
    for (let n = 1; n <= 40; n++) {
      growing.push(moves[n - 1])
      variant.replaySetup(setup, growing)
    }
  },
  10,
)

// Opening name per move (the board asks on every move).
time('openingAt_last', () => invoke('openingAt', setup, moves, moves.length), 200)
// Flexible openingAt for growing lists (a move at every ply).
time(
  'openingAt_growing',
  () => {
    for (let n = 1; n <= 40; n++) invoke('openingAt', setup, moves.slice(0, n), n)
  },
  20,
)

// Material diff for both players on every position change.
time(
  'materialDiff_x2',
  () => {
    invoke('materialDiff', lastFen)
    invoke('materialDiff', lastFen)
  },
  500,
)

// One AnalysisLine row, old shape versus the current helpers.
const node = tree.children[0]
const parentPath = ''
time(
  'analysis_row_old',
  () => {
    analysisTree.pathOf([...analysisTree.movesOf(parentPath), node.uci])
    analysisTree.moveNumber(0, true)
    analysisTree.moveGlyph(node)
    for (const comment of node.comments ?? []) analysisTree.commentText(comment)
  },
  500,
)
time(
  'analysis_row_new',
  () => {
    const path = parentPath ? `${parentPath} ${node.uci}` : node.uci
    analysisTree.moveNumber(0, true)
    analysisTree.moveGlyph(node)
    for (const comment of node.comments ?? []) analysisTree.commentText(comment)
    return path
  },
  500,
)

// The review marks loop: a path per ply, old pathOf(slice) versus incremental concat.
time(
  'review_paths_old',
  () => {
    for (let i = 1; i <= 100; i++) analysisTree.pathOf(moves.slice(0, i))
  },
  50,
)
time(
  'review_paths_new',
  () => {
    let path = ''
    for (let i = 1; i <= 100; i++) path = path ? `${path} ${moves[i - 1]}` : moves[i - 1]
  },
  50,
)

// formatEval in a template: same line on every engine update.
const line = { cp: 34, mate: null }
time(
  'formatEval_x100',
  () => {
    for (let i = 0; i < 100; i++) analysisTree.formatEval(line)
  },
  100,
)

// The full engine-lines render: 3 lines per update.
const lines = [line, { cp: -12, mate: null }, { cp: null, mate: 3 }]
time('engine_lines_3', () => lines.map((entry) => analysisTree.formatEval(entry)), 500)

// The history list asks the rule for each row; a real game carries moves and a PGN.
const game = {
  id: 'abc123',
  account: 'player',
  color: 'white',
  winner: 'white',
  opponent: 'rival',
  moves: moves.map((uci) => `${uci}`).join(' '),
  pgn: pgn.repeat(4),
  createdAt: timestamp,
  lastMoveAt: timestamp + 60_000,
  rated: true,
  speed: 'blitz',
  perf: 'blitz',
  status: 'mate',
  opponentRating: 1500,
  playerRating: 1510,
  ratingDiff: 10,
  opening: 'Italian Game',
}
time(
  'gameResult_x100',
  () => {
    for (let i = 0; i < 100; i++) gameStatus.gameResult(game)
  },
  100,
)

// A clock tick reads both sides' remaining time several times a second.
time(
  'formatClock_x100',
  () => {
    for (let i = 0; i < 100; i++) clock.formatClock(300_000 - i * 100)
  },
  100,
)
time(
  'formatClock_x100_tenths',
  () => {
    for (let i = 0; i < 100; i++) clock.formatClock(9_900 - i * 10)
  },
  100,
)

const output = { label, root, moves: moves.length, sessionBytes, samples }
console.log(JSON.stringify(output, null, 2))
const outDir = process.env.PERF_OUT ? resolve(process.env.PERF_OUT) : resolve(root, 'perf/results')
await mkdir(outDir, { recursive: true })
await writeFile(resolve(outDir, `renderer-${label}.json`), `${JSON.stringify(output, null, 2)}\n`)
