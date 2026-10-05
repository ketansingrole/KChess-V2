<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { gameResult } from '../utils/games'
import { useAnalysisStore } from '../stores/analysis'
import { useReviewStore } from '../stores/review'
import { judgmentCounts } from '../utils/review'

const store = useKChessStore()
const {
  busy,
  reviewGame,
  reviewPgn,
  settings,
  historyAccount,
  historyResult,
  historyRated,
  historyPage,
  historyPageSize,
  data,
  gameCount,
  historyTotal,
  historyLoading,
  historyError,
  visibleGames,
} = storeToRefs(store)
const { sync, date, openReview, selectPage } = store

const analysis = useAnalysisStore()
const reviews = useReviewStore()
const { summaries } = storeToRefs(reviews)
// A new page (or the same page re-fetched after a sync) brings its own summaries.
watch(visibleGames, (games) => void reviews.loadSummaries(games.map((game) => game.id)), {
  immediate: true,
})
const reviewer = ref<{ ply: number } | null>(null)
/** Review the game on the analysis board, opened at the move being looked at. */
function openGameReview(): void {
  const game = reviewGame.value
  if (!game || !analysis.loadPgn(reviewPgn.value, reviewer.value?.ply ?? 0)) return
  analysis.orientation = game.color
  analysis.origin = {
    gameId: game.id,
    account: game.account,
    white: game.color === 'white' ? game.account : game.opponent,
    black: game.color === 'black' ? game.account : game.opponent,
  }
  void analysis.requestReview()
  selectPage('analysis')
}
/** The review of the game being looked at, from its player's side. */
const reviewSide = computed(() => {
  const game = reviewGame.value
  const summary = game && summaries.value[game.id]
  return summary ? summary[game.color] : null
})

const accountItems = computed(() => [
  { label: 'All accounts', value: 'all' },
  ...data.value.accounts.map((account) => ({ label: account.username, value: account.username })),
])
const resultItems = [
  { label: 'All', value: 'all' },
  { label: 'Wins', value: 'win' },
  { label: 'Losses', value: 'loss' },
  { label: 'Draws', value: 'draw' },
]
const ratedItems = [
  { label: 'Rated & casual', value: 'all' },
  { label: 'Rated', value: 'rated' },
  { label: 'Casual', value: 'casual' },
]
const pageSizeItems = [10, 20, 30, 40, 50].map((size) => ({
  label: `${size} per page`,
  value: size,
}))
const filtered = computed(
  () =>
    historyAccount.value !== 'all' || historyResult.value !== 'all' || historyRated.value !== 'all',
)
function clearFilters(): void {
  historyAccount.value = 'all'
  historyResult.value = 'all'
  historyRated.value = 'all'
}
/** The store pages from 0; UPagination counts from 1. */
const pageNumber = computed({
  get: () => historyPage.value + 1,
  set: (value: number) => {
    historyPage.value = value - 1
  },
})
const rangeText = computed(() => {
  const total = historyTotal.value
  if (!total) return '0 games'
  const from = historyPage.value * historyPageSize.value + 1
  return `${from}–${Math.min(total, from + historyPageSize.value - 1)} of ${total} games`
})
const reviewResult = computed(() =>
  reviewGame.value ? { win: 'Win', loss: 'Loss', draw: 'Draw' }[gameResult(reviewGame.value)] : '',
)
const reviewColor = computed(() =>
  reviewGame.value
    ? ({ win: 'success', loss: 'error', draw: 'neutral' } as const)[gameResult(reviewGame.value)]
    : 'neutral',
)
</script>

<template>
  <div>
    <PageHeader title="History">
      <UButton
        icon="i-lucide-refresh-cw"
        variant="outline"
        color="neutral"
        :loading="busy"
        @click="sync()"
        >Sync games</UButton
      >
    </PageHeader>

    <template v-if="reviewGame">
      <div class="toolbar-row mb-4">
        <UButton
          icon="i-lucide-arrow-left"
          variant="ghost"
          color="neutral"
          size="sm"
          @click="reviewGame = null"
          >All games</UButton
        >
        <USeparator orientation="vertical" class="h-5" />
        <UBadge :color="reviewColor" variant="soft" size="sm">{{ reviewResult }}</UBadge>
        <span class="text-sm"
          ><strong>{{ reviewGame.account }}</strong> vs
          <strong>{{ reviewGame.opponent }}</strong></span
        >
        <span class="muted text-sm">
          · {{ date(reviewGame.createdAt) }}
          <template v-if="reviewGame.opening"> · {{ reviewGame.opening }}</template>
        </span>
        <span class="ms-auto" />
        <span v-if="reviewSide" class="review-line text-sm">
          <template v-if="reviewSide.accuracy !== undefined"
            >{{ reviewSide.accuracy }}% accuracy</template
          >
          <template v-for="count in judgmentCounts(reviewSide)" :key="count.judgment">
            · <span :class="count.judgment">{{ count.text }}</span>
          </template>
        </span>
        <UButton
          icon="i-lucide-sparkles"
          variant="soft"
          color="neutral"
          size="sm"
          @click="openGameReview"
          >Review</UButton
        >
      </div>
      <GameReview
        ref="reviewer"
        :pgn="reviewPgn"
        :orientation="reviewGame.color"
        :theme="settings.boardTheme"
        :coordinates="settings.coordinates"
        :piece-set="settings.pieceSet"
        :animation="settings.pieceAnimation"
        :player-name="reviewGame.account"
        :opponent-name="reviewGame.opponent"
      />
    </template>

    <template v-else>
      <div v-if="!gameCount" class="card">
        <UEmpty
          variant="naked"
          icon="i-lucide-history"
          title="No games yet"
          :description="
            data.accounts.length
              ? 'Sync your accounts to pull in your Lichess games.'
              : 'Add a Lichess account in Settings, then sync to see your games here.'
          "
          :actions="
            data.accounts.length
              ? [{ label: 'Sync games', icon: 'i-lucide-refresh-cw', onClick: () => sync() }]
              : [
                  {
                    label: 'Open Settings',
                    icon: 'i-lucide-settings-2',
                    onClick: () => selectPage('settings'),
                  },
                ]
          "
        />
      </div>

      <template v-else>
        <div class="toolbar-row mb-4">
          <USelect
            v-if="data.accounts.length > 1"
            v-model="historyAccount"
            :items="accountItems"
            icon="i-lucide-user"
            aria-label="Filter by account"
          />
          <UTabs
            v-model="historyResult"
            :items="resultItems"
            aria-label="Filter by result"
            :content="false"
            variant="pill"
          />
          <USelect v-model="historyRated" :items="ratedItems" aria-label="Filter by rating type" />
          <UButton
            v-if="filtered"
            size="md"
            variant="link"
            color="neutral"
            icon="i-lucide-x"
            @click="clearFilters"
            >Clear filters</UButton
          >
        </div>

        <div class="card">
          <p v-if="historyLoading" class="p-6 text-muted" role="status">Loading games…</p>
          <p v-else-if="historyError" class="p-6 text-error" role="alert">{{ historyError }}</p>
          <UEmpty
            v-else-if="!visibleGames.length"
            variant="naked"
            size="sm"
            icon="i-lucide-search-x"
            title="No games match these filters"
            :actions="[
              {
                label: 'Clear filters',
                variant: 'outline',
                color: 'neutral',
                onClick: clearFilters,
              },
            ]"
          />
          <div v-else class="game-list">
            <div class="game-head" aria-hidden="true">
              <span>Result</span><span>Opponent</span><span>Mode</span><span>Rating</span
              ><span>Accuracy</span><span>Date</span>
            </div>
            <GameRow
              v-for="game in visibleGames"
              :key="`${game.account}-${game.id}`"
              :game="game"
              :review="summaries[game.id]"
              detailed
              @select="openReview(game)"
            />
          </div>
          <div class="pager">
            <span class="muted text-xs tabular">{{ rangeText }}</span>
            <span class="flex-1" />
            <USelect
              v-model="historyPageSize"
              :items="pageSizeItems"
              size="sm"
              aria-label="Games per page"
            />
            <UPagination
              v-model:page="pageNumber"
              :total="historyTotal"
              :items-per-page="historyPageSize"
              :sibling-count="1"
              size="sm"
              variant="outline"
              color="neutral"
              active-variant="subtle"
              active-color="primary"
            />
          </div>
        </div>
      </template>
    </template>
  </div>
</template>
