import { expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { INITIAL_FEN } from 'chessops/fen'
import { analysisContext } from '../../../../core/src/domain/analysisContext'
import type { AnalysisRequest, AnalysisUpdate } from '../../../../core/src/contracts/types'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop } from './fixtures'

// Regression: a completed search must survive a later interrupted update for the
// same position (equal depth), so the board reuses it instead of searching again.
// Reproducer: complete depth 20, force a second search, interrupt it at depth 20.
// Replay: deterministic, no random seed or generated path.
it('keeps a completed evaluation when an equal-depth interrupted update arrives', async () => {
  let receive: (update: AnalysisUpdate) => void = () => {}
  let nextId = 0
  const startAnalysis = vi.fn(async (_request: AnalysisRequest) => ++nextId)
  desktop({
    startAnalysis,
    stopAnalysis: async () => {},
    reviewGet: async () => null,
    onAnalysis: (fn) => {
      receive = fn
      return vi.fn()
    },
  })
  const app = useKChessStore()
  await app.init()
  const analysis = useAnalysisStore()
  analysis.attach()
  await flushPromises()
  expect(startAnalysis).toHaveBeenCalledTimes(1)
  const first = startAnalysis.mock.calls[0]![0] as AnalysisRequest
  const context = analysisContext(INITIAL_FEN, [])
  const lines = (cp: number) =>
    Array.from({ length: 3 }, (_, i) => ({ rank: i + 1, depth: 20, cp, pv: ['e2e4'] }))
  receive({
    id: 1,
    clientId: first.clientId,
    fen: INITIAL_FEN,
    context,
    depth: 20,
    lines: lines(10),
    done: true,
    reason: 'completed',
  })
  expect(analysis.evaluation?.reason).toBe('completed')
  // More lines than cached forces a second search on the same position.
  analysis.engineLines = 5
  await flushPromises()
  expect(startAnalysis).toHaveBeenCalledTimes(2)
  const second = startAnalysis.mock.calls[1]![0] as AnalysisRequest
  receive({
    id: 2,
    clientId: second.clientId,
    fen: INITIAL_FEN,
    context,
    depth: 20,
    lines: lines(99),
    done: true,
    reason: 'interrupted',
  })
  // Back to the cached shape: the completed result must be reused, not searched again.
  analysis.engineLines = 3
  await flushPromises()
  expect(startAnalysis).toHaveBeenCalledTimes(2)
  analysis.detach()
})

// Failed searches must not become cached evaluations.
it('does not cache failed searches as evaluations', async () => {
  let receive: (update: AnalysisUpdate) => void = () => {}
  const startAnalysis = vi.fn(async (_request: AnalysisRequest) => 1)
  desktop({
    startAnalysis,
    stopAnalysis: async () => {},
    reviewGet: async () => null,
    onAnalysis: (fn) => {
      receive = fn
      return vi.fn()
    },
  })
  const app = useKChessStore()
  await app.init()
  const analysis = useAnalysisStore()
  analysis.attach()
  await flushPromises()
  const first = startAnalysis.mock.calls[0]![0] as AnalysisRequest
  const context = analysisContext(INITIAL_FEN, [])
  receive({
    id: 1,
    clientId: first.clientId,
    fen: INITIAL_FEN,
    context,
    depth: 4,
    lines: [{ rank: 1, depth: 4, cp: 50, pv: ['e2e4'] }],
    done: true,
    reason: 'failed',
    error: 'Stockfish failed.',
  })
  expect(analysis.engineError).toBe('Stockfish failed.')
  // Step away and back: no cached failure may stand in as the position's evaluation.
  expect(analysis.play('e2e4')).toBe(true)
  await flushPromises()
  analysis.back()
  await flushPromises()
  expect(analysis.evaluation).toBeNull()
  analysis.detach()
})
