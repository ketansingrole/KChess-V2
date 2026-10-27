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

it('asks Lichess for the open online game again after the core restarts', async () => {
  let reconnected = () => {}
  const resumeOnline = vi.fn(async () => ({ account: 'me', id: 'game1' }))
  desktop({
    resumeOnline: resumeOnline as never,
    onCoreReconnected: (callback: () => void) => {
      reconnected = callback
      return () => {}
    },
  })
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  expect(resumeOnline).toHaveBeenCalledOnce()
  expect(store.onlinePhase).toBe('disconnected')
  reconnected()
  expect(store.onlineConnection?.phase).toBe('checking')
  await flushPromises()
  expect(resumeOnline).toHaveBeenCalledTimes(2)
  expect(store.onlineId).toBe('game1')
})

it('verifies online play again after a restart even when no game was open', async () => {
  let reconnected = () => {}
  const resumeOnline = vi.fn(async () => null)
  desktop({
    resumeOnline,
    onCoreReconnected: (callback: () => void) => {
      reconnected = callback
      return () => {}
    },
  })
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  // A fresh core holds back the engine until it knows no game is live.
  reconnected()
  await flushPromises()
  expect(resumeOnline).toHaveBeenCalledTimes(2)
  expect(store.onlinePhase).toBe('idle')
})

it('keeps a finished game on the board when a restart finds no game in progress', async () => {
  let reconnected = () => {}
  const resumeOnline = vi.fn(async () => null)
  desktop({
    resumeOnline,
    onCoreReconnected: (callback: () => void) => {
      reconnected = callback
      return () => {}
    },
  })
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  store.onlinePhase = 'finished'
  reconnected()
  await flushPromises()
  expect(resumeOnline).toHaveBeenCalledTimes(2)
  expect(store.onlinePhase).toBe('finished')
})

it('reads settings again when a read from before this window’s save arrives after it', async () => {
  const { api } = desktop()
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  const before = store.settings!.soundVolume
  const stale = await api.loadData()
  const read = deferred<typeof stale>()
  const loadData = vi
    .spyOn(api, 'loadData')
    .mockImplementationOnce(() => read.promise)
    .mockImplementation(async () => ({
      ...stale,
      settings: { ...stale.settings, soundVolume: store.settings!.soundVolume },
    }))
  const reloading = store.reloadData()
  store.settings!.soundVolume = before === 0.8 ? 0.2 : 0.8
  const edited = store.settings!.soundVolume
  await store.save()
  read.resolve(stale)
  await reloading
  await flushPromises()
  expect(loadData).toHaveBeenCalledTimes(2)
  expect(store.settings!.soundVolume).toBe(edited)
})
