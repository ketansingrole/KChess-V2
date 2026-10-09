import { repetitionKey } from './position.ts'
import { setupStart, type GameSetup } from './variant.ts'

export interface OpeningName {
  eco: string
  name: string
}

/** Lichess's CC0 opening names (tooling/make-openings.mjs), keyed by EPD; loaded on first use. */
let table: Promise<Map<string, string>> | undefined
export function loadOpenings(): Promise<Map<string, string>> {
  table ??= import('./data/openings.json').then(
    (module) => new Map(Object.entries(module.default as Record<string, string>)),
  )
  return table
}

/**
 * The name of the opening reached after `ply` moves. As Lichess suggests, step back from the
 * position until a named one is found. Only games under standard rules have opening names.
 */
export function openingAt(
  openings: Map<string, string>,
  setup: GameSetup,
  moves: readonly string[],
  ply = moves.length,
): OpeningName | undefined {
  if (setup.variant !== 'standard') return undefined
  const pos = setupStart(setup)
  if (!pos) return undefined
  // EPD: placement, side, castling and en passant.
  const seen = [pos.fen, ...pos.line(moves.slice(0, ply)).map((move) => move.fen)].map(
    repetitionKey,
  )
  for (let i = seen.length - 1; i >= 0; i--) {
    const hit = openings.get(seen[i]!)
    if (!hit) continue
    const split = hit.indexOf('|')
    return { eco: hit.slice(0, split), name: hit.slice(split + 1) }
  }
  return undefined
}
