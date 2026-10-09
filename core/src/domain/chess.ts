import { Chess, normalizeMove, type Position } from 'chessops/chess'
import { chessgroundDests } from 'chessops/compat'
import { makeFen, parseFen } from 'chessops/fen'
import { extend, defaultGame, makePgn, type PgnNodeData } from 'chessops/pgn'
import { makeSanAndPlay } from 'chessops/san'
import { makeSquare, parseSquare, parseUci } from 'chessops/util'
import type { Color, SquareName } from 'chessops/types'
import { UCI_MOVE } from './patterns.ts'
import { isChess960, replaySetup, setupStart, STANDARD_SETUP, type GameSetup } from './variant.ts'

export type Dests = Map<SquareName, SquareName[]>

export function fen(pos: Position): string {
  return makeFen(pos.toSetup())
}

/** Replay a list of UCI moves from the standard starting position. */
export function positionAfter(moves: readonly string[]): Position {
  const pos = Chess.default()
  for (const uci of moves) {
    const move = parseUci(uci.trim())
    if (!move || !pos.isLegal(move)) break
    pos.play(move)
  }
  return pos
}

/** The position a FEN describes, or undefined when it is not a playable one. */
export function positionFromFen(text: string): Position | undefined {
  const setup = parseFen(text)
  if (setup.isErr) return undefined
  const pos = Chess.fromSetup(setup.value)
  return pos.isOk ? pos.value : undefined
}

/** Play a single UCI move on a position; returns the SAN, or false when illegal. */
export function playUci(pos: Position, uci: string): string | false {
  const move = parseUci(uci.trim())
  if (!move || !pos.isLegal(move)) return false
  return makeSanAndPlay(pos, move)
}

/** SAN move list for a game, used by the move list UI. */
export function sanHistory(moves: readonly string[]): string[] {
  const pos = Chess.default()
  const sans: string[] = []
  for (const uci of moves) {
    const move = parseUci(uci.trim())
    if (!move || !pos.isLegal(move)) break
    sans.push(makeSanAndPlay(pos, move))
  }
  return sans
}

/** Where an arrow key moves a move-list position (←/→ one ply, ↑/↓ first/last); undefined for other keys. */
export function navigatePly(key: string, ply: number, max: number): number | undefined {
  if (key === 'ArrowLeft') return Math.max(0, ply - 1)
  if (key === 'ArrowRight') return Math.min(max, ply + 1)
  if (key === 'ArrowUp') return 0
  if (key === 'ArrowDown') return max
  return undefined
}

/** Legal destinations in the format chessground expects. */
export function destsFor(pos: Position): Dests {
  return chessgroundDests(pos) as Dests
}

/** Last move as squares, for the board highlight. */
export function lastMoveKeys(moves: readonly string[]): SquareName[] | undefined {
  const last = moves.at(-1)
  const move = last && parseUci(last.trim())
  return move && 'from' in move ? [makeSquare(move.from), makeSquare(move.to)] : undefined
}

/** Color of the king in check, or false. */
export function checkColor(pos: Position): Color | false {
  return pos.isCheck() ? pos.turn : false
}

export function turnColor(pos: Position): Color {
  return pos.turn
}

export function statusText(pos: Position): string {
  if (pos.isCheckmate()) return 'Checkmate'
  if (pos.isStalemate()) return 'Stalemate'
  if (pos.isInsufficientMaterial()) return 'Draw'
  if (pos.isEnd()) return 'Draw'
  if (pos.isCheck()) return 'Check'
  return `${pos.turn === 'white' ? 'White' : 'Black'} to play`
}

/** True when moving `orig`→`dest` on this FEN is a pawn reaching the last rank. */
export function isPromotionMove(fen: string, orig: string, dest: string): boolean {
  const setup = parseFen(fen)
  if (setup.isErr) return false
  const from = parseSquare(orig)
  if (from === undefined) return false
  const piece = setup.value.board.get(from)
  return piece?.role === 'pawn' && dest[1] === (piece.color === 'white' ? '8' : '1')
}

/**
 * Moves to keep after a takeback: undo the player's last move and any reply,
 * so it is the player's turn again (or the computer's, when playing black at the start).
 */
export function takebackMoves(moves: readonly string[], userColor: Color): string[] {
  const kept = moves.slice(0, -1)
  while (kept.length && positionAfter(kept).turn !== userColor) kept.pop()
  return kept
}

export type DrawReason = 'Threefold repetition' | 'Fifty-move rule'

/** Draws chessops' `isEnd()` does not cover: repetition and the 50-move rule. */
export function drawReason(moves: readonly string[]): DrawReason | undefined {
  const pos = Chess.default()
  const key = (p: Position): string => fen(p).split(' ').slice(0, 4).join(' ')
  const seen = new Map<string, number>([[key(pos), 1]])
  for (const uci of moves) {
    const move = parseUci(uci.trim())
    if (!move || !pos.isLegal(move)) break
    pos.play(move)
    const k = key(pos)
    const count = (seen.get(k) ?? 0) + 1
    seen.set(k, count)
    if (count >= 3) return 'Threefold repetition'
  }
  if (!pos.isEnd() && pos.halfmoves >= 100) return 'Fifty-move rule'
  return undefined
}

/**
 * Build a PGN for games stored without one.
 * Synced Lichess games keep SAN tokens in `moves` (e.g. "e4 e5 Nf3"),
 * while local/online moves are UCI (e.g. "e2e4 e7e5 g1f3").
 */
const PGN_RESULT = /^(1-0|0-1|1\/2-1\/2|\*)$/

export function pgnFromSan(sans: readonly string[]): string {
  const game = defaultGame<PgnNodeData>()
  extend(
    game.moves,
    sans.map((san) => ({ san })),
  ) // mutates the root's children
  return makePgn(game)
}

/** Tolerant fallback: accepts UCI or SAN tokens and returns a PGN. */
export function pgnFromMoves(tokens: readonly string[]): string {
  const cleaned = tokens
    .map((token) => token.trim())
    .filter((token) => token && !PGN_RESULT.test(token))
  if (cleaned.length > 0 && cleaned.every((token) => UCI_MOVE.test(token.toLowerCase())))
    return pgnFromUci(cleaned)
  return pgnFromSan(cleaned)
}

/** Build a PGN from UCI moves. The result has SAN moves but no clock comments. */
export function pgnFromUci(moves: readonly string[]): string {
  const game = defaultGame<PgnNodeData>()
  extend(
    game.moves,
    sanHistory(moves).map((san) => ({ san })),
  ) // mutates the root's children
  return makePgn(game)
}

/* ── Any variant, from any start ─────────────────────────────────────── */

/** The position after `moves` from `setup` (illegal moves end the replay). */
export function setupPositionAfter(setup: GameSetup, moves: readonly string[]): Position {
  return replaySetup(setup, moves)?.position ?? setupStart(STANDARD_SETUP)!
}

/** SAN of each move from `setup`. */
export function setupSanHistory(setup: GameSetup, moves: readonly string[]): string[] {
  return replaySetup(setup, moves)?.played.map((move) => move.san) ?? []
}

/** Chessground destinations under the setup's rules (Chess960 castles king onto rook). */
export function setupDests(setup: GameSetup, pos: Position): Dests {
  return chessgroundDests(pos, { chess960: isChess960(setup.variant) }) as Dests
}

/** Draws by repetition and the fifty-move rule, counted from the setup's start. */
export function setupDrawReason(
  setup: GameSetup,
  moves: readonly string[],
): DrawReason | undefined {
  const replayed = replaySetup(setup, moves)
  if (!replayed) return undefined
  // Antichess, horde and racing kings have no repetition rule worth claiming here.
  if (!['standard', 'chess960', 'kingOfTheHill', 'threeCheck', 'atomic'].includes(setup.variant))
    return undefined
  const pos = replayed.start.clone()
  const key = (p: Position): string => fen(p).split(' ').slice(0, 4).join(' ')
  const seen = new Map<string, number>([[key(pos), 1]])
  for (const move of replayed.played) {
    const parsed = parseUci(move.uci)
    if (!parsed) break
    pos.play(normalizeMove(pos, parsed))
    const k = key(pos)
    const count = (seen.get(k) ?? 0) + 1
    seen.set(k, count)
    if (count >= 3) return 'Threefold repetition'
  }
  if (!pos.isEnd() && pos.halfmoves >= 100) return 'Fifty-move rule'
  return undefined
}

/** A PGN of moves from a setup, with the headers another program needs to replay it. */
export function setupPgn(
  setup: GameSetup,
  moves: readonly string[],
  headers: Record<string, string> = {},
): string {
  const game = defaultGame<PgnNodeData>()
  for (const [key, value] of Object.entries(headers)) game.headers.set(key, value)
  if (setup.variant !== 'standard')
    game.headers.set('Variant', setup.variant === 'chess960' ? 'Chess960' : setup.variant)
  if (setup.fen !== STANDARD_SETUP.fen || setup.variant === 'chess960') {
    game.headers.set('SetUp', '1')
    game.headers.set('FEN', setup.fen)
  }
  extend(
    game.moves,
    setupSanHistory(setup, moves).map((san) => ({ san })),
  )
  return makePgn(game)
}
