import { rules } from './engine.ts'

/** Board-vision drills: naming squares, finding them, and telling their color. */

export const FILES = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] as const
export const RANKS = ['1', '2', '3', '4', '5', '6', '7', '8'] as const
export type Square = `${(typeof FILES)[number]}${(typeof RANKS)[number]}`

export const ALL_SQUARES: readonly Square[] = FILES.flatMap((file) =>
  RANKS.map((rank) => `${file}${rank}` as Square),
)

/**
 * A random square that is not `previous`. The rules pick from the draws in order; this draws
 * one more number each time the first ones give `previous`.
 */
export function randomSquare(previous?: string, random: () => number = Math.random): Square {
  const draws: number[] = []
  for (;;) {
    draws.push(random())
    const square = rules<Square | null>('randomSquare', previous ?? null, draws)
    if (square) return square
  }
}

/** a1 is dark, h1 is light. */
export function squareColor(square: string): 'light' | 'dark' {
  return rules<'light' | 'dark'>('squareColor', square)
}

/** The empty board. */
export const EMPTY_FEN = '8/8/8/8/8/8/8/8 w - - 0 1'
