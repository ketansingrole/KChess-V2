<script setup lang="ts">
import { computed, onMounted, watch } from 'vue'
import { formatCount } from '../utils/format'

/** Storm and Streak results as Lichess has them: fetched, never written. */
const kchess = useKChessStore()
const puzzles = usePuzzleStore()
const { storm, profile, stats, account } = storeToRefs(puzzles)

onMounted(() => {
  if (stats.value.account !== account.value || (!storm.value && !stats.value.loading))
    void puzzles.loadStats()
})
watch(account, () => void puzzles.loadStats())

const highs = computed(() => [
  { label: 'Today', value: storm.value?.high.day },
  { label: 'This week', value: storm.value?.high.week },
  { label: 'This month', value: storm.value?.high.month },
  { label: 'All time', value: storm.value?.high.allTime },
])
const days = computed(() =>
  (storm.value?.days ?? []).slice(0, 8).map((day) => ({
    ...day,
    accuracy: day.moves + day.errors ? Math.round((day.moves / (day.moves + day.errors)) * 100) : 0,
    label: day._id.replace(/\//g, '-'),
  })),
)
</script>

<template>
  <div class="card">
    <div class="card-header">
      <div>
        <h2 class="section-title">Your results on Lichess</h2>
      </div>
    </div>

    <UEmpty
      v-if="!account"
      variant="naked"
      size="sm"
      icon="i-lucide-user-round-plus"
      title="No Lichess account connected"
      :actions="[
        { label: 'Connect Lichess', icon: 'i-lucide-log-in', onClick: () => kchess.connect() },
      ]"
    />
    <div v-else-if="stats.loading && !storm" class="muted text-sm" role="status">
      <UIcon name="i-lucide-loader-circle" class="animate-spin" /> Loading from Lichess…
    </div>
    <template v-else>
      <h3 class="stat-label mt-4">Storm high scores</h3>
      <div class="storm-highs">
        <div v-for="high in highs" :key="high.label">
          <span class="stat-label">{{ high.label }}</span>
          <strong class="tabular">{{ high.value ?? '—' }}</strong>
        </div>
      </div>
      <div class="storm-highs">
        <div>
          <span class="stat-label">Storm best</span>
          <strong class="tabular">{{ profile?.perfs?.storm?.score ?? '—' }}</strong>
          <span class="muted text-xs"
            >{{ formatCount(profile?.perfs?.storm?.runs ?? 0) }} runs</span
          >
        </div>
        <div>
          <span class="stat-label">Streak best</span>
          <strong class="tabular">{{ profile?.perfs?.streak?.score ?? '—' }}</strong>
          <span class="muted text-xs"
            >{{ formatCount(profile?.perfs?.streak?.runs ?? 0) }} runs</span
          >
        </div>
      </div>
      <div v-if="days.length" class="table-scroll">
        <table class="data-table">
          <caption class="sr-only">
            Recent Storm days on Lichess
          </caption>
          <thead>
            <tr>
              <th scope="col">Day</th>
              <th scope="col" class="num">Runs</th>
              <th scope="col" class="num">Best score</th>
              <th scope="col" class="num">Accuracy</th>
              <th scope="col" class="num">Best combo</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="day in days" :key="day._id">
              <td>{{ day.label }}</td>
              <td class="num tabular">{{ day.runs }}</td>
              <td class="num tabular">{{ day.score }}</td>
              <td class="num tabular">{{ day.accuracy }}%</td>
              <td class="num tabular">{{ day.combo }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <p v-else class="section-hint">No Storm runs on Lichess in the last 30 days.</p>
    </template>
  </div>
</template>

<style scoped>
.storm-highs {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(110px, 1fr));
  gap: 12px;
  margin: 8px 0 14px;
}
.storm-highs > div {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.storm-highs strong {
  font-size: 20px;
}
</style>
