import { describe, expect, it } from 'vitest'
import { analyseReview, replay, reviewKey } from '@kchess/rules/review'
import type { StoredReview } from '@kchess/contracts/types'

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
