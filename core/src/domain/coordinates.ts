/** Board-vision drills: naming squares, finding them, and telling their color. */

export const FILES = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] as const
export const RANKS = ['1', '2', '3', '4', '5', '6', '7', '8'] as const
export type Square = `${(typeof FILES)[number]}${(typeof RANKS)[number]}`

export const ALL_SQUARES: readonly Square[] = FILES.flatMap((file) =>
  RANKS.map((rank) => `${file}${rank}` as Square),
)

/** A random square that is not `previous`. */
export function randomSquare(previous?: string, random: () => number = Math.random): Square {
  let square: Square
  do square = ALL_SQUARES[Math.floor(random() * ALL_SQUARES.length)]!
  while (square === previous)
  return square
}

/** a1 is dark, h1 is light. */
export function squareColor(square: string): 'light' | 'dark' {
  const file = square.charCodeAt(0) - 97
  const rank = Number(square[1]) - 1
  return (file + rank) % 2 === 0 ? 'dark' : 'light'
}

/** The empty board. */
export const EMPTY_FEN = '8/8/8/8/8/8/8/8 w - - 0 1'
