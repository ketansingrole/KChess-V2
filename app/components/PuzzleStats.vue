<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { themeName } from '../utils/puzzleThemes'
import { formatCount } from '../utils/format'

/** The account's puzzle numbers as Lichess reports them. Read-only: nothing here can be changed from KChess. */
const kchess = useKChessStore()
const puzzles = usePuzzleStore()
const { profile, ratingHistory, dashboard, dashboardDays, stats, account } = storeToRefs(puzzles)
const { data } = storeToRefs(kchess)

onMounted(() => void puzzles.loadStats())
watch([account, dashboardDays], () => void puzzles.loadStats())

const accounts = computed(() =>
  data.value.accounts.filter((entry) => entry.connected).map((entry) => entry.username),
)
const dayItems = [
  { label: 'Last 7 days', value: 7 },
  { label: 'Last 30 days', value: 30 },
  { label: 'Last 90 days', value: 90 },
  { label: 'Last year', value: 365 },
]
const sortItems = [
  { label: 'Weakest first', value: 'weak' },
  { label: 'Strongest first', value: 'strong' },
  { label: 'Most played', value: 'played' },
]
const sort = ref('weak')

const perf = computed(() => profile.value?.perfs?.puzzle)
const points = computed(() => {
  const entry = ratingHistory.value.find((item) => /^puzzles?$/i.test(item.name ?? ''))
  return (entry?.points ?? [])
    .filter((point) => point.length >= 4)
    .map((point) => ({ date: Date.UTC(point[0]!, point[1]!, point[2]!), rating: point[3]! }))
})
const global = computed(() => dashboard.value?.global)
const firstTry = computed(() =>
  global.value?.nb ? Math.round((global.value.firstWins / global.value.nb) * 100) : undefined,
)
const themeRows = computed(() => {
  const rows = Object.entries(dashboard.value?.themes ?? {}).map(([key, entry]) => ({
    key,
    name: themeName(key),
    attempts: entry.results.nb,
    firstTry: entry.results.nb ? Math.round((entry.results.firstWins / entry.results.nb) * 100) : 0,
    performance: entry.results.performance,
    average: entry.results.puzzleRatingAvg,
  }))
  // Themes tried only once or twice say little about strength.
  const sorted =
    sort.value === 'played'
      ? rows.sort((a, b) => b.attempts - a.attempts)
      : rows
          .filter((row) => row.attempts >= 3)
          .sort((a, b) =>
            sort.value === 'weak' ? a.performance - b.performance : b.performance - a.performance,
          )
  return sorted.slice(0, 15)
})
</script>

<template>
  <div>
    <UEmpty
      v-if="!account"
      class="card"
      icon="i-lucide-user-round-plus"
      title="Connect a Lichess account"
      :actions="[
        { label: 'Connect Lichess', icon: 'i-lucide-log-in', onClick: () => kchess.connect() },
      ]"
    />
    <template v-else>
      <div class="stats-bar">
        <div class="toolbar-row stats-bar-controls">
          <USelect
            v-if="accounts.length > 1"
            :model-value="account"
            :items="accounts"
            icon="i-lucide-user"
            size="sm"
            aria-label="Account"
            @update:model-value="puzzles.setAccount($event as string)"
          />
          <USelect
            v-model="dashboardDays"
            :items="dayItems"
            size="sm"
            aria-label="Dashboard period"
          />
          <UButton
            size="sm"
            variant="outline"
            color="neutral"
            icon="i-lucide-refresh-cw"
            :loading="stats.loading"
            @click="puzzles.loadStats()"
            >Refresh</UButton
          >
        </div>
      </div>

      <div class="stats-grid">
        <div class="card stat-card">
          <div class="stat-label">Puzzle rating</div>
          <div class="stat-value">{{ perf?.rating ?? '—' }}{{ perf?.prov ? '?' : '' }}</div>
          <div class="stat-sub">
            <span v-if="perf?.prog" :class="perf.prog > 0 ? 'up' : 'down'"
              >{{ perf.prog > 0 ? '▲' : '▼' }} {{ Math.abs(perf.prog) }}</span
            >
            <template v-if="perf?.prog"> recently</template>
            <template v-else>&nbsp;</template>
          </div>
        </div>
        <div class="card stat-card">
          <div class="stat-label">Puzzles played</div>
          <div class="stat-value">{{ perf ? formatCount(perf.games) : '—' }}</div>
        </div>
        <div class="card stat-card">
          <div class="stat-label">Solved first try</div>
          <div class="stat-value">{{ firstTry !== undefined ? `${firstTry}%` : '—' }}</div>
          <div class="stat-sub">Last {{ dashboardDays }} days</div>
        </div>
        <div class="card stat-card">
          <div class="stat-label">Performance</div>
          <div class="stat-value">{{ global?.performance ?? '—' }}</div>
          <div class="stat-sub">
            {{ global ? `${formatCount(global.nb)} puzzles` : `Last ${dashboardDays} days` }}
          </div>
        </div>
      </div>

      <div class="two-columns">
        <div class="card">
          <div class="card-header">
            <h2 class="section-title">Puzzle rating history</h2>
          </div>
          <LazyRatingChart v-if="points.length >= 2" :points="points" label="Puzzle" />
          <UEmpty
            v-else
            variant="naked"
            size="sm"
            icon="i-lucide-chart-line"
            title="No rating history yet"
          />
        </div>

        <div class="card">
          <div class="card-header">
            <div>
              <h2 class="section-title">Themes</h2>
              <p class="section-hint">
                Where you are strongest and weakest, last {{ dashboardDays }} days.
              </p>
            </div>
            <USelect v-model="sort" :items="sortItems" size="sm" aria-label="Sort themes" />
          </div>
          <UEmpty
            v-if="stats.needsReconnect"
            variant="naked"
            size="sm"
            icon="i-lucide-key-round"
            title="Lichess needs your permission"
            description="Reconnect this account to load puzzle statistics."
            :actions="[
              { label: 'Reconnect', icon: 'i-lucide-log-in', onClick: () => puzzles.reconnect() },
            ]"
          />
          <p v-else-if="stats.error" class="text-error text-sm" role="alert">{{ stats.error }}</p>
          <div v-else-if="themeRows.length" class="table-scroll">
            <table class="data-table">
              <thead>
                <tr>
                  <th scope="col">Theme</th>
                  <th scope="col" class="num">Played</th>
                  <th scope="col" class="num">First try</th>
                  <th scope="col" class="num">Perf.</th>
                  <th scope="col"><span class="sr-only">Train</span></th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="row in themeRows" :key="row.key">
                  <td>{{ row.name }}</td>
                  <td class="num tabular">{{ row.attempts }}</td>
                  <td class="num tabular">{{ row.firstTry }}%</td>
                  <td class="num tabular">{{ row.performance }}</td>
                  <td class="num">
                    <UButton
                      size="xs"
                      variant="link"
                      color="neutral"
                      @click="puzzles.startTheme(row.key)"
                      >Train</UButton
                    >
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          <UEmpty
            v-else
            variant="naked"
            size="sm"
            icon="i-lucide-puzzle"
            title="Nothing in this period"
            description="Solve some puzzles on Lichess or in rated mode here, then refresh."
          />
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.stats-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 12px;
  margin-bottom: 14px;
}
.stats-bar-controls {
  margin-left: auto;
}
</style>
