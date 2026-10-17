import { expect, it, vi } from 'vitest'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useKChessStore } from '../../app/stores/kchess'
import { useReplaceAnalysis } from '../../app/composables/useReplaceAnalysis'
import { desktop } from './fixtures'

// "Review game" and the editor's "Analyse this position" replaced an unsaved analysis board
// without the question opening a study asks.
it('asks before replacing moves on the board that no study keeps', () => {
  desktop()
  const analysis = useAnalysisStore()
  const store = useKChessStore()
  const replace = useReplaceAnalysis()
  const load = vi.fn()

  replace(load)
  expect(load).toHaveBeenCalledTimes(1)
  expect(store.confirmation).toBeNull()

  expect(analysis.loadPgn('1. e4 e5 *')).toBe(true)
  replace(load)
  expect(load).toHaveBeenCalledTimes(1)
  expect(store.confirmation).toMatchObject({
    title: 'Replace the unsaved analysis?',
    label: 'Replace',
  })
  store.confirmation?.run()
  expect(load).toHaveBeenCalledTimes(2)
})
