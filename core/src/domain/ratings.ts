import type { LichessGame, LichessRatingHistory } from '../contracts/types'

const DAY_MS = 86_400_000

type RatingName = NonNullable<LichessRatingHistory[number]['name']>

/** Display label per rating-history key (`blitz` → `Blitz`, `puzzle` → `Puzzles`). */
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

const KEY_BY_NORMALIZED: Record<string, RatingName> = {
  ultrabullet: 'ultraBullet',
  bullet: 'bullet',
  blitz: 'blitz',
  rapid: 'rapid',
  classical: 'classical',
  correspondence: 'correspondence',
  chess960: 'chess960',
  crazyhouse: 'crazyhouse',
  antichess: 'antichess',
  atomic: 'atomic',
  horde: 'horde',
  kingofthehill: 'kingOfTheHill',
  racingkings: 'racingKings',
  threecheck: 'threeCheck',
  puzzle: 'puzzle',
  puzzles: 'puzzle',
}

/**
 * Canonical rating-history key. Accepts the perf keys Lichess sends since
 * `@lichess-org/types` 2.0.176 (`blitz`, `puzzle`) and the legacy display
 * names (`Blitz`, `Puzzles`, `King of the Hill`). Unknown names pass through.
 */
export function normalizeRatingKey(name: unknown): string {
  if (typeof name !== 'string' || !name) return ''
  const key = name.toLowerCase().replace(/[\s_-]/g, '')
  return KEY_BY_NORMALIZED[key] ?? name
}

/** Human label for a rating-history entry (`blitz` and `Blitz` both → `Blitz`). */
export function ratingDisplayName(name: unknown): string {
  if (typeof name !== 'string' || !name) return ''
  return RATING_DISPLAY_NAMES[normalizeRatingKey(name)] ?? name
}

/** Puzzle histories are charted elsewhere, under either `puzzle` or `Puzzles`. */
export function isPuzzleHistory(name: unknown): boolean {
  return normalizeRatingKey(name) === 'puzzle'
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
  const byPerf = new Map<string, Map<number, number>>()
  for (const game of [...games].sort((a, b) => a.createdAt - b.createdAt)) {
    if (!game.rated || game.playerRating === undefined || game.ratingDiff === undefined) continue
    const key = normalizeRatingKey(game.perf) || game.perf
    const day = Math.floor(game.createdAt / DAY_MS) * DAY_MS
    const days = byPerf.get(key) ?? new Map<number, number>()
    days.set(day, game.playerRating + game.ratingDiff) // later games overwrite earlier ones that day
    byPerf.set(key, days)
  }
  return [...byPerf].map(([perf, days]) => ({
    name: perf as RatingName,
    points: [...days].map(([day, rating]) => {
      const date = new Date(day)
      return [date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate(), rating]
    }),
  }))
}

/** Lichess's own history wins; games only fill perfs it has no points for. */
export function mergeRatingHistories(
  official: LichessRatingHistory,
  fromGames: LichessRatingHistory,
): LichessRatingHistory {
  const covered = new Set(
    official.filter((entry) => entry.points?.length).map((e) => normalizeRatingKey(e.name)),
  )
  return [...official, ...fromGames.filter((entry) => !covered.has(normalizeRatingKey(entry.name)))]
}
