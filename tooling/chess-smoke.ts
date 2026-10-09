import {
  checkColor,
  destsFor,
  drawReason,
  fen,
  isPromotionMove,
  lastMoveKeys,
  pgnFromMoves,
  pgnFromSan,
  pgnFromUci,
  playUci,
  positionAfter,
  sanHistory,
  statusText,
  takebackMoves,
  turnColor,
} from '../core/src/domain/chess.ts'
import { Clock, formatClock } from '../core/src/domain/clock.ts'
import { installNativeRules } from './native-rules.ts'

installNativeRules()

const assert = (label: string, actual: unknown, expected: unknown): void => {
  const ok = JSON.stringify(actual) === JSON.stringify(expected)
  console.log(
    `${ok ? 'ok  ' : 'FAIL'} ${label}: ${JSON.stringify(actual)}${ok ? '' : ` (expected ${JSON.stringify(expected)})`}`,
  )
  if (!ok) process.exitCode = 1
}

// Scholar's mate: checkmate detection and SAN history.
const scholar = ['e2e4', 'e7e5', 'f1c4', 'b8c6', 'd1h5', 'g8f6', 'h5f7']
assert('scholar SAN', sanHistory(scholar).join(' '), 'e4 e5 Bc4 Nc6 Qh5 Nf6 Qxf7#')
assert('scholar checkmate', positionAfter(scholar).isCheckmate(), true)
assert('scholar status', statusText(positionAfter(scholar)), 'Checkmate')

// Test expectation bug: this one is the mate line, and check is correctly reported.
const checkOnly = ['e2e4', 'e7e5', 'd1h5', 'b8c6', 'h5e5']
assert('check color', checkColor(positionAfter(checkOnly)), 'black')
assert('check status', statusText(positionAfter(checkOnly)), 'Check')

// Castling sent as king-to-rook (Lichess notation) works.
const castle = ['e2e4', 'e7e5', 'g1f3', 'b8c6', 'f1c4', 'g8f6', 'e1h1']
assert('castling SAN', sanHistory(castle).at(-1), 'O-O')
assert(
  'castling king g1',
  fen(positionAfter(castle)).startsWith('r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQ1RK1'),
  true,
)

// Promotion by capturing on the back rank.
const promotion = ['b2b4', 'h7h6', 'b4b5', 'h6h5', 'b5b6', 'h5h4', 'b6a7', 'h4h3', 'a7b8q']
assert('promotion SAN', sanHistory(promotion).at(-1), 'axb8=Q')
assert(
  'promotion dests include b8',
  (destsFor(positionAfter(promotion.slice(0, 8))).get('a7') ?? []).includes('b8'),
  true,
)

// Dests contain both castling representations, as chessground expects.
const preCastle = positionAfter(['e2e4', 'e7e5', 'g1f3', 'b8c6', 'f1c4', 'g8f6'])
assert('castle dests', (destsFor(preCastle).get('e1') ?? []).slice().sort(), [
  'e2',
  'f1',
  'g1',
  'h1',
])

// lastMoveKeys / turnColor.
assert('last move', lastMoveKeys(['e2e4', 'e7e5']), ['e7', 'e5'])
assert('turn color', turnColor(positionAfter(['e2e4'])), 'black')

// Illegal moves are rejected.
assert('illegal rejected', playUci(positionAfter([]), 'e2e5'), false)

// PGN fallback for games stored without one.
const pgn = pgnFromUci(scholar)
console.log('--- generated PGN ---')
console.log(pgn)
console.log('---------------------')
assert('pgn contains mate SAN', pgn.includes('Qxf7#'), true)

// Synced Lichess games store SAN tokens: the fallback must keep them.
const sanTokens = ['d4', 'd5', 'c4', 'dxc4', 'Nc3', 'Nf6', 'O-O', 'e6', 'Qb5+', 'axb8=Q']
const sanPgn = pgnFromMoves(sanTokens)
assert('san fallback keeps castling', sanPgn.includes('O-O'), true)
assert('san fallback keeps check', sanPgn.includes('Qb5+'), true)
assert('san fallback keeps promotion', sanPgn.includes('axb8=Q'), true)
assert('uci still routes to sanHistory', pgnFromMoves(scholar).includes('Qxf7#'), true)
assert(
  'pgnFromSan direct',
  pgnFromSan(['e4', 'e5']).includes('e4 e5') || pgnFromSan(['e4', 'e5']).includes('e4'),
  true,
)

// Promotion detection (the board decides between auto-move and the picker with this).
const preFen = fen(positionAfter(promotion.slice(0, 8)))
assert('pawn a7-a8 is a promotion', isPromotionMove(preFen, 'a7', 'a8'), true)
assert('capture a7xb8 is a promotion', isPromotionMove(preFen, 'a7', 'b8'), true)
assert('pawn push is not a promotion', isPromotionMove(fen(positionAfter([])), 'e2', 'e4'), false)
assert('piece move is not a promotion', isPromotionMove(fen(positionAfter([])), 'g1', 'f3'), false)
assert(
  'black pawn to rank 1 is a promotion',
  isPromotionMove('8/8/8/8/8/8/p7/K6k b - - 0 1', 'a2', 'a1'),
  true,
)
assert(
  'promotion suffix is legal',
  playUci(positionAfter(promotion.slice(0, 8)), 'a7b8n'),
  'axb8=N',
)
assert(
  'unpromoted last-rank move is rejected',
  playUci(positionAfter(promotion.slice(0, 8)), 'a7b8'),
  false,
)

// Takeback returns to the player's turn without leaving the computer stuck.
const game = ['e2e4', 'e7e5', 'g1f3']
assert('takeback (white) after full round', takebackMoves(['e2e4', 'e7e5'], 'white'), [])
assert('takeback (white) while computer thinks', takebackMoves(game, 'white'), ['e2e4', 'e7e5'])
assert(
  'takeback (black) keeps computer opening',
  takebackMoves(['e2e4', 'e7e5', 'g1f3', 'b8c6', 'f1b5'], 'black'),
  ['e2e4', 'e7e5', 'g1f3'],
)
assert(
  'takeback (black) while computer thinks',
  takebackMoves(['e2e4', 'e7e5', 'g1f3', 'b8c6'], 'black'),
  ['e2e4', 'e7e5', 'g1f3'],
)
assert('takeback (black) to start', takebackMoves(['e2e4'], 'black'), [])
assert('takeback on empty game', takebackMoves([], 'white'), [])

// Draws chessops' isEnd() does not report.
const shuffle = ['g1f3', 'g8f6', 'f3g1', 'f6g8', 'g1f3', 'g8f6', 'f3g1', 'f6g8']
assert('no draw after one repetition', drawReason(shuffle.slice(0, 4)), undefined)
assert('threefold repetition', drawReason(shuffle), 'Threefold repetition')
assert('normal game is not a draw', drawReason(scholar), undefined)

// Clock: interpolates from the last server update.
const clock = new Clock()
clock.set({ white: 10_000, black: 20_000, ticking: 'white', initialSeconds: 300 })
await new Promise((resolve) => setTimeout(resolve, 60))
const white = clock.remaining('white')
assert('clock ticks down', white < 10_000 && white > 9_800, true)
assert('clock stopped side', clock.remaining('black'), 20_000)
assert('clock format', formatClock(white), '0:09')
assert('low time alert fires once', clock.lowTimeAlert('white'), true)
assert('low time alert once only', clock.lowTimeAlert('white'), false)
