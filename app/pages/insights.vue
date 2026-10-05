<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { InsightsReport } from '../../src/shared/types'

const store = useKChessStore()
const { data, activeOnlineAccount } = storeToRefs(store)
const account = ref(activeOnlineAccount.value || data.value.accounts[0]?.username || '')
const speed = ref<string>('all')
const period = ref<string>('all')
const rated = ref<string>('all')
const report = ref<InsightsReport | null>(null)
const error = ref('')
const loading = ref(false)

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

async function load(): Promise<void> {
  if (!account.value) return
  loading.value = true
  error.value = ''
  try {
    report.value = await window.kchess.insights({
      account: account.value,
      speed: speed.value === 'all' ? undefined : speed.value,
      rated: rated.value === 'all' ? undefined : rated.value === 'rated',
      days: period.value === 'all' ? undefined : Number(period.value),
    })
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    loading.value = false
  }
}
watch([account, speed, period, rated], () => void load(), { immediate: true })

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
    <PageHeader title="Insights">
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
        title="No games yet"
        description="Add a Lichess account and sync its games to see insights."
      />
    </div>
    <p v-else-if="error" class="text-error text-sm" role="alert">{{ error }}</p>
    <div v-else-if="report && !report.total" class="card">
      <UEmpty
        variant="naked"
        icon="i-lucide-chart-pie"
        title="No finished games match"
        description="Sync your games from the History page, or widen the filters."
      />
    </div>
    <div v-else-if="report" class="flex flex-col gap-5" :aria-busy="loading">
      <div class="insight-tiles">
        <div class="card insight-tile">
          <span class="muted text-xs">Games</span>
          <strong class="tabular">{{ report.total.toLocaleString() }}</strong>
          <span class="text-xs muted tabular"
            >{{ report.record.win }} won · {{ report.record.draw }} drawn ·
            {{ report.record.loss }} lost</span
          >
        </div>
        <div class="card insight-tile">
          <span class="muted text-xs">Score</span>
          <strong class="tabular"
            >{{
              Math.round((100 * (report.record.win + report.record.draw / 2)) / report.total)
            }}%</strong
          >
          <span class="text-xs muted">wins plus half the draws</span>
        </div>
        <div class="card insight-tile">
          <span class="muted text-xs">Streaks</span>
          <strong class="tabular"
            >{{ report.streaks.longestWin }} / {{ report.streaks.longestLoss }}</strong
          >
          <span class="text-xs muted"
            >longest winning / losing ·
            {{
              report.streaks.current > 0
                ? `${report.streaks.current} ${report.streaks.current === 1 ? 'win' : 'wins'} in a row now`
                : report.streaks.current < 0
                  ? `${-report.streaks.current} ${report.streaks.current === -1 ? 'loss' : 'losses'} in a row now`
                  : 'no streak now'
            }}</span
          >
        </div>
        <div v-if="report.accuracy" class="card insight-tile">
          <span class="muted text-xs">Accuracy ({{ report.accuracy.games }} reviewed games)</span>
          <strong class="tabular">{{
            report.accuracy.average !== undefined ? `${report.accuracy.average.toFixed(1)}%` : '–'
          }}</strong>
          <span class="text-xs muted tabular"
            >{{ report.accuracy.perGame.blunder.toFixed(1) }} blunders ·
            {{ report.accuracy.perGame.mistake.toFixed(1) }} mistakes per game<template
              v-if="report.accuracy.acpl !== undefined"
            >
              · ACPL {{ Math.round(report.accuracy.acpl) }}</template
            ></span
          >
        </div>
      </div>

      <p class="legend text-xs">
        <span><i class="swatch win" /> Won</span>
        <span><i class="swatch draw" /> Drawn</span>
        <span><i class="swatch loss" /> Lost</span>
        <span class="muted"
          >· the percentage is your score, the last column the number of games</span
        >
      </p>

      <div class="insight-grid">
        <section class="card">
          <h2 class="section-title mb-3">By colour</h2>
          <ResultBar label="White" :record="report.byColor.white" />
          <ResultBar label="Black" :record="report.byColor.black" />
          <h2 class="section-title mt-5 mb-3">By speed</h2>
          <ResultBar
            v-for="entry in report.bySpeed"
            :key="entry.speed"
            :label="entry.speed"
            :record="entry.record"
          />
        </section>
        <section class="card">
          <h2 class="section-title mb-3">Against stronger and weaker players</h2>
          <ResultBar
            v-for="entry in report.byOpponent"
            :key="entry.label"
            :label="entry.label"
            :record="entry.record"
          />
          <h2 class="section-title mt-5 mb-3">By game length</h2>
          <ResultBar
            v-for="entry in report.byLength"
            :key="entry.label"
            :label="entry.label"
            :record="entry.record"
          />
        </section>
        <section class="card">
          <h2 class="section-title mb-1">Openings</h2>
          <p v-if="best.top && best.bottom && best.top !== best.bottom" class="section-hint mb-3">
            Best: {{ best.top.name }} · weakest: {{ best.bottom.name }} (at least 5 games)
          </p>
          <p v-if="!report.byOpening.length" class="muted text-sm">
            Lichess names openings of synced games; none have two games yet.
          </p>
          <ResultBar
            v-for="entry in report.byOpening"
            :key="entry.name"
            :label="entry.name"
            :hint="`${entry.asWhite} as White`"
            :record="entry.record"
          />
        </section>
        <section class="card">
          <h2 class="section-title mb-3">By day</h2>
          <ResultBar
            v-for="(record, day) in report.byWeekday"
            :key="day"
            :label="WEEKDAYS[day]!"
            :record="record"
          />
          <h2 class="section-title mt-5 mb-3">By time of day</h2>
          <ResultBar
            v-for="entry in byTime"
            :key="entry.label"
            :label="entry.label"
            :record="entry.record"
          />
          <h2 class="section-title mt-5 mb-3">How games ended</h2>
          <ResultBar
            v-for="entry in report.endings"
            :key="entry.status"
            :label="ENDINGS[entry.status] ?? entry.status"
            :record="entry.record"
          />
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.insight-tiles {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 14px;
}
.insight-tile {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 16px;
}
.insight-tile strong {
  font-size: 26px;
}
.insight-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(380px, 1fr));
  gap: 20px;
}
.insight-grid section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.legend {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin: 0;
}
.swatch {
  display: inline-block;
  width: 10px;
  height: 10px;
  border-radius: 3px;
  vertical-align: -1px;
}
.swatch.win {
  background: var(--ui-success);
}
.swatch.draw {
  background: var(--ui-border-accented);
}
.swatch.loss {
  background: var(--ui-error);
}
</style>
