/**
 * A chess position as every frontend holds it: its rules and its FEN. The Rust rules answer every
 * question about it (`crates/kchess-domain/src/position.rs`), so a position is a plain immutable
 * value — playing a move returns a new one — that can live in reactive state and cross IPC.
 */
import { rules } from './engine.ts'
import type { GameSetup, Variant } from './variant.ts'

export const INITIAL_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
export const INITIAL_BOARD_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR'

export type Color = 'white' | 'black'
export type Role = 'pawn' | 'knight' | 'bishop' | 'rook' | 'queen' | 'king'
type File = 'a' | 'b' | 'c' | 'd' | 'e' | 'f' | 'g' | 'h'
type Rank = '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8'
export type SquareName = `${File}${Rank}`
export interface Piece {
  color: Color
  role: Role
}
/** Each piece's destinations, as chessground takes them. */
export type Dests = Map<SquareName, SquareName[]>

export const opposite = (color: Color): Color => (color === 'white' ? 'black' : 'white')

/** What the rules report about a position (`position::Info`). */
interface Info {
  fen: string
  turn: Color
  check: boolean
  checkmate: boolean
  stalemate: boolean
  insufficient: boolean
  cannotWin: Record<Color, boolean>
  variantEnd: boolean
  end: boolean
  /** Absent while the game goes on; no winner for a draw. */
  outcome?: { winner?: Color }
  halfmoves: number
  fullmoves: number
  castling: boolean
  pieces: number
}

/** A move a position can make, listed for searching (`legalMoves`). */
export interface LegalMove {
  from: SquareName
  to: SquareName
  role: Role
  promotion?: Role
  san: string
}

/** A move played: UCI as chessops writes it (castling king-to-rook), SAN and the new position. */
export interface PlayedMove {
  uci: string
  san: string
  position: Position
}

/** One step of a line (`Position.line`). */
export interface LineMove {
  uci: string
  san: string
  fen: string
}

const ROLES: Record<string, Role> = {
  p: 'pawn',
  n: 'knight',
  b: 'bishop',
  r: 'rook',
  q: 'queen',
  k: 'king',
}

export class Position {
  // Plain fields rather than `#private` ones: positions live inside Vue's reactive proxies.
  private readonly info: Info
  private destsCache: Partial<Record<'rules' | 'board' | 'board960', Dests>> = {}
  private boardCache?: Map<SquareName, Piece>

  readonly variant: Variant

  private constructor(variant: Variant, info: Info) {
    this.variant = variant
    this.info = info
  }

  /** The position a setup describes, or undefined when its FEN is not legal under its rules. */
  static from(setup: GameSetup): Position | undefined {
    const info = rules<Info | null>('position', setup)
    return info ? new Position(setup.variant, info) : undefined
  }

  /** A standard-rules position from a FEN, or undefined when it is not a playable one. */
  static fromFen(fen: string): Position | undefined {
    return Position.from({ variant: 'standard', fen })
  }

  /** The standard starting position. */
  static initial(): Position {
    return Position.fromFen(INITIAL_FEN)!
  }

  get setup(): GameSetup {
    return { variant: this.variant, fen: this.info.fen }
  }
  /** chessops `makeFen(pos.toSetup())`. */
  get fen(): string {
    return this.info.fen
  }
  get turn(): Color {
    return this.info.turn
  }
  get halfmoves(): number {
    return this.info.halfmoves
  }
  get fullmoves(): number {
    return this.info.fullmoves
  }
  /** Pieces on the board. */
  get pieceCount(): number {
    return this.info.pieces
  }
  hasCastlingRights(): boolean {
    return this.info.castling
  }
  isCheck(): boolean {
    return this.info.check
  }
  isCheckmate(): boolean {
    return this.info.checkmate
  }
  isStalemate(): boolean {
    return this.info.stalemate
  }
  /** Neither side can win. */
  isInsufficientMaterial(): boolean {
    return this.info.insufficient
  }
  /** `color` cannot possibly win. */
  hasInsufficientMaterial(color: Color): boolean {
    return this.info.cannotWin[color]
  }
  isVariantEnd(): boolean {
    return this.info.variantEnd
  }
  isEnd(): boolean {
    return this.info.end
  }
  /** Undefined while the game goes on; `{}` for a draw. */
  outcome(): { winner?: Color } | undefined {
    return this.info.outcome
  }

  /** The piece on a square. */
  pieceAt(square: string): Piece | undefined {
    return this.board().get(square as SquareName)
  }

  /** Every piece on the board. */
  board(): Map<SquareName, Piece> {
    if (this.boardCache) return this.boardCache
    const board = new Map<SquareName, Piece>()
    const rows = this.info.fen.split(' ')[0]!.replace(/\[.*$/, '').split('/')
    rows.slice(0, 8).forEach((row, index) => {
      let file = 0
      for (const char of row.replace(/~/g, '')) {
        if (/\d/.test(char)) file += Number(char)
        else {
          const role = ROLES[char.toLowerCase()]
          if (!role) continue
          if (file < 8)
            board.set(`${'abcdefgh'[file]}${8 - index}` as SquareName, {
              color: char === char.toLowerCase() ? 'black' : 'white',
              role,
            })
          file++
        }
      }
    })
    this.boardCache = board
    return board
  }

  /**
   * Legal destinations. `board` lists standard castling on both the rook's and the king's
   * square, as chessground expects; `board960` only on the rook's; `rules` is chessops
   * `allDests()`.
   */
  dests(mode: 'rules' | 'board' | 'board960' = 'board'): Dests {
    const cached = this.destsCache[mode]
    if (cached) return cached
    const dests: Dests = new Map(rules<[SquareName, SquareName[]][]>('dests', this.setup, mode))
    this.destsCache[mode] = dests
    return dests
  }

  /** Every legal move with its SAN, promotions to queen, rook, bishop and knight. */
  legalMoves(): LegalMove[] {
    return rules<LegalMove[]>('legalMoves', this.setup)
  }

  private played(result: { uci: string; san: string; position: Info } | null) {
    return result
      ? { uci: result.uci, san: result.san, position: new Position(this.variant, result.position) }
      : undefined
  }

  /** Play a UCI move (castling either king-to-rook or two squares); undefined when illegal. */
  play(uci: string): PlayedMove | undefined {
    return this.played(rules('play', this.setup, uci))
  }

  /** Play a SAN move; undefined when it is not a legal one. */
  playSan(san: string): PlayedMove | undefined {
    return this.played(rules('playSan', this.setup, san))
  }

  /**
   * Play moves one after another until one is illegal: UCI moves, or SAN with `san`. With
   * `trim`, UCI moves are trimmed first.
   */
  line(moves: readonly string[], options: { san?: boolean; trim?: boolean } = {}): LineMove[] {
    return rules<{ moves: LineMove[] }>('line', this.setup, moves, options).moves
  }

  /** The position after a line from `line()`; this one when it is empty. */
  after(line: readonly LineMove[]): Position {
    const last = line.at(-1)
    return last ? Position.from({ variant: this.variant, fen: last.fen })! : this
  }
}

/** The first four FEN fields: what makes two positions the same for repetition. */
export const repetitionKey = (fen: string): string => fen.split(' ').slice(0, 4).join(' ')

/** The squares of a UCI move as chessops `parseUci` reads it; undefined for drops and non-moves. */
export function uciSquares(uci: string): [SquareName, SquareName] | undefined {
  const match = /^([a-h][1-8])([a-h][1-8])[pnbrqkPNBRQK]?$/.exec(uci)
  return match ? [match[1] as SquareName, match[2] as SquareName] : undefined
}

/** The piece on a square of a FEN placement field (`e1` → white king), or undefined. */
export function pieceOn(board: string, square: string): Piece | undefined {
  const file = square.charCodeAt(0) - 97
  const row = board.split('/')[8 - Number(square[1])]
  if (!row || file < 0 || file > 7) return undefined
  let index = 0
  for (const char of row) {
    if (/\d/.test(char)) index += Number(char)
    else if (ROLES[char.toLowerCase()] && index++ === file)
      return {
        color: char === char.toLowerCase() ? 'black' : 'white',
        role: ROLES[char.toLowerCase()]!,
      }
    if (index > file) return undefined
  }
  return undefined
}
