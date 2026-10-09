import { rules } from './engine.ts'
import { INITIAL_BOARD_FEN, type Role } from './position.ts'

/** What the editor knows besides where the pieces stand. */
export interface EditorSetup {
  /** Piece placement, the first field of a FEN. */
  board: string
  turn: 'white' | 'black'
  /** Castling rights the player has ticked; only those the pieces allow are written. */
  castling: { K: boolean; Q: boolean; k: boolean; q: boolean }
  /** En passant target square, or empty. */
  ep: string
}

export const START_SETUP: EditorSetup = {
  board: INITIAL_BOARD_FEN,
  turn: 'white',
  castling: { K: true, Q: true, k: true, q: true },
  ep: '',
}
export const EMPTY_BOARD = '8/8/8/8/8/8/8/8'

/** The board field with `square` holding `piece`, or emptied when `piece` is undefined. */
export function withPiece(
  board: string,
  square: string,
  piece: { color: 'white' | 'black'; role: Role } | undefined,
): string {
  return rules<string>('withPiece', board, square, piece ?? null)
}

/** Which castling rights the king and rook squares still allow. */
export function castlingAvailable(board: string): EditorSetup['castling'] {
  return rules<EditorSetup['castling']>('castlingAvailable', board)
}

/**
 * Squares a pawn could just have skipped with a double step, so the side to move might capture
 * en passant there: the pawn stands on its fourth rank with both squares behind it empty.
 */
export function enPassantSquares(board: string, turn: EditorSetup['turn']): string[] {
  return rules<string[]>('enPassantSquares', board, turn)
}

export function setupFen(setup: EditorSetup): string {
  return rules<string>('setupFen', setup)
}

/** Read a FEN typed or pasted into the editor; undefined when it is not one. */
export function setupFromFen(text: string): EditorSetup | undefined {
  const trimmed = text.trim()
  const parsed = rules<{ board: string; turn: EditorSetup['turn'] } | null>('fenSetup', trimmed)
  if (!parsed) return undefined
  const fields = trimmed.split(/\s+/)
  const rights = fields[2] ?? '-'
  return {
    board: parsed.board,
    turn: parsed.turn,
    castling: {
      K: rights.includes('K'),
      Q: rights.includes('Q'),
      k: rights.includes('k'),
      q: rights.includes('q'),
    },
    ep: /^[a-h][36]$/.test(fields[3] ?? '') ? fields[3]! : '',
  }
}

const PROBLEMS: Record<string, string> = {
  fen: 'That is not a valid FEN.',
  empty: 'The board is empty.',
  kings: 'Each side needs exactly one king.',
  pawnsOnBackrank: 'Pawns can’t stand on the first or last rank.',
  oppositeCheck: 'The side not to move is in check.',
}

/** Why this position can’t be played or analysed, or undefined when it can. */
export function positionProblem(fen: string): string | undefined {
  const problem = rules<string | null>('fenProblem', fen)
  return problem ? (PROBLEMS[problem] ?? 'This position is not legal.') : undefined
}
