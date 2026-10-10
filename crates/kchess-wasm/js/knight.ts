import { rules } from './engine.ts'
import type { Square } from './coordinates.ts'

/** Squares a knight on `square` can jump to. */
export function knightMoves(square: string): Square[] {
  return rules<Square[]>('knightMoves', square)
}

/** Fewest jumps from `from` to `to`. */
export function knightDistance(from: string, to: string): number {
  // The rules report no path as null (JSON has no Infinity).
  return rules<number | null>('knightDistance', from, to) ?? Infinity
}

/**
 * A start and target squares `min`–`max` jumps apart. The rules take the draws in order; this
 * draws two more squares each time the ones so far have no pair in range.
 */
export function knightChallenge(
  min: number,
  max: number,
  random: () => number = Math.random,
): { from: Square; to: Square; best: number } {
  const draws: number[] = []
  for (;;) {
    draws.push(random(), random())
    const found = rules<{ from: Square; to: Square; best: number | null } | null>(
      'knightChallenge',
      min,
      max,
      draws,
    )
    if (found) return { ...found, best: found.best ?? Infinity }
  }
}

/** A board with one white knight, as a FEN (the position is not a legal game; the board only draws it). */
export function knightFen(square: string): string {
  return rules<string>('knightFen', square)
}
