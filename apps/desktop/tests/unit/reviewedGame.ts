import { vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useReviewStore } from '../../app/stores/review'
import { useKChessStore } from '../../app/stores/kchess'
import { useMistakeStore } from '../../app/stores/mistakes'
import { reviewKey } from '@kchess/rules/review'
import { INITIAL_FEN } from '@kchess/rules/position'
import { desktop } from './fixtures'

export const moves = ['e2e4', 'e7e5', 'g1f3', 'f7f6', 'f3e5', 'f6e5', 'd1h5', 'g7g6']
export const key = reviewKey(INITIAL_FEN, moves)
/** Black's 4… g6 (the eighth move): after 4. Qh5+ only 4… Ke7 holds. */
export const BLUNDER_PLY = 7

/**
 * A completed review of `moves` on the analysis board in which only Black errs. The core keeps
 * practice positions; here adding Black's mistakes saves the one before 4… g6.
 */
export async function reviewedGame(origin?: { player: 'white' | 'black' }) {
  desktop({
    reviewGet: async () => null,
    onReviewUpdate: () => () => {},
    onAnalysis: () => () => {},
    startAnalysis: async () => 0,
    stopAnalysis: async () => {},
  })
  const app = useKChessStore()
  await app.init()
  const analysis = useAnalysisStore()
  analysis.engineOn = false
  for (const uci of moves) analysis.play(uci)
  analysis.goToPly(BLUNDER_PLY)
  const fen = analysis.node.fen
  analysis.goToPly(moves.length)
  if (origin) analysis.origin = origin
  const cps = [20, 25, 20, 120, 110, 90, 80, 70, 700]
  useReviewStore().reviews.set(key, {
    key,
    fen: INITIAL_FEN,
    moves,
    source: 'local',
    evals: cps.map((cp) => ({ cp, depth: 18 })),
    depth: 18,
    complete: true,
    updatedAt: 0,
  })
  const practice = useMistakeStore()
  practice.add = vi.fn(async (_review, color) => {
    if (color !== 'black') return 0
    const item = {
      id: `${key}:${BLUNDER_PLY}`,
      fen,
      solution: ['e8e7'],
      judgment: 'blunder',
      dueAt: 0,
      streak: 0,
      attempts: 0,
    }
    practice.items = [...practice.items, item]
    return 1
  })
  practice.answered = vi.fn(async () => {})
  await flushPromises()
  return { app, analysis, practice }
}
