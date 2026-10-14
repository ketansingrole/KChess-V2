import { rules } from './engine.ts'

/** Material balance of a FEN's placement in pawns: positive when White is ahead. */
export function materialBalance(fen: string): number {
  return rules<number>('materialBalance', fen)
}

export type MaterialRole = 'king' | 'queen' | 'rook' | 'bishop' | 'knight' | 'pawn'

/** Lichess's material difference: each side's surplus pieces, most valuable first. */
export interface MaterialDiff {
  white: MaterialRole[]
  black: MaterialRole[]
  /** Balance in pawns: positive when White is ahead. */
  score: number
}

export function materialDiff(fen: string): MaterialDiff {
  return rules<MaterialDiff>('materialDiff', fen)
}
