import { describe, expect, it } from 'vitest'
import {
  analyseReview,
  judge,
  moveAccuracy,
  replay,
  reviewKey,
  winChances,
} from '../../src/shared/review'
import type { StoredReview } from '../../src/shared/types'

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const FOOLS_MATE = ['f2f3', 'e7e5', 'g2g4', 'd8h4']

const review = (fields: Partial<StoredReview>): StoredReview => ({
  key: reviewKey(START, FOOLS_MATE),
  fen: START,
  moves: FOOLS_MATE,
  source: 'local',
  evals: [],
  depth: 16,
  complete: true,
  updatedAt: 0,
  ...fields,
})

describe('review keys', () => {
  it('are stable for the same game and differ for another', () => {
    expect(reviewKey(START, FOOLS_MATE)).toBe(reviewKey(START, [...FOOLS_MATE]))
    expect(reviewKey(START, FOOLS_MATE)).toMatch(/^[0-9a-f]{16}$/)
    expect(reviewKey(START, FOOLS_MATE.slice(0, 3))).not.toBe(reviewKey(START, FOOLS_MATE))
  })
})

describe('replaying a game', () => {
  it('lists each position and marks the end', () => {
    const positions = replay(START, FOOLS_MATE)
    expect(positions).toHaveLength(5)
    expect(positions.map((p) => p.turn)).toEqual(['white', 'black', 'white', 'black', 'white'])
    expect(positions.at(-1)?.end).toBe('checkmate')
  })
  it('stops at the first illegal move', () => {
    expect(replay(START, ['e2e4', 'e2e4'])).toHaveLength(2)
  })
})

describe('winning chances', () => {
  it('follow Lichess’s curve and treat checkmate as lost for the side to move', () => {
    expect(winChances({ cp: 0 }, 'white')).toBe(0)
    expect(winChances({ cp: 5000 }, 'white')).toBeCloseTo(winChances({ cp: 1000 }, 'white'))
    expect(winChances({ mate: 3 }, 'black')).toBeGreaterThan(0.99)
    expect(winChances({ mate: 0 }, 'white')).toBe(-1)
    expect(winChances({ mate: 0 }, 'black')).toBe(1)
  })
})

describe('judging a move', () => {
  it('labels by the drop in the mover’s winning chances', () => {
    expect(judge({ cp: 0 }, { cp: -20 }, 'white')).toBeUndefined()
    expect(judge({ cp: 0 }, { cp: -60 }, 'white')).toBe('inaccuracy')
    expect(judge({ cp: 0 }, { cp: -120 }, 'white')).toBe('mistake')
    expect(judge({ cp: 0 }, { cp: -300 }, 'white')).toBe('blunder')
    // The same for Black, whose losses are White's gains.
    expect(judge({ cp: 0 }, { cp: 60 }, 'black')).toBe('inaccuracy')
    expect(judge({ cp: 0 }, { cp: -300 }, 'black')).toBeUndefined()
  })
  it('hardly minds a lost position getting more lost', () => {
    expect(judge({ cp: -900 }, { cp: -1100 }, 'white')).toBeUndefined()
  })
  it('judges walking into mate by how good the position still was', () => {
    expect(judge({ cp: 50 }, { mate: -3 }, 'white')).toBe('blunder')
    expect(judge({ cp: -800 }, { mate: -3 }, 'white')).toBe('mistake')
    expect(judge({ cp: -1200 }, { mate: -3 }, 'white')).toBe('inaccuracy')
    expect(judge({ cp: -50 }, { mate: 3 }, 'black')).toBe('blunder')
  })
  it('judges missing a forced mate by what is left', () => {
    expect(judge({ mate: 2 }, { cp: 1500 }, 'white')).toBe('inaccuracy')
    expect(judge({ mate: 2 }, { cp: 800 }, 'white')).toBe('mistake')
    expect(judge({ mate: 2 }, { cp: 300 }, 'white')).toBe('blunder')
    expect(judge({ mate: 2 }, { mate: -4 }, 'white')).toBe('blunder')
    // A slower mate is not a mistake.
    expect(judge({ mate: 2 }, { mate: 4 }, 'white')).toBeUndefined()
  })
  it('never judges the mating move', () => {
    expect(judge({ mate: -1 }, { mate: 0 }, 'black')).toBeUndefined()
  })
})

describe('move accuracy', () => {
  it('is full for a move that keeps the winning chances, and falls with the loss', () => {
    expect(moveAccuracy(50, 50)).toBe(100)
    expect(moveAccuracy(50, 60)).toBe(100)
    expect(moveAccuracy(50, 30)).toBeCloseTo(41.0, 0)
    expect(moveAccuracy(100, 0)).toBe(0)
  })
})

describe('reviewing a game', () => {
  it('labels, counts and scores each side', () => {
    const result = analyseReview(
      review({ evals: [{ cp: 20 }, { cp: -50 }, { cp: -40 }, { mate: -1 }, null] }),
    )
    expect(result.moves.map((m) => m.judgment)).toEqual([
      'inaccuracy',
      undefined,
      'blunder',
      undefined,
    ])
    expect(result.white).toMatchObject({ inaccuracy: 1, mistake: 0, blunder: 1 })
    expect(result.black).toMatchObject({ inaccuracy: 0, mistake: 0, blunder: 0 })
    expect(result.black.accuracy).toBeGreaterThan(result.white.accuracy!)
    expect(result.white.acpl).toBeGreaterThan(result.black.acpl!)
    // The final position is checkmate even though nothing scored it.
    expect(result.moves.at(-1)?.chances).toBe(-1)
  })
  it('scores what it can while the analysis is still running', () => {
    const result = analyseReview(
      review({ evals: [{ cp: 20 }, { cp: -50 }, null, null, null], complete: false }),
    )
    expect(result.moves[0]?.judgment).toBe('inaccuracy')
    expect(result.moves[2]?.judgment).toBeUndefined()
    // Game accuracy needs every position.
    expect(result.white.accuracy).toBeUndefined()
  })
  it('keeps Lichess’s own labels and accuracy', () => {
    const result = analyseReview(
      review({
        source: 'lichess',
        depth: 0,
        evals: [{ cp: 15 }, { cp: -50 }, { cp: -40 }, { mate: -1 }, null],
        judgments: [null, null, 'blunder', null],
        accuracy: { white: 12, black: 97 },
      }),
    )
    expect(result.moves.map((m) => m.judgment)).toEqual([
      undefined,
      undefined,
      'blunder',
      undefined,
    ])
    expect(result.white.accuracy).toBe(12)
    expect(result.black.accuracy).toBe(97)
  })
})
