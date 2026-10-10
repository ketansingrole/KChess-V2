import { rules } from './engine.ts'
import {
  pieceOn,
  Position,
  uciSquares,
  type Color,
  type Dests,
  type SquareName,
} from './position.ts'
import { isChess960, replaySetup, STANDARD_SETUP, type GameSetup } from './variant.ts'

export type { Dests }

export function fen(pos: Position): string {
  return pos.fen
}

/** Replay a list of UCI moves from the standard starting position. */
export function positionAfter(moves: readonly string[]): Position {
  const start = Position.initial()
  return start.after(start.line(moves, { trim: true }))
}

/** The position a FEN describes, or undefined when it is not a playable one. */
export function positionFromFen(text: string): Position | undefined {
  return Position.fromFen(text)
}

/** The SAN of a UCI move from a position, or false when illegal. The position is unchanged. */
export function playUci(pos: Position, uci: string): string | false {
  return pos.play(uci.trim())?.san ?? false
}

/** SAN move list for a game, used by the move list UI. */
export function sanHistory(moves: readonly string[]): string[] {
  return rules<string[]>('sanHistory', moves)
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
  return pos.dests('board')
}

/** Last move as squares, for the board highlight. */
export function lastMoveKeys(moves: readonly string[]): SquareName[] | undefined {
  const last = moves.at(-1)
  return last === undefined ? undefined : uciSquares(last.trim())
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
  const setup = rules<{ board: string } | null>('fenSetup', fen)
  const piece = setup && pieceOn(setup.board, orig)
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

/** Draws `isEnd()` does not cover: repetition and the 50-move rule. */
export function drawReason(moves: readonly string[]): DrawReason | undefined {
  return rules<DrawReason | null>('drawReason', moves) ?? undefined
}

/**
 * Build a PGN for games stored without one.
 * Synced Lichess games keep SAN tokens in `moves` (e.g. "e4 e5 Nf3"),
 * while local/online moves are UCI (e.g. "e2e4 e7e5 g1f3").
 */
export function pgnFromSan(sans: readonly string[]): string {
  return rules<string>('pgnFromSan', sans)
}

/** Tolerant fallback: accepts UCI or SAN tokens and returns a PGN. */
export function pgnFromMoves(tokens: readonly string[]): string {
  return rules<string>('pgnFromMoves', tokens)
}

/** Build a PGN from UCI moves. The result has SAN moves but no clock comments. */
export function pgnFromUci(moves: readonly string[]): string {
  return rules<string>('pgnFromUci', moves)
}

/* ── Any variant, from any start ─────────────────────────────────────── */

/** The position after `moves` from `setup` (illegal moves end the replay). */
export function setupPositionAfter(setup: GameSetup, moves: readonly string[]): Position {
  return replaySetup(setup, moves)?.position ?? Position.from(STANDARD_SETUP)!
}

/** SAN of each move from `setup`. */
export function setupSanHistory(setup: GameSetup, moves: readonly string[]): string[] {
  return (
    rules<{ played: { san: string }[] } | null>('setupReplay', setup, moves)?.played.map(
      (move) => move.san,
    ) ?? []
  )
}

/** Chessground destinations under the setup's rules (Chess960 castles king onto rook). */
export function setupDests(setup: GameSetup, pos: Position): Dests {
  return pos.dests(isChess960(setup.variant) ? 'board960' : 'board')
}

/** Draws by repetition and the fifty-move rule, counted from the setup's start. */
export function setupDrawReason(
  setup: GameSetup,
  moves: readonly string[],
): DrawReason | undefined {
  return rules<DrawReason | null>('setupDrawReason', setup, moves) ?? undefined
}

/** A PGN of moves from a setup, with the headers another program needs to replay it. */
export function setupPgn(
  setup: GameSetup,
  moves: readonly string[],
  headers: Record<string, string> = {},
): string {
  return rules<string>('setupPgn', setup, moves, headers)
}
