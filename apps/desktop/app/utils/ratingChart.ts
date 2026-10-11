/** Shared rating-chart scales: one date format and one padded domain for every rating view. */

export const RATING_DAY_MS = 86_400_000
const RATING_PAD = 15
const SPAN_FOR_YEARS_MS = 400 * RATING_DAY_MS

/** True when the span reads better as month/year than month/day. */
export function spansRatingYears(first: number, last: number): boolean {
  return last - first > SPAN_FOR_YEARS_MS
}

export function formatRatingDate(timestamp: number, long = false, shortYears = false): string {
  return new Date(timestamp).toLocaleDateString(
    undefined,
    long
      ? { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' }
      : shortYears
        ? { month: 'short', year: '2-digit', timeZone: 'UTC' }
        : { month: 'short', day: 'numeric', timeZone: 'UTC' },
  )
}

/** Padded y-domain for ratings, or [0, 1] when there is nothing to chart. */
export function ratingYDomain(values: readonly number[]): [number, number] {
  if (!values.length) return [0, 1]
  return [Math.min(...values) - RATING_PAD, Math.max(...values) + RATING_PAD]
}
