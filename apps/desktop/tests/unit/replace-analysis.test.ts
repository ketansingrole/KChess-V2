import { expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
import { flushPromises } from '@vue/test-utils'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useKChessStore } from '../../app/stores/kchess'
import { useReplaceAnalysis } from '../../app/composables/useReplaceAnalysis'
import { useTvPosition } from '../../app/composables/useTvPosition'
import { desktop, deferred } from './fixtures'
import type { TvPastGame } from '@kchess/contracts/types'

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

const pastGame = (gameId: string) =>
  ({
    gameId,
    orientation: 'white',
    white: { name: 'Alice' },
    black: { name: 'Bob' },
  }) as TvPastGame

// Audit 60a61e8 #2: the replacement question was asked before the download, so a late export
// replaced analysis entered while it was on its way.
it('asks again when the board was edited while a TV game downloaded', async () => {
  const gate = deferred<string>()
  desktop({ exportGame: () => gate.promise, reviewGet: async () => null })
  const scope = effectScope()
  const tv = scope.run(() => useTvPosition())!
  const analysis = useAnalysisStore()
  const store = useKChessStore()
  tv.openPast(pastGame('abcdefgh'))
  expect(store.confirmation).toBeNull()
  expect(analysis.loadPgn('1. d4 d5 2. c4 *')).toBe(true)
  gate.resolve('1. e4 e5 *')
  await flushPromises()
  expect(analysis.mainline).toEqual(['d2d4', 'd7d5', 'c2c4'])
  expect(store.confirmation).toMatchObject({ title: 'Replace the unsaved analysis?' })
  store.confirmation?.run()
  expect(analysis.mainline).toEqual(['e2e4', 'e7e5'])
  scope.stop()
})

it('keeps the newer TV game when two exports finish in reverse order', async () => {
  const replies = new Map([
    ['first', deferred<string>()],
    ['second', deferred<string>()],
  ])
  desktop({ exportGame: (id: string) => replies.get(id)!.promise, reviewGet: async () => null })
  const scope = effectScope()
  const tv = scope.run(() => useTvPosition())!
  const analysis = useAnalysisStore()
  tv.openPast(pastGame('first'))
  tv.openPast(pastGame('second'))
  replies.get('second')!.resolve('1. d4 *')
  await flushPromises()
  replies.get('first')!.resolve('1. e4 *')
  await flushPromises()
  expect(analysis.mainline).toEqual(['d2d4'])
  expect(analysis.origin?.gameId).toBe('second')
  scope.stop()
})

it('drops a TV download once another document replaces the board or the page closes', async () => {
  const gate = deferred<string>()
  desktop({ exportGame: () => gate.promise, reviewGet: async () => null })
  const scope = effectScope()
  const tv = scope.run(() => useTvPosition())!
  const analysis = useAnalysisStore()
  const replace = useReplaceAnalysis()
  tv.openPast(pastGame('abcdefgh'))
  replace(() => void analysis.loadPgn('1. c4 *'))
  gate.resolve('1. e4 *')
  await flushPromises()
  expect(analysis.mainline).toEqual(['c2c4'])

  const later = deferred<string>()
  window.kchess.exportGame = () => later.promise
  analysis.load()
  tv.openPast(pastGame('ijklmnop'))
  scope.stop()
  later.resolve('1. e4 *')
  await flushPromises()
  expect(analysis.mainline).toEqual([])
})
