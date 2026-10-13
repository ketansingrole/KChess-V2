import { describe, expect, it } from 'vitest'
import { toRatingPoint, toRatingPoints } from '../../app/utils/ratingPoints'
import { RESTART_UPDATE_DESCRIPTION } from '../../app/utils/appUpdate'

describe('rating points', () => {
  it('maps a valid Lichess tuple to a chart point', () => {
    expect(toRatingPoint([2026, 0, 15, 1500])).toEqual({
      date: Date.UTC(2026, 0, 15),
      rating: 1500,
    })
  })

  it('drops short or non-finite tuples instead of throwing', () => {
    expect(toRatingPoint([2026, 0])).toBeNull()
    expect(toRatingPoint([2026, 0, 15, Number.NaN])).toBeNull()
    expect(toRatingPoint(['2026', 0, 15, 1500])).toBeNull()
    expect(toRatingPoints([[2026, 0, 15, 1500], [2026], [2026, 1, 2, 1600]])).toEqual([
      { date: Date.UTC(2026, 0, 15), rating: 1500 },
      { date: Date.UTC(2026, 1, 2), rating: 1600 },
    ])
    expect(toRatingPoints([])).toEqual([])
  })
})

describe('update copy', () => {
  it('keeps one restart warning for every surface', () => {
    expect(RESTART_UPDATE_DESCRIPTION).toContain('close and reopen')
    expect(RESTART_UPDATE_DESCRIPTION).toContain('clock will keep running')
  })
})
