import { describe, it, expect } from 'vitest'
import { nextTick, effectScope, ref } from 'vue'
import { createPinia, setActivePinia, disposePinia, getActivePinia } from 'pinia'
import { useLocalGameStore } from '../../app/stores/local'
import {
  useGameArchiveStore,
  useGameArchive,
  decodeArchive,
  type GameSnapshot,
} from '../../app/stores/gameArchive'
import { STANDARD_SETUP } from '../../src/shared/variant'
import { setupPgn } from '../../app/utils/chess'
import { DEFAULT_OAUTH_LOOK, oauthLook, oauthPage } from '../../src/main/oauthPage'

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
    disposePinia(getActivePinia()!)
    setActivePinia(createPinia())
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
    disposePinia(getActivePinia()!)
    setActivePinia(createPinia())
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
