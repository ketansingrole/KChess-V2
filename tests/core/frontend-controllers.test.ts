import { expect, it, vi } from 'vitest'
import { OnlineGame, onlineGameState } from '@kchess/rules/onlineGame'
import {
  PuzzleSession,
  puzzleSessionState,
  type PuzzleSelection,
} from '@kchess/rules/puzzleSession'
import type { OnlineEvent, Puzzle, PuzzleSolveResult } from '@kchess/contracts/types'
import { deferred } from '../fixtures/deferred'

function online(api = {}) {
  let time = 0
  const state = onlineGameState()
  state.onlineAccount = 'Alice'
  const failed = vi.fn()
  const game = new OnlineGame(state, {
    api: {
      onlineChat: async () => [],
      startOnline: async () => ({}),
      cancelOnline: async () => {},
      resumeOnline: async () => null,
      openGame: async () => {},
      playOnline: async () => {},
      onlineAction: async () => {},
      ...api,
    },
    activeAccount: () => 'Alice',
    chatEnabled: () => true,
    now: () => time,
    failed,
  })
  const full = (id = 'AbCd1234', moves = '') =>
    ({
      type: 'gameFull',
      id,
      rated: false,
      initialFen: 'startpos',
      variant: { key: 'standard' },
      white: { id: 'alice' },
      black: { id: 'bob' },
      clock: { initial: 60000, increment: 1000 },
      state: {
        type: 'gameState',
        moves,
        status: 'started',
        wtime: 60000,
        btime: 60000,
        bdraw: true,
      },
    }) as unknown as OnlineEvent
  return {
    state,
    game,
    failed,
    full,
    tick: (ms: number) => {
      time += ms
    },
  }
}
it('plain state handles authoritative moves, clocks, offers, account recovery and stale connections', () => {
  const { state, game, full, tick } = online()
  game.readOnlineEvent(full('AbCd1234', 'e2e4'))
  expect(state.drawOffer).toBe('theirs')
  tick(2000)
  expect(game.clock.remaining('black')).toBe(58000)
  game.readOnlineState({
    session: 2,
    lane: 'game',
    account: 'Alice',
    gameId: 'AbCd1234',
    phase: 'reconnecting',
  })
  game.readOnlineState({
    session: 1,
    lane: 'game',
    account: 'Other',
    gameId: 'Other123',
    phase: 'idle',
  })
  expect(state.onlineAccount).toBe('Alice')
  expect(state.onlinePhase).toBe('disconnected')
  tick(5000)
  expect(game.clock.remaining('black')).toBe(58000)
  game.readOnlineEvent(full('AbCd1234', 'e2e4 e7e5'))
  expect(game.history).toEqual(['e4', 'e5'])
  game.readOnlineEvent({
    type: 'gameFinish',
    game: { gameId: 'AbCd1234', winner: 'white', status: { name: 'resign' } },
  } as OnlineEvent)
  expect(state.lastGame).toMatchObject({ account: 'Alice', opponent: 'bob', color: 'white' })
  expect(state.onlinePhase).toBe('finished')
})
it('does not replay an ambiguous move mutation', async () => {
  const playOnline = vi.fn(async () => {
    throw new Error('lost response')
  })
  const { state, game, failed, full } = online({ playOnline })
  game.readOnlineEvent(full())
  await game.onlineMove('e2e4')
  await game.onlineMove('e2e4')
  expect(playOnline).toHaveBeenCalledOnce()
  expect(state.onlineMoves).toEqual([])
  expect(state.onlinePhase).toBe('disconnected')
  expect(failed).toHaveBeenCalledOnce()
})

const puzzle: Puzzle = {
  id: 'p1',
  fen: 'startpos',
  solution: ['e2e4'],
  rating: 1500,
  themes: [],
  plays: 0,
}
function puzzles(overrides = {}) {
  const state = puzzleSessionState()
  const selection: PuzzleSelection = {
    account: 'Alice',
    mode: 'rated',
    angle: 'mix',
    difficulty: 'normal',
    color: 'random',
  }
  const api = {
    localPuzzles: vi.fn(async () => [puzzle]),
    puzzleNext: vi.fn(async () => ({ puzzle, glicko: { rating: 1500 } })),
    puzzleSolve: vi.fn(async (): Promise<PuzzleSolveResult> => ({ ratingDiff: 5 })),
    ...overrides,
  }
  const trainer = new PuzzleSession(state, {
    api,
    selection: () => selection,
    online: () => true,
  })
  return { state, selection, api, trainer }
}
it('reports a rated attempt once and keeps retries/practice local', async () => {
  const { state, selection, api, trainer } = puzzles()
  await trainer.loadNext()
  await Promise.all([trainer.report(true), trainer.report(false)])
  expect(api.puzzleSolve).toHaveBeenCalledOnce()
  expect(state.session).toEqual({ solved: 1, failed: 0, ratingChange: 5 })
  trainer.retry(puzzle)
  await trainer.report(true)
  expect(api.puzzleSolve).toHaveBeenCalledOnce()
  selection.mode = 'practice'
  await trainer.loadNext()
  await trainer.report(false)
  expect(api.puzzleSolve).toHaveBeenCalledOnce()
  expect(state.session.failed).toBe(1)
})
it('ignores a late rated result after an account/session changes', async () => {
  const pending = deferred<PuzzleSolveResult>()
  const { state, selection, trainer } = puzzles({ puzzleSolve: () => pending.promise })
  await trainer.loadNext()
  const report = trainer.report(true)
  selection.account = 'Bob'
  trainer.invalidate()
  trainer.resetSession()
  pending.resolve({ ratingDiff: 100 })
  await report
  expect(state.session).toEqual({ solved: 0, failed: 0, ratingChange: 0 })
  expect(state.lastReport).toBeNull()
})
it('selects offline difficulty through the same workflow without submitting a result', async () => {
  const { selection, api, trainer } = puzzles()
  selection.mode = 'offline'
  selection.difficulty = 'harder'
  await trainer.loadNext()
  await trainer.report(true)
  expect(api.localPuzzles).toHaveBeenCalledWith({
    theme: 'mix',
    minRating: 1650,
    maxRating: 1950,
    count: 1,
  })
  expect(api.puzzleSolve).not.toHaveBeenCalled()
})

it('cancels a seek in the core and ignores its late completion', async () => {
  const pending = deferred<{ seeking: boolean }>()
  const cancelOnline = vi.fn(async () => {})
  const { state, game } = online({ startOnline: () => pending.promise, cancelOnline })
  const started = game.start({ minutes: 5, increment: 0, rated: false, color: 'random' })
  expect(state.onlinePhase).toBe('seeking')
  await game.stop()
  pending.resolve({ seeking: true })
  expect(await started).toBe(false)
  expect(cancelOnline).toHaveBeenCalledOnce()
  expect(state.onlinePhase).toBe('idle')
})
it('does not let an old move failure overwrite a recovered connection', async () => {
  const pending = deferred<undefined>()
  const { state, game, full } = online({ playOnline: () => pending.promise })
  game.readOnlineEvent(full())
  const move = game.onlineMove('e2e4')
  game.readOnlineState({
    session: 2,
    lane: 'game',
    account: 'Alice',
    gameId: 'AbCd1234',
    phase: 'connected',
  })
  game.readOnlineEvent(full('AbCd1234', 'e2e4'))
  pending.reject(new Error('late network failure'))
  await move
  expect(state.onlinePhase).toBe('playing')
  expect(state.onlineMoves).toEqual(['e2e4'])
})
it('does not apply a puzzle drawn for a different account without Vue watchers', async () => {
  const pending = deferred<{ puzzle: Puzzle }>()
  const { selection, state, trainer } = puzzles({ puzzleNext: () => pending.promise })
  const load = trainer.loadNext()
  selection.account = 'Bob'
  pending.resolve({ puzzle })
  await load
  expect(state.current).toBeNull()
  expect(state.rating).toBeUndefined()
})
