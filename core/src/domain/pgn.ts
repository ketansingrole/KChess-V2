/** Reading PGN documents through the Rust rules (`crates/kchess-domain/src/position.rs`). */
import { rules } from './engine.ts'

/** One game of a PGN document. */
export interface PgnGame {
  /** In document order. */
  headers: [string, string][]
  /** The game as the PGN writer writes it, so equal games compare equal. */
  pgn: string
  /** Why it cannot be replayed: its starting position, or an illegal move in any variation. */
  problem: 'start' | 'move' | null
}

/** Every game of a PGN document. */
export function pgnGames(pgn: string): PgnGame[] {
  return rules<PgnGame[]>('pgnGames', pgn)
}

export interface MainlineMove {
  /** As written in the PGN. */
  san: string
  /** Castling is king-to-rook. */
  uci: string
  /** After the move. */
  fen: string
  mover: 'white' | 'black'
  /** The side to move after it is in check. */
  check: boolean
  /** Seconds left on the mover's clock (`[%clk]`), when the comment has one. */
  clock?: number
}

export interface Mainline {
  headers: [string, string][]
  /** The game's starting FEN; null when it cannot be set up (the moves then start from the usual position). */
  start: string | null
  startCheck: boolean
  /** Played until the first illegal move. */
  moves: MainlineMove[]
}

/** The first game's main line, at most `limit` moves; null when the text has no game. */
export function pgnMainline(pgn: string, limit?: number): Mainline | null {
  return rules<Mainline | null>('pgnMainline', pgn, limit)
}
