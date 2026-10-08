import { Chess, IllegalSetup } from 'chessops/chess'
import { INITIAL_BOARD_FEN, makeFen, parseFen } from 'chessops/fen'

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

/** The piece letter on a square of a FEN board field (`e1` → `K`), or undefined. */
function pieceOn(board: string, square: string): string | undefined {
  const file = square.charCodeAt(0) - 97
  const row = board.split('/')[8 - Number(square[1])]
  if (!row) return undefined
  let index = 0
  for (const char of row) {
    if (/\d/.test(char)) index += Number(char)
    else if (index++ === file) return char
    if (index > file) return undefined
  }
  return undefined
}

const LETTERS = { king: 'k', queen: 'q', rook: 'r', bishop: 'b', knight: 'n', pawn: 'p' } as const

/** The board field with `square` holding `piece`, or emptied when `piece` is undefined. */
export function withPiece(
  board: string,
  square: string,
  piece: { color: 'white' | 'black'; role: keyof typeof LETTERS } | undefined,
): string {
  const rows = board.split('/').map((row) => row.replace(/\d/g, (n) => '.'.repeat(Number(n))))
  const rank = 8 - Number(square[1])
  const cells = rows[rank]!.split('')
  const letter = piece ? LETTERS[piece.role] : '.'
  cells[square.charCodeAt(0) - 97] = piece?.color === 'white' ? letter.toUpperCase() : letter
  rows[rank] = cells.join('')
  return rows.map((row) => row.replace(/\.+/g, (dots) => String(dots.length))).join('/')
}

/** Which castling rights the king and rook squares still allow. */
export function castlingAvailable(board: string): EditorSetup['castling'] {
  const at = (square: string, piece: string): boolean => pieceOn(board, square) === piece
  return {
    K: at('e1', 'K') && at('h1', 'R'),
    Q: at('e1', 'K') && at('a1', 'R'),
    k: at('e8', 'k') && at('h8', 'r'),
    q: at('e8', 'k') && at('a8', 'r'),
  }
}

/**
 * Squares a pawn could just have skipped with a double step, so the side to move might capture
 * en passant there: the pawn stands on its fourth rank with both squares behind it empty.
 */
export function enPassantSquares(board: string, turn: EditorSetup['turn']): string[] {
  const [pawn, rank, passed, start] = turn === 'white' ? ['p', '5', '6', '7'] : ['P', '4', '3', '2']
  return [...'abcdefgh']
    .filter(
      (file) =>
        pieceOn(board, `${file}${rank}`) === pawn &&
        !pieceOn(board, `${file}${passed}`) &&
        !pieceOn(board, `${file}${start}`),
    )
    .map((file) => `${file}${passed}`)
}

export function setupFen(setup: EditorSetup): string {
  const available = castlingAvailable(setup.board)
  const castling = (['K', 'Q', 'k', 'q'] as const)
    .filter((right) => setup.castling[right] && available[right])
    .join('')
  const ep = enPassantSquares(setup.board, setup.turn).includes(setup.ep) ? setup.ep : '-'
  return `${setup.board} ${setup.turn === 'white' ? 'w' : 'b'} ${castling || '-'} ${ep} 0 1`
}

/** Read a FEN typed or pasted into the editor; undefined when it is not one. */
export function setupFromFen(text: string): EditorSetup | undefined {
  const trimmed = text.trim()
  const parsed = parseFen(trimmed)
  if (parsed.isErr) return undefined
  const fields = trimmed.split(/\s+/)
  const board = makeFen(parsed.value).split(' ')[0]!
  const rights = fields[2] ?? '-'
  return {
    board,
    turn: parsed.value.turn,
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
  [IllegalSetup.Empty]: 'The board is empty.',
  [IllegalSetup.Kings]: 'Each side needs exactly one king.',
  [IllegalSetup.PawnsOnBackrank]: 'Pawns can’t stand on the first or last rank.',
  [IllegalSetup.OppositeCheck]: 'The side not to move is in check.',
}

/** Why this position can’t be played or analysed, or undefined when it can. */
export function positionProblem(fen: string): string | undefined {
  const setup = parseFen(fen)
  if (setup.isErr) return 'That is not a valid FEN.'
  const pos = Chess.fromSetup(setup.value)
  if (pos.isErr) return PROBLEMS[pos.error.message] ?? 'This position is not legal.'
  return undefined
}
