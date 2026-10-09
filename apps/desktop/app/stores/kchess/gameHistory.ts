import { computed, onScopeDispose, ref, watch, type Ref } from 'vue'
import type {
  AppData,
  GameLibraryOverview,
  LichessGame,
  LichessRatingHistory,
} from '@kchess/core/contracts/types'
import { pgnFromMoves } from '@kchess/core/domain/chess'
import { mergeRatingHistories, normalizeRatingKey } from '@kchess/core/domain/ratings'

export function useGameHistory(options: {
  data: Ref<AppData | null>
  selectedAccount: Ref<string>
  ratingHistories: Ref<LichessRatingHistory>
  chartMode: Ref<string>
  chartRange: Ref<string>
  selectPage: (page: 'history') => void
}) {
  const { data, selectedAccount, ratingHistories, chartMode, chartRange, selectPage } = options
  const historyResult = ref('all')
  const historyRated = ref('all')
  const historyAccount = ref('all')
  const historyPage = ref(0)
  const historyPageSize = ref(20)
  const reviewGame = ref<LichessGame | null>(null)
  const reviewMoves = computed(() => reviewGame.value?.moves.split(/\s+/).filter(Boolean) ?? [])
  const reviewPgn = computed(() => reviewGame.value?.pgn?.trim() || pgnFromMoves(reviewMoves.value))

  const gameCount = computed(() => data.value?.gameCount ?? 0)
  const visibleGames = ref<LichessGame[]>([])
  const recentGames = ref<LichessGame[]>([])
  const historyTotal = ref(0)
  const historyLoading = ref(false)
  const historyError = ref('')
  const libraryOverview = ref<GameLibraryOverview>({ byAccount: {}, versus: {} })
  const fallbackRatings = ref<LichessRatingHistory>([])
  const selectedRecord = computed(
    () => libraryOverview.value.byAccount[selectedAccount.value.toLowerCase()],
  )
  const selectedGameCount = computed(() => selectedRecord.value?.total ?? 0)
  const counts = computed(() => selectedRecord.value ?? { win: 0, loss: 0, draw: 0 })
  const historyPageCount = computed(() =>
    Math.max(1, Math.ceil(historyTotal.value / historyPageSize.value)),
  )
  let pageEpoch = 0
  let overviewEpoch = 0
  let accountEpoch = 0
  onScopeDispose(() => {
    ++pageEpoch
    ++overviewEpoch
    ++accountEpoch
  })

  watch(
    [historyResult, historyRated, historyAccount, historyPageSize],
    () => {
      historyPage.value = 0
    },
    { flush: 'sync' },
  )
  watch(
    [data, historyResult, historyRated, historyAccount, historyPageSize, historyPage],
    async () => {
      const epoch = ++pageEpoch
      visibleGames.value = []
      historyError.value = ''
      if (!data.value) {
        historyTotal.value = 0
        historyLoading.value = false
        return
      }
      historyLoading.value = true
      try {
        const result = await window.kchess.gamePage({
          account: historyAccount.value === 'all' ? undefined : historyAccount.value,
          result:
            historyResult.value === 'all'
              ? undefined
              : (historyResult.value as 'win' | 'loss' | 'draw'),
          rated: historyRated.value === 'all' ? undefined : historyRated.value === 'rated',
          offset: historyPage.value * historyPageSize.value,
          limit: historyPageSize.value,
        })
        if (epoch !== pageEpoch) return
        historyTotal.value = result.total
        const lastPage = Math.max(0, Math.ceil(result.total / historyPageSize.value) - 1)
        if (historyPage.value > lastPage) {
          historyPage.value = lastPage
          return
        }
        visibleGames.value = result.games
      } catch (cause) {
        if (epoch === pageEpoch) {
          console.warn('[game-history] loading game page failed:', cause)
          historyError.value = cause instanceof Error ? cause.message : String(cause)
        }
      } finally {
        if (epoch === pageEpoch) historyLoading.value = false
      }
    },
    { immediate: true },
  )

  watch(
    data,
    async () => {
      const epoch = ++overviewEpoch
      libraryOverview.value = { byAccount: {}, versus: {} }
      if (!data.value) return
      try {
        const result = await window.kchess.gameLibraryOverview()
        if (epoch === overviewEpoch) libraryOverview.value = result
      } catch (cause) {
        console.warn('[game-history] loading library overview failed:', cause)
        /* An unavailable overview must not show another account's counts. */
      }
    },
    { immediate: true },
  )
  watch(
    [data, selectedAccount],
    async () => {
      const epoch = ++accountEpoch
      recentGames.value = []
      fallbackRatings.value = []
      if (!data.value || !selectedAccount.value) return
      const results = await Promise.allSettled([
        window.kchess.gamePage({ account: selectedAccount.value, offset: 0, limit: 6 }),
        window.kchess.gameRatingHistory(selectedAccount.value),
      ])
      if (epoch !== accountEpoch) return
      if (results[0].status === 'fulfilled') recentGames.value = results[0].value.games
      if (results[1].status === 'fulfilled') fallbackRatings.value = results[1].value
    },
    { immediate: true },
  )
  const ratingHistoriesResolved = computed(() =>
    mergeRatingHistories(ratingHistories.value, fallbackRatings.value),
  )
  /**
   * History entries are keyed by perf key since `@lichess-org/types` 2.0.176
   * (`blitz`, not `Blitz`); the selected mode may still hold a legacy display
   * name, so both sides normalize before comparing.
   */
  const findByMode = (history: LichessRatingHistory, mode: string) => {
    const want = normalizeRatingKey(mode)
    return history.find((item) => normalizeRatingKey(item.name) === want)
  }
  /** The chosen mode's points come from synced games rather than Lichess. */
  const chartFromGames = computed(
    () =>
      !findByMode(ratingHistories.value, chartMode.value)?.points?.length &&
      Boolean(findByMode(ratingHistoriesResolved.value, chartMode.value)?.points?.length),
  )
  // Keep the selected mode on one that has data.
  watch(ratingHistoriesResolved, (history) => {
    if (!findByMode(history, chartMode.value)?.points?.length) {
      const next = history.find((item) => item.points?.length)?.name
      chartMode.value = next ? normalizeRatingKey(next) : 'blitz'
    }
  })
  /** Rating points for the chosen mode and range, oldest first. */
  const chartSeries = computed(() => {
    const cutoffDays: Record<string, number> = { '1M': 30, '3M': 90, '6M': 180, '1Y': 365 }
    const cutoff =
      chartRange.value === 'YTD'
        ? new Date(new Date().getFullYear(), 0, 1).getTime()
        : chartRange.value === 'All'
          ? 0
          : Date.now() - (cutoffDays[chartRange.value] ?? 0) * 86_400_000
    const points = findByMode(ratingHistoriesResolved.value, chartMode.value)?.points ?? []
    return points
      .filter((p) => p.length >= 4)
      .map((p) => ({ date: Date.UTC(p[0]!, p[1]!, p[2]!), rating: p[3]! }))
      .filter((point) => point.date >= cutoff)
  })
  const chartValues = computed(() => chartSeries.value.map((point) => point.rating))

  async function openReview(game: LichessGame): Promise<void> {
    // List rows carry no PGN; it is read locally on demand.
    const pgn =
      game.pgn ??
      (await window.kchess.gamePgn(game.account, game.id).catch((cause: unknown) => {
        console.warn('[game-history] loading game PGN failed:', cause)
        return null
      }))
    reviewGame.value = pgn ? { ...game, pgn } : game
    selectPage('history')
  }

  return {
    historyResult,
    historyRated,
    historyAccount,
    historyPage,
    historyPageSize,
    reviewGame,
    reviewPgn,
    gameCount,
    selectedGameCount,
    libraryOverview,
    historyTotal,
    historyLoading,
    historyError,
    historyPageCount,
    visibleGames,
    recentGames,
    counts,
    chartSeries,
    chartValues,
    chartFromGames,
    openReview,
  }
}
