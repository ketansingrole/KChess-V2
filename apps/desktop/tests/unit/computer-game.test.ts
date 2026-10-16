import { it, expect, vi } from 'vitest'
import { seedSaved } from './libraryBackend'
import { flushPromises } from '@vue/test-utils'
import { effectScope, nextTick, ref } from 'vue'
import { useComputerGame } from '../../app/stores/kchess/computerGame'
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
  seedSaved(
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
  seedSaved(
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

/** A fresh computer game (nothing restored) whose engine turn online play cancels. */
function freshGame(bestMove: () => Promise<string>) {
  desktop({ bestMove })
  const scope = effectScope()
  const allowed = ref(true)
  const game = scope.run(() =>
    useComputerGame({
      engineReady: ref(true),
      assistanceAllowed: () => allowed.value,
      fail: vi.fn(),
      recheckEngine: async () => {},
      notifyDesktop: vi.fn(),
    }),
  )!
  return { game, allowed, scope }
}

it('resumes a cancelled computer turn in a fresh game when assistance returns', async () => {
  const first = deferred<string>()
  const bestMove = vi
    .fn()
    .mockImplementationOnce(() => first.promise)
    .mockResolvedValue('e7e5')
  const { game, allowed, scope } = freshGame(bestMove)
  game.makeMove('e2e4')
  await nextTick()
  expect(bestMove).toHaveBeenCalledTimes(1)
  allowed.value = false
  await nextTick()
  first.resolve('c7c5')
  await flushPromises()
  allowed.value = true
  await flushPromises()
  expect(bestMove).toHaveBeenCalledTimes(2)
  expect(game.localMoves.value).toEqual(['e2e4', 'e7e5'])
  expect(game.localInteractive.value).toBe(true)
  scope.stop()
})

it('resumes the computer turn when assistance returns before the cancelled reply settles', async () => {
  const first = deferred<string>()
  const bestMove = vi
    .fn()
    .mockImplementationOnce(() => first.promise)
    .mockResolvedValue('e7e5')
  const { game, allowed, scope } = freshGame(bestMove)
  game.makeMove('e2e4')
  await nextTick()
  allowed.value = false
  await nextTick()
  allowed.value = true
  await nextTick()
  first.resolve('c7c5')
  await flushPromises()
  expect(bestMove).toHaveBeenCalledTimes(2)
  expect(game.localMoves.value).toEqual(['e2e4', 'e7e5'])
  scope.stop()
})
