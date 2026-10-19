import type { ArchivedGame } from '@kchess/rules/library'

export function formatGameDate(timestamp: number): string {
  return new Date(timestamp).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  })
}

export type ResultBadge = {
  label: string
  color: 'success' | 'error' | 'neutral' | 'info'
  variant: 'soft' | 'subtle' | 'outline' | 'solid'
}

/** A finished game's outcome for the player whose history it is. */
export const OUTCOME_BADGES = {
  win: { label: 'Win', color: 'success', variant: 'soft' },
  loss: { label: 'Loss', color: 'error', variant: 'soft' },
  draw: { label: 'Draw', color: 'neutral', variant: 'subtle' },
} as const satisfies Record<'win' | 'loss' | 'draw', ResultBadge>

/**
 * A saved local game's outcome: won or lost for the side named “You” (computer games), or the
 * winning colour when both players were at the board.
 */
export function archivedResultBadge(game: ArchivedGame): ResultBadge {
  if (game.source === 'clock') return { label: 'Clock', color: 'neutral', variant: 'soft' }
  if (game.result === '*')
    return game.finished
      ? { label: 'Unfinished', color: 'neutral', variant: 'soft' }
      : { label: 'In progress', color: 'info', variant: 'soft' }
  if (game.result === '1/2-1/2') return OUTCOME_BADGES.draw
  const winner = game.result === '1-0' ? 'white' : 'black'
  const you = game.white === 'You' ? 'white' : game.black === 'You' ? 'black' : null
  if (you) return OUTCOME_BADGES[you === winner ? 'win' : 'loss']
  // The badge takes the winning side's colour: light outline for White, solid for Black.
  return winner === 'white'
    ? { label: 'White won', color: 'neutral', variant: 'outline' }
    : { label: 'Black won', color: 'neutral', variant: 'solid' }
}
