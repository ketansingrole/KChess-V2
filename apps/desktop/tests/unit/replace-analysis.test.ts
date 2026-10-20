import { expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
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

it('asks before a past TV game replaces the unsaved analysis', async () => {
  // TV, the players page and the explorer loaded their games without asking.
  const exportGame = vi.fn(async () => '1. d4 d5 *')
  desktop({ exportGame })
  const { useTvPosition } = await import('../../app/composables/useTvPosition')
  const scope = effectScope()
  const tv = scope.run(() => useTvPosition())!
  const analysis = useAnalysisStore()
  const store = useKChessStore()
  expect(analysis.loadPgn('1. e4 e5 *')).toBe(true)
  tv.openPast({
    gameId: 'Abcdefgh',
    orientation: 'white',
    white: { name: 'Alice' },
    black: { name: 'Bob' },
  } as Parameters<typeof tv.openPast>[0])
  expect(exportGame).not.toHaveBeenCalled()
  expect(store.confirmation).toMatchObject({ title: 'Replace the unsaved analysis?' })
  store.confirmation?.run()
  await vi.waitFor(() => expect(exportGame).toHaveBeenCalledWith('Abcdefgh'))
  scope.stop()
})
