import { rules } from './engine.ts'

/** What a study's card shows: its final position and how much is in it. */
export interface StudySummary {
  fen: string
  /** Half-moves in the main line. */
  plies: number
  comments: number
  variations: number
  players: string
  event: string
}

const cache = new Map<string, StudySummary | null>()

/**
 * Summarise a study's PGN (remembered, since the library page shows every study at once). The
 * summary is worked out by the Rust rules (`crates/kchess-domain/src/records/studies.rs`).
 */
export function summarizeStudy(pgn: string): StudySummary | null {
  const known = cache.get(pgn)
  if (known !== undefined) return known
  const summary = rules<StudySummary | null>('summarizeStudy', pgn)
  if (cache.size > 200) cache.clear()
  cache.set(pgn, summary)
  return summary
}
