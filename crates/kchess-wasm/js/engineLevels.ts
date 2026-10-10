import type { EngineLevel } from '@kchess/contracts/types'
import { rules } from './engine.ts'
import training from './data/training.json' with { type: 'json' }

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

/** Weakest to strongest (shared data, `data/training.json`). */
export const ENGINE_LADDER = training.engineLadder as readonly EngineLevelInfo[]

/** The levels offered until the player picks their own in Settings. */
export const DEFAULT_ENGINE_LEVELS = training.defaultEngineLevels as readonly EngineLevel[]

export const engineLevelInfo = (id: EngineLevel): EngineLevelInfo =>
  (rules<EngineLevelInfo | null>('engineLevelInfo', id) ?? undefined) as EngineLevelInfo

/** "Grandmaster · ~2550", "Stockfish Max · Full strength". */
export const engineLevelLabel = (id: EngineLevel): string => rules<string>('engineLevelLabel', id)

/** Keep only known levels, in ladder order; never empty. */
export function normalizeEngineLevels(levels: readonly string[]): EngineLevel[] {
  return rules<EngineLevel[]>('normalizeEngineLevels', [...levels])
}

/** The enabled level closest in strength to `wanted`, e.g. after the player turns `wanted` off. */
export function nearestEngineLevel(
  wanted: EngineLevel,
  enabled: readonly EngineLevel[],
): EngineLevel {
  return rules<EngineLevel>('nearestEngineLevel', wanted, [...enabled])
}
