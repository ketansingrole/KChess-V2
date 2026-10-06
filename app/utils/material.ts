const VALUES: Record<string, number> = { p: 1, n: 3, b: 3, r: 5, q: 9 }

/** Material balance of a FEN's placement in pawns: positive when White is ahead. */
export function materialBalance(fen: string): number {
  let balance = 0
  for (const char of fen.split(' ')[0] ?? '') {
    const value = VALUES[char.toLowerCase()]
    if (value) balance += char === char.toUpperCase() ? value : -value
  }
  return balance
}
