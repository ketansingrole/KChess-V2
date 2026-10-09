import { expect, it, vi } from 'vitest'
import { seedSaved } from './libraryBackend'
import { nextTick } from 'vue'
import { flushPromises } from '@vue/test-utils'
import { useLocalGameStore } from '../../app/stores/local'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop, deferred } from './fixtures'

it('local board must reject a move after expiry, before the display timer runs', async () => {
  vi.useFakeTimers()
  let time = 0
  vi.spyOn(performance, 'now').mockImplementation(() => time)
  desktop()
  const store = useLocalGameStore()
  store.start(undefined, {
    white: { minutes: 1, increment: 3 },
    black: { minutes: 1, increment: 3 },
  })
  store.move('e2e4')
  await nextTick()
  store.times = { white: 60000, black: 50 }
  time = 60
  expect(store.clockText('black')).toBe('0:00.0')
  expect(store.move('e7e5')).toBe(false)
})

it('computer game must reject an expired human move before the display timer runs', async () => {
  vi.useFakeTimers()
  let time = 0
  vi.spyOn(performance, 'now').mockImplementation(() => time)
  seedSaved(
    'kchess:computer:v1',
    JSON.stringify({
      version: 2,
      moves: ['e2e4', 'e7e5'],
      ply: 2,
      level: 'club',
      color: 'white',
      clock: { minutes: 1, increment: 3 },
      times: { white: 50, black: 60000 },
    }),
  )
  desktop({ bestMove: () => new Promise(() => {}) })
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  time = 60
  expect(store.computerClockText('white')).toBe('0:00.0')
  store.makeMove('g1f3')
  expect(store.localMoves).toEqual(['e2e4', 'e7e5'])
})

it('standalone clock must flag expiry before accepting a clock press', async () => {
  vi.useFakeTimers()
  let time = 0
  vi.spyOn(performance, 'now').mockImplementation(() => time)
  desktop()
  const store = useLocalGameStore()
  store.otbConfig.minutes = 1
  store.otbConfig.bottomMinutes = 1
  store.otbConfig.increment = 3
  await nextTick()
  store.otbReset()
  store.otbPress('bottom')
  await nextTick()
  time = 60001
  expect(store.otbLeft('top')).toBe(0)
  store.otbPress('top')
  expect(store.otbFlagged).toBe('top')
})

it.each([0, 3])(
  'rejects an engine reply at expiry with %i seconds increment',
  async (increment) => {
    vi.useFakeTimers()
    let time = 0
    vi.spyOn(performance, 'now').mockImplementation(() => time)
    seedSaved(
      'kchess:computer:v1',
      JSON.stringify({
        version: 2,
        moves: ['e2e4', 'e7e5', 'g1f3'],
        ply: 3,
        level: 'club',
        color: 'white',
        clock: { minutes: 1, increment },
        times: { white: 60000, black: 50 },
      }),
    )
    const move = deferred<string>()
    const stop = vi.fn(async () => {})
    desktop({ bestMove: () => move.promise, stopEngine: stop })
    const store = useKChessStore()
    await store.init()
    await flushPromises()
    expect(store.thinking).toBe(true)
    time = 50
    move.resolve('b8c6')
    await flushPromises()
    expect(store.localMoves).toEqual(['e2e4', 'e7e5', 'g1f3'])
    expect(store.localResult).toMatchObject({ kind: 'win', detail: 'Stockfish ran out of time' })
    expect(store.thinking).toBe(false)
    expect(stop).toHaveBeenCalled()
  },
)

it.each([49, 50, 51])('checks local move deadline at %i ms with zero increment', async (at) => {
  vi.useFakeTimers()
  let time = 0
  vi.spyOn(performance, 'now').mockImplementation(() => time)
  desktop()
  const store = useLocalGameStore()
  store.start(undefined, {
    white: { minutes: 1, increment: 0 },
    black: { minutes: 1, increment: 0 },
  })
  store.move('e2e4')
  await nextTick()
  store.times = { white: 60000, black: 50 }
  time = at
  expect(store.move('e7e5')).toBe(at < 50)
  expect(store.moves).toHaveLength(at < 50 ? 2 : 1)
  if (at >= 50) expect(store.result).toMatchObject({ winner: 'white', reason: 'Time out' })
})

it('records an expired board clock when pause is pressed before a timer tick', async () => {
  vi.useFakeTimers()
  let time = 0
  vi.spyOn(performance, 'now').mockImplementation(() => time)
  desktop()
  const store = useLocalGameStore()
  store.start(undefined, {
    white: { minutes: 1, increment: 3 },
    black: { minutes: 1, increment: 3 },
  })
  store.move('e2e4')
  await nextTick()
  store.times = { white: 60000, black: 50 }
  time = 50
  store.togglePause()
  expect(store.result).toMatchObject({ winner: 'white', reason: 'Time out' })
})

it('preserves the draw result when the opponent of an expired player cannot mate', async () => {
  vi.useFakeTimers()
  let time = 0
  vi.spyOn(performance, 'now').mockImplementation(() => time)
  desktop()
  const store = useLocalGameStore()
  store.start(
    { variant: 'standard', fen: '7k/8/8/8/8/8/8/KR6 w - - 0 1' },
    { white: { minutes: 1, increment: 3 }, black: { minutes: 1, increment: 3 } },
  )
  store.move('b1b2')
  store.move('h8h7')
  await nextTick()
  store.times = { white: 50, black: 60000 }
  time = 50
  expect(store.move('b2b3')).toBe(false)
  expect(store.result).toEqual({ reason: 'Time out, but mate was impossible' })
})

it('records standalone clock expiry when paused before a timer tick', async () => {
  vi.useFakeTimers()
  let time = 0
  vi.spyOn(performance, 'now').mockImplementation(() => time)
  desktop()
  const store = useLocalGameStore()
  store.otbPress('bottom')
  await nextTick()
  time = 300001
  store.otbPause()
  expect(store.otbFlagged).toBe('top')
  expect(store.otbRunning).toBeNull()
})
