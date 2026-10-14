<script setup lang="ts">
import { computed, ref } from 'vue'
import { useInsightsReport } from '../composables/useInsightsReport'
import { formatCount } from '../utils/format'

const store = useKChessStore()
const { data, activeOnlineAccount } = storeToRefs(store)
const account = ref(activeOnlineAccount.value || data.value.accounts[0]?.username || '')
const speed = ref<string>('all')
const period = ref<string>('all')
const rated = ref<string>('all')
const query = computed(() => ({
  account: account.value,
  speed: speed.value === 'all' ? undefined : speed.value,
  rated: rated.value === 'all' ? undefined : rated.value === 'rated',
  days: period.value === 'all' ? undefined : Number(period.value),
}))
const { data: report, error: requestError, status, refresh } = useInsightsReport(query)
const error = computed(() => requestError.value?.message ?? '')
const loading = computed(() => status.value === 'pending')

const accounts = computed(() =>
  data.value.accounts.map((a) => ({ label: `@${a.username}`, value: a.username })),
)
const SPEEDS = [
  { label: 'All speeds', value: 'all' },
  { label: 'Bullet', value: 'bullet' },
  { label: 'Blitz', value: 'blitz' },
  { label: 'Rapid', value: 'rapid' },
  { label: 'Classical', value: 'classical' },
  { label: 'Correspondence', value: 'correspondence' },
]
const PERIODS = [
  { label: 'All time', value: 'all' },
  { label: 'Last 30 days', value: '30' },
  { label: 'Last 90 days', value: '90' },
  { label: 'Last year', value: '365' },
]
const RATED = [
  { label: 'Rated and casual', value: 'all' },
  { label: 'Rated only', value: 'rated' },
  { label: 'Casual only', value: 'casual' },
]
const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']
const ENDINGS: Record<string, string> = {
  mate: 'Checkmate',
  resign: 'Resignation',
  outoftime: 'Time out',
  timeout: 'Left the game',
  draw: 'Draw',
  stalemate: 'Stalemate',
  aborted: 'Aborted',
  noStart: 'Never started',
  cheat: 'Cheat detected',
  variantEnd: 'Variant win',
}

/** Four-hour blocks read better than 24 thin rows. */
const byTime = computed(() => {
  const hours = report.value?.byHour ?? []
  return [0, 4, 8, 12, 16, 20].map((start) => {
    const record = { total: 0, win: 0, loss: 0, draw: 0 }
    for (let h = start; h < start + 4; h++) {
      const entry = hours[h]
      if (!entry) continue
      record.total += entry.total
      record.win += entry.win
      record.loss += entry.loss
      record.draw += entry.draw
    }
    return {
      label: `${String(start).padStart(2, '0')}:00–${String(start + 4).padStart(2, '0')}:00`,
      record,
    }
  })
})
/** Lichess's speed keys, as titles. */
const speedName = (key: string): string =>
  key === 'ultraBullet' ? 'UltraBullet' : key.charAt(0).toUpperCase() + key.slice(1)
const score = computed(() =>
  report.value?.total
    ? Math.round(
        (100 * (report.value.record.win + report.value.record.draw / 2)) / report.value.total,
      )
    : 0,
)
const streakNow = computed(() => {
  const current = report.value?.streaks.current ?? 0
  if (current > 0) return `${current} ${current === 1 ? 'win' : 'wins'} in a row now`
  if (current < 0) return `${-current} ${current === -1 ? 'loss' : 'losses'} in a row now`
  return 'No streak now'
})
const best = computed(() => {
  const openings = (report.value?.byOpening ?? []).filter((o) => o.record.total >= 5)
  const score = (o: (typeof openings)[number]) =>
    (o.record.win + o.record.draw / 2) / o.record.total
  const sorted = [...openings].sort((a, b) => score(b) - score(a))
  return { top: sorted[0], bottom: sorted.at(-1) }
})
</script>

<template>
  <div>
    <PageHeader title="Lichess insights">
      <!-- The headline numbers sit beside the filters that shape them. -->
      <template v-if="report?.total && !error" #lead>
        <dl class="insight-stats">
          <div>
            <dt>Games</dt>
            <dd>
              <strong class="tabular">{{ formatCount(report.total) }}</strong>
              <span class="tabular"
                >{{ formatCount(report.record.win) }} won ·
                {{ formatCount(report.record.draw) }} drawn ·
                {{ formatCount(report.record.loss) }} lost</span
              >
            </dd>
          </div>
          <div>
            <dt title="Wins plus half the draws">Score</dt>
            <dd>
              <strong class="tabular">{{ score }}%</strong>
            </dd>
          </div>
          <div>
            <dt title="Longest winning and losing runs">Streaks</dt>
            <dd>
              <strong class="tabular"
                >{{ report.streaks.longestWin }} / {{ report.streaks.longestLoss }}</strong
              >
              <span>{{ streakNow }}</span>
            </dd>
          </div>
          <div v-if="report.accuracy">
            <dt>Accuracy</dt>
            <dd>
              <strong
                class="tabular"
                :title="`${report.accuracy.perGame.blunder.toFixed(1)} blunders and ${report.accuracy.perGame.mistake.toFixed(1)} mistakes a game`"
                >{{
                  report.accuracy.average !== undefined
                    ? `${report.accuracy.average.toFixed(1)}%`
                    : '–'
                }}</strong
              >
              <span class="tabular"
                >{{ formatCount(report.accuracy.games) }} reviewed
                {{ report.accuracy.games === 1 ? 'game' : 'games'
                }}<template v-if="report.accuracy.acpl !== undefined">
                  · ACPL {{ Math.round(report.accuracy.acpl) }}</template
                ></span
              >
            </dd>
          </div>
        </dl>
      </template>
      <USelect
        v-if="accounts.length > 1"
        v-model="account"
        :items="accounts"
        size="sm"
        class="min-w-36"
        aria-label="Account"
      />
      <USelect v-model="speed" :items="SPEEDS" size="sm" aria-label="Speed" />
      <USelect v-model="period" :items="PERIODS" size="sm" aria-label="Period" />
      <USelect v-model="rated" :items="RATED" size="sm" aria-label="Rated or casual" />
    </PageHeader>

    <div v-if="!accounts.length" class="card">
      <UEmpty
        variant="naked"
        icon="i-lucide-chart-pie"
        title="No saved Lichess games to explore"
        :actions="[
          {
            label: 'Follow a player',
            icon: 'i-lucide-user-plus',
            onClick: () => store.selectPage('friends'),
          },
          {
            label: 'Analyze a PGN',
            variant: 'outline',
            color: 'neutral',
            onClick: () => store.selectPage('analysis'),
          },
        ]"
      />
    </div>
    <div v-else-if="error" class="card">
      <p class="text-error text-sm" role="alert">{{ error }}</p>
      <UButton class="mt-3" icon="i-lucide-rotate-cw" @click="refresh()">Try again</UButton>
    </div>
    <p v-else-if="loading" class="text-muted text-sm" role="status">Loading insights…</p>
    <div v-else-if="report && !report.total" class="card">
      <UEmpty
        variant="naked"
        icon="i-lucide-chart-pie"
        title="No finished games match"
        description="Sync games from Lichess history, or widen the filters."
        :actions="[
          { label: 'Open Lichess history', onClick: () => navigateTo('/history?source=lichess') },
        ]"
      />
    </div>
    <div v-else-if="report" class="insight-body" :aria-busy="loading">
      <!-- Short breakdowns flow into balanced columns; openings, the long list, spans them. -->
      <div class="insight-sections">
        <section>
          <h2 class="section-title">By colour</h2>
          <ResultBar label="White" :record="report.byColor.white" />
          <ResultBar label="Black" :record="report.byColor.black" />
        </section>
        <section>
          <h2 class="section-title">By speed</h2>
          <ResultBar
            v-for="entry in report.bySpeed"
            :key="entry.speed"
            :label="speedName(entry.speed)"
            :record="entry.record"
          />
        </section>
        <section>
          <h2 class="section-title">By opponent rating</h2>
          <ResultBar
            v-for="entry in report.byOpponent"
            :key="entry.label"
            :label="entry.label"
            :record="entry.record"
          />
        </section>
        <section>
          <h2 class="section-title">By game length</h2>
          <ResultBar
            v-for="entry in report.byLength"
            :key="entry.label"
            :label="entry.label"
            :record="entry.record"
          />
        </section>
        <section>
          <h2 class="section-title">By day</h2>
          <ResultBar
            v-for="(record, day) in report.byWeekday"
            :key="day"
            :label="WEEKDAYS[day]!"
            :record="record"
          />
        </section>
        <section>
          <h2 class="section-title">By time of day</h2>
          <ResultBar
            v-for="entry in byTime"
            :key="entry.label"
            :label="entry.label"
            :record="entry.record"
          />
        </section>
        <section>
          <h2 class="section-title">How games ended</h2>
          <ResultBar
            v-for="entry in report.endings"
            :key="entry.status"
            :label="ENDINGS[entry.status] ?? entry.status"
            :record="entry.record"
          />
        </section>
      </div>
      <section class="insight-openings">
        <h2 class="section-title">Openings</h2>
        <p v-if="best.top && best.bottom && best.top !== best.bottom" class="section-hint">
          Best: {{ best.top.name }} · weakest: {{ best.bottom.name }} (at least 5 games)
        </p>
        <p v-if="!report.byOpening.length" class="muted text-sm">
          Lichess names openings of synced games; none have two games yet.
        </p>
        <div v-else class="insight-opening-list">
          <ResultBar
            v-for="entry in report.byOpening"
            :key="entry.name"
            :label="entry.name"
            :hint="`${entry.asWhite} as White`"
            :record="entry.record"
          />
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.insight-stats {
  display: flex;
  flex-wrap: wrap;
  /* Top-aligned, so a figure without a detail line (Score) lines up with the others. */
  align-items: flex-start;
  gap: 8px 32px;
  margin: 0;
}
.insight-stats > div {
  display: grid;
  gap: 2px;
}
.insight-stats dt {
  color: var(--ui-text-muted);
  font-size: 12px;
}
/* Each figure's detail sits under it, so the four stay side by side beside the filters. */
.insight-stats dd {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 0;
}
.insight-stats strong {
  font-size: 22px;
  font-weight: 600;
  line-height: 1.1;
}
.insight-stats dd span {
  color: var(--ui-text-muted);
  font-size: 12px;
  white-space: nowrap;
}
.insight-body {
  display: flex;
  flex-direction: column;
  gap: 28px;
}
.insight-sections {
  columns: 3 300px;
  column-gap: 40px;
}
.insight-sections section,
.insight-openings {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.insight-sections section {
  break-inside: avoid;
  margin-bottom: 28px;
}
.insight-sections .section-title,
.insight-openings .section-title {
  margin-bottom: 4px;
}
.insight-opening-list {
  columns: 2 360px;
  column-gap: 40px;
}
.insight-opening-list > * {
  break-inside: avoid;
}
</style>
