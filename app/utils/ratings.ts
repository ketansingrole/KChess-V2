import type { LichessGame, LichessRatingHistory } from '../../src/shared/types'

const DAY_MS = 86_400_000

/** Lichess's display name for a perf key, e.g. `ultraBullet` → `UltraBullet`. */
function perfName(perf: string): string {
  return perf.charAt(0).toUpperCase() + perf.slice(1)
}

/**
 * Rebuild a rating history from synced games, for accounts where Lichess
 * returns none. A game's `playerRating` is the rating *before* it and
 * `ratingDiff` the change it caused, so the rating afterwards is their sum.
 * One point per UTC day per perf (the day's last game), in Lichess's own
 * `[year, month0, day, rating]` shape. Casual games do not move ratings.
 */
export function ratingHistoryFromGames(games: readonly LichessGame[]): LichessRatingHistory {
  const byPerf = new Map<string, Map<number, number>>()
  for (const game of [...games].sort((a, b) => a.createdAt - b.createdAt)) {
    if (!game.rated || game.playerRating === undefined || game.ratingDiff === undefined) continue
    const day = Math.floor(game.createdAt / DAY_MS) * DAY_MS
    const days = byPerf.get(game.perf) ?? new Map<number, number>()
    days.set(day, game.playerRating + game.ratingDiff) // later games overwrite earlier ones that day
    byPerf.set(game.perf, days)
  }
  return [...byPerf].map(([perf, days]) => ({
    name: perfName(perf),
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
  const covered = new Set(official.filter((entry) => entry.points?.length).map((e) => e.name))
  return [...official, ...fromGames.filter((entry) => !covered.has(entry.name))]
}
