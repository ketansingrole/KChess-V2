import { describe, expect, it, vi } from 'vitest'
import type { components } from '@lichess-org/types'
import { analyseReview, reviewKey } from '../../src/shared/review'

vi.mock('../../src/core/store', () => ({}))
vi.mock('../../src/core/reviewStore', () => ({ writeReview: vi.fn(), markChecked: vi.fn() }))
const { reviewFromLichess } = await import('../../src/core/lichess')

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
type GameJson = components['schemas']['GameJson']

const player = (name: string, accuracy: number) => ({
  user: { id: name, name },
  rating: 1500,
  analysis: { inaccuracy: 0, mistake: 0, blunder: accuracy < 50 ? 1 : 0, acpl: 10, accuracy },
})
/** 1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6?? 4. Qxf7#, as Lichess exports it with its analysis. */
const game = (fields: Partial<GameJson> = {}): GameJson =>
  ({
    id: 'abcdefgh',
    rated: true,
    variant: 'standard',
    speed: 'blitz',
    perf: 'blitz',
    createdAt: 0,
    lastMoveAt: 0,
    status: 'mate',
    players: { white: player('alice', 95), black: player('bob', 40) },
    moves: 'e4 e5 Qh5 Nc6 Bc4 Nf6 Qxf7#',
    analysis: [
      { eval: 30 },
      { eval: 40 },
      { eval: 20 },
      { eval: 30 },
      { eval: 50 },
      {
        mate: 1,
        best: 'g7g6',
        variation: 'g6 Qf3 Nf6',
        judgment: { name: 'Blunder', comment: 'Checkmate is now unavoidable. g6 was best.' },
      },
    ],
    ...fields,
  }) as GameJson

const MOVES = ['e2e4', 'e7e5', 'd1h5', 'b8c6', 'f1c4', 'g8f6', 'h5f7']

describe('Lichess analysis as a review', () => {
  it('maps scores, the better move and the labels onto the positions', () => {
    const review = reviewFromLichess(game())!
    expect(review.key).toBe(reviewKey(START, MOVES))
    expect(review.moves).toEqual(MOVES)
    expect(review).toMatchObject({ source: 'lichess', complete: true, gameId: 'abcdefgh' })
    // Lichess counts the start as +0.15; entry i scores the position after move i+1.
    expect(review.evals[0]).toEqual({ cp: 15 })
    expect(review.evals[1]).toEqual({ cp: 30 })
    expect(review.evals[6]).toEqual({ mate: 1 })
    // The better move belongs to the position before the blunder.
    expect(review.evals[5]).toMatchObject({ cp: 50, best: 'g7g6', pv: ['g7g6', 'h5f3', 'g8f6'] })
    // The mating move has no entry: the position speaks for itself.
    expect(review.evals[7]).toBeNull()
    expect(review.judgments).toEqual([null, null, null, null, null, 'blunder', null])
    expect(review.accuracy).toEqual({ white: 95, black: 40 })
  })

  it('keeps Lichess’s labels and accuracy when summarized', () => {
    const result = analyseReview(reviewFromLichess(game())!)
    expect(result.black).toMatchObject({ accuracy: 40, blunder: 1 })
    expect(result.white).toMatchObject({ accuracy: 95, blunder: 0 })
    expect(result.moves.at(-1)?.chances).toBe(1)
  })

  it('skips games Lichess has not analysed, and variants', () => {
    expect(reviewFromLichess(game({ analysis: undefined }))).toBeUndefined()
    expect(reviewFromLichess(game({ variant: 'chess960' }))).toBeUndefined()
  })
})
