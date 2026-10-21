<script setup lang="ts">
import { computed, ref } from 'vue'
import { useOnline } from '@vueuse/core'
import { moveNumber, pathOf } from '@kchess/rules/analysisTree'
import { useAnalysisStore } from '../stores/analysis'
import { usePuzzleAnalysis, variationNode } from '../stores/puzzleAnalysis'
import { usePuzzleStore } from '../stores/puzzles'
import { useReplaceAnalysis } from '../composables/useReplaceAnalysis'
import { puzzleThemeText } from '../utils/puzzleThemes'
import { errorText } from '../utils/format'

/**
 * The puzzle a study on the analysis board came from: the moves tried in its attempt, trying it
 * again, and the game it was taken from.
 */
const store = useKChessStore()
const analysis = useAnalysisStore()
const studied = usePuzzleAnalysis()
const puzzles = usePuzzleStore()
const replaceAnalysis = useReplaceAnalysis()
const online = useOnline()
/** Between the puzzle and its game: asks first only when the board has analysis of its own. */
const replace = (load: () => void): void => (studied.untouched() ? load() : replaceAnalysis(load))

const puzzle = computed(() => analysis.origin?.puzzle)
/** The board holds the puzzle itself, rather than the game it comes from. */
const onPuzzle = computed(() => !!puzzle.value && studied.holds(puzzle.value))
const themes = computed(() => (puzzle.value ? puzzleThemeText(puzzle.value.themes) : ''))
/** The moves tried instead of the solution's, where they still are on the board. */
const tried = computed(() => {
  const shown = puzzle.value
  if (!shown || !onPuzzle.value) return []
  return shown.wrongMoves.flatMap(({ index, uci }) => {
    const path = pathOf([...shown.solution.slice(0, index), uci])
    const node = variationNode(analysis.root, path)
    return node ? [{ path, label: `${moveNumber(node.ply - 1, true)} ${node.san}` }] : []
  })
})

const fetching = ref(false)
const error = ref('')
async function openSource(): Promise<void> {
  const shown = puzzle.value
  if (!shown?.gameId) return
  fetching.value = true
  error.value = ''
  let pgn: string
  try {
    pgn = await window.kchess.exportGame(shown.gameId)
  } catch (cause) {
    console.warn('[analysis-puzzle] Fetching the source game failed:', cause)
    // A game opened before comes back without a connection; this one never was.
    error.value = online.value
      ? errorText(cause)
      : 'Can’t reach Lichess. Connect to the internet to open this game.'
    return
  } finally {
    fetching.value = false
  }
  replace(() => {
    if (!studied.showSource(shown, pgn)) error.value = 'That game could not be read.'
  })
}
function backToPuzzle(): void {
  const shown = puzzle.value
  if (shown) replace(() => void studied.open(shown, shown.wrongMoves))
}
function tryAgain(): void {
  const shown = puzzle.value
  if (!shown) return
  puzzles.retry(shown)
  store.selectPage('puzzles')
}
</script>

<template>
  <section v-if="puzzle" class="analysis-puzzle" aria-label="Puzzle">
    <div class="puzzle-head">
      <UIcon name="i-lucide-puzzle" class="puzzle-icon" />
      <strong>Puzzle</strong>
      <span class="tabular muted">{{ puzzle.rating }}</span>
      <span v-if="themes" class="puzzle-themes muted" :title="themes">{{ themes }}</span>
    </div>
    <p v-if="tried.length" class="puzzle-tried">
      You played
      <template v-for="(move, index) in tried" :key="move.path"
        ><template v-if="index">, </template
        ><button type="button" class="tried-move" @click="analysis.goTo(move.path)">
          {{ move.label }}
        </button></template
      >
    </p>
    <div class="puzzle-actions">
      <UButton
        size="xs"
        variant="outline"
        color="neutral"
        icon="i-lucide-rotate-ccw"
        @click="tryAgain"
        >Try again</UButton
      >
      <UButton
        v-if="onPuzzle && puzzle.gameId"
        size="xs"
        variant="outline"
        color="neutral"
        icon="i-lucide-swords"
        :loading="fetching"
        @click="openSource"
        >Source game</UButton
      >
      <UButton
        v-else-if="!onPuzzle"
        size="xs"
        variant="outline"
        color="neutral"
        icon="i-lucide-puzzle"
        @click="backToPuzzle"
        >Back to the puzzle</UButton
      >
    </div>
    <p v-if="error" class="text-error text-xs" role="alert">{{ error }}</p>
  </section>
</template>

<style scoped>
.analysis-puzzle {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px 10px;
  border-radius: 10px;
  background: var(--ui-bg-elevated);
  font-size: 13px;
}
.puzzle-head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.puzzle-icon {
  flex: none;
  color: var(--ui-text-muted);
}
.puzzle-themes {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.puzzle-tried {
  margin: 0;
  color: var(--ui-text-muted);
}
.tried-move {
  padding: 0 2px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--judgment-blunder);
  font: inherit;
  font-weight: 600;
  cursor: pointer;
}
.tried-move:hover {
  background: var(--ui-bg-accented);
}
.puzzle-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
</style>
