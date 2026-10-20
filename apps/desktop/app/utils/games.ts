import type { ArchivedGame } from '@kchess/rules/library'

const GAME_DATE = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
})

export function formatGameDate(timestamp: number): string {
  return GAME_DATE.format(new Date(timestamp))
}

export type ResultBadge = {
  label: string
  color: 'success' | 'error' | 'neutral' | 'info'
  variant: 'soft' | 'subtle' | 'outline'
  /** The winning side, drawn as a piece-coloured dot that reads the same in either theme. */
  side?: 'white' | 'black'
}

/** A finished game's outcome for the player whose history it is. */
export const OUTCOME_BADGES = {
  win: { label: 'Win', color: 'success', variant: 'soft' },
  loss: { label: 'Loss', color: 'error', variant: 'soft' },
  draw: { label: 'Draw', color: 'neutral', variant: 'subtle' },
} as const satisfies Record<'win' | 'loss' | 'draw', ResultBadge>

export type ArchivedOutcome =
  'win' | 'loss' | 'draw' | 'white' | 'black' | 'unfinished' | 'ongoing' | 'clock'

/** The side named “You” in a computer game; over-the-board games have none. */
export function archivedPlayerSide(game: ArchivedGame): 'white' | 'black' | null {
  return game.white === 'You' ? 'white' : game.black === 'You' ? 'black' : null
}

/**
 * A saved local game's outcome: won or lost for the side named “You” (computer games), or the
 * winning colour when both players were at the board.
 */
export function archivedOutcome(game: ArchivedGame): ArchivedOutcome {
  if (game.source === 'clock') return 'clock'
  if (game.result === '*') return game.finished ? 'unfinished' : 'ongoing'
  if (game.result === '1/2-1/2') return 'draw'
  const winner = game.result === '1-0' ? 'white' : 'black'
  const you = archivedPlayerSide(game)
  if (!you) return winner
  return you === winner ? 'win' : 'loss'
}

const ARCHIVED_BADGES: Record<ArchivedOutcome, ResultBadge> = {
  ...OUTCOME_BADGES,
  white: { label: 'White won', color: 'neutral', variant: 'subtle', side: 'white' },
  black: { label: 'Black won', color: 'neutral', variant: 'subtle', side: 'black' },
  unfinished: { label: 'Unfinished', color: 'neutral', variant: 'soft' },
  ongoing: { label: 'In progress', color: 'info', variant: 'soft' },
  clock: { label: 'Clock', color: 'neutral', variant: 'soft' },
}

export function archivedResultBadge(game: ArchivedGame): ResultBadge {
  return ARCHIVED_BADGES[archivedOutcome(game)]
}

/** “1 move”, “12 moves”: full moves, counting a lone white move as one. */
export function moveCount(plies: number): string {
  const moves = Math.ceil(plies / 2)
  return `${moves} ${moves === 1 ? 'move' : 'moves'}`
}
