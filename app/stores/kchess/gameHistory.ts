import { computed, ref, watch, type Ref } from 'vue'
import type { AppData, LichessGame, LichessRatingHistory } from '../../../src/shared/types'
import { pgnFromMoves } from '../../utils/chess'
import { gameResult } from '../../utils/games'
import { mergeRatingHistories, ratingHistoryFromGames } from '../../utils/ratings'

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

  const games = computed(() => data.value?.games ?? [])
  const filteredGames = computed(() =>
    games.value.filter((game) => {
      if (historyAccount.value !== 'all' && game.account !== historyAccount.value) return false
      if (historyRated.value !== 'all' && (game.rated ? 'rated' : 'casual') !== historyRated.value)
        return false
      return historyResult.value === 'all' || gameResult(game) === historyResult.value
    }),
  )
  const historyPageCount = computed(() =>
    Math.max(1, Math.ceil(filteredGames.value.length / historyPageSize.value)),
  )
  const visibleGames = computed(() =>
    filteredGames.value.slice(
      historyPage.value * historyPageSize.value,
      (historyPage.value + 1) * historyPageSize.value,
    ),
  )
  const recentGames = computed(() =>
    games.value.filter((g) => g.account === selectedAccount.value).slice(0, 6),
  )
  const counts = computed(() => {
    const own = games.value.filter((g) => g.account === selectedAccount.value)
    return {
      win: own.filter((g) => gameResult(g) === 'win').length,
      loss: own.filter((g) => gameResult(g) === 'loss').length,
      draw: own.filter((g) => gameResult(g) === 'draw').length,
    }
  })
  /**
   * Lichess's rating history, completed from synced games where it has none
   * (some accounts return an empty history despite thousands of rated games).
   */
  const ratingHistoriesResolved = computed(() =>
    mergeRatingHistories(
      ratingHistories.value,
      ratingHistoryFromGames(games.value.filter((game) => game.account === selectedAccount.value)),
    ),
  )
  /** The chosen mode's points come from synced games rather than Lichess. */
  const chartFromGames = computed(
    () =>
      !ratingHistories.value.find((item) => item.name === chartMode.value)?.points?.length &&
      Boolean(
        ratingHistoriesResolved.value.find((item) => item.name === chartMode.value)?.points?.length,
      ),
  )
  // Keep the selected mode on one that has data.
  watch(ratingHistoriesResolved, (history) => {
    if (!history.find((item) => item.name === chartMode.value)?.points?.length)
      chartMode.value = history.find((item) => item.points?.length)?.name ?? 'Blitz'
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
    const points =
      ratingHistoriesResolved.value.find((item) => item.name === chartMode.value)?.points ?? []
    return points
      .filter((p) => p.length >= 4)
      .map((p) => ({ date: Date.UTC(p[0]!, p[1]!, p[2]!), rating: p[3]! }))
      .filter((point) => point.date >= cutoff)
  })
  const chartValues = computed(() => chartSeries.value.map((point) => point.rating))

  watch([historyResult, historyRated, historyAccount, historyPageSize], () => {
    historyPage.value = 0
  })
  async function openReview(game: LichessGame): Promise<void> {
    // List rows carry no PGN; it is read locally on demand.
    const pgn = game.pgn ?? (await window.kchess.gamePgn(game.account, game.id).catch(() => null))
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
    games,
    filteredGames,
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
