import { ALL_SQUARES, FILES, type Square } from './coordinates.ts'

const JUMPS: readonly [number, number][] = [
  [1, 2],
  [2, 1],
  [2, -1],
  [1, -2],
  [-1, -2],
  [-2, -1],
  [-2, 1],
  [-1, 2],
]

const index = (square: string): [number, number] => [
  square.charCodeAt(0) - 97,
  Number(square[1]) - 1,
]
const name = (file: number, rank: number): Square => `${FILES[file]}${rank + 1}` as Square

/** Squares a knight on `square` can jump to. */
export function knightMoves(square: string): Square[] {
  const [file, rank] = index(square)
  return JUMPS.flatMap(([df, dr]) => {
    const f = file + df
    const r = rank + dr
    return f >= 0 && f < 8 && r >= 0 && r < 8 ? [name(f, r)] : []
  })
}

/** Fewest jumps from `from` to `to`. */
export function knightDistance(from: string, to: string): number {
  if (from === to) return 0
  const seen = new Set<string>([from])
  let frontier: string[] = [from]
  for (let steps = 1; frontier.length; steps++) {
    const next: string[] = []
    for (const square of frontier)
      for (const target of knightMoves(square)) {
        if (target === to) return steps
        if (!seen.has(target)) {
          seen.add(target)
          next.push(target)
        }
      }
    frontier = next
  }
  return Infinity
}

/** A start and target squares `min`–`max` jumps apart. */
export function knightChallenge(
  min: number,
  max: number,
  random: () => number = Math.random,
): { from: Square; to: Square; best: number } {
  for (;;) {
    const from = ALL_SQUARES[Math.floor(random() * 64)]!
    const to = ALL_SQUARES[Math.floor(random() * 64)]!
    const best = knightDistance(from, to)
    if (best >= min && best <= max) return { from, to, best }
  }
}

/** A board with one white knight, as a FEN (the position is not a legal game; the board only draws it). */
export function knightFen(square: string): string {
  const [file, rank] = index(square)
  const rows = Array.from({ length: 8 }, (_, r) => {
    const row = 7 - r
    return row === rank ? `${file || ''}N${7 - file || ''}` : '8'
  })
  return `${rows.join('/')} w - - 0 1`
}
