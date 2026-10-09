import {
  opponentReply,
  playerColor,
  playerMove,
  puzzleFromApi,
  puzzleFromDb,
  startPuzzle,
  type DbPuzzle,
  type PuzzleState,
} from '../core/src/domain/puzzle.ts'
import { FEN, PUZZLE_ANGLE } from '../core/src/domain/patterns.ts'
import { sampleZstdCsv, type ChunkSampler } from '../core/src/services/puzzleSampler.ts'
import { installNativeRules } from './native-rules.ts'
import {
  queryLadder,
  queryPuzzles,
  readStatus,
  storeSample,
} from '../core/src/services/puzzleQueries.ts'
import { MIGRATIONS, migrate } from '../core/src/services/migrations.ts'
import { DatabaseSync } from 'node:sqlite'
import { Readable } from 'node:stream'
import { zstdCompressSync } from 'node:zlib'
import {
  RUSH_CONFIGS,
  accuracy,
  comboBonusMs,
  comboProgress,
  correctMove,
  formatRunClock,
  mistake,
  newRush,
  puzzleSolved,
  skip,
  tick,
} from '../core/src/domain/rush.ts'
import {
  knightChallenge,
  knightDistance,
  knightFen,
  knightMoves,
} from '../core/src/domain/knight.ts'
import { ALL_SQUARES, randomSquare, squareColor } from '../core/src/domain/coordinates.ts'
import { ENDGAME_DRILLS, evaluateEndgame } from '../core/src/domain/endgames.ts'
import { themeName } from '../apps/desktop/app/utils/puzzleThemes.ts'

// The native rules, resolved as the core resolves them; puzzle moves are played through them.
const native = installNativeRules()

const assert = (label: string, actual: unknown, expected: unknown): void => {
  const ok = JSON.stringify(actual) === JSON.stringify(expected)
  console.log(
    `${ok ? 'ok  ' : 'FAIL'} ${label}: ${JSON.stringify(actual)}${ok ? '' : ` (expected ${JSON.stringify(expected)})`}`,
  )
  if (!ok) process.exitCode = 1
}

// ── Lichess API puzzle (the daily puzzle of 2026-09-30, as the API returned it) ─────────────────
const daily = puzzleFromApi({
  game: {
    id: 'qLcxVHzB',
    pgn: 'd4 Nf6 c4 d6 Nc3 g6 e4 Bg7 Be2 O-O e5 dxe5 dxe5 Qxd1+ Nxd1 Ne4 Be3 Bxe5 Bd3 Nf6',
  },
  puzzle: {
    id: '9862H',
    rating: 1850,
    plays: 53523,
    solution: ['f2f4', 'e5d6', 'c4c5', 'd6c5', 'e3c5'],
    themes: ['opening', 'advantage', 'trappedPiece', 'long'],
    fen: 'rnb2rk1/ppp1pp1p/5np1/4b3/2P5/3BB3/PP3PPP/R2NK1NR w KQ - 1 1',
    lastMove: 'e4f6',
    initialPly: 19,
  },
})!
assert('api puzzle color', playerColor(daily), 'white')
assert('api puzzle last move squares', startPuzzle(daily).lastMove, ['e4', 'f6'])

// The same puzzle without `fen`/`lastMove` is rebuilt from the game's moves.
const rebuilt = puzzleFromApi({
  game: {
    pgn: 'd4 Nf6 c4 d6 Nc3 g6 e4 Bg7 Be2 O-O e5 dxe5 dxe5 Qxd1+ Nxd1 Ne4 Be3 Bxe5 Bd3 Nf6',
  },
  puzzle: {
    id: '9862H',
    rating: 1850,
    solution: ['f2f4'],
    themes: [],
    initialPly: 19,
  },
})!
assert(
  'rebuilt from pgn: fen',
  rebuilt.fen.split(' ').slice(0, 2).join(' '),
  'rnb2rk1/ppp1pp1p/5np1/4b3/2P5/3BB3/PP3PPP/R2NK1NR w',
)
assert('rebuilt from pgn: last move', rebuilt.lastMove, 'e4f6')

// A puzzle as `/api/puzzle/next` returns it: only the game's moves and `initialPly`, no `fen`.
const fromNext = puzzleFromApi({
  game: {
    id: 'w2kIkptI',
    pgn: 'e4 e5 Nf3 Nc6 Bb5 Nge7 d4 exd4 Nxd4 Nxd4 Qxd4 a6 Bd3 Nc6 Qe3 Be7 O-O O-O Nc3 d6 b3 Nb4 Bb2 c5 Be2 Nxc2 Qg3 Nxa1 Nd5 f6 Bc4 Nc2 Nxe7+ Kh8',
  },
  puzzle: {
    id: '5L2xw',
    rating: 1410,
    plays: 2365,
    solution: ['e7g6', 'h7g6', 'g3h4'],
    themes: ['mateIn2'],
    initialPly: 33,
  },
})!
assert('next-style puzzle: last move', fromNext.lastMove, 'g8h8')
assert(
  'next-style puzzle: the solution starts legally',
  playerMove(fromNext, startPuzzle(fromNext), 'e7g6').correct,
  true,
)

// ── Solving: wrong move, right moves, reply, solved ───────────────────────────────────────────────
let state: PuzzleState = startPuzzle(daily)
const wrong = playerMove(daily, state, 'a2a3')
assert('wrong move is not correct', wrong.correct, false)
assert('wrong move leaves the position', wrong.state.fen, state.fen)
const first = playerMove(daily, state, 'f2f4')
assert('first move correct', first.correct, true)
assert('reply offered', first.reply, 'e5d6')
state = opponentReply(first.state, first.reply!)
const second = playerMove(daily, state, 'c4c5')
assert('second move correct', second.correct && second.reply, 'd6c5')
state = opponentReply(second.state, second.reply!)
const third = playerMove(daily, state, 'e3c5')
assert('last move solves', third.state.status, 'solved')
assert('move list', third.state.sans, ['f4', 'Bd6', 'c5', 'Bxc5', 'Bxc5'])

// Any checkmate is accepted, even if it is not the listed move.
const mateIn1 = {
  id: 'm1',
  fen: '6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1',
  solution: ['a1a8'],
  rating: 600,
  themes: ['mateIn1'],
}
assert(
  'a mate that is the solution',
  playerMove(mateIn1, startPuzzle(mateIn1), 'a1a8').state.status,
  'solved',
)
const twoMates = { ...mateIn1, fen: 'k7/8/1K6/8/8/8/8/6R1 w - - 0 1', solution: ['g1a1'] }
assert(
  'a different mate is accepted',
  playerMove(twoMates, startPuzzle(twoMates), 'g1g8').state.status,
  'solved',
)

// Castling: king-takes-rook and the two-square step are the same move.
const castle = {
  id: 'c1',
  fen: 'r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1',
  solution: ['e1g1', 'a8a1'],
  rating: 1000,
  themes: [],
}
assert('castling as e1g1', playerMove(castle, startPuzzle(castle), 'e1g1').correct, true)
assert('castling as e1h1', playerMove(castle, startPuzzle(castle), 'e1h1').correct, true)
assert(
  'castling highlights the king step',
  playerMove(castle, startPuzzle(castle), 'e1h1').state.lastMove,
  ['e1', 'g1'],
)

// ── Puzzle database rows ─────────────────────────────────────────────────────────────────────────
const dbPuzzle = puzzleFromDb({
  id: '00sHx',
  fen: 'q3k1nr/1pp1nQpp/3p4/1P2p3/4P3/B1PP1b2/B5PP/5K2 b k - 0 17',
  moves: 'e8d7 a2e6 d7d8 f7f8',
  rating: 1760,
  plays: 550,
  themes: 'mate mateIn2 middlegame short',
})!
assert(
  'db puzzle starts after the opponent move',
  dbPuzzle.fen.split(' ').slice(0, 2).join(' '),
  'q5nr/1ppknQpp/3p4/1P2p3/4P3/B1PP1b2/B5PP/5K2 w',
)
assert('db puzzle solution drops the opening move', dbPuzzle.solution, ['a2e6', 'd7d8', 'f7f8'])
assert('db puzzle themes', dbPuzzle.themes, ['mate', 'mateIn2', 'middlegame', 'short'])
assert(
  'db puzzle with an illegal opening move is dropped',
  puzzleFromDb({
    id: 'x',
    fen: 'q3k1nr/1pp1nQpp/3p4/1P2p3/4P3/B1PP1b2/B5PP/5K2 b k - 0 17',
    moves: 'a1a8 a2e6',
    rating: 1000,
    themes: '',
  }),
  undefined,
)

// Sampler (the native rules): parses the CSV, keeps solid puzzles, skips the header.
function nativeSampler(): ChunkSampler {
  const inner = new native.PuzzleSampler(1)
  return {
    push: (chunk) => inner.push(chunk),
    finish: () => inner.finish(),
    get count() {
      return inner.count
    },
    get lines() {
      return inner.lines
    },
    kept: () => JSON.parse(inner.kept()) as DbPuzzle[],
  }
}
const sampler = nativeSampler()
sampler.push(
  Buffer.from(
    [
      'PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl,OpeningTags',
      '00sHx,q3k1nr/1pp1nQpp/3p4/1P2p3/4P3/B1PP1b2/B5PP/5K2 b k - 0 17,e8d7 a2e6 d7d8 f7f8,1760,80,90,5000,mate mateIn2 middlegame short,https://lichess.org/yyznGmXs/black#34,Italian_Game',
      '00sJ9,r3r1k1/p4ppp/2p2n2/1p6/3P1qb1/2P1R3/PPB2PP1/RN1Q2K1 b - - 5 18,f4g4 d1g4 f6g4 e3e8,1490,74,82,1234,mate short,https://lichess.org/x,',
      'bad,short',
    ].join('\n'),
  ),
)
sampler.finish()
assert(
  'sampler keeps solid puzzles only',
  sampler
    .kept()
    .map((row) => row.id)
    .sort(),
  ['00sHx', '00sJ9'],
)

// ── Patterns ─────────────────────────────────────────────────────────────────────────────────────
assert('fen pattern', FEN.test('4k3/8/8/8/8/8/8/3QK3 w - - 0 1'), true)
assert('fen pattern rejects junk', FEN.test('rm -rf /'), false)
assert(
  'angle pattern',
  [PUZZLE_ANGLE.test('mateIn2'), PUZZLE_ANGLE.test('mix'), PUZZLE_ANGLE.test('a b')],
  [true, true, false],
)
assert(
  'theme names',
  [themeName('mateIn2'), themeName('someNewTheme'), themeName('mix')],
  ['Mate in 2', 'Some New Theme', 'Healthy mix'],
)

// ── Storm, Streak, Rush ──────────────────────────────────────────────────────────────────────────
const storm = RUSH_CONFIGS.storm!
const run = newRush(storm)
for (let i = 0; i < 5; i++) correctMove(run, storm)
assert('storm: 5-move combo earns 3s', [run.combo, run.timeLeftMs, run.bonusMs], [5, 183_000, 3000])
mistake(run, storm)
assert(
  'storm: a mistake costs 10s and breaks the combo',
  [run.combo, run.timeLeftMs, run.over],
  [0, 173_000, undefined],
)
puzzleSolved(run, 1200)
tick(run, 173_000)
assert(
  'storm: time runs out',
  [run.over, run.timeLeftMs, run.score, run.highest],
  ['time', 0, 1, 1200],
)
assert(
  'combo bonuses',
  [5, 12, 20, 30, 40, 41].map(comboBonusMs),
  [3000, 5000, 7000, 10_000, 10_000, 0],
)
assert(
  'combo bar',
  [comboProgress(0), comboProgress(5), comboProgress(30), comboProgress(35)],
  [0, 0, 0, 0.5],
)
const streak = RUSH_CONFIGS.streak!
const streakRun = newRush(streak)
assert('streak: one skip', [skip(streakRun), skip(streakRun)], [true, false])
mistake(streakRun, streak)
assert('streak: the first mistake ends it', streakRun.over, 'strikes')
const rush = RUSH_CONFIGS['3min']!
const rushRun = newRush(rush)
mistake(rushRun, rush)
mistake(rushRun, rush)
assert('rush: two strikes are survivable', rushRun.over, undefined)
correctMove(rushRun, rush)
mistake(rushRun, rush)
assert('rush: the third ends it', [rushRun.over, accuracy(rushRun)], ['strikes', 25])
assert(
  'run clock',
  [formatRunClock(183_000), formatRunClock(9_400), formatRunClock(0)],
  ['3:03', '0:09.4', '0:00'],
)

// ── Practice drills ──────────────────────────────────────────────────────────────────────────────
assert('knight moves from a1', knightMoves('a1'), ['b3', 'c2'])
assert(
  'knight distances',
  [
    knightDistance('a1', 'a1'),
    knightDistance('a1', 'b3'),
    knightDistance('a1', 'h8'),
    knightDistance('b1', 'c3'),
  ],
  [0, 1, 6, 1],
)
const challenge = knightChallenge(3, 4)
assert(
  'knight challenge is as far as asked',
  challenge.best >= 3 &&
    challenge.best <= 4 &&
    knightDistance(challenge.from, challenge.to) === challenge.best,
  true,
)
assert('knight fen', knightFen('d4'), '8/8/8/8/3N4/8/8/8 w - - 0 1')
assert(
  'square colors',
  [squareColor('a1'), squareColor('h1'), squareColor('e4'), squareColor('d4')],
  ['dark', 'light', 'light', 'dark'],
)
assert('random square differs from the previous one', randomSquare('e4', () => 0.5) !== 'e4', true)
assert('64 squares', ALL_SQUARES.length, 64)

for (const drill of ENDGAME_DRILLS) {
  const start = evaluateEndgame(drill.fen, [], drill.player, drill.goal)
  assert(`drill ${drill.id} starts in play`, start.over, false)
}
// Scholar-style mate on a tiny position: king and queen mate.
const mated = evaluateEndgame('7k/5Q2/6K1/8/8/8/8/8 w - - 0 1', ['f7g7'], 'white', 'win')
assert('mate wins a win drill', [mated.over, mated.success], [true, true])
const stale = evaluateEndgame('7k/8/6K1/8/8/8/8/5Q2 w - - 0 1', ['f1f7'], 'white', 'win')
assert('stalemate fails a win drill', [stale.over, stale.success], [true, false])
const held = evaluateEndgame('7k/8/6K1/8/8/8/8/5Q2 w - - 0 1', ['f1f7'], 'black', 'draw')
assert('stalemate holds a draw drill', [held.over, held.success], [true, true])
const repeated = evaluateEndgame(
  '4k3/8/8/8/8/8/8/3QK3 w - - 0 1',
  ['d1d2', 'e8f8', 'd2d1', 'f8e8', 'd1d2', 'e8f8', 'd2d1', 'f8e8'],
  'white',
  'win',
)
assert('threefold repetition draws', repeated.title.includes('repetition'), true)

// ── Puzzle database: zstd CSV stream → sample → SQLite → queries ────────────────────────────────
const themesFor = ['mate mateIn2 middlegame', 'fork middlegame', 'endgame rookEndgame', 'pin short']
const csvRows = [
  'PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl,OpeningTags',
]
for (let i = 0; i < 6000; i++)
  csvRows.push(
    `p${String(i).padStart(4, '0')},q3k1nr/1pp1nQpp/3p4/1P2p3/4P3/B1PP1b2/B5PP/5K2 b k - 0 17,e8d7 a2e6 d7d8 f7f8,${500 + (i % 2400)},60,92,2000,${themesFor[i % 4]},https://lichess.org/x,`,
  )
csvRows.push('broken,line')
const compressed = zstdCompressSync(Buffer.from(csvRows.join('\n')))
const streamed = nativeSampler()
// Deliver it in small pieces, as a network would, to exercise chunk boundaries inside lines.
const pieces: Buffer[] = []
for (let at = 0; at < compressed.length; at += 997) pieces.push(compressed.subarray(at, at + 997))
await sampleZstdCsv(Readable.from(pieces), streamed, new AbortController().signal)
assert('streamed every well-formed line', streamed.lines, 6000)
const sample = streamed.kept()
assert(
  'the sample keeps puzzles from every rating bucket',
  new Set(sample.map((row) => Math.floor(row.rating / 50))).size,
  48,
)
const database = new DatabaseSync(':memory:')
migrate(database)
assert('a database with no puzzles is not installed', readStatus(database, false).installed, false)
let emptyError = ''
try {
  queryPuzzles(database, { count: 1 })
} catch (cause) {
  emptyError = (cause as Error).message
}
assert(
  'querying without puzzles explains what to do',
  emptyError.startsWith('Download the puzzle database'),
  true,
)
const stored = storeSample(database, sample)
assert('every sampled puzzle was stored', stored, sample.length)
assert(
  'status after import',
  [readStatus(database, false).installed, readStatus(database, false).count],
  [true, stored],
)
const forks = queryPuzzles(database, { theme: 'fork', count: 20 })
assert(
  'theme query returns that theme only',
  forks.length === 20 && forks.every((puzzle) => puzzle.themes.includes('fork')),
  true,
)
assert(
  'theme query is not fooled by prefixes',
  queryPuzzles(database, { theme: 'mate', count: 5 }).every((puzzle) =>
    puzzle.themes.includes('mate'),
  ),
  true,
)
const banded = queryPuzzles(database, { minRating: 1000, maxRating: 1100, count: 30 })
assert(
  'rating range respected',
  banded.every((puzzle) => puzzle.rating >= 1000 && puzzle.rating <= 1100),
  true,
)
const ladder = queryLadder(database, { from: 600, to: 2600, count: 60 })
assert('ladder has the steps asked for', ladder.length, 60)
assert(
  'ladder rises',
  ladder.every((puzzle, i) => i === 0 || puzzle.rating >= ladder[i - 1]!.rating - 60),
  true,
)
assert('ladder puzzles are distinct', new Set(ladder.map((puzzle) => puzzle.id)).size, 60)
assert(
  'a migration creates the puzzle tables',
  MIGRATIONS.some((migration) => migration.includes('CREATE TABLE IF NOT EXISTS puzzles')),
  true,
)
// A second import replaces the first completely.
storeSample(database, sample.slice(0, 10))
assert('re-importing replaces the puzzles', readStatus(database, false).count, 10)

// A failed replacement must keep the existing puzzles and metadata.
const previousStatus = readStatus(database, false)
for (const replacement of [[], [{ ...sample[0]!, fen: 'invalid' }]]) {
  let rejected = false
  try {
    storeSample(database, replacement)
  } catch {
    rejected = true
  }
  assert('invalid replacement rejected', rejected, true)
  assert('invalid replacement retains previous sample', readStatus(database, false), previousStatus)
}
try {
  storeSample(database, sample, () => true)
} catch {
  /* expected cancellation */
}
assert('cancelled replacement retains previous sample', readStatus(database, false), previousStatus)
assert('duplicate IDs counted once', storeSample(database, [sample[0]!, sample[0]!]), 1)
