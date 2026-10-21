<script setup lang="ts">
import { computed, onUnmounted, ref, shallowRef, watch } from 'vue'
import { checkColor, destsFor, pgnFromUci } from '@kchess/rules/chess'
import type { OpeningDescription, PopularityMonth } from '@kchess/contracts/types'
import { openingAt, openingKey } from '@kchess/rules/openings'
import { RequestScope } from '@kchess/rules/requestScope'
import { INITIAL_FEN, Position, uciSquares } from '@kchess/rules/position'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import type { Key } from '@lichess-org/chessground/types'
import { useAnalysisStore } from '../../stores/analysis'
import { usePuzzleStore } from '../../stores/puzzles'
import { useRecentOpenings, useTrainOpening } from '../../composables/useOpenings'
import { openingLink } from './openingLink'
import { useReplaceAnalysis } from '../../composables/useReplaceAnalysis'

/** One opening: its board and moves, its popular continuations and games, and its drill. */
const props = defineProps<{ moves: readonly string[] }>()
const store = useKChessStore()
const { settings } = storeToRefs(store)
const analysis = useAnalysisStore()
const router = useRouter()

/** The moves as far as they are legal. */
const steps = computed(() => Position.initial().line(props.moves))
const line = computed(() => steps.value.map((step) => step.uci))
const fen = computed(() => steps.value.at(-1)?.fen ?? INITIAL_FEN)
const position = computed(() => Position.fromFen(fen.value))
const opening = computed(() => openingAt(STANDARD_SETUP, line.value))
const title = computed(() => opening.value?.name ?? 'Starting position')
const lastMove = computed(() => {
  const last = line.value.at(-1)
  return last ? (uciSquares(last) as Key[] | undefined) : undefined
})
const orientation = ref<'white' | 'black'>('white')

function go(moves: readonly string[]): void {
  void router.push(openingLink(moves))
}

const startTraining = useTrainOpening()
function train(): void {
  startTraining({ name: title.value, moves: line.value })
}
// The landing page lists the named openings shown here most recently.
const { remember } = useRecentOpenings()
watch(
  [opening, line],
  ([named, moves]) => {
    if (named) remember({ name: named.name, moves })
  },
  { immediate: true },
)
function trainPuzzles(): void {
  const name = opening.value?.name
  if (name) usePuzzleStore().startOpening(openingKey(name), name)
}

/** The line's Wikibooks page; nothing shows when it has none or it cannot be loaded. */
const description = shallowRef<OpeningDescription | null>(null)
const fullDescription = ref(false)
const shownBlocks = computed(() => {
  const blocks = description.value?.blocks ?? []
  if (fullDescription.value) return blocks
  const first = blocks.find((block) => !block.heading)
  return first ? [first] : []
})
/** Month by month, how often the line's last move was chosen; shown once there is a year of it. */
const popularity = shallowRef<PopularityMonth[]>([])
const popularityPoints = computed(() =>
  popularity.value.length >= 12
    ? popularity.value.map((month) => ({
        date: Date.UTC(Number(month.month.slice(0, 4)), Number(month.month.slice(5, 7)) - 1, 1),
        rating: Math.round(month.share * 1000) / 10,
      }))
    : [],
)
const popularityMax = computed(() =>
  Math.min(100, Math.max(1, ...popularityPoints.value.map((point) => point.rating)) * 1.15),
)
const percent = (value: number): string => `${value < 10 ? value.toFixed(1) : Math.round(value)}%`
const lastMoveSan = computed(() => steps.value.at(-1)?.san ?? '')
const extras = new RequestScope()
watch(
  line,
  (moves) => {
    const request = extras.next()
    description.value = null
    fullDescription.value = false
    popularity.value = []
    if (!moves.length) return
    // Plain copies: Vue's reactive arrays cannot cross IPC.
    window.kchess.openingDescription([...moves]).then(
      (page) => {
        if (request.current()) description.value = page
      },
      (cause: unknown) => console.warn('[openings] Loading the description failed:', cause),
    )
    window.kchess.openingPopularity([...moves]).then(
      (months) => {
        if (request.current()) popularity.value = months
      },
      (cause: unknown) => console.warn('[openings] Loading the popularity failed:', cause),
    )
  },
  { immediate: true },
)
onUnmounted(() => extras.invalidate())

const replaceAnalysis = useReplaceAnalysis()
function analyse(): void {
  const moves = line.value
  replaceAnalysis(() => {
    if (analysis.loadPgn(pgnFromUci(moves), moves.length)) store.selectPage('analysis')
  })
}
</script>

<template>
  <div v-board-fit class="play-layout">
    <div class="board-stack">
      <ChessBoard
        keep-move-input
        :fen="fen"
        :orientation="orientation"
        :theme="settings.boardTheme"
        :coordinates="settings.coordinates"
        :piece-set="settings.pieceSet"
        :animation="settings.pieceAnimation"
        movable
        interactive
        movable-color="both"
        :show-dests="settings.showLegalMoves"
        :dests="position ? destsFor(position) : undefined"
        :last-move="lastMove"
        :check="position ? checkColor(position) : false"
        :turn-color="position?.turn"
        @move="(uci: string) => go([...line, uci])"
      />
    </div>
    <div class="card side-panel">
      <div class="flex items-center gap-2">
        <UButton
          icon="i-lucide-arrow-left"
          variant="ghost"
          color="neutral"
          size="sm"
          :to="{ path: '/openings' }"
          >All openings</UButton
        >
        <span class="flex-1" />
        <UButton
          size="xs"
          variant="ghost"
          color="neutral"
          icon="i-lucide-repeat"
          aria-label="Flip board"
          @click="orientation = orientation === 'white' ? 'black' : 'white'"
        />
      </div>
      <div>
        <h2 class="section-title">{{ title }}</h2>
        <p v-if="opening" class="muted tabular text-sm">{{ opening.eco }}</p>
      </div>
      <ol v-if="steps.length" class="opening-moves" aria-label="Moves">
        <li v-for="(step, index) in steps" :key="index">
          <span v-if="index % 2 === 0" class="muted tabular">{{ index / 2 + 1 }}.</span>
          <button
            type="button"
            class="opening-move"
            :aria-current="index === steps.length - 1 ? 'step' : undefined"
            @click="go(line.slice(0, index + 1))"
          >
            {{ step.san }}
          </button>
        </li>
      </ol>
      <div class="panel-actions">
        <UButton icon="i-lucide-graduation-cap" :disabled="!steps.length" @click="train"
          >Train this line</UButton
        >
        <UButton variant="outline" color="neutral" icon="i-lucide-microscope" @click="analyse"
          >Analyse</UButton
        >
        <UButton
          v-if="opening"
          variant="outline"
          color="neutral"
          icon="i-lucide-puzzle"
          @click="trainPuzzles"
          >Train with puzzles</UButton
        >
      </div>
      <section v-if="description" class="opening-description" aria-label="Description">
        <template v-for="(block, index) in shownBlocks" :key="index">
          <h3 v-if="block.heading" class="text-sm font-semibold">{{ block.text }}</h3>
          <p v-else class="text-sm">{{ block.text }}</p>
        </template>
        <UButton
          v-if="description.blocks.length > shownBlocks.length || fullDescription"
          size="xs"
          variant="link"
          class="px-0"
          @click="fullDescription = !fullDescription"
          >{{ fullDescription ? 'Less' : 'More' }}</UButton
        >
        <p class="text-xs muted">
          Excerpt from
          <a :href="description.url" target="_blank" rel="noopener" class="underline">Wikibooks</a>
          ·
          <a
            href="https://creativecommons.org/licenses/by-sa/4.0/"
            target="_blank"
            rel="noopener"
            class="underline"
            >CC BY-SA 4.0</a
          >
        </p>
      </section>
      <section v-if="popularityPoints.length" aria-label="Popularity">
        <h3 class="text-xs font-semibold muted flex items-center gap-1">
          Popularity of {{ lastMoveSan }}
          <UTooltip
            text="Share of Lichess blitz, rapid and classical games from the previous position that continued with this move, by month."
          >
            <UIcon name="i-lucide-info" aria-label="About popularity" />
          </UTooltip>
        </h3>
        <LazyRatingChart
          :points="popularityPoints"
          label="Popularity"
          :format="percent"
          :y-domain="[0, popularityMax]"
          :height="110"
          :summary="`${lastMoveSan} was chosen in ${percent(popularityPoints[0]?.rating ?? 0)} of games in ${popularity[0]?.month} and ${percent(popularityPoints.at(-1)?.rating ?? 0)} in ${popularity.at(-1)?.month}`"
        />
      </section>
      <div class="opening-explorer panel-divider pt-3.5">
        <PositionExplorer :fen="fen" :moves="line" grid @play="(uci) => go([...line, uci])" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.opening-moves {
  display: flex;
  flex-wrap: wrap;
  gap: 2px 6px;
  font-size: 14px;
}
.opening-moves li {
  display: flex;
  gap: 4px;
}
.opening-move {
  padding: 0 3px;
  border-radius: 4px;
}
.opening-move:hover {
  background: var(--ui-bg-accented);
}
.opening-move[aria-current='step'] {
  font-weight: 600;
}
.opening-description {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.opening-explorer {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}
</style>
