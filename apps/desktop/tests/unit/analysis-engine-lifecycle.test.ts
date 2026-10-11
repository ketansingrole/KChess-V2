import { expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import type { AnalysisRequest } from '@kchess/contracts/types'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop } from './fixtures'

it('releases the engine when analysis is turned off or detached, then reuses saved settings', async () => {
  const startAnalysis = vi.fn(async (_request: AnalysisRequest) => startAnalysis.mock.calls.length)
  const stopAnalysis = vi.fn(async (_kill?: boolean) => {})
  desktop({ startAnalysis, stopAnalysis, onAnalysis: () => () => {} })

  const app = useKChessStore()
  await app.init()
  const analysis = useAnalysisStore()
  analysis.engineOn = true
  analysis.engineLines = 2
  analysis.infinite = true
  analysis.attach()
  await flushPromises()

  expect(startAnalysis).toHaveBeenCalledTimes(1)
  expect(startAnalysis.mock.calls[0]![0]).toMatchObject({ lines: 2, infinite: true })

  analysis.engineOn = false
  await flushPromises()
  expect(stopAnalysis).toHaveBeenLastCalledWith(true)

  analysis.engineOn = true
  await flushPromises()
  expect(startAnalysis).toHaveBeenCalledTimes(2)
  expect(startAnalysis.mock.calls[1]![0]).toMatchObject({ lines: 2, infinite: true })

  analysis.detach()
  expect(stopAnalysis).toHaveBeenLastCalledWith(true)
})
