import type { PieceAnimation } from '@kchess/contracts/types'
import {
  ANIMATION_MS,
  DEFAULT_PIECE_SET as SHARED_DEFAULT_PIECE_SET,
  PIECE_SETS,
} from '@kchess/rules/looks'

/**
 * Lichess piece sets whose licenses are compatible with this project's GPL-3.0-or-later
 * (see `apps/desktop/app/assets/ATTRIBUTION.md`). Each folder in `apps/desktop/app/assets/pieces` holds `wK.svg` … `bP.svg`.
 */
export const pieceSets = PIECE_SETS

export type PieceSet = string
export const DEFAULT_PIECE_SET: PieceSet = SHARED_DEFAULT_PIECE_SET

const urls = import.meta.glob<string>('../assets/pieces/*/*.svg', {
  eager: true,
  // Keep all 132 piece images out of the JavaScript bundle; load only displayed sets.
  query: '?url&no-inline',
  import: 'default',
})
const ROLES = ['P', 'N', 'B', 'R', 'Q', 'K'] as const
const COLORS = ['w', 'b'] as const

/**
 * CSS custom properties (`--piece-wK`, …) that point at one set's images. Put them on any element
 * and `main.css` paints the pieces beneath it; an unknown id falls back to the default set.
 */
export function pieceVars(id: string): Record<string, string> {
  const set = pieceSets.some((s) => s.id === id) ? id : DEFAULT_PIECE_SET
  const vars: Record<string, string> = {}
  for (const color of COLORS)
    for (const role of ROLES)
      vars[`--piece-${color}${role}`] =
        `url("${urls[`../assets/pieces/${set}/${color}${role}.svg`]}")`
  return vars
}

/** Slide time in milliseconds for each animation setting. */
export const animationMs: Record<PieceAnimation, number> = ANIMATION_MS

/** The image of one piece of a set (`color` w/b, `role` P N B R Q K), for drawing on a canvas. */
export function pieceUrl(id: string, color: 'w' | 'b', role: (typeof ROLES)[number]): string {
  const set = pieceSets.some((s) => s.id === id) ? id : DEFAULT_PIECE_SET
  const url = urls[`../assets/pieces/${set}/${color}${role}.svg`]
  if (!url) throw new Error(`Unknown piece image: ${set} ${color}${role}.`)
  return url
}
