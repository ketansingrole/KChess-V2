import { describe, expect, it } from 'vitest'
import {
  canBoardSeek,
  canDirectChallenge,
  canPlayOnline,
  perfFor,
  totalSeconds,
} from '@kchess/rules/timeControl'

/** Lichess Board API time controls: estimated length is initial time + 40 × increment. */
describe('totalSeconds', () => {
  it('adds the initial time to forty times the increment', () => {
    expect(totalSeconds(3, 2)).toBe(260)
    expect(totalSeconds(1, 0)).toBe(60)
    expect(totalSeconds(0.5, 0)).toBe(30)
    expect(totalSeconds(0, 0)).toBe(0)
    expect(totalSeconds(5, 4.5)).toBe(480)
  })
})

/** Lichess perfs: UltraBullet < 30 s, Bullet < 180 s, Blitz < 480 s, Rapid < 1500 s. */
describe('perfFor', () => {
  it('names the perf of common controls', () => {
    expect(perfFor(0.25, 0)).toBe('UltraBullet')
    expect(perfFor(1, 0)).toBe('Bullet')
    expect(perfFor(3, 2)).toBe('Blitz')
    expect(perfFor(10, 0)).toBe('Rapid')
    expect(perfFor(15, 10)).toBe('Rapid')
    expect(perfFor(30, 0)).toBe('Classical')
  })

  it('changes perf exactly at the cutoffs', () => {
    expect(perfFor(0, 0.7)).toBe('UltraBullet')
    expect(perfFor(0, 0.75)).toBe('Bullet')
    expect(perfFor(0, 4)).toBe('Bullet')
    expect(perfFor(0, 4.5)).toBe('Blitz')
    expect(perfFor(5, 4)).toBe('Blitz')
    expect(perfFor(5, 4.5)).toBe('Rapid')
    expect(perfFor(20, 5)).toBe('Rapid')
    expect(perfFor(25, 0)).toBe('Classical')
  })
})

describe('online play', () => {
  it('lets public matchmaking seek Rapid and slower only', () => {
    expect(canBoardSeek(10, 0)).toBe(true)
    expect(canBoardSeek(5, 4.5)).toBe(true)
    expect(canBoardSeek(5, 4)).toBe(false)
    expect(canBoardSeek(5, 0)).toBe(false)
    expect(canBoardSeek(1, 0)).toBe(false)
  })

  it('lets direct challenges play Blitz and slower', () => {
    expect(canDirectChallenge(3, 2)).toBe(true)
    expect(canDirectChallenge(3, 0)).toBe(true)
    expect(canDirectChallenge(0, 4.5)).toBe(true)
    expect(canDirectChallenge(0, 4)).toBe(false)
    expect(canDirectChallenge(1, 0)).toBe(false)
    expect(canDirectChallenge(0.5, 0)).toBe(false)
  })

  it('picks the cutoff by whether the game is targeted', () => {
    expect(canPlayOnline(3, 2, true)).toBe(true)
    expect(canPlayOnline(0, 4.5, true)).toBe(true)
    expect(canPlayOnline(1, 0, true)).toBe(false)
    expect(canPlayOnline(10, 0, false)).toBe(true)
    expect(canPlayOnline(5, 4.5, false)).toBe(true)
    expect(canPlayOnline(3, 2, false)).toBe(false)
    expect(canPlayOnline(1, 0, false)).toBe(false)
  })
})
