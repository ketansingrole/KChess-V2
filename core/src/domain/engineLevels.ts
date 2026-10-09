import type { EngineLevel } from '../contracts/types.ts'

export interface EngineLevelInfo {
  id: EngineLevel
  /** A chess title where one exists at this strength (CM 2200, FM 2300, IM 2400, GM 2500), a plain description below that. */
  label: string
  /** Approximate playing strength. Stockfish's own scale, not a FIDE or Lichess rating. */
  elo: number | null
  /** Stockfish `UCI_Elo`; it cannot go below 1320. */
  uciElo?: number
  /** Stockfish `Skill Level` (0–20), for levels below `UCI_Elo`'s floor. */
  skill?: number
  /** Chance of a random legal move instead of a searched one, to play below Skill Level 0. */
  randomMove?: number
  /** Default think time in milliseconds. */
  time: number
}

/** Weakest to strongest. */
export const ENGINE_LADDER: readonly EngineLevelInfo[] = [
  { id: 'beginner', label: 'Beginner', elo: 800, skill: 0, randomMove: 0.4, time: 300 },
  { id: 'novice', label: 'Novice', elo: 1100, skill: 0, randomMove: 0.15, time: 500 },
  { id: 'casual', label: 'Casual', elo: 1350, uciElo: 1350, time: 700 },
  { id: 'club', label: 'Club Player', elo: 1600, uciElo: 1600, time: 1000 },
  { id: 'strong-club', label: 'Strong Club Player', elo: 1800, uciElo: 1800, time: 1400 },
  { id: 'expert', label: 'Expert', elo: 2000, uciElo: 2000, time: 1600 },
  { id: 'cm', label: 'Candidate Master', elo: 2200, uciElo: 2200, time: 1800 },
  { id: 'fm', label: 'FIDE Master', elo: 2300, uciElo: 2300, time: 2000 },
  { id: 'im', label: 'International Master', elo: 2400, uciElo: 2400, time: 2000 },
  { id: 'gm', label: 'Grandmaster', elo: 2550, uciElo: 2550, time: 2200 },
  { id: 'super-gm', label: 'Super Grandmaster', elo: 2750, uciElo: 2750, time: 2500 },
  { id: 'max', label: 'Stockfish Max', elo: null, time: 2500 },
]

/** The levels offered until the player picks their own in Settings. */
export const DEFAULT_ENGINE_LEVELS: readonly EngineLevel[] = [
  'beginner',
  'club',
  'expert',
  'fm',
  'im',
  'gm',
  'max',
]

export const engineLevelInfo = (id: EngineLevel): EngineLevelInfo =>
  ENGINE_LADDER.find((entry) => entry.id === id)!

/** "Grandmaster · ~2550", "Stockfish Max · Full strength". */
export const engineLevelLabel = (id: EngineLevel): string => {
  const { label, elo } = engineLevelInfo(id)
  return `${label} · ${elo ? `~${elo}` : 'Full strength'}`
}

/** Keep only known levels, in ladder order; never empty. */
export function normalizeEngineLevels(levels: readonly string[]): EngineLevel[] {
  const kept = ENGINE_LADDER.filter((entry) => levels.includes(entry.id)).map((entry) => entry.id)
  return kept.length ? kept : [...DEFAULT_ENGINE_LEVELS]
}

/** The enabled level closest in strength to `wanted`, e.g. after the player turns `wanted` off. */
export function nearestEngineLevel(
  wanted: EngineLevel,
  enabled: readonly EngineLevel[],
): EngineLevel {
  if (enabled.includes(wanted) || !enabled.length) return wanted
  const rank = (id: EngineLevel): number => ENGINE_LADDER.findIndex((entry) => entry.id === id)
  const target = rank(wanted)
  return [...enabled].sort((a, b) => Math.abs(rank(a) - target) - Math.abs(rank(b) - target))[0]!
}
