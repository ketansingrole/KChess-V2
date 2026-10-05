import { it, expect, vi } from 'vitest'
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

it('waits to resume a restored computer turn until startup recovery confirms no live game', async () => {
  const recovery = deferred<null>()
  localStorage.setItem(
    'kchess:computer:v1',
    JSON.stringify({ version: 1, moves: ['e2e4'], ply: 1, level: 'club', color: 'white' }),
  )
  let searches = 0
  desktop({
    resumeOnline: () => recovery.promise,
    bestMove: async () => {
      searches++
      return 'e7e5'
    },
  })
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  expect(searches).toBe(0)
  expect(store.localCanPlay).toBe(false)
  recovery.resolve(null)
  await flushPromises()
  expect(searches).toBe(1)
  expect(store.localMoves).toEqual(['e2e4', 'e7e5'])
})

it('pauses a restored computer clock while startup recovery is unverified', async () => {
  vi.useFakeTimers()
  const recovery = deferred<null>()
  localStorage.setItem(
    'kchess:computer:v1',
    JSON.stringify({
      version: 2,
      moves: ['e2e4', 'e7e5'],
      ply: 2,
      level: 'club',
      color: 'white',
      clock: { minutes: 1, increment: 0 },
      times: { white: 1000, black: 1000 },
    }),
  )
  desktop({ resumeOnline: () => recovery.promise })
  const store = useKChessStore()
  await store.init()
  await vi.advanceTimersByTimeAsync(5000)
  expect(store.localResult).toBeNull()
  expect(store.computerClockText('white')).toBe('0:01.0')
  recovery.resolve(null)
  await vi.advanceTimersByTimeAsync(0)
  await vi.advanceTimersByTimeAsync(1200)
  expect(store.localResult?.detail).toBe('You ran out of time')
})
