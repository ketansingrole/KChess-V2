import { Chess, type Position } from 'chessops/chess'
import { chessgroundDests } from 'chessops/compat'
import { makeFen, parseFen } from 'chessops/fen'
import { extend, defaultGame, makePgn, type PgnNodeData } from 'chessops/pgn'
import { makeSanAndPlay } from 'chessops/san'
import { makeSquare, parseSquare, parseUci } from 'chessops/util'
import type { Color, Key } from '@lichess-org/chessground/types'
import { UCI_MOVE } from '../../src/shared/patterns.ts'

export type Dests = Map<Key, Key[]>

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

/** Last move as chessground keys, for the board highlight. */
export function lastMoveKeys(moves: readonly string[]): Key[] | undefined {
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
export function isPromotionMove(fen: string, orig: Key, dest: Key): boolean {
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
