import { expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop } from './fixtures'
import { chess960Fen } from '../../src/shared/variant'
import type { OnlineEvent } from '../../src/shared/types'

const full = (overrides: Record<string, unknown> = {}, state: Record<string, unknown> = {}) =>
  ({
    type: 'gameFull',
    id: 'AbCd1234',
    rated: false,
    speed: 'rapid',
    variant: { key: 'standard' },
    initialFen: 'startpos',
    clock: { initial: 600_000, increment: 0 },
    white: { id: 'alice', name: 'Alice' },
    black: { id: 'bob', name: 'Bob', rating: 1800 },
    state: {
      type: 'gameState',
      moves: '',
      status: 'started',
      wtime: 600_000,
      btime: 600_000,
      ...state,
    },
    ...overrides,
  }) as unknown as OnlineEvent

async function setup() {
  const fixture = desktop({ onlineChat: async () => [{ user: 'bob', text: 'hi', room: 'player' }] })
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  store.onlineAccount = 'Alice'
  return { store, ...fixture }
}

it('shows the opponent’s draw and takeback offers, and the chat', async () => {
  const { store, emit } = await setup()
  emit(full())
  await flushPromises()
  expect(store.onlinePhase).toBe('playing')
  expect(store.chat).toEqual([{ user: 'bob', text: 'hi', room: 'player' }])
  emit({
    type: 'gameState',
    id: 'AbCd1234',
    moves: 'e2e4 e7e5',
    status: 'started',
    wtime: 590_000,
    btime: 590_000,
    bdraw: true,
    btakeback: true,
  } as unknown as OnlineEvent)
  expect(store.drawOffer).toBe('theirs')
  expect(store.takebackOffer).toBe('theirs')
  emit({
    type: 'chatLine',
    id: 'AbCd1234',
    username: 'bob',
    text: 'gl',
    room: 'player',
  } as unknown as OnlineEvent)
  expect(store.chat.at(-1)?.text).toBe('gl')
})

it('plays Chess960 games from their own start, castling king onto rook', async () => {
  const { store, emit } = await setup()
  const fen = 'r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1'
  emit(full({ variant: { key: 'chess960' }, initialFen: fen }, { moves: 'e1h1 e8a8' }))
  await flushPromises()
  expect(store.onlineSetup).toEqual({ variant: 'chess960', fen })
  expect(store.onlineHistory).toEqual(['O-O', 'O-O-O'])
  // Chessground is told to castle onto the rook.
  expect(store.onlineDests.get('e1')).toBeUndefined()
  expect(chess960Fen(518)).toContain('RNBQKBNR')
})

it('counts down to claiming the win when the opponent leaves', async () => {
  const { store, emit } = await setup()
  emit(full())
  emit({
    type: 'opponentGone',
    id: 'AbCd1234',
    gone: true,
    claimWinInSeconds: 0,
  } as unknown as OnlineEvent)
  expect(store.claimIn).toBe(0)
  emit({ type: 'opponentGone', id: 'AbCd1234', gone: false } as unknown as OnlineEvent)
  expect(store.claimIn).toBeNull()
})

it('flags an unsupported variant instead of showing a wrong board', async () => {
  const { store, emit } = await setup()
  emit(full({ variant: { key: 'crazyhouse' } }, { moves: 'e2e4 d7d5 e4d5 Q@e4' }))
  expect(store.onlineUnsupported).toBe('crazyhouse')
  expect(store.onlineCanPlay).toBe(false)
})

it('remembers the finished game for a rematch with colours swapped', async () => {
  const { store, emit, api } = await setup()
  const start = vi.fn(async () => ({ id: 'Next1234', url: 'https://lichess.org/Next1234' }))
  api.startOnline = start
  emit(full())
  emit({
    type: 'gameState',
    id: 'AbCd1234',
    moves: 'e2e4',
    status: 'resign',
    winner: 'white',
  } as unknown as OnlineEvent)
  expect(store.onlinePhase).toBe('finished')
  expect(store.lastGame).toMatchObject({ opponent: 'bob', color: 'white', minutes: 10 })
  await store.rematch()
  expect(start).toHaveBeenCalledWith(
    expect.objectContaining({ target: 'bob', color: 'black', minutes: 10 }),
  )
})

it('runs the computer game clock and loses on time', async () => {
  vi.useFakeTimers()
  try {
    desktop({ bestMove: () => new Promise(() => {}) })
    const store = useKChessStore()
    await store.init()
    await flushPromises()
    store.newGame(undefined, { minutes: 1, increment: 0 })
    expect(store.computerClockText('white')).toBe('1:00')
    store.makeMove('e2e4')
    expect(store.localMoves).toEqual(['e2e4'])
  } finally {
    vi.useRealTimers()
  }
})
