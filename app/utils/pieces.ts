import type { PieceAnimation } from '../../src/shared/types'

/**
 * Lichess piece sets whose licenses are compatible with this project's GPL-3.0-or-later
 * (see `app/assets/ATTRIBUTION.md`). Each folder in `app/assets/pieces` holds `wK.svg` … `bP.svg`.
 */
export const pieceSets = [
  { id: 'cburnett', name: 'Cburnett' },
  { id: 'merida', name: 'Merida' },
  { id: 'chessnut', name: 'Chessnut' },
  { id: 'pirouetti', name: 'Pirouetti' },
  { id: 'letter', name: 'Letter' },
  { id: 'mpchess', name: 'MPChess' },
  { id: 'fantasy', name: 'Fantasy' },
  { id: 'spatial', name: 'Spatial' },
  { id: 'celtic', name: 'Celtic' },
  { id: 'rhosgfx', name: 'RhosGFX' },
  { id: 'pixel', name: 'Pixel' },
] as const

export type PieceSet = (typeof pieceSets)[number]['id']
export const DEFAULT_PIECE_SET: PieceSet = 'cburnett'

const urls = import.meta.glob<string>('../assets/pieces/*/*.svg', {
  eager: true,
  query: '?url',
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
export const animationMs: Record<PieceAnimation, number> = {
  none: 0,
  fast: 100,
  normal: 180,
  slow: 350,
}
