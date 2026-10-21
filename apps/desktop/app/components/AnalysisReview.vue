<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useMistakeStore } from '../stores/mistakes'
import { useAnalysisStore } from '../stores/analysis'
import { useReviewStore } from '../stores/review'
import { useReviewPractice } from '../stores/reviewPractice'
import { reviewPhases, type MoveClassification, type PhaseAccuracy } from '@kchess/rules/review'
import { MOVE_CLASSIFICATIONS } from '../utils/moveClassification'

/**
 * The analysis board's game review: both sides' accuracy, labelled moves, centipawn loss and
 * accuracy by phase, side by side, and the evaluation chart.
 */
const store = useKChessStore()
const { engineReady } = storeToRefs(store)
const analysis = useAnalysisStore()
const {
  origin,
  mainline,
  mainlineKey,
  review,
  gameAnalysis,
  reviewMarks,
  reviewedPlies,
  reviewWhole,
  reviewChart,
  reviewPly,
  reviewProgress,
  reviewRequestError,
} = storeToRefs(analysis)

/** Asked for in this visit and not started yet: another review is ahead of it. */
const requested = ref('')
watch(mainlineKey, () => (requested.value = ''))
const complete = computed(() => !!review.value?.complete && reviewWhole.value)
const reviews = useReviewStore()
/** Why the review asked for could not be done. */
const failure = computed(() => {
  const failed = reviews.status.failed
  return failed?.key === mainlineKey.value ? failed.message : reviewRequestError.value
})
const queued = computed(
  () =>
    requested.value === mainlineKey.value &&
    !reviewProgress.value &&
    !complete.value &&
    !failure.value,
)
async function start(): Promise<void> {
  requested.value = mainlineKey.value
  try {
    await analysis.requestReview()
  } catch (cause) {
    console.warn('[analysis-review] Review request failed:', cause)
    requested.value = ''
    reviewRequestError.value = cause instanceof Error ? cause.message : String(cause)
  }
}
function stop(): void {
  requested.value = ''
  analysis.cancelReview()
}
/** A Lichess game can be reviewed from Lichess's analysis even without a local engine. */
const canReview = computed(
  () =>
    !['playing', 'seeking', 'disconnected'].includes(store.onlinePhase) &&
    (engineReady.value || !!origin.value?.gameId),
)
const practice = useMistakeStore()
const practiceFeedback = ref('')
/** Sides asked for that had nothing to add, by review. */
const tried = ref(new Set<string>())
/** This review's positions already in practice, and the sides they are from. */
const saved = computed(() => {
  const key = review.value?.key
  const items = key ? practice.items.filter((item) => item.id.startsWith(`${key}:`)) : []
  const sides = new Set(items.map((item) => (item.fen.split(' ')[1] === 'b' ? 'black' : 'white')))
  return { count: items.length, sides }
})
/**
 * The sides whose labelled moves this review puts into practice: yours (saved as soon as the
 * review completes), or both in a game you did not play.
 */
const practiceSides = computed(() => {
  const result = gameAnalysis.value
  if (!complete.value || !result) return []
  const player = origin.value?.player
  return (player ? [player] : (['white', 'black'] as const)).filter(
    (color) => result[color].inaccuracy + result[color].mistake + result[color].blunder > 0,
  )
})
const canPractice = computed(
  () =>
    saved.value.count > 0 ||
    practiceSides.value.some((color) => !tried.value.has(`${review.value?.key}:${color}`)),
)
const reviewPractice = useReviewPractice()
/**
 * The board holds a studied puzzle: its line is a few moves, so side-by-side figures and practice
 * mean nothing there; its chart and move labels still do.
 */
const puzzleLine = computed(() => {
  const puzzle = origin.value?.puzzle
  return !!puzzle && analysis.root.fen === puzzle.fen
})
/** Saves whichever of those sides is not in practice yet, then practises them on the board. */
async function practiceMistakes(): Promise<void> {
  const stored = review.value
  if (!stored) return
  practiceFeedback.value = ''
  try {
    for (const color of practiceSides.value) {
      if (saved.value.sides.has(color) || tried.value.has(`${stored.key}:${color}`)) continue
      if (!(await practice.add(stored, color))) tried.value.add(`${stored.key}:${color}`)
    }
  } catch (cause) {
    console.warn('[analysis-review] Adding mistakes failed:', cause)
    practiceFeedback.value = cause instanceof Error ? cause.message : String(cause)
    return
  }
  if (!saved.value.count) {
    practiceFeedback.value = 'No verified mistakes to practice.'
    return
  }
  reviewPractice.start(stored.key)
  // The move list beside the practice card shows the moves before and after each position.
  analysis.tool = 'moves'
}

const sourceLabel = computed(() => {
  const stored = review.value
  if (!stored || !reviewedPlies.value) return ''
  if (stored.source === 'lichess') return 'From Lichess'
  if (stored.complete || reviewProgress.value) return ''
  return 'Partly reviewed'
})

const sides = computed(() => {
  const result = gameAnalysis.value
  if (!result || !reviewWhole.value) return []
  return (['white', 'black'] as const).map((color) => ({
    color,
    name: analysis.playerNames[color] ?? (color === 'white' ? 'White' : 'Black'),
    side: result[color],
    brilliant: [...reviewMarks.value.values()].filter(
      (mark) => mark.color === color && mark.classification === 'brilliant',
    ).length,
  }))
})
type Side = (typeof sides.value)[number]
const percent = (value: number | undefined): string => (value === undefined ? '—' : `${value}%`)

type Counted = 'brilliant' | 'inaccuracy' | 'mistake' | 'blunder'
const COUNTS: { key: Counted; label: string; one: string; many: string }[] = [
  { key: 'brilliant', label: 'Brilliant', one: 'brilliant move', many: 'brilliant moves' },
  { key: 'inaccuracy', label: 'Inaccuracies', one: 'inaccuracy', many: 'inaccuracies' },
  { key: 'mistake', label: 'Mistakes', one: 'mistake', many: 'mistakes' },
  { key: 'blunder', label: 'Blunders', one: 'blunder', many: 'blunders' },
]
const countOf = (row: Side, key: Counted): number =>
  key === 'brilliant' ? row.brilliant : row.side[key]
/** Brilliant moves are rare: their row shows only when there is one. */
const counts = computed(() =>
  COUNTS.filter(({ key }) => key !== 'brilliant' || sides.value.some((row) => row.brilliant)),
)

const PHASES: { key: keyof PhaseAccuracy; label: string }[] = [
  { key: 'opening', label: 'Opening' },
  { key: 'middlegame', label: 'Middlegame' },
  { key: 'endgame', label: 'Endgame' },
]
/** The phases the game reached, with each side's accuracy in them. */
const phases = computed(() => {
  const found = sides.value.length && review.value ? reviewPhases(review.value) : null
  if (!found) return []
  return PHASES.filter(
    ({ key }) => found.white[key] !== undefined || found.black[key] !== undefined,
  ).map(({ key, label }) => ({ key, label, white: found.white[key], black: found.black[key] }))
})

/** Jump to the next move of `color` with this label, after the one on the board (wrapping). */
function next(color: 'white' | 'black', judgment: MoveClassification): void {
  const marks = [...reviewMarks.value.values()].filter(
    (mark) => mark.color === color && mark.classification === judgment,
  )
  const from = reviewPly.value
  const target = marks.find((mark) => mark.index + 1 > from) ?? marks[0]
  if (target) analysis.goToPly(target.index + 1)
}
</script>

<template>
  <section v-if="mainline.length" class="review" aria-label="Game review">
    <div class="review-head">
      <strong class="review-title">Game review</strong>
      <span v-if="sourceLabel" class="review-source">{{ sourceLabel }}</span>
      <span class="flex-1" />
      <UButton
        v-if="reviewProgress || queued"
        size="xs"
        variant="ghost"
        color="neutral"
        icon="i-lucide-square"
        @click="stop"
        >Stop</UButton
      >
      <UTooltip
        v-else-if="!complete"
        :text="canReview ? 'Label each move and score both sides' : 'Stockfish is needed'"
      >
        <UButton
          size="xs"
          variant="soft"
          icon="i-lucide-sparkles"
          :disabled="!canReview"
          @click="start"
          >{{ review && reviewedPlies ? 'Finish review' : 'Review game' }}</UButton
        >
      </UTooltip>
    </div>

    <div v-if="reviewProgress" class="review-progress">
      <span>Reviewing…</span>
      <UProgress :model-value="reviewProgress.done" :max="reviewProgress.total" size="xs" />
    </div>
    <p v-else-if="queued" class="review-note">Waiting for another review to finish…</p>
    <p v-else-if="failure" class="review-note text-error">{{ failure }}</p>

    <table v-if="sides.length && !puzzleLine" class="review-table">
      <colgroup>
        <col />
        <col class="player-col" />
        <col class="player-col" />
      </colgroup>
      <thead>
        <tr>
          <th scope="col"><span class="sr-only">Statistic</span></th>
          <th v-for="row in sides" :key="row.color" scope="col" class="player" :title="row.name">
            <span class="player-head">
              <span :class="['side-dot', row.color]" aria-hidden="true" />
              <span class="player-name">{{ row.name }}</span>
            </span>
          </th>
        </tr>
      </thead>
      <tbody>
        <tr class="accuracy">
          <th scope="row">Accuracy</th>
          <td v-for="row in sides" :key="row.color">{{ percent(row.side.accuracy) }}</td>
        </tr>
      </tbody>
      <tbody>
        <tr v-for="kind in counts" :key="kind.key">
          <th scope="row" :style="{ color: MOVE_CLASSIFICATIONS[kind.key].color }">
            {{ kind.label }}
          </th>
          <td v-for="row in sides" :key="row.color">
            <button
              type="button"
              class="count"
              :disabled="!countOf(row, kind.key)"
              :style="
                countOf(row, kind.key) ? { color: MOVE_CLASSIFICATIONS[kind.key].color } : undefined
              "
              :aria-label="`${countOf(row, kind.key)} ${countOf(row, kind.key) === 1 ? kind.one : kind.many} by ${row.name}: go to the next`"
              @click="next(row.color, kind.key)"
            >
              {{ countOf(row, kind.key) }}
            </button>
          </td>
        </tr>
        <tr>
          <th scope="row">Avg. centipawn loss</th>
          <td v-for="row in sides" :key="row.color">{{ row.side.acpl ?? '—' }}</td>
        </tr>
      </tbody>
      <tbody v-if="phases.length">
        <tr v-for="phase in phases" :key="phase.key">
          <th scope="row">{{ phase.label }}</th>
          <td v-for="row in sides" :key="row.color">{{ percent(phase[row.color]) }}</td>
        </tr>
      </tbody>
    </table>

    <UButton
      v-if="complete && canPractice && !reviewPractice.active && !puzzleLine"
      block
      variant="soft"
      icon="i-lucide-target"
      @click="practiceMistakes"
      >{{ origin?.player ? 'Practice your mistakes' : 'Practice mistakes' }}</UButton
    >
    <p v-if="practiceFeedback" role="status" class="review-note">{{ practiceFeedback }}</p>
    <!-- While practising, the chart is on the practice card. -->
    <EvalChart
      v-if="reviewChart && !reviewPractice.active"
      :points="reviewChart.points"
      :judgments="reviewChart.judgments"
      :current="reviewPly"
      @select="analysis.goToPly"
    />

    <p v-if="!review && !canReview" class="review-note">
      Set up Stockfish in Settings to review games.
    </p>
  </section>
</template>

<style scoped>
.review {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.review-head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 24px;
}
.review-title {
  font-size: 13px;
}
.review-source,
.review-note {
  color: var(--ui-text-muted);
  font-size: 12px;
}
.review-progress {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  color: var(--ui-text-muted);
}
.review-progress > :last-child {
  flex: 1;
}
.review-table {
  width: 100%;
  border-collapse: collapse;
  table-layout: fixed;
  font-size: 13px;
}
.player-col {
  width: 33%;
}
.review-table th,
.review-table td {
  padding: 3px 4px;
  text-align: center;
  font-variant-numeric: tabular-nums;
}
.review-table tbody th {
  padding-left: 0;
  color: var(--ui-text-muted);
  font-weight: 500;
  text-align: left;
}
.review-table td {
  font-weight: 600;
}
/* Rows come in groups: accuracy, moves, phases. */
.review-table tbody + tbody tr:first-child > * {
  border-top: 1px solid var(--ui-border);
  padding-top: 6px;
}
.review-table tbody tr:last-child > * {
  padding-bottom: 6px;
}
.player {
  vertical-align: bottom;
  font-size: 12.5px;
  font-weight: 600;
}
/* The side's dot sits above the name, so the name has the column's whole width. */
.player-head {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
}
.player-name {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.accuracy td {
  font-size: 16px;
  font-weight: 700;
}
.accuracy th {
  color: var(--ui-text);
}
.count {
  min-width: 30px;
  padding: 1px 6px;
  border: 0;
  border-radius: 5px;
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: pointer;
}
.count:hover:not(:disabled) {
  background: var(--ui-bg-accented);
}
.count:disabled {
  color: var(--ui-text-dimmed);
  font-weight: 400;
  cursor: default;
}
</style>
