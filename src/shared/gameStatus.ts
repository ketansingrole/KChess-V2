/**
 * Lichess `GameStatusName` values for a game that is still being played.
 * `created` is a live game before its first move, not a finished one.
 */
export function isGameInProgress(status?: string): boolean {
  return !status || status === 'started' || status === 'created'
}
