<script setup lang="ts">
import { computed, onMounted, watch } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import { puzzleThemeText } from '../utils/puzzleThemes'
import { formatCount } from '../utils/format'
import {
  HISTORY_FILTERS,
  HISTORY_MODES,
  moveAccuracy,
  playedText,
  puzzleHistoryRows,
  rowMatches,
  type PuzzleHistoryFilter,
} from '../utils/puzzleAttempts'
import { usePuzzleAttemptActions } from '../composables/usePuzzleAttemptActions'

/**
 * Every puzzle attempt: the ones kept on this computer (with partial credit) and the ones Lichess
 * recorded (read-only), in one list. Each can be tried again or studied on the analysis board.
 */
const puzzles = usePuzzleStore()
const { activity, activityState, account, history } = storeToRefs(puzzles)
const attemptActions = usePuzzleAttemptActions()

onMounted(() => {
  void puzzles.loadHistory()
  void puzzles.loadActivity()
})
watch(account, () => void puzzles.loadActivity())

const filter = useLocalStorage<PuzzleHistoryFilter>('kchess:puzzle-history-filter', 'all')
const allRows = computed(() => puzzleHistoryRows(history.value?.recent ?? [], activity.value))
const rows = computed(() =>
  allRows.value
    .filter((row) => rowMatches(row, filter.value))
    .map((row) => ({
      ...row,
      modeInfo: HISTORY_MODES[row.mode],
      themeText: puzzleThemeText(row.puzzle.themes),
    })),
)

/** Totals of first attempts kept here; retries are listed but not counted. */
const summary = computed(() => {
  const progress = history.value
  if (!progress?.all.attempts) return null
  const { today, all } = progress
  return {
    today,
    all,
    accuracy: moveAccuracy(all.movesFound, all.movesTried),
    solvedShare: Math.round(((all.solved + all.hinted) / all.attempts) * 100),
    dayStreak: progress.dayStreak,
    bestDayStreak: progress.bestDayStreak,
  }
})
</script>

<template>
  <div class="form-stack">
    <div v-if="summary" class="stats-grid">
      <div class="card stat-card">
        <div class="stat-label">Today</div>
        <div class="stat-value tabular">{{ formatCount(summary.today.attempts) }}</div>
        <div class="stat-sub tabular">
          {{ summary.today.solved + summary.today.hinted }} solved ·
          {{ summary.today.movesFound }}/{{ summary.today.movesTried }} moves
        </div>
      </div>
      <div
        class="card stat-card"
        title="Puzzles trained in KChess; attempts known only to Lichess are not counted"
      >
        <div class="stat-label">All time</div>
        <div class="stat-value tabular">{{ formatCount(summary.all.attempts) }}</div>
        <div class="stat-sub tabular">
          {{ summary.solvedShare }}% solved<template v-if="summary.all.partial">
            · {{ formatCount(summary.all.partial) }} partial</template
          >
        </div>
      </div>
      <div class="card stat-card" title="Moves right at the first try, of moves tried">
        <div class="stat-label">Move accuracy</div>
        <div class="stat-value tabular">
          {{ summary.accuracy !== undefined ? `${summary.accuracy}%` : '—' }}
        </div>
        <div class="stat-sub tabular">
          {{ formatCount(summary.all.movesFound) }} of {{ formatCount(summary.all.movesTried) }}
          moves right
        </div>
      </div>
      <div class="card stat-card" title="Days in a row with a solved puzzle">
        <div class="stat-label">Days in a row</div>
        <div class="stat-value tabular">{{ summary.dayStreak }}</div>
        <div class="stat-sub tabular">Best {{ summary.bestDayStreak }}</div>
      </div>
    </div>

    <div class="card">
      <div class="card-header">
        <div>
          <h2 class="sr-only">Puzzle history</h2>
          <USelect
            v-if="allRows.length"
            v-model="filter"
            :items="[...HISTORY_FILTERS]"
            icon="i-lucide-list-filter"
            size="sm"
            aria-label="Show puzzles played in"
            class="w-40"
          />
        </div>
        <div class="toolbar-row">
          <UButton
            v-if="account"
            size="sm"
            variant="outline"
            color="neutral"
            icon="i-lucide-refresh-cw"
            title="Load the latest puzzle history from Lichess"
            :loading="activityState.loading"
            @click="puzzles.loadActivity(true)"
            >Refresh</UButton
          >
        </div>
      </div>
      <div v-if="account && activityState.needsReconnect" class="history-note">
        <span class="muted text-sm">Lichess needs your permission to show its puzzle history.</span>
        <UButton size="xs" variant="soft" icon="i-lucide-log-in" @click="puzzles.reconnect()"
          >Reconnect</UButton
        >
      </div>
      <p v-else-if="account && activityState.error" class="text-error text-sm" role="alert">
        {{ activityState.error }}
      </p>
      <div v-if="activityState.loading && !rows.length" class="muted text-sm" role="status">
        <UIcon name="i-lucide-loader-circle" class="animate-spin" /> Loading from Lichess…
      </div>
      <p v-else-if="allRows.length && !rows.length" class="muted text-sm" role="status">
        No {{ HISTORY_MODES[filter === 'all' ? 'practice' : filter].label }} puzzles yet.
      </p>
      <UEmpty
        v-else-if="!rows.length"
        variant="naked"
        icon="i-lucide-puzzle"
        title="No puzzles yet"
        :actions="[
          { label: 'Train', icon: 'i-lucide-puzzle', onClick: () => (puzzles.tab = 'train') },
        ]"
      />
      <div v-else class="game-list">
        <div class="game-head puzzle-cols" aria-hidden="true">
          <span>Result</span><span class="puzzle-mode">Mode</span><span>Themes</span
          ><span class="puzzle-moves">Moves</span><span class="puzzle-rating">Rating</span
          ><span class="puzzle-when">Played</span><span />
        </div>
        <div v-for="row in rows" :key="row.key" class="game-row puzzle-cols">
          <span><ResultBadge :badge="row.badge" class="w-20" /></span>
          <span class="puzzle-mode muted" :title="row.modeInfo.tip"
            ><UIcon :name="row.modeInfo.icon" aria-hidden="true" /><span>{{
              row.modeInfo.label
            }}</span></span
          >
          <span class="puzzle-themes">{{ row.themeText }}</span>
          <span
            v-if="row.moves"
            class="puzzle-moves tabular muted"
            title="Moves right at the first try, of the solution's"
            >{{ row.moves }}</span
          >
          <span v-else class="puzzle-moves text-dimmed" title="Lichess records only the result"
            >—</span
          >
          <span class="puzzle-rating tabular">{{ row.puzzle.rating }}</span>
          <span class="puzzle-when muted">{{ playedText(row.at) }}</span>
          <span class="puzzle-actions">
            <UButton
              size="xs"
              variant="soft"
              color="neutral"
              @click="attemptActions.retry(row.puzzle)"
              >Try again</UButton
            >
            <UButton
              size="xs"
              variant="ghost"
              color="neutral"
              icon="i-lucide-microscope"
              aria-label="Open on the analysis board"
              title="Analysis board"
              @click="attemptActions.analyse(row.puzzle, row.wrongMoves)"
            />
          </span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.puzzle-mode {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  white-space: nowrap;
}
.history-note {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 8px;
}
</style>
