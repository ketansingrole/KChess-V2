import type { LichessGame } from '@kchess/contracts/types'
import { rules } from './engine.ts'

/**
 * Lichess `GameStatusName` values for a game that is still being played.
 * `created` is a live game before its first move, not a finished one.
 */
export function isGameInProgress(status?: string): boolean {
  return rules<boolean>('isGameInProgress', status ?? null)
}

/** The result from the account's side. */
export function gameResult(game: LichessGame): 'win' | 'loss' | 'draw' {
  // The rule reads only these two fields; sending the whole game also serialized its moves and
  // PGN once per row on the history list.
  return rules<'win' | 'loss' | 'draw'>('gameResult', {
    winner: game.winner,
    color: game.color,
  })
}
