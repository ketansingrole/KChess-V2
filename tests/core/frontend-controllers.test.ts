import { expect, it, vi } from 'vitest'
import { OnlineGame, onlineGameState } from '@kchess/rules/onlineGame'
import {
  PuzzleSession,
  puzzleSessionState,
  type PuzzleSelection,
} from '@kchess/rules/puzzleSession'
import type { OnlineEvent, Puzzle, PuzzleSolveResult } from '@kchess/contracts/types'
import { rulesLossless } from '@kchess/rules/engine'
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
it('reports a rated attempt once and keeps retries local', async () => {
  const { state, api, trainer } = puzzles()
  await trainer.loadNext()
  await Promise.all([trainer.report(true), trainer.report(false)])
  trainer.end()
  expect(api.puzzleSolve).toHaveBeenCalledOnce()
  expect(state.session).toMatchObject({ solved: 1, missed: 0, ratingChange: 5 })
  trainer.retry(puzzle)
  await trainer.report(true)
  expect(api.puzzleSolve).toHaveBeenCalledOnce()
})
// Train showed only "N solved · N missed": partial progress, hints and reveals were lost.
it('scores every move by its first try, logs each attempt at its end, retries uncounted', async () => {
  const record = vi.fn()
  const state = puzzleSessionState()
  const selection: PuzzleSelection = {
    account: '',
    mode: 'offline',
    angle: 'mix',
    difficulty: 'normal',
    color: 'random',
  }
  const trainer = new PuzzleSession(state, {
    api: {
      localPuzzles: vi.fn(async () => [puzzle]),
      puzzleNext: vi.fn(),
      puzzleSolve: vi.fn(),
    } as never,
    selection: () => selection,
    online: () => true,
    record,
  })
  /** One attempt as the board reports it: first tries per move, the verdict, then its end. */
  async function play(
    moves: [number, boolean][],
    win: boolean,
    detail: { revealed?: boolean; hinted?: boolean; total: number },
  ) {
    let reported = false
    for (const [index, correct] of moves) {
      trainer.move(index, correct)
      if (!correct && !reported) {
        reported = true
        await trainer.report(false, { revealed: false, hinted: false, ...detail })
      }
    }
    if (!reported) await trainer.report(win, { revealed: false, hinted: false, ...detail })
    trainer.end()
  }
  await trainer.loadNext()
  await play(
    [
      [0, true],
      [1, true],
      [2, true],
    ],
    true,
    { total: 3 },
  )
  await trainer.loadNext()
  await play(
    [
      [0, true],
      [1, true],
    ],
    true,
    { hinted: true, total: 2 },
  )
  await trainer.loadNext()
  // The reported case: wrong on the first move (then found), right on the second, wrong on the
  // third. Every move is scored by its first try: 1 found, 2 wrong, of 3.
  await play(
    [
      [0, false],
      [0, true],
      [1, true],
      [2, false],
      [2, true],
    ],
    false,
    { total: 3 },
  )
  expect(record.mock.lastCall![0]).toMatchObject({
    puzzle: { id: 'p1' },
    mode: 'offline',
    retry: false,
    win: false,
    found: 1,
    wrong: 2,
    total: 3,
    day: expect.stringMatching(/^\d{4}-\d{2}-\d{2}$/),
  })
  expect(state.session).toMatchObject({ partial: 1, movesFound: 6, movesTried: 8 })
  await trainer.loadNext()
  // Wrong, then Next before finishing: the attempt ends when the next puzzle is drawn.
  await trainer.report(false, { revealed: false, hinted: false, total: 2 })
  trainer.move(0, false)
  await trainer.loadNext()
  expect(state.session.missed).toBe(1)
  // The solution shown before any wrong move: the moves it played were never tried.
  trainer.move(0, true)
  await trainer.report(false, { revealed: true, hinted: false, total: 3 })
  trainer.end()
  // Ending twice, or moving after the end, changes nothing.
  trainer.end()
  trainer.move(1, false)
  expect(state.session).toEqual({
    solved: 1,
    hinted: 1,
    partial: 1,
    missed: 1,
    revealed: 1,
    movesFound: 7,
    movesTried: 10,
    ratingChange: 0,
  })
  expect(record).toHaveBeenCalledTimes(5)
  trainer.retry(puzzle)
  await play([[0, true]], true, { total: 1 })
  expect(record).toHaveBeenCalledTimes(6)
  expect(record.mock.lastCall![0]).toMatchObject({ retry: true, win: true })
  expect(state.session.solved).toBe(1)
})
// Analysing an attempt showed nothing of what was played: only counts were kept.
it('logs what was played at each wrong first try, as the core accepts it', async () => {
  const opening: Puzzle = {
    id: 'Ab12c',
    fen: 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1',
    solution: ['e2e4', 'e7e5', 'g1f3'],
    rating: 1500,
    themes: ['opening'],
  }
  const record = vi.fn()
  const state = puzzleSessionState()
  const selection: PuzzleSelection = {
    account: '',
    mode: 'offline',
    angle: 'mix',
    difficulty: 'normal',
    color: 'random',
  }
  const trainer = new PuzzleSession(state, {
    api: {
      localPuzzles: vi.fn(async () => [opening]),
      puzzleNext: vi.fn(),
      puzzleSolve: vi.fn(),
    } as never,
    selection: () => selection,
    online: () => true,
    record,
  })
  await trainer.loadNext()
  trainer.move(0, false, 'a2a3')
  await trainer.report(false, { revealed: false, hinted: false, total: 2 })
  trainer.move(0, true, 'e2e4')
  // A second wrong try at a move is not its first try.
  trainer.move(1, false, 'h2h3')
  trainer.move(1, false, 'h2h4')
  trainer.move(1, true, 'g1f3')
  trainer.end()
  const logged = record.mock.lastCall![0]
  // Indexes in the solution: the player's second move is its third.
  expect(logged.wrongMoves).toEqual([
    { index: 0, uci: 'a2a3' },
    { index: 2, uci: 'h2h3' },
  ])
  expect(rulesLossless('assertPuzzleAttempt', logged)).toMatchObject({
    wrongMoves: logged.wrongMoves,
  })
  // A clean solve logs none.
  await trainer.loadNext()
  trainer.move(0, true, 'e2e4')
  await trainer.report(true, { revealed: false, hinted: false, total: 1 })
  trainer.end()
  expect(record.mock.lastCall![0]).not.toHaveProperty('wrongMoves')
})
// Lichess draws the account's next unplayed puzzle: an unreported one came back on every Next.
it('sends practice results unrated so the next draw is a new puzzle', async () => {
  const { state, selection, api, trainer } = puzzles({
    puzzleSolve: vi.fn(async (): Promise<PuzzleSolveResult> => ({})),
  })
  selection.mode = 'practice'
  await trainer.loadNext()
  await trainer.report(false)
  trainer.end()
  expect(api.puzzleSolve).toHaveBeenCalledOnce()
  expect(api.puzzleSolve).toHaveBeenCalledWith(
    expect.objectContaining({ account: 'Alice', id: 'p1', win: false, rated: false }),
  )
  expect(state.session).toMatchObject({ solved: 0, missed: 1, ratingChange: 0 })
  expect(state.lastReport).toBeNull()
})
it('keeps practice going when Lichess rejects an unrated result', async () => {
  const { state, selection, trainer } = puzzles({
    puzzleSolve: vi.fn(async () => ({ needsReconnect: true }) as never),
  })
  selection.mode = 'practice'
  await trainer.loadNext()
  await trainer.report(true)
  expect(state.phase).toBe('ready')
  expect(state.lastReport?.failed).toBeTruthy()
})
it('does not report a practice puzzle drawn without the account', async () => {
  let draws = 0
  const { selection, api, trainer } = puzzles({
    puzzleNext: vi.fn(async () =>
      draws++ === 0 ? { needsReconnect: true } : { puzzle, glicko: null },
    ),
  })
  selection.mode = 'practice'
  await trainer.loadNext()
  await trainer.report(true)
  expect(api.puzzleSolve).not.toHaveBeenCalled()
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
  expect(state.session).toEqual({
    solved: 0,
    hinted: 0,
    partial: 0,
    missed: 0,
    revealed: 0,
    movesFound: 0,
    movesTried: 0,
    ratingChange: 0,
  })
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
