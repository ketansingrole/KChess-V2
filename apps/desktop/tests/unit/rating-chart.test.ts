import { describe, expect, it } from 'vitest'
import {
  RATING_DAY_MS,
  formatRatingDate,
  ratingYDomain,
  spansRatingYears,
} from '../../app/utils/ratingChart'

describe('rating chart scales', () => {
  it('pads the y-domain and falls back when empty', () => {
    expect(ratingYDomain([1500, 1600])).toEqual([1485, 1615])
    expect(ratingYDomain([])).toEqual([0, 1])
  })

  it('uses month/year only for long spans', () => {
    expect(spansRatingYears(0, 30 * RATING_DAY_MS)).toBe(false)
    expect(spansRatingYears(0, 401 * RATING_DAY_MS)).toBe(true)
  })

  it('formats the same date every chart shows', () => {
    const stamp = Date.UTC(2026, 0, 15)
    expect(formatRatingDate(stamp, true)).toContain('2026')
    expect(formatRatingDate(stamp)).not.toContain('2026')
  })
})
