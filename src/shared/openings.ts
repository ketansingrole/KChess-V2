import { normalizeMove, type Position } from 'chessops/chess'
import { makeFen } from 'chessops/fen'
import { parseUci } from 'chessops/util'
import { setupStart, type GameSetup } from './variant.ts'

export interface OpeningName {
  eco: string
  name: string
}

/** Lichess's CC0 opening names (scripts/make-openings.mjs), keyed by EPD; loaded on first use. */
let table: Promise<Map<string, string>> | undefined
export function loadOpenings(): Promise<Map<string, string>> {
  table ??= import('./data/openings.json').then(
    (module) => new Map(Object.entries(module.default as Record<string, string>)),
  )
  return table
}

const epd = (pos: Position): string => makeFen(pos.toSetup()).split(' ').slice(0, 4).join(' ')

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
  const seen = [epd(pos)]
  for (const uci of moves.slice(0, ply)) {
    const parsed = parseUci(uci)
    if (!parsed) break
    const move = normalizeMove(pos, parsed)
    if (!pos.isLegal(move)) break
    pos.play(move)
    seen.push(epd(pos))
  }
  for (let i = seen.length - 1; i >= 0; i--) {
    const hit = openings.get(seen[i]!)
    if (!hit) continue
    const split = hit.indexOf('|')
    return { eco: hit.slice(0, split), name: hit.slice(split + 1) }
  }
  return undefined
}
