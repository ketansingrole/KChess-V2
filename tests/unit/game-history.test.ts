import { beforeEach, describe, it, expect, vi } from 'vitest'
import { nextTick, effectScope, ref } from 'vue'
import { flushPromises } from '@vue/test-utils'
import { useLocalGameStore } from '../../app/stores/local'
import { useGameHistory } from '../../app/stores/kchess/gameHistory'
import { desktop } from './fixtures'
import { DEFAULT_SETTINGS } from '../../src/shared/defaultSettings'
import type { AppData, LichessRatingHistory } from '../../src/shared/types'
import {
  useGameArchiveStore,
  useGameArchive,
  type GameSnapshot,
  type ArchivedGame,
} from '../../app/stores/gameArchive'
import { decodeArchive } from '../../src/shared/library'
import { restart, storedLibrary } from './libraryBackend'
import { STANDARD_SETUP } from '../../src/shared/variant'
import { setupPgn } from '../../src/shared/chess'
import { DEFAULT_OAUTH_LOOK, oauthLook } from '../../src/shared/oauthLook'
import { oauthPage } from '../../src/core/oauthPage'

// Every test here keeps its games in the core's library.
beforeEach(() => void desktop())
describe('local game history', () => {
  it('keeps results and earlier games when starting a new board game', async () => {
    const board = useLocalGameStore()
    const history = useGameArchiveStore()
    board.move('e2e4')
    board.move('e7e5')
    board.resign('white')
    await nextTick()
    expect(history.games).toHaveLength(1)
    expect(history.games[0]).toMatchObject({
      source: 'board',
      moves: ['e2e4', 'e7e5'],
      result: '0-1',
      finished: true,
    })
    const id = history.games[0]!.id
    board.start()
    board.move('d2d4')
    await nextTick()
    expect(history.games).toHaveLength(2)
    expect(history.games.find((g) => g.id === id)?.moves).toEqual(['e2e4', 'e7e5'])
    board.takeback()
    await nextTick()
    expect(history.games).toHaveLength(1)
  })
  it('updates one computer session across reload, and archives an unfinished replacement', async () => {
    const snapshot = ref<GameSnapshot>({
      source: 'computer',
      setup: STANDARD_SETUP,
      moves: ['e2e4'],
      white: 'You',
      black: 'Stockfish (Club Player)',
      result: '*',
      reason: 'In progress',
      timeControl: '300+3',
    })
    let scope = effectScope()
    scope.run(() =>
      useGameArchive(
        () => snapshot.value,
        () => snapshot.value.moves.length > 0,
      ),
    )
    const id = useGameArchiveStore().games[0]!.id
    scope.stop()
    await restart()
    scope = effectScope()
    const tracker = scope.run(() =>
      useGameArchive(
        () => snapshot.value,
        () => snapshot.value.moves.length > 0,
      ),
    )!
    expect(useGameArchiveStore().games).toHaveLength(1)
    expect(useGameArchiveStore().games[0]!.id).toBe(id)
    tracker.reset()
    snapshot.value = { ...snapshot.value, moves: ['d2d4'] }
    await nextTick()
    expect(useGameArchiveStore().games).toHaveLength(2)
    expect(useGameArchiveStore().games.find((g) => g.id === id)?.finished).toBe(true)
    scope.stop()
  })
  it('preserves custom starts and exports result headers, rejecting illegal saved moves', async () => {
    const board = useLocalGameStore()
    board.start({ variant: 'standard', fen: '7k/8/8/8/8/8/8/KR6 w - - 0 1' })
    board.move('b1b2')
    board.agreeDraw()
    await nextTick()
    const game = useGameArchiveStore().games[0]!
    const pgn = setupPgn(game.setup, game.moves, { Result: game.result })
    expect(pgn).toContain('[FEN "7k/8/8/8/8/8/8/KR6 w - - 0 1"]')
    expect(pgn).toContain('[Result "1/2-1/2"]')
    expect(decodeArchive({ version: 1, games: [game] })).toHaveLength(1)
    expect(
      decodeArchive({ version: 1, games: [{ ...game, moves: ['b1b8', 'b1b2'] }] }),
    ).toBeUndefined()
  })
  it('keeps clock-only sessions separate without inventing chess moves or results', async () => {
    const board = useLocalGameStore()
    board.otbPress('bottom')
    board.otbPress('top')
    board.otbPause()
    await nextTick()
    board.otbReset()
    await nextTick()
    const game = useGameArchiveStore().games[0]!
    expect(game).toMatchObject({ source: 'clock', moves: [], result: '*', finished: true })
    expect(game.clockSummary).toContain('Top: 1 moves')
    const oldId = game.id
    await restart()
    const nextBoard = useLocalGameStore()
    nextBoard.otbPress('bottom')
    nextBoard.otbPress('top')
    nextBoard.otbPause()
    await nextTick()
    expect(useGameArchiveStore().games).toHaveLength(2)
    expect(useGameArchiveStore().games.find((g) => g.id === oldId)?.finished).toBe(true)
  })
})

describe('OAuth callback presentation', () => {
  it('provides responsive self-contained success and cancellation pages', () => {
    for (const authorized of [true, false]) {
      const page = oauthPage(authorized)
      expect(page).toContain('<!doctype html>')
      expect(page).toContain('name="viewport"')
      expect(page).toContain('close this browser tab')
      expect(page).not.toMatch(/https?:\/\/|<script/)
    }
    expect(oauthPage(true)).toContain('Signed in to Lichess')
    expect(oauthPage(false)).toContain('Connection cancelled')
  })

  it('follows the app theme and appearance, rejecting unsafe colors', () => {
    const colors = {
      bg: '#101010',
      elevated: 'color-mix(in srgb, #101010 91%, #eeeeee)',
      text: '#eeeeee',
      textMuted: '#999',
      primary: '#ff8800',
      border: '#333333',
    }
    const look = oauthLook({ appearance: 'dark', light: colors, dark: colors })
    expect(look.appearance).toBe('dark')
    const page = oauthPage(true, look)
    expect(page).toContain('color-scheme:dark;--bg:#101010')
    expect(page).not.toContain('prefers-color-scheme')
    expect(oauthPage(true)).toContain('@media(prefers-color-scheme:dark)')
    for (const bad of ['red;}body{display:none', '#fff</style><script>', 'url(x)'])
      expect(oauthLook({ appearance: 'light', light: { ...colors, bg: bad }, dark: colors })).toBe(
        DEFAULT_OAUTH_LOOK,
      )
    expect(oauthLook({ appearance: 'neon', light: colors, dark: colors })).toBe(DEFAULT_OAUTH_LOOK)
    expect(oauthLook(undefined)).toBe(DEFAULT_OAUTH_LOOK)
  })
})

const archived = (id: string): ArchivedGame => ({
  id,
  source: 'board',
  startedAt: 0,
  updatedAt: 0,
  white: 'White',
  black: 'Black',
  result: '*',
  reason: 'In progress',
  finished: false,
  setup: STANDARD_SETUP,
  moves: ['e2e4'],
  timeControl: '-',
})

it('batches history writes and sends only the latest version of changed games', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  const history = useGameArchiveStore()
  for (let n = 0; n < 100; n++) history.save(archived(String(n)))
  await history.flush()
  const write = vi.spyOn(api, 'saveArchivedGame')
  history.save({ ...archived('0'), moves: ['e2e4', 'e7e5'] })
  history.save({ ...archived('0'), moves: ['e2e4', 'e7e5', 'g1f3'] })
  expect(write).not.toHaveBeenCalled()
  await vi.advanceTimersByTimeAsync(150)
  expect(write).toHaveBeenCalledTimes(1)
  const saved = storedLibrary().games
  expect(saved).toHaveLength(100)
  expect(saved[0]!.moves).toEqual(['e2e4', 'e7e5', 'g1f3'])
  await vi.advanceTimersByTimeAsync(300)
  expect(write).toHaveBeenCalledTimes(1)
})

it('flushes pending history on window unload and persists removals immediately', async () => {
  vi.useFakeTimers()
  const history = useGameArchiveStore()
  history.save(archived('first'))
  expect(storedLibrary().games).toEqual([])
  window.dispatchEvent(new Event('beforeunload'))
  await flushPromises()
  expect(storedLibrary().games).toHaveLength(1)
  await history.remove('first')
  expect(storedLibrary().games).toEqual([])
})

it('shows the saved version again when the core refuses a change', async () => {
  const { api } = desktop()
  const history = useGameArchiveStore()
  history.save({ ...archived('kept'), finished: true })
  await flushPromises()
  vi.spyOn(api, 'saveArchivedGame').mockRejectedValueOnce(
    new Error('Game history is full. Export saved games before making room for new games.'),
  )
  history.save({ ...archived('kept'), reason: 'Edited', finished: true })
  expect(history.games[0]!.reason).toBe('Edited')
  await flushPromises()
  expect(history.error).toContain('full')
  expect(history.games[0]!.reason).toBe('In progress')
  history.save({ ...archived('new'), finished: true })
  vi.spyOn(api, 'saveArchivedGame').mockRejectedValueOnce(new Error('Game history is full.'))
  history.save({ ...archived('refused'), finished: true })
  await flushPromises()
  expect(history.games.map((g) => g.id)).toEqual(['new', 'kept'])
  expect(storedLibrary().games.map((g) => g.id)).toEqual(['new', 'kept'])
})

it('keeps the saved history when a write fails and saves the latest game next time', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  const history = useGameArchiveStore()
  const write = vi.spyOn(api, 'saveArchivedGame').mockRejectedValueOnce(new Error('Storage full'))
  history.save(archived('first'))
  await vi.advanceTimersByTimeAsync(150)
  expect(history.error).toBe('Storage full')
  expect(storedLibrary().games).toEqual([])
  history.save({ ...archived('first'), result: '1-0', finished: true })
  await flushPromises()
  expect(history.error).toBe('')
  expect(storedLibrary().games[0]!.result).toBe('1-0')
  expect(write).toHaveBeenCalledTimes(2)
})

it('keeps all 500 games when the archive is full and permits replacing an existing game', async () => {
  vi.useFakeTimers()
  const history = useGameArchiveStore()
  for (let n = 0; n < 500; n++) history.save(archived(String(n)))
  await history.flush()
  history.save(archived('overflow'))
  await history.flush()
  expect(history.error).toContain('full')
  expect(history.games).toHaveLength(500)
  expect(history.games.some((g) => g.id === 'overflow')).toBe(false)
  history.save({ ...archived('0'), result: '1-0', finished: true })
  await flushPromises()
  expect(history.error).toBe('')
  expect(history.games).toHaveLength(500)
  const saved = storedLibrary().games
  expect(saved).toHaveLength(500)
  expect(saved[0]).toMatchObject({ id: '0', result: '1-0' })
})

describe('rating chart mode', () => {
  it('matches perf keys and legacy display names', async () => {
    desktop()
    const data = ref<AppData | null>({
      settings: { ...DEFAULT_SETTINGS },
      accounts: [],
      gameCount: 0,
    })
    const selectedAccount = ref('Alice')
    const ratingHistories = ref<LichessRatingHistory>([
      {
        name: 'blitz',
        points: [
          [2026, 0, 1, 1500],
          [2026, 0, 2, 1510],
        ],
      },
    ])
    const chartMode = ref('Blitz')
    const chartRange = ref('All')
    const scope = effectScope()
    const state = scope.run(() =>
      useGameHistory({
        data,
        selectedAccount,
        ratingHistories,
        chartMode,
        chartRange,
        selectPage: vi.fn(),
      }),
    )!
    await flushPromises()
    // A legacy display name still resolves key-named histories.
    expect(state.chartSeries.value).toHaveLength(2)
    // A history with only bullet moves the mode to its perf key.
    ratingHistories.value = [
      {
        name: 'bullet',
        points: [
          [2026, 0, 1, 1600],
          [2026, 0, 2, 1610],
        ],
      },
    ]
    await flushPromises()
    expect(chartMode.value).toBe('bullet')
    expect(state.chartSeries.value).toHaveLength(2)
    scope.stop()
  })
})
