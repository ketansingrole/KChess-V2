<script setup lang="ts">
import { computed } from 'vue'
import { gameResult } from '../utils/games'

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
  games,
  filteredGames,
  visibleGames,
} = storeToRefs(store)
const { sync, date, openReview, selectPage } = store

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
  const total = filteredGames.value.length
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
    <PageHeader title="History" subtitle="Review your synced Lichess games">
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
      </div>
      <GameReview
        :pgn="reviewPgn"
        :orientation="reviewGame.color"
        :theme="settings.boardTheme"
        :coordinates="settings.coordinates"
        :player-name="reviewGame.account"
        :opponent-name="reviewGame.opponent"
      />
    </template>

    <template v-else>
      <div v-if="!games.length" class="card">
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
            size="sm"
            variant="link"
            color="neutral"
            icon="i-lucide-x"
            @click="clearFilters"
            >Clear filters</UButton
          >
        </div>

        <div class="card">
          <UEmpty
            v-if="!visibleGames.length"
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
              ><span>Date</span>
            </div>
            <GameRow
              v-for="game in visibleGames"
              :key="`${game.account}-${game.id}`"
              :game="game"
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
              size="xs"
              aria-label="Games per page"
            />
            <UPagination
              v-model:page="pageNumber"
              :total="filteredGames.length"
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
