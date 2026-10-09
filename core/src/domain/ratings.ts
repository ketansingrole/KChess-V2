import type { LichessGame, LichessRatingHistory } from '../contracts/types'
import { rules } from './engine.ts'

/**
 * Display label per rating-history key (`blitz` → `Blitz`, `puzzle` → `Puzzles`). The Rust rules
 * hold the same table (`records/ratings.rs`); the golden test keeps the two equal. It stays a
 * constant because a module-level call into the rules would run before a renderer has loaded them.
 */
export const RATING_DISPLAY_NAMES: Record<string, string> = {
  ultraBullet: 'UltraBullet',
  bullet: 'Bullet',
  blitz: 'Blitz',
  rapid: 'Rapid',
  classical: 'Classical',
  correspondence: 'Correspondence',
  chess960: 'Chess960',
  crazyhouse: 'Crazyhouse',
  antichess: 'Antichess',
  atomic: 'Atomic',
  horde: 'Horde',
  kingOfTheHill: 'King of the Hill',
  racingKings: 'Racing Kings',
  threeCheck: 'Three-check',
  puzzle: 'Puzzles',
}

/**
 * Canonical rating-history key. Accepts the perf keys Lichess sends since
 * `@lichess-org/types` 2.0.176 (`blitz`, `puzzle`) and the legacy display
 * names (`Blitz`, `Puzzles`, `King of the Hill`). Unknown names pass through.
 */
export function normalizeRatingKey(name: unknown): string {
  return rules<string>('normalizeRatingKey', name)
}

/** Human label for a rating-history entry (`blitz` and `Blitz` both → `Blitz`). */
export function ratingDisplayName(name: unknown): string {
  return rules<string>('ratingDisplayName', name)
}

/** Puzzle histories are charted elsewhere, under either `puzzle` or `Puzzles`. */
export function isPuzzleHistory(name: unknown): boolean {
  return rules<boolean>('isPuzzleHistory', name)
}

/**
 * Rebuild a rating history from synced games, for accounts where Lichess
 * returns none. A game's `playerRating` is the rating *before* it and
 * `ratingDiff` the change it caused, so the rating afterwards is their sum.
 * One point per UTC day per perf (the day's last game), in Lichess's own
 * `[year, month0, day, rating]` shape. Casual games do not move ratings.
 * Names are perf keys (`blitz`, not `Blitz`) to match the history endpoint.
 */
export function ratingHistoryFromGames(games: readonly LichessGame[]): LichessRatingHistory {
  return rules<LichessRatingHistory>('ratingHistoryFromGames', games)
}

/** Lichess's own history wins; games only fill perfs it has no points for. */
export function mergeRatingHistories(
  official: LichessRatingHistory,
  fromGames: LichessRatingHistory,
): LichessRatingHistory {
  return rules<LichessRatingHistory>('mergeRatingHistories', official, fromGames)
}
