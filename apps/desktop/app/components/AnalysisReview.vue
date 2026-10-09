<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { explainReviewedMove } from '../../../../core/src/domain/coach'
import { useMistakeStore } from '../stores/mistakes'
import { movesOf, pathOf } from '../../../../core/src/domain/analysisTree'
import { useAnalysisStore, type ReviewMark } from '../stores/analysis'
import { useReviewStore } from '../stores/review'
import type { Judgment } from '../../../../core/src/contracts/types'

/**
 * The analysis board's game review: each side's accuracy and labelled moves, the evaluation chart,
 * and what the review says about the move on the board.
 */
const store = useKChessStore()
const { engineReady } = storeToRefs(store)
const analysis = useAnalysisStore()
const {
  path,
  origin,
  mainline,
  mainlineKey,
  review,
  gameAnalysis,
  reviewMarks,
  reviewedPlies,
  reviewWhole,
  reviewProgress,
} = storeToRefs(analysis)

const GLYPHS: Record<Judgment, string> = { inaccuracy: '?!', mistake: '?', blunder: '??' }
const ARTICLE: Record<Judgment, string> = { inaccuracy: 'an', mistake: 'a', blunder: 'a' }

/** Asked for in this visit and not started yet: another review is ahead of it. */
const requested = ref('')
watch(mainlineKey, () => (requested.value = ''))
const complete = computed(() => !!review.value?.complete && reviewWhole.value)
const reviews = useReviewStore()
/** Why the review asked for could not be done. */
const failure = computed(() => {
  const failed = reviews.status.failed
  return failed?.key === mainlineKey.value ? failed.message : ''
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
async function addPractice(color: 'white' | 'black'): Promise<void> {
  if (!review.value || !complete.value) return
  try {
    const count = await practice.add(review.value, color)
    practiceFeedback.value = count
      ? `${count} positions added to Practice → Your mistakes.`
      : 'No new verified mistakes to add.'
  } catch (cause) {
    console.warn('[analysis-review] Adding mistakes failed:', cause)
    practiceFeedback.value = cause instanceof Error ? cause.message : String(cause)
  }
}
const explanation = computed(() =>
  review.value && currentIndex.value > 0
    ? explainReviewedMove(review.value, currentIndex.value - 1)
    : '',
)

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
    name: origin.value?.[color] ?? (color === 'white' ? 'White' : 'Black'),
    side: result[color],
  }))
})

/* ── Chart ─────────────────────────────────────────────────────────── */

const chart = computed(() => {
  const result = gameAnalysis.value
  if (!result || !reviewedPlies.value) return null
  const moves = result.moves.slice(0, reviewedPlies.value)
  return {
    points: [result.startChances, ...moves.map((move) => move.chances)],
    judgments: moves.map((move) => move.judgment),
  }
})
/** The board's position along the reviewed main line, or −1 off it. */
const currentIndex = computed(() => {
  const moves = movesOf(path.value)
  if (moves.length > reviewedPlies.value) return -1
  return moves.every((uci, i) => mainline.value[i] === uci) ? moves.length : -1
})
function select(index: number): void {
  analysis.goTo(pathOf(mainline.value.slice(0, index)))
}

const PLURALS: Record<Judgment, string> = {
  inaccuracy: 'inaccuracies',
  mistake: 'mistakes',
  blunder: 'blunders',
}
const countLabel = (count: number, judgment: Judgment, name: string): string =>
  `${count} ${count === 1 ? judgment : PLURALS[judgment]} by ${name}: go to the next`

/** Jump to the next move of `color` with this label, after the one on the board (wrapping). */
function next(color: 'white' | 'black', judgment: Judgment): void {
  const marks = [...reviewMarks.value.values()].filter(
    (mark) => mark.color === color && mark.judgment === judgment,
  )
  const from = currentIndex.value
  const target = marks.find((mark) => mark.index + 1 > from) ?? marks[0]
  if (target) select(target.index + 1)
}

/* ── The move on the board ─────────────────────────────────────────── */

const currentMark = computed<ReviewMark | undefined>(() => reviewMarks.value.get(path.value))
const currentSan = computed(() => analysis.node.san)
/** Show the engine's move instead, as a variation. */
function showBest(): void {
  const best = currentMark.value?.best
  if (!best) return
  analysis.goTo(pathOf(movesOf(path.value).slice(0, -1)))
  analysis.play(best.uci)
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

    <table v-if="sides.length" class="review-table">
      <thead>
        <tr>
          <th scope="col"><span class="sr-only">Player</span></th>
          <th scope="col">Accuracy</th>
          <th scope="col" title="Inaccuracies" class="inaccuracy">?!</th>
          <th scope="col" title="Mistakes" class="mistake">?</th>
          <th scope="col" title="Blunders" class="blunder">??</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in sides" :key="row.color">
          <th scope="row">
            <span :class="['side-dot', row.color]" aria-hidden="true" />{{ row.name }}
          </th>
          <td class="tabular">
            {{ row.side.accuracy === undefined ? '—' : `${row.side.accuracy}%` }}
          </td>
          <td v-for="judgment in ['inaccuracy', 'mistake', 'blunder'] as const" :key="judgment">
            <button
              type="button"
              class="count"
              :class="judgment"
              :disabled="!row.side[judgment]"
              :aria-label="countLabel(row.side[judgment], judgment, row.name)"
              @click="next(row.color, judgment)"
            >
              {{ row.side[judgment] }}
            </button>
          </td>
        </tr>
      </tbody>
    </table>

    <div v-if="complete" class="flex flex-wrap gap-2">
      <UButton size="xs" variant="outline" @click="addPractice('white')"
        >Practice White's mistakes</UButton
      >
      <UButton size="xs" variant="outline" @click="addPractice('black')"
        >Practice Black's mistakes</UButton
      >
    </div>
    <p v-if="practiceFeedback" role="status" class="review-note">{{ practiceFeedback }}</p>
    <p v-if="explanation" class="review-note">{{ explanation }}</p>
    <EvalChart
      v-if="chart"
      :points="chart.points"
      :judgments="chart.judgments"
      :current="currentIndex"
      @select="select"
    />

    <p v-if="currentMark?.judgment" class="review-comment" :class="currentMark.judgment">
      <strong>{{ currentSan }}{{ GLYPHS[currentMark.judgment] }}</strong>
      is {{ ARTICLE[currentMark.judgment] }} {{ currentMark.judgment }}.
      <template v-if="currentMark.best">
        Best was <strong>{{ currentMark.best.san }}</strong
        >.
        <UButton size="xs" variant="link" class="px-0" @click="showBest">Show</UButton>
      </template>
    </p>
    <p v-else-if="!review && !canReview" class="review-note">
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
  font-size: 12.5px;
}
.review-table th,
.review-table td {
  padding: 2px 4px;
  text-align: center;
}
.review-table thead th {
  color: var(--ui-text-dimmed);
  font-size: 11px;
  font-weight: 600;
}
.review-table tbody th {
  max-width: 0;
  width: 100%;
  overflow: hidden;
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}
.side-dot {
  display: inline-block;
  width: 9px;
  height: 9px;
  margin-right: 6px;
  border: 1px solid var(--ui-border-accented);
  border-radius: 50%;
  vertical-align: -1px;
}
.side-dot.white {
  background: #f2f0ea;
}
.side-dot.black {
  background: #403d39;
}
.count {
  min-width: 26px;
  padding: 1px 6px;
  border: 0;
  border-radius: 5px;
  background: transparent;
  color: inherit;
  font: inherit;
  font-variant-numeric: tabular-nums;
  font-weight: 600;
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
.inaccuracy {
  color: var(--judgment-inaccuracy);
}
.mistake {
  color: var(--judgment-mistake);
}
.blunder {
  color: var(--judgment-blunder);
}
.count:disabled.inaccuracy,
.count:disabled.mistake,
.count:disabled.blunder {
  color: var(--ui-text-dimmed);
}
.review-comment {
  margin: 0;
  padding: 6px 8px;
  border-left: 3px solid currentColor;
  border-radius: 4px;
  background: var(--ui-bg-elevated);
  font-size: 12.5px;
  color: var(--ui-text);
}
.review-comment.inaccuracy {
  border-left-color: var(--judgment-inaccuracy);
}
.review-comment.mistake {
  border-left-color: var(--judgment-mistake);
}
.review-comment.blunder {
  border-left-color: var(--judgment-blunder);
}
</style>
