import { describe, expect, it } from 'vitest'
import type { LichessGame, LichessRatingHistory } from '../../src/shared/types'
import {
  isPuzzleHistory,
  mergeRatingHistories,
  normalizeRatingKey,
  ratingDisplayName,
  ratingHistoryFromGames,
} from '../../app/utils/ratings'

function game(overrides: Partial<LichessGame> = {}): LichessGame {
  return {
    id: 'Abcd0001',
    account: 'Alice',
    createdAt: Date.UTC(2026, 9, 1),
    lastMoveAt: Date.UTC(2026, 9, 1),
    rated: true,
    speed: 'blitz',
    perf: 'blitz',
    status: 'mate',
    color: 'white',
    winner: 'white',
    opponent: 'Bob',
    playerRating: 1500,
    ratingDiff: 8,
    moves: 'e4 e5',
    ...overrides,
  }
}

describe('rating history keys (@lichess-org/types 2.0.176)', () => {
  it('normalizes perf keys and legacy display names to one key', () => {
    expect(normalizeRatingKey('blitz')).toBe('blitz')
    expect(normalizeRatingKey('Blitz')).toBe('blitz')
    expect(normalizeRatingKey('ultraBullet')).toBe('ultraBullet')
    expect(normalizeRatingKey('UltraBullet')).toBe('ultraBullet')
    expect(normalizeRatingKey('kingOfTheHill')).toBe('kingOfTheHill')
    expect(normalizeRatingKey('King of the Hill')).toBe('kingOfTheHill')
    expect(normalizeRatingKey('puzzle')).toBe('puzzle')
    expect(normalizeRatingKey('Puzzles')).toBe('puzzle')
  })

  it('labels keys and legacy names for display', () => {
    expect(ratingDisplayName('blitz')).toBe('Blitz')
    expect(ratingDisplayName('Blitz')).toBe('Blitz')
    expect(ratingDisplayName('puzzle')).toBe('Puzzles')
    expect(ratingDisplayName('Puzzles')).toBe('Puzzles')
    expect(ratingDisplayName('kingOfTheHill')).toBe('King of the Hill')
  })

  it('treats puzzle histories alike under either spelling', () => {
    expect(isPuzzleHistory('puzzle')).toBe(true)
    expect(isPuzzleHistory('Puzzles')).toBe(true)
    expect(isPuzzleHistory('blitz')).toBe(false)
  })

  it('rebuilds game histories with perf keys, not display names', () => {
    const history = ratingHistoryFromGames([game()])
    expect(history).toHaveLength(1)
    expect(history[0]?.name).toBe('blitz')
  })

  it('dedupes official and rebuilt histories across key/display spellings', () => {
    const official = [{ name: 'blitz', points: [[2026, 9, 1, 1508]] }] as LichessRatingHistory
    // 'Blitz' is the legacy display name, not a perf key in @lichess-org/types
    // 2.0.176, so it goes through unknown.
    const fromGames = [
      { name: 'Blitz', points: [[2026, 9, 2, 1516]] },
    ] as unknown as LichessRatingHistory
    expect(mergeRatingHistories(official, fromGames)).toEqual(official)
  })
})
