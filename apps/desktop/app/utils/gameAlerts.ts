import type { NotificationKind } from '@kchess/contracts/types'

/** The page that shows the game each alert is about; challenges and tests concern no open board. */
const GAME_PAGE: Partial<Record<NotificationKind, string>> = {
  opponentMove: 'online',
  lowTime: 'online',
  gameEvents: 'online',
  computerMove: 'computer',
}

/**
 * A game alert repeats what the board already shows while the player is looking at it, so it
 * is dropped then. Elsewhere in the app, or with KChess in the background, it still goes out.
 */
export function redundantGameAlert(
  kind: NotificationKind,
  page: string,
  watching: boolean,
): boolean {
  return watching && GAME_PAGE[kind] === page
}
