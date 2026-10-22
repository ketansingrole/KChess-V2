import type { PieceAnimation } from '@kchess/contracts/types'
import looks from '../../kchess-domain/data/looks.json' with { type: 'json' }

/**
 * How boards and pieces look, shared by every frontend (`data/looks.json`): the Lichess 2D board
 * themes with their coordinate colors, and the piece sets whose licenses suit this project
 * (see `apps/desktop/app/assets/ATTRIBUTION.md`).
 */
export interface BoardLook {
  id: string
  name: string
  /** The image in `apps/desktop/app/assets/boards`. */
  image: string
  /** Coordinate labels on dark squares, and on light ones. */
  coordWhite: string
  coordBlack: string
}

export const BOARD_LOOKS = looks.boards as readonly BoardLook[]
export const PIECE_SETS = looks.pieceSets as readonly { id: string; name: string }[]
export const DEFAULT_PIECE_SET = looks.defaultPieceSet
export const DEFAULT_BOARD = looks.defaultBoard
/** Slide time in milliseconds for each animation setting. */
export const ANIMATION_MS = looks.animationMs as Record<PieceAnimation, number>
