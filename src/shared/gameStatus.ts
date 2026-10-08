import type { LichessGame } from './types'

/**
 * Lichess `GameStatusName` values for a game that is still being played.
 * `created` is a live game before its first move, not a finished one.
 */
export function isGameInProgress(status?: string): boolean {
  return !status || status === 'started' || status === 'created'
}

/** The result from the account's side. */
export function gameResult(game: LichessGame): 'win' | 'loss' | 'draw' {
  return game.winner ? (game.winner === game.color ? 'win' : 'loss') : 'draw'
}
