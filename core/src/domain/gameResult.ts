import type { Position } from './position.ts'
import type { Variant } from './variant'

type Color = 'white' | 'black'
export interface GameResult {
  /** None for a draw. */
  winner?: Color
  reason: string
}

const VARIANT_WIN: Partial<Record<Variant, string>> = {
  kingOfTheHill: 'King reached the centre',
  threeCheck: 'Third check',
  antichess: 'Lost every piece',
  atomic: 'King exploded',
  horde: 'Horde captured',
  racingKings: 'King reached the eighth rank',
}

export const opponent = (color: Color): Color => (color === 'white' ? 'black' : 'white')

/** How the position ended the game; `draw` is a repetition or fifty-move draw already found. */
export function boardResult(pos: Position, variant: Variant, draw?: string): GameResult | null {
  const outcome = pos.outcome()
  if (outcome) {
    const reason = pos.isCheckmate()
      ? 'Checkmate'
      : pos.isStalemate()
        ? 'Stalemate'
        : pos.isVariantEnd()
          ? (VARIANT_WIN[variant] ?? 'Game over')
          : 'Insufficient material'
    return { ...(outcome.winner ? { winner: outcome.winner } : {}), reason }
  }
  return draw ? { reason: draw } : null
}

/** Who wins when `flagged` runs out of time: nobody when the other side cannot possibly mate. */
export function timeoutWinner(pos: Position, flagged: Color): Color | undefined {
  const winner = opponent(flagged)
  return pos.hasInsufficientMaterial(winner) ? undefined : winner
}

/** The PGN result token. */
export function pgnResult(over: boolean, winner?: Color): '*' | '1-0' | '0-1' | '1/2-1/2' {
  if (!over) return '*'
  return winner === 'white' ? '1-0' : winner === 'black' ? '0-1' : '1/2-1/2'
}
