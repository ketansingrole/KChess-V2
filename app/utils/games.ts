import type { LichessGame } from '../../src/shared/types'

export function gameResult(game: LichessGame): 'win' | 'loss' | 'draw' {
  return game.winner ? (game.winner === game.color ? 'win' : 'loss') : 'draw'
}

export function formatGameDate(timestamp: number): string {
  return new Date(timestamp).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  })
}
