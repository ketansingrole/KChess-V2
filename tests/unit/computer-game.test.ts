import { it, expect } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop, deferred } from './fixtures'

it('rejects an old engine response after starting a new game', async () => {
  const pending = deferred<string>()
  desktop({ bestMove: () => pending.promise })
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  store.makeMove('e2e4')
  expect(store.thinking).toBe(true)
  store.newGame()
  pending.resolve('e7e5')
  await flushPromises()
  expect(store.localMoves).toEqual([])
  expect(store.thinking).toBe(false)
})

it('keeps computer-game state across navigation and invalidates engine replies on takeback', async () => {
  const pending = deferred<string>()
  desktop({ bestMove: () => pending.promise })
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  store.makeMove('e2e4')
  store.selectPage('history')
  expect(store.localMoves).toEqual(['e2e4'])
  store.takeback()
  pending.resolve('e7e5')
  await flushPromises()
  expect(store.localMoves).toEqual([])
})
