import { expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { INITIAL_FEN } from 'chessops/fen'
import { desktop, deferred } from './fixtures'
import { useWatchStore } from '../../app/stores/watch'
import type { BroadcastTourDetail, WatchFrame, WatchState } from '../../src/shared/types'

function bridge() {
  let state: (s: WatchState) => void = () => {},
    frame: (f: WatchFrame) => void = () => {}
  const api = desktop({
    onWatchState: (fn) => {
      state = fn
      return vi.fn()
    },
    onWatch: (fn) => {
      frame = fn
      return vi.fn()
    },
    onBroadcast: () => vi.fn(),
    watch: async () => 1,
    stopWatching: async () => {},
  }).api
  return { api, state: (s: WatchState) => state(s), frame: (f: WatchFrame) => frame(f) }
}
const frame: WatchFrame = {
  session: 1,
  source: 'tv',
  gameId: 'Game0001',
  white: { name: 'A' },
  black: { name: 'B' },
  orientation: 'white',
  fen: INITIAL_FEN,
  lastMove: 'e7e5',
  whiteClock: 60,
  blackClock: 60,
  variant: 'standard',
  finished: false,
}
it('does not reopen a delayed tour after leaving or closing Watch', async () => {
  const b = bridge(),
    pending = deferred<BroadcastTourDetail>(),
    start = vi.fn(async () => 2)
  b.api.broadcastTour = () => pending.promise
  b.api.watchBroadcast = start
  const store = useWatchStore(),
    work = store.openTour('Tour0001')
  await flushPromises()
  await store.stop()
  store.unlisten()
  pending.resolve({
    rounds: [{ id: 'Round001', name: 'Round', ongoing: true, finished: false }],
  } as BroadcastTourDetail)
  await work
  expect(start).not.toHaveBeenCalled()
  expect(store.tour).toBeNull()
  expect(store.roundId).toBe('')
})
it('ignores an older tour response when a newer tour is selected', async () => {
  const b = bridge(),
    old = deferred<BroadcastTourDetail>(),
    recent = deferred<BroadcastTourDetail>()
  b.api.broadcastTour = vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(recent.promise)
  const start = vi.fn(async () => 2)
  b.api.watchBroadcast = start
  const store = useWatchStore(),
    first = store.openTour('Old00001')
  await flushPromises()
  const second = store.openTour('New00001')
  await flushPromises()
  recent.resolve({
    name: 'New',
    rounds: [{ id: 'Round002', ongoing: true }],
  } as BroadcastTourDetail)
  await second
  old.resolve({ name: 'Old', rounds: [{ id: 'Round001', ongoing: true }] } as BroadcastTourDetail)
  await first
  expect(store.tour?.name).toBe('New')
  expect(start).toHaveBeenCalledTimes(1)
  expect(start).toHaveBeenCalledWith('Round002')
})
it('freezes the last clock and accepts early lifecycle events before the IPC reply', async () => {
  const b = bridge(),
    reply = deferred<number>()
  b.api.watch = () => reply.promise
  const store = useWatchStore(),
    work = store.watch({ channel: 'rapid' })
  b.frame({ ...frame })
  b.state({ session: 1, phase: 'error', message: 'Feed stalled' })
  reply.resolve(1)
  await work
  expect(store.connection?.phase).toBe('error')
  expect(store.error).toBe('Feed stalled')
  const left = store.frame?.whiteClock
  b.state({ session: 0, phase: 'connected' })
  expect(store.connection?.phase).toBe('error')
  expect(store.frame?.whiteClock).toBe(left)
  await store.stop()
  b.frame(frame)
  expect(store.frame).toBeNull()
})
it('rejects late session acknowledgments after stop or a newer selection', async () => {
  const b = bridge(),
    old = deferred<number>(),
    recent = deferred<number>()
  b.api.watch = vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(recent.promise)
  const store = useWatchStore(),
    first = store.watch({ channel: 'blitz' }),
    second = store.watch({ channel: 'rapid' })
  recent.resolve(2)
  await second
  old.resolve(1)
  await first
  b.frame({ ...frame, session: 2, gameId: 'Game0002' })
  expect(store.frame?.gameId).toBe('Game0002')
})
