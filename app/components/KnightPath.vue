<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { SquareName } from 'chessops/types'
import type { RunSaved, RunSummary } from '../../src/shared/types'
import { knightChallenge, knightFen, knightMoves } from '../../src/shared/knight'
import type { Square } from '../../src/shared/coordinates'
import { useCountdown } from '../utils/countdown'
import { formatRunClock } from '../../src/shared/rush'
import { playMoveSound } from '../utils/sound'

/**
 * Knight vision: get the knight to the marked square in the fewest jumps. The shortest path is worked out
 * for you and told up front; the skill is finding it. Sixty seconds, best score kept on this computer.
 */
const store = useKChessStore()
const { settings } = storeToRefs(store)

const LEVELS = {
  easy: { label: 'Easy · 2–3 jumps', min: 2, max: 3 },
  medium: { label: 'Medium · 3–4 jumps', min: 3, max: 4 },
  hard: { label: 'Hard · 4–6 jumps', min: 4, max: 6 },
} as const
const level = ref<keyof typeof LEVELS>('medium')
const stage = ref<'idle' | 'running' | 'over'>('idle')
const knight = ref<Square>('a1')
const goal = ref<Square>('h8')
const best = ref(0)
const jumps = ref(0)
const optimal = ref(0)
const reached = ref(0)
const missedRounds = ref(0)
const notice = ref('')
const saved = ref<RunSaved | null>(null)
const summary = ref<RunSummary | null>(null)

const countdown = useCountdown(60_000, () => void finish())
const bestScore = computed(() => summary.value?.best[level.value]?.score)
const fen = computed(() => knightFen(knight.value))
const dests = computed(
  () =>
    new Map<SquareName, SquareName[]>([
      [knight.value as SquareName, knightMoves(knight.value) as SquareName[]],
    ]),
)
const shapes = computed<DrawShape[]>(() =>
  stage.value === 'running' ? [{ orig: goal.value, brush: 'green' }] : [],
)

onMounted(async () => {
  summary.value = await window.kchess.runSummary('knightPath')
  setBoard('e4', 'e4', 0)
})

function setBoard(from: Square, to: Square, shortest: number): void {
  knight.value = from
  goal.value = to
  best.value = shortest
  jumps.value = 0
}
function nextRound(): void {
  const { min, max } = LEVELS[level.value]
  const round = knightChallenge(min, max)
  setBoard(round.from, round.to, round.best)
}

function start(): void {
  optimal.value = 0
  reached.value = 0
  missedRounds.value = 0
  notice.value = ''
  saved.value = null
  nextRound()
  stage.value = 'running'
  countdown.start()
}

function onMove(uci: string): void {
  if (stage.value !== 'running') return
  const to = uci.slice(2, 4) as Square
  if (!knightMoves(knight.value).includes(to)) return
  knight.value = to
  jumps.value++
  playMoveSound()
  if (to !== goal.value) return
  reached.value++
  if (jumps.value <= best.value) {
    optimal.value++
    notice.value = 'Shortest path!'
  } else {
    missedRounds.value++
    notice.value = `Reached it in ${jumps.value}; the shortest was ${best.value}.`
  }
  setTimeout(() => stage.value === 'running' && nextRound(), 450)
}

function skipRound(): void {
  missedRounds.value++
  notice.value = `Skipped — the shortest path was ${best.value} jumps.`
  nextRound()
}

async function finish(): Promise<void> {
  if (stage.value !== 'running') return
  countdown.stop()
  stage.value = 'over'
  try {
    saved.value = await window.kchess.saveRun({
      kind: 'knightPath',
      variant: level.value,
      score: optimal.value,
      detail: { reached: reached.value, skipped: missedRounds.value },
    })
    summary.value = saved.value.summary
  } catch (cause) {
    console.warn('[knight-path] Could not save run:', cause)
    // The score is still on screen; only the record is lost.
  }
}
</script>

<template>
  <div class="play-layout">
    <div class="board-stack">
      <div class="coord-prompt" role="status" aria-live="polite">
        <template v-if="stage === 'running'">
          <strong class="knight-target"
            >Reach {{ goal }} in {{ best }} {{ best === 1 ? 'jump' : 'jumps' }}</strong
          >
          <span class="muted tabular">{{ jumps }} so far</span>
        </template>
        <span v-else class="muted">Press Start to begin a 60-second run.</span>
      </div>
      <div class="board-shell">
        <ChessBoard
          :fen="fen"
          orientation="white"
          :theme="settings.boardTheme"
          :coordinates="settings.coordinates === 'none' ? 'inside' : settings.coordinates"
          :piece-set="settings.pieceSet"
          :animation="settings.pieceAnimation"
          :movable="stage === 'running'"
          :interactive="stage === 'running'"
          movable-color="white"
          turn-color="white"
          :show-dests="true"
          :dests="stage === 'running' ? dests : undefined"
          :shapes="shapes"
          @move="onMove"
        />
      </div>
    </div>

    <div class="card side-panel puzzle-panel">
      <h2 class="sr-only">Knight paths</h2>
      <p class="section-hint">Reach the green square in the fewest moves.</p>
      <div class="field">
        <span id="knight-level" class="field-label">Difficulty</span>
        <USelect
          v-model="level"
          :items="Object.entries(LEVELS).map(([value, entry]) => ({ value, label: entry.label }))"
          aria-labelledby="knight-level"
          size="sm"
          :disabled="stage === 'running'"
          :ui="{ base: 'w-full' }"
        />
      </div>
      <div class="panel-divider" />
      <template v-if="stage === 'running'">
        <div class="run-hud">
          <div class="run-clock" :class="{ low: countdown.left.value < 10_000 }" role="timer">
            {{ formatRunClock(countdown.left.value) }}
          </div>
          <div class="run-score">
            <strong class="tabular">{{ optimal }}</strong>
            <span>shortest paths</span>
          </div>
        </div>
        <p class="section-hint" role="status">{{ notice || 'Find the shortest path.' }}</p>
        <UButton variant="outline" color="neutral" icon="i-lucide-skip-forward" @click="skipRound"
          >Skip</UButton
        >
        <UButton variant="ghost" color="neutral" icon="i-lucide-flag" @click="finish"
          >End run</UButton
        >
      </template>
      <template v-else>
        <div
          v-if="stage === 'over'"
          class="status-banner"
          :class="saved?.isBest ? 'win' : ''"
          role="status"
        >
          <UIcon :name="saved?.isBest ? 'i-lucide-trophy' : 'i-lucide-timer'" class="status-icon" />
          <div>
            <strong>{{ optimal }} shortest paths · {{ reached }} reached</strong>
            <span class="detail">{{
              saved?.isBest ? 'A new personal best!' : `Best so far: ${bestScore ?? optimal}`
            }}</span>
          </div>
        </div>
        <p v-else class="section-hint tabular">Best ({{ level }}): {{ bestScore ?? '—' }}</p>
        <UButton size="lg" icon="i-lucide-play" @click="start">{{
          stage === 'over' ? 'Play again' : 'Start'
        }}</UButton>
      </template>
    </div>
  </div>
</template>

<style scoped>
.coord-prompt {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  min-height: 44px;
  padding: 0 4px;
}
.knight-target {
  font-size: 22px;
}
</style>
