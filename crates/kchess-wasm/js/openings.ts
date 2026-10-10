import { rules } from './engine.ts'
import type { GameSetup } from './variant.ts'

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
 * The Rust rules read the same table (`crates/kchess-domain/src/training/openings.rs`), so the
 * `openings` argument is not needed here; callers may still pass the loaded table.
 */
export function openingAt(
  _openings: Map<string, string>,
  setup: GameSetup,
  moves: readonly string[],
  ply = moves.length,
): OpeningName | undefined {
  // A non-finite `ply` travels as its text, which the rules read as the number it stands for.
  const wire = Number.isFinite(ply) ? ply : String(ply)
  return rules<OpeningName | null>('openingAt', setup, [...moves], wire) ?? undefined
}
