import { rules } from './engine.ts'

/** Material balance of a FEN's placement in pawns: positive when White is ahead. */
export function materialBalance(fen: string): number {
  return rules<number>('materialBalance', fen)
}
