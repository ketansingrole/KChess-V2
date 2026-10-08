import { vi } from 'vitest'
import { defineComponent } from 'vue'
import type { DesktopApi, OnlineEvent, Puzzle } from '../../src/shared/types'
import { DEFAULT_SETTINGS } from '../../src/shared/defaultSettings'
import { libraryApi } from './libraryBackend'

export const puzzle: Puzzle = {
  id: 'mate123',
  fen: '6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1',
  solution: ['a1a8'],
  rating: 600,
  themes: [],
}

export function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}

export function desktop(overrides: Partial<DesktopApi> = {}) {
  let onEvent: (event: OnlineEvent) => void = () => {}
  const api = {
    ...libraryApi(),
    loadData: async () => ({
      settings: { ...DEFAULT_SETTINGS },
      accounts: [
        { username: 'Alice', connected: true },
        { username: 'Bob', connected: true },
      ],
      gameCount: 0,
    }),
    gamePage: async () => ({ games: [], total: 0 }),
    gameLibraryOverview: async () => ({ byAccount: {}, versus: {} }),
    gameRatingHistory: async () => [],
    engineStatus: async () => ({
      ready: true,
      path: '',
      bundled: true,
      canDownload: false,
      managed: { installed: false, path: '' },
    }),
    loadThemes: async () => ({ themes: [], problems: [], dir: '' }),
    onOnlineEvent: (fn: (event: OnlineEvent) => void) => {
      onEvent = fn
      return vi.fn()
    },
    stopEngine: async () => {},
    onWatchState: () => vi.fn(),
    onOnlineState: () => vi.fn(),
    onOnlineError: () => vi.fn(),
    onNotification: () => vi.fn(),
    resumeOnline: async () => null,
    cachedProfile: async () => ({ profile: null, ratingHistory: null }),
    profile: async () => ({}),
    ratingHistory: async () => [],
    notify: async () => ({}),
    presence: async () => ({ users: {}, latencyMs: 0 }),
    startOnline: async () => ({ seeking: true }),
    cancelOnline: async () => {},
    puzzleNext: async () => ({ puzzle, glicko: { rating: 1500 } }),
    puzzleSolve: async () => ({}),
    puzzleDashboard: async () => ({ global: { nb: 42 }, themes: {} }),
    stormDashboard: async () => ({}),
    puzzleDbStatus: async () => ({ installed: true, busy: false, count: 20, bytes: 0 }),
    onPuzzleDbProgress: () => vi.fn(),
    localLadder: async () => Array.from({ length: 20 }, () => puzzle),
    runSummary: async (kind: string) => ({ kind, best: {}, recent: [], total: 0 }),
    saveSettings: async (settings: typeof DEFAULT_SETTINGS) => ({ ...settings }),
    ...overrides,
  }
  // Fixtures deliberately implement only the desktop methods each test exercises.
  window.kchess = api as unknown as DesktopApi
  return { api: window.kchess, emit: (event: OnlineEvent) => onEvent(event) }
}

export const ChessBoardStub = defineComponent({
  name: 'ChessBoard',
  props: ['fen'],
  emits: ['move'],
  template: '<div data-board />',
})
export const ButtonStub = defineComponent({
  name: 'UButton',
  props: ['disabled'],
  template: '<button :disabled="disabled"><slot /></button>',
})
export const componentStubs = {
  ChessBoard: ChessBoardStub,
  UButton: ButtonStub,
  UIcon: true,
  SourceBadge: true,
  StormStats: true,
  PuzzleDbCard: true,
  USelect: true,
}
