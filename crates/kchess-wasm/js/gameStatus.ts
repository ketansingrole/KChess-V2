import type { LichessGame } from '@kchess/contracts/types'
import type { ArchivedGame } from '@kchess/contracts/library'
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

export type ArchivedOutcome =
  'win' | 'loss' | 'draw' | 'white' | 'black' | 'unfinished' | 'ongoing' | 'clock'

/** The side named “You” in a computer game; over-the-board games have none. */
export function archivedPlayerSide(game: ArchivedGame): 'white' | 'black' | null {
  return rules<'white' | 'black' | null>('archivedPlayerSide', {
    white: game.white,
    black: game.black,
  })
}

/**
 * A saved local game's outcome: won or lost for the side named “You” (computer games), or the
 * winning colour when both players were at the board.
 */
export function archivedOutcome(game: ArchivedGame): ArchivedOutcome {
  // The rule reads only these fields; the moves would be serialized once per history row.
  return rules<ArchivedOutcome>('archivedOutcome', {
    source: game.source,
    result: game.result,
    finished: game.finished,
    white: game.white,
    black: game.black,
  })
}
