import { expect, it, vi } from 'vitest'
import { ComputerGame, computerState, LocalGame, localState } from '../../src/domain/gameSession'
import { STANDARD_SETUP } from '../../src/domain/variant'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}
it('plain Node-compatible state plays, takes back and resigns a computer game', async () => {
  const state = computerState()
  const game = new ComputerGame(state, {
    bestMove: async () => 'e7e5',
    stopEngine: async () => {},
    ready: () => true,
    allowed: () => true,
    now: () => 0,
    failed: (cause) => {
      throw cause
    },
  })
  expect(game.move('e2e5')).toBe(false)
  expect(game.move('e2e4')).toBe(true)
  await game.computerTurn()
  expect(state.moves).toEqual(['e2e4', 'e7e5'])
  game.resign()
  expect(game.result?.kind).toBe('loss')
  game.takeback()
  expect(state.moves).toEqual([])
  expect(game.result).toBeNull()
  game.dispose()
})
it('cancels real work and discards a late reply when assistance is revoked', async () => {
  const reply = deferred<string>()
  let allowed = true
  const stop = vi.fn(async () => {})
  const state = computerState()
  const game = new ComputerGame(state, {
    bestMove: () => reply.promise,
    stopEngine: stop,
    ready: () => true,
    allowed: () => allowed,
    now: () => 0,
    failed: (cause) => {
      throw cause
    },
  })
  game.move('e2e4')
  const turn = game.computerTurn()
  allowed = false
  game.availabilityChanged()
  expect(stop).toHaveBeenCalledOnce()
  reply.resolve('e7e5')
  await turn
  expect(state.moves).toEqual(['e2e4'])
  expect(state.thinking).toBe(false)
  expect(game.move('g1f3')).toBe(false)
})
it('replaces a pending engine turn without accepting the previous game reply', async () => {
  const reply = deferred<string>()
  const state = computerState()
  const game = new ComputerGame(state, {
    bestMove: () => reply.promise,
    stopEngine: async () => {},
    ready: () => true,
    allowed: () => true,
    now: () => 0,
    failed: (cause) => {
      throw cause
    },
  })
  game.move('e2e4')
  const turn = game.computerTurn()
  game.start()
  reply.resolve('e7e5')
  await turn
  expect(state.moves).toEqual([])
})
it('local games settle deadlines before adding increment and keep root-FEN repetition context', () => {
  let now = 0
  const state = localState()
  const game = new LocalGame(state, () => now)
  game.start(STANDARD_SETUP, {
    white: { minutes: 1, increment: 3 },
    black: { minutes: 1, increment: 3 },
  })
  expect(game.move('e2e4')).toBe('e4')
  state.times = { white: 60000, black: 50 }
  now = 50
  expect(game.move('e7e5')).toBeUndefined()
  expect(game.result).toMatchObject({ winner: 'white', reason: 'Time out' })
  game.start()
  for (const move of ['g1f3', 'g8f6', 'f3g1', 'f6g8', 'g1f3', 'g8f6', 'f3g1', 'f6g8'])
    expect(game.move(move)).toBeTruthy()
  expect(game.result?.reason).toMatch(/repetition/i)
})
it('snapshots cannot mutate the original session', () => {
  const state = computerState()
  const game = new ComputerGame(state, {
    bestMove: async () => 'e7e5',
    stopEngine: async () => {},
    ready: () => true,
    allowed: () => true,
    now: () => 0,
    failed: (cause) => {
      throw cause
    },
  })
  game.snapshot().moves.push('e2e4')
  expect(state.moves).toEqual([])
})
