import { effectScope, ref } from 'vue'
import { flushPromises } from '@vue/test-utils'
import { expect, it, vi } from 'vitest'
import { usePositionEngine } from '../../app/composables/usePositionEngine'
import type { AnalysisRequest, AnalysisUpdate } from '@kchess/core/contracts/types'
import { analysisContext } from '@kchess/core/domain/analysisContext'
import { INITIAL_FEN } from 'chessops/fen'
import { desktop, deferred } from './fixtures'

it('cancels real searches on change/off and rejects late updates and acknowledgements', async () => {
  vi.useFakeTimers()
  let receive!: (value: AnalysisUpdate) => void
  const unsubscribe = vi.fn()
  const ack = deferred<number>()
  const startAnalysis = vi
    .fn()
    .mockImplementationOnce(() => ack.promise)
    .mockResolvedValue(2)
  const stopAnalysis = vi.fn(async () => {})
  desktop({
    startAnalysis,
    stopAnalysis,
    onAnalysis: (callback) => {
      receive = callback
      return unsubscribe
    },
  })
  const request = ref<AnalysisRequest | null>({
    fen: INITIAL_FEN,
    rootFen: INITIAL_FEN,
    moves: [],
    lines: 3,
  })
  const scope = effectScope()
  const engine = scope.run(() => usePositionEngine(() => request.value))!
  await vi.advanceTimersByTimeAsync(100)
  const first = startAnalysis.mock.calls[0]![0] as AnalysisRequest
  const value: AnalysisUpdate = {
    id: 1,
    fen: INITIAL_FEN,
    context: analysisContext(INITIAL_FEN),
    clientId: first.clientId,
    depth: 12,
    lines: [{ rank: 1, depth: 12, cp: 20, pv: ['e2e4'] }],
    done: false,
  }
  receive(value)
  expect(engine.update.value?.depth).toBe(12)
  receive({ ...value, done: true, error: 'Engine failed' })
  expect(engine.update.value).toBeNull()
  expect(engine.error.value).toBe('Engine failed')
  expect(engine.busy.value).toBe(false)
  request.value = {
    ...request.value!,
    moves: ['e2e4'],
    fen: 'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1',
  }
  expect(engine.update.value).toBeNull()
  expect(stopAnalysis).toHaveBeenCalledTimes(2)
  await vi.advanceTimersByTimeAsync(100)
  receive(value)
  expect(engine.update.value).toBeNull()
  ack.resolve(1)
  await flushPromises()
  request.value = null
  expect(engine.busy.value).toBe(false)
  expect(stopAnalysis).toHaveBeenCalledTimes(3)
  receive({ ...value, clientId: startAnalysis.mock.calls[1]![0].clientId })
  expect(engine.update.value).toBeNull()
  scope.stop()
  expect(unsubscribe).toHaveBeenCalledOnce()
  expect(stopAnalysis).toHaveBeenCalledTimes(4)
})

it('does not start a queued search after leaving the board', async () => {
  vi.useFakeTimers()
  const startAnalysis = vi.fn(async () => 1)
  desktop({ startAnalysis, stopAnalysis: async () => {}, onAnalysis: () => () => {} })
  const scope = effectScope()
  scope.run(() => usePositionEngine(() => ({ fen: INITIAL_FEN, moves: [], lines: 1 })))
  scope.stop()
  await vi.advanceTimersByTimeAsync(200)
  expect(startAnalysis).not.toHaveBeenCalled()
})
