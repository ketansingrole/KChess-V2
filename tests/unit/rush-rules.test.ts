import { describe, expect, it } from 'vitest'
import {
  accuracy,
  comboBonusMs,
  comboProgress,
  correctMove,
  finish,
  formatRunClock,
  mistake,
  newRush,
  puzzleSolved,
  RUSH_CHOICES,
  RUSH_CONFIGS,
  RUSH_MODES,
  skip,
  tick,
  type RushConfig,
  type RushState,
} from '@kchess/rules/rush'

const storm = (): RushConfig => RUSH_CONFIGS.storm as RushConfig
const streak = (): RushConfig => RUSH_CONFIGS.streak as RushConfig
const rush3 = (): RushConfig => RUSH_CONFIGS['3min'] as RushConfig

describe('rush configs', () => {
  it('offers the run kinds in picker order', () => {
    expect(RUSH_MODES).toEqual(['storm', 'streak', 'rush'])
    expect(RUSH_CHOICES).toEqual(['storm', 'streak', '3min', '5min', 'survival'])
    expect(Object.keys(RUSH_CONFIGS)).toEqual(RUSH_CHOICES)
  })

  it('describes each run kind', () => {
    expect(storm()).toMatchObject({
      mode: 'storm',
      variant: 'standard',
      durationMs: 180000,
      skips: 0,
      penaltyMs: 10000,
      combo: true,
    })
    expect(storm().strikes).toBeUndefined()
    expect(streak()).toMatchObject({
      mode: 'streak',
      variant: 'standard',
      strikes: 1,
      skips: 1,
      penaltyMs: 0,
      combo: false,
    })
    expect(streak().durationMs).toBeUndefined()
    expect(rush3()).toMatchObject({
      mode: 'rush',
      variant: '3min',
      durationMs: 180000,
      strikes: 3,
      skips: 0,
      penaltyMs: 0,
      combo: false,
    })
    expect(RUSH_CONFIGS['5min']).toMatchObject({ mode: 'rush', durationMs: 300000, strikes: 3 })
    expect(RUSH_CONFIGS.survival).toMatchObject({ mode: 'rush', variant: 'survival', strikes: 3 })
    expect(RUSH_CONFIGS.survival?.durationMs).toBeUndefined()
    for (const key of RUSH_CHOICES) {
      const config = RUSH_CONFIGS[key] as RushConfig
      expect(config.title.length).toBeGreaterThan(0)
      expect(config.rules.length).toBeGreaterThan(0)
      expect(config.ladder.from).toBeLessThan(config.ladder.to)
    }
  })
})

describe('newRush', () => {
  it('starts every run kind scoreless', () => {
    expect(newRush(storm())).toEqual({
      score: 0,
      moves: 0,
      mistakes: 0,
      combo: 0,
      maxCombo: 0,
      timeLeftMs: 180000,
      skipsLeft: 0,
      highest: 0,
      bonusMs: 0,
    })
    expect(newRush(streak())).toMatchObject({ score: 0, moves: 0, skipsLeft: 1 })
    expect(newRush(streak()).timeLeftMs).toBeUndefined()
    expect(newRush(rush3()).timeLeftMs).toBe(180000)
    expect(newRush(RUSH_CONFIGS['5min'] as RushConfig).timeLeftMs).toBe(300000)
  })
})

describe('combo helpers', () => {
  it('pays bonus time at combos 5, 12, 20, 30 and every 10 after', () => {
    expect(comboBonusMs(0)).toBe(0)
    expect(comboBonusMs(4)).toBe(0)
    expect(comboBonusMs(5)).toBe(3000)
    expect(comboBonusMs(6)).toBe(0)
    expect(comboBonusMs(11)).toBe(0)
    expect(comboBonusMs(12)).toBe(5000)
    expect(comboBonusMs(19)).toBe(0)
    expect(comboBonusMs(20)).toBe(7000)
    expect(comboBonusMs(29)).toBe(0)
    expect(comboBonusMs(30)).toBe(10000)
    expect(comboBonusMs(40)).toBe(10000)
    expect(comboBonusMs(50)).toBe(10000)
  })

  it('ignores non-finite combos instead of throwing', () => {
    for (const combo of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY])
      expect(comboBonusMs(combo)).toBe(0)
  })

  it('tracks progress toward the next bonus', () => {
    expect(comboProgress(0)).toBe(0)
    expect(comboProgress(1)).toBeCloseTo(0.2, 10)
    expect(comboProgress(2)).toBeCloseTo(0.4, 10)
    expect(comboProgress(4)).toBeCloseTo(0.8, 10)
    expect(comboProgress(5)).toBe(0)
    expect(comboProgress(6)).toBeCloseTo(1 / 7, 10)
    expect(comboProgress(12)).toBe(0)
    expect(comboProgress(30)).toBe(0)
    expect(comboProgress(40)).toBe(0)
  })

  it('passes non-finite combos through progress and the clock', () => {
    expect(String(comboProgress(Number.NaN))).toBe('NaN')
    expect(String(comboProgress(Number.POSITIVE_INFINITY))).toBe('NaN')
    expect(String(comboProgress(Number.NEGATIVE_INFINITY))).toBe('-Infinity')
    expect(formatRunClock(Number.NaN)).toBe('NaN:NaN')
    expect(formatRunClock(Number.POSITIVE_INFINITY)).toBe('Infinity:NaN')
    expect(formatRunClock(Number.NEGATIVE_INFINITY)).toBe('0:00')
  })
})

describe('formatRunClock', () => {
  it('formats m:ss, with tenths under ten seconds', () => {
    expect(formatRunClock(0)).toBe('0:00')
    expect(formatRunClock(0.4)).toBe('0:00.0')
    expect(formatRunClock(1)).toBe('0:00.0')
    expect(formatRunClock(9400)).toBe('0:09.4')
    expect(formatRunClock(9999)).toBe('0:09.9')
    expect(formatRunClock(10000)).toBe('0:10')
    expect(formatRunClock(59999)).toBe('0:59')
    expect(formatRunClock(60000)).toBe('1:00')
    expect(formatRunClock(183000)).toBe('3:03')
    expect(formatRunClock(3599999)).toBe('59:59')
    expect(formatRunClock(3600000)).toBe('60:00')
  })

  it('clamps negative clocks at zero', () => {
    expect(formatRunClock(-5)).toBe('0:00')
  })
})

describe('storm runs', () => {
  it('banks bonus time on a run of correct moves', () => {
    const state = newRush(storm())
    const bonuses = Array.from({ length: 6 }, () => correctMove(state, storm()))
    expect(bonuses).toEqual([0, 0, 0, 0, 3000, 0])
    expect(state).toMatchObject({
      moves: 6,
      mistakes: 0,
      combo: 6,
      maxCombo: 6,
      timeLeftMs: 183000,
      bonusMs: 3000,
    })
  })

  it('costs ten seconds and the combo on a mistake', () => {
    const state = newRush(storm())
    mistake(state, storm())
    expect(state).toMatchObject({
      moves: 0,
      mistakes: 1,
      combo: 0,
      maxCombo: 0,
      timeLeftMs: 170000,
    })
  })

  it('counts solved puzzles toward the score and highest rating', () => {
    const state = newRush(storm())
    correctMove(state, storm())
    mistake(state, storm())
    puzzleSolved(state, 1500)
    expect(state).toMatchObject({ score: 1, moves: 1, mistakes: 1, highest: 1500 })
    expect(accuracy(state)).toBe(50)
  })
})

describe('streak runs', () => {
  it('ends on the first strike and ignores later solves', () => {
    const state = newRush(streak())
    correctMove(state, streak())
    expect(state).toMatchObject({ moves: 1, combo: 1 })
    expect(state.over).toBeUndefined()
    mistake(state, streak())
    expect(state).toMatchObject({ moves: 1, mistakes: 1, combo: 0, over: 'strikes' })
    puzzleSolved(state, 1500)
    expect(state).toMatchObject({ score: 0, highest: 0, over: 'strikes' })
    expect(accuracy(state)).toBe(50)
  })

  it('spends its single skip, then reports none left', () => {
    const state = newRush(streak())
    expect(skip(state)).toBe(true)
    expect(state.skipsLeft).toBe(0)
    expect(skip(state)).toBe(false)
    expect(state.skipsLeft).toBe(0)
  })

  it('has no clock to tick', () => {
    const state = newRush(streak())
    tick(state, 60_000)
    expect(state.timeLeftMs).toBeUndefined()
    expect(state.over).toBeUndefined()
  })
})

describe('rush clock and finishing', () => {
  it('ticks the clock down and ends at zero', () => {
    const state = newRush(storm())
    tick(state, 5000)
    expect(state.timeLeftMs).toBe(175000)
    expect(state.over).toBeUndefined()
    tick(state, 200000)
    expect(state.timeLeftMs).toBe(0)
    expect(state.over).toBe('time')
  })

  it('finishes with any reason', () => {
    for (const reason of ['time', 'strikes', 'complete', 'ended'] as const) {
      const state = newRush(storm())
      finish(state, reason)
      expect(state.over).toBe(reason)
    }
  })

  it('rejects skips a run without any', () => {
    for (const key of ['storm', '3min', '5min', 'survival'] as const) {
      const state = newRush(RUSH_CONFIGS[key] as RushConfig)
      expect(skip(state)).toBe(false)
      expect(state.skipsLeft).toBe(0)
    }
  })
})

describe('accuracy', () => {
  it('is zero before any move and ignores solves without moves', () => {
    const fresh = newRush(storm())
    expect(accuracy(fresh)).toBe(0)
    puzzleSolved(fresh, 2000)
    expect(fresh).toMatchObject({ score: 1, highest: 2000 })
    expect(accuracy(fresh)).toBe(0)
  })

  it('counts correct moves against mistakes', () => {
    const state: RushState = newRush(rush3())
    correctMove(state, rush3())
    mistake(state, rush3())
    expect(accuracy(state)).toBe(50)
    expect(state).toMatchObject({ score: 0, moves: 1, mistakes: 1, highest: 0 })
    puzzleSolved(state, 1500)
    expect(state).toMatchObject({ score: 1, highest: 1500 })
  })
})
