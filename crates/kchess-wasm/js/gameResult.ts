import type { Position } from './position.ts'
import type { Variant } from './variant.ts'
import { rules } from './engine.ts'

type Color = 'white' | 'black'
export interface GameResult {
  /** None for a draw. */
  winner?: Color
  reason: string
}

export function opponent(color: Color): Color {
  return rules<Color>('opponent', color)
}

/** How the position ended the game; `draw` is a repetition or fifty-move draw already found. */
export function boardResult(pos: Position, variant: Variant, draw?: string): GameResult | null {
  return rules<GameResult | null>('boardResult', pos.setup, variant, draw ?? null)
}

/** Who wins when `flagged` runs out of time: nobody when the other side cannot possibly mate. */
export function timeoutWinner(pos: Position, flagged: Color): Color | undefined {
  return rules<Color | null>('timeoutWinner', pos.setup, flagged) ?? undefined
}

/** The PGN result token. */
export function pgnResult(over: boolean, winner?: Color): '*' | '1-0' | '0-1' | '1/2-1/2' {
  return rules<'*' | '1-0' | '0-1' | '1/2-1/2'>('pgnResult', over, winner ?? null)
}
