/** Shared Lichess rating-history point mapping: one shape for every rating view. */

export interface RatingPoint {
  date: number
  rating: number
}

/**
 * A raw Lichess point is `[year, month, day, rating]` (month is 0-based for
 * `Date.UTC`). Returns null when the tuple is short or holds non-finite
 * values, so charts skip corrupt entries instead of throwing.
 */
export function toRatingPoint(point: readonly unknown[]): RatingPoint | null {
  const [year, month, day, rating] = point
  if (
    typeof year !== 'number' ||
    typeof month !== 'number' ||
    typeof day !== 'number' ||
    typeof rating !== 'number' ||
    !Number.isFinite(year) ||
    !Number.isFinite(month) ||
    !Number.isFinite(day) ||
    !Number.isFinite(rating)
  )
    return null
  const date = Date.UTC(year, month, day)
  if (!Number.isFinite(date)) return null
  return { date, rating }
}

/** Map raw points to chart points, dropping corrupt entries. */
export function toRatingPoints(points: readonly (readonly unknown[])[]): RatingPoint[] {
  const out: RatingPoint[] = []
  for (const point of points) {
    if (point.length < 4) continue
    const mapped = toRatingPoint(point)
    if (mapped) out.push(mapped)
  }
  return out
}
