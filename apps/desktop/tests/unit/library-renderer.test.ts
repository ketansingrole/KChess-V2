import { describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { effectScope, ref } from 'vue'
import { INITIAL_FEN } from '@kchess/rules/position'
import { useGameHistory } from '../../app/stores/kchess/gameHistory'
import { useKChessStore } from '../../app/stores/kchess'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useReviewStore } from '../../app/stores/review'
import { analysisContext } from '@kchess/rules/analysisContext'
import { DEFAULT_SETTINGS } from '@kchess/contracts/defaultSettings'
import type {
  AnalysisRequest,
  AnalysisUpdate,
  AppData,
  GamePage,
  GamePageQuery,
  LichessGame,
  LichessRatingHistory,
  ReviewSummary,
  ReviewUpdate,
} from '@kchess/contracts/types'
import { deferred, desktop } from './fixtures'

const row = (id: string): LichessGame => ({
  id,
  account: 'Alice',
  createdAt: 1,
  lastMoveAt: 1,
  rated: true,
  speed: 'blitz',
  perf: 'blitz',
  status: 'mate',
  color: 'white',
  opponent: 'Bob',
  moves: 'e4',
})
function history() {
  const data = ref<AppData | null>({
    settings: { ...DEFAULT_SETTINGS },
    accounts: [],
    gameCount: 100,
  })
  const selectedAccount = ref('Alice')
  const scope = effectScope()
  const state = scope.run(() =>
    useGameHistory({
      data,
      selectedAccount,
      ratingHistories: ref<LichessRatingHistory>([]),
      chartMode: ref('Blitz'),
      chartRange: ref('All'),
      selectPage: vi.fn(),
    }),
  )!
  return { ...state, data, selectedAccount, scope }
}

describe('bounded renderer history', () => {
  it('ignores stale filter and account responses and clamps pages after deletions', async () => {
    const oldPage = deferred<GamePage>()
    const oldRecent = deferred<GamePage>()
    const queries: GamePageQuery[] = []
    desktop({
      gamePage: async (query) => {
        queries.push(query)
        if (query.limit === 6)
          return query.account === 'Alice'
            ? oldRecent.promise
            : { games: [row('BobNew01')], total: 1 }
        if (!query.result) return oldPage.promise
        return { games: [row('NewPage1')], total: 1 }
      },
    })
    const state = history()
    try {
      state.historyPage.value = 3
      state.historyResult.value = 'win'
      state.selectedAccount.value = 'Bob'
      await flushPromises()
      expect(state.historyPage.value).toBe(0)
      expect(state.visibleGames.value.map((g) => g.id)).toEqual(['NewPage1'])
      oldPage.resolve({ games: [row('Stale001')], total: 100 })
      oldRecent.resolve({ games: [row('Stale002')], total: 100 })
      await flushPromises()
      expect(state.visibleGames.value.map((g) => g.id)).toEqual(['NewPage1'])
      expect(state.recentGames.value.map((g) => g.id)).toEqual(['BobNew01'])
      state.historyPage.value = 4
      state.data.value = { ...state.data.value!, gameCount: 1 }
      await flushPromises()
      expect(state.historyPage.value).toBe(0)
      expect(state.historyTotal.value).toBe(1)
      expect(queries.every((q) => q.limit <= 20)).toBe(true)
    } finally {
      state.scope.stop()
    }
  })
  it('clears stale rows while loading and shows errors without publishing late results after disposal', async () => {
    const pending = deferred<GamePage>()
    const gamePage = vi.fn(async (query: GamePageQuery) =>
      query.limit === 6 ? { games: [], total: 0 } : pending.promise,
    )
    desktop({ gamePage })
    const state = history()
    expect(state.historyLoading.value).toBe(true)
    pending.reject(new Error('Library unavailable'))
    await flushPromises()
    expect(state.historyError.value).toBe('Library unavailable')
    expect(state.historyLoading.value).toBe(false)
    const late = deferred<GamePage>()
    gamePage.mockImplementation(async () => late.promise)
    state.historyRated.value = 'casual'
    await flushPromises()
    state.scope.stop()
    late.resolve({ games: [row('Late0001')], total: 1 })
    await flushPromises()
    expect(state.visibleGames.value).toEqual([])
  })
})

describe('analysis engine cache identity', () => {
  it('starts a fresh search when the engine changes while analysis is detached, rejecting old updates', async () => {
    let receive: (update: AnalysisUpdate) => void = () => {}
    const startAnalysis = vi.fn(async (_request: AnalysisRequest) => 1)
    desktop({
      startAnalysis,
      engineStatus: async () => ({
        identity: 'old-engine',
        ready: true,
        path: '',
        bundled: true,
        canDownload: false,
        managed: { installed: false, path: '' },
      }),
      stopAnalysis: async () => {},
      onAnalysis: (fn) => {
        receive = fn
        return vi.fn()
      },
      reviewGet: async () => null,
    })
    const app = useKChessStore()
    await app.init()
    await flushPromises()
    const analysis = useAnalysisStore()
    analysis.attach()
    await flushPromises()
    const request = startAnalysis.mock.calls[0]![0]!
    const completed: AnalysisUpdate = {
      id: 1,
      clientId: request.clientId,
      fen: INITIAL_FEN,
      context: analysisContext(INITIAL_FEN, []),
      engine: 'old-engine',
      depth: 20,
      lines: Array.from({ length: 3 }, (_, i) => ({
        rank: i + 1,
        depth: 20,
        cp: 10,
        pv: ['e2e4'],
      })),
      done: true,
      reason: 'completed',
    }
    receive(completed)
    expect(analysis.evaluation?.engine).toBe('old-engine')
    analysis.detach()
    app.settings.enginePath = '/another/stockfish'
    expect(analysis.evaluation).toBeNull()
    analysis.attach()
    await flushPromises()
    expect(startAnalysis).toHaveBeenCalledTimes(2)
    receive(completed)
    expect(analysis.evaluation).toBeNull()
    analysis.detach()
  })
  it('invalidates a completed result when a downloaded engine is replaced at the same path', async () => {
    let identity = 'native:100:1'
    let receive: (update: AnalysisUpdate) => void = () => {}
    const startAnalysis = vi.fn(async (_request: AnalysisRequest) => 1)
    desktop({
      startAnalysis,
      stopAnalysis: async () => {},
      reviewGet: async () => null,
      onAnalysis: (fn) => {
        receive = fn
        return vi.fn()
      },
      engineStatus: async () => ({
        identity,
        ready: true,
        path: '/engine',
        bundled: false,
        canDownload: false,
        managed: { installed: true, path: '/engine' },
      }),
    })
    const app = useKChessStore()
    await app.init()
    await flushPromises()
    const analysis = useAnalysisStore()
    analysis.attach()
    await flushPromises()
    receive({
      id: 1,
      clientId: startAnalysis.mock.calls[0]![0].clientId,
      fen: INITIAL_FEN,
      context: analysisContext(INITIAL_FEN, []),
      engine: identity,
      depth: 20,
      lines: Array.from({ length: 3 }, (_, i) => ({
        rank: i + 1,
        depth: 20,
        cp: 10,
        pv: ['e2e4'],
      })),
      done: true,
      reason: 'completed',
    })
    expect(analysis.evaluation?.engine).toBe(identity)
    analysis.detach()
    await app.recheckEngine()
    analysis.attach()
    await flushPromises()
    expect(startAnalysis).toHaveBeenCalledTimes(1)
    analysis.detach()
    identity = 'native:200:2'
    await app.recheckEngine()
    expect(analysis.evaluation).toBeNull()
    analysis.attach()
    await flushPromises()
    expect(startAnalysis).toHaveBeenCalledTimes(2)
    receive({
      id: 2,
      clientId: startAnalysis.mock.calls[1]![0].clientId,
      fen: INITIAL_FEN,
      context: analysisContext(INITIAL_FEN, []),
      engine: '',
      depth: 0,
      lines: [],
      done: true,
      reason: 'failed',
      error: 'Engine startup failed',
    })
    expect(analysis.engineError).toBe('Engine startup failed')
    expect(analysis.engineBusy).toBe(false)
    analysis.detach()
  })
})

describe('review summaries for the visible page', () => {
  const summary = (accuracy: number): ReviewSummary => ({
    key: `key${accuracy}`,
    source: 'lichess',
    complete: true,
    white: { accuracy, inaccuracy: 0, mistake: 0, blunder: 0 },
    black: { accuracy, inaccuracy: 0, mistake: 0, blunder: 0 },
  })

  it('looks up only the page, keeps other pages, and never lets a lookup undo a streamed review', async () => {
    const lookup = deferred<Record<string, ReviewSummary>>()
    let stream: (update: ReviewUpdate) => void = () => {}
    const asked: string[][] = []
    desktop({
      reviewStatus: async () => ({ waiting: 0 }),
      onReviewUpdate: (callback) => {
        stream = callback
        return vi.fn()
      },
      onReviewStatus: () => vi.fn(),
      reviewSummaries: async (ids) => {
        asked.push(ids)
        return asked.length === 1
          ? { Page1Aaa: summary(70), Gone0001: summary(10) }
          : lookup.promise
      },
    })
    const reviews = useReviewStore()
    reviews.listen()
    // Only looked up when a page asks; nothing is transferred on startup.
    expect(asked).toEqual([])
    await reviews.loadSummaries(['Page1Aaa', 'Gone0001'])
    expect(reviews.summaries.Page1Aaa?.white.accuracy).toBe(70)
    const pending = reviews.loadSummaries(['Page2Aaa', 'Gone0001'])
    stream({
      review: { gameId: 'Page2Aaa' } as ReviewUpdate['review'],
      summary: summary(95),
    })
    lookup.resolve({})
    await pending
    expect(asked).toEqual([
      ['Page1Aaa', 'Gone0001'],
      ['Page2Aaa', 'Gone0001'],
    ])
    expect(reviews.summaries.Page1Aaa?.white.accuracy).toBe(70)
    expect(reviews.summaries.Page2Aaa?.white.accuracy).toBe(95)
    expect(reviews.summaries.Gone0001).toBeUndefined()
    reviews.dispose()
  })
})
