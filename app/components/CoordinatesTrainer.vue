<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import type { RunSaved, RunSummary } from '../../src/shared/types'
import { EMPTY_FEN, FILES, RANKS, randomSquare, type Square } from '../utils/coordinates'
import { useCountdown } from '../utils/countdown'
import { play } from '../utils/sound'
import { formatRunClock } from '../utils/rush'

/**
 * Board coordinates, the way Lichess trains them: name a square shown on the board, or find one that is
 * named for you. Thirty seconds, best score kept on this computer.
 */
const store = useKChessStore()
const { settings } = storeToRefs(store)

const RUN_MS = 30_000
const mode = ref<'find' | 'name'>('find')
const side = ref<'white' | 'black' | 'random'>('white')
const showCoordinates = ref(false)
const stage = ref<'idle' | 'running' | 'over'>('idle')
const orientation = ref<'white' | 'black'>('white')
const target = ref<Square>('e4')
const upcoming = ref<Square>('d5')
const score = ref(0)
const mistakes = ref(0)
const pendingFile = ref('')
const flash = ref<{ square: Key; brush: 'green' | 'red' } | null>(null)
const saved = ref<RunSaved | null>(null)
const summary = ref<RunSummary | null>(null)
let flashTimer: ReturnType<typeof setTimeout> | undefined

const countdown = useCountdown(RUN_MS, () => void finish())
const best = computed(() => summary.value?.best[mode.value]?.score)
const accuracy = computed(() =>
  score.value + mistakes.value
    ? Math.round((score.value / (score.value + mistakes.value)) * 100)
    : 0,
)
const boardCoordinates = computed(() =>
  showCoordinates.value
    ? settings.value.coordinates === 'none'
      ? 'inside'
      : settings.value.coordinates
    : 'none',
)
const shapes = computed<DrawShape[]>(() => {
  const list: DrawShape[] = []
  if (stage.value === 'running' && mode.value === 'name')
    list.push({ orig: target.value, brush: 'yellow' })
  if (flash.value) list.push({ orig: flash.value.square, brush: flash.value.brush })
  return list
})

onMounted(async () => {
  summary.value = await window.kchess.runSummary('coordinates')
  window.addEventListener('keydown', keydown)
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', keydown)
  clearTimeout(flashTimer)
})

function start(): void {
  orientation.value =
    side.value === 'random' ? (Math.random() < 0.5 ? 'white' : 'black') : side.value
  score.value = 0
  mistakes.value = 0
  pendingFile.value = ''
  saved.value = null
  target.value = randomSquare()
  upcoming.value = randomSquare(target.value)
  stage.value = 'running'
  countdown.start()
}

function advance(): void {
  target.value = upcoming.value
  upcoming.value = randomSquare(target.value)
  pendingFile.value = ''
}

function answer(square: string): void {
  if (stage.value !== 'running') return
  const correct = square === target.value
  clearTimeout(flashTimer)
  flash.value = { square: square as Key, brush: correct ? 'green' : 'red' }
  flashTimer = setTimeout(() => (flash.value = null), 350)
  if (correct) {
    score.value++
    advance()
  } else {
    mistakes.value++
    pendingFile.value = ''
    void play('lowTime', 0.5)
  }
}

/** Find mode: the click on the board is the answer. */
function onSelect(square: Key): void {
  if (mode.value === 'find') answer(square)
}

/** Name mode: a file then a rank, from the buttons or the keyboard. */
function press(key: string): void {
  if (mode.value !== 'name' || stage.value !== 'running') return
  if ((FILES as readonly string[]).includes(key)) pendingFile.value = key
  else if ((RANKS as readonly string[]).includes(key) && pendingFile.value)
    answer(`${pendingFile.value}${key}`)
}
function keydown(event: KeyboardEvent): void {
  if (event.metaKey || event.ctrlKey || event.altKey) return
  const key = event.key.toLowerCase()
  if (key.length === 1) press(key)
}

async function finish(): Promise<void> {
  if (stage.value !== 'running') return
  countdown.stop()
  stage.value = 'over'
  try {
    saved.value = await window.kchess.saveRun({
      kind: 'coordinates',
      variant: mode.value,
      score: score.value,
      detail: { mistakes: mistakes.value, accuracy: accuracy.value, side: orientation.value },
    })
    summary.value = saved.value.summary
  } catch {
    // The score is still on screen; only the record is lost.
  }
}
</script>

<template>
  <div class="play-layout">
    <div class="board-stack">
      <div class="coord-prompt" role="status" aria-live="polite">
        <template v-if="stage === 'running' && mode === 'find'">
          <strong class="coord-target">{{ target }}</strong>
          <span class="muted tabular">next: {{ upcoming }}</span>
        </template>
        <template v-else-if="stage === 'running'">
          <strong class="coord-target">{{ pendingFile || '?' }}?</strong>
          <span class="muted">Name the marked square</span>
        </template>
        <span v-else class="muted">Press Start to begin a 30-second run.</span>
      </div>
      <div class="board-shell">
        <ChessBoard
          :fen="EMPTY_FEN"
          :orientation="orientation"
          :theme="settings.boardTheme"
          :coordinates="boardCoordinates"
          :piece-set="settings.pieceSet"
          :animation="settings.pieceAnimation"
          :movable="false"
          :interactive="false"
          :shapes="shapes"
          @select="onSelect"
        />
      </div>
    </div>

    <div class="card side-panel puzzle-panel">
      <h2 class="sr-only">Board coordinates</h2>
      <div class="form-stack">
        <div class="field">
          <span id="coord-mode" class="field-label">Mode</span>
          <UTabs
            v-model="mode"
            :items="[
              { label: 'Find the square', value: 'find', icon: 'i-lucide-mouse-pointer-click' },
              { label: 'Name the square', value: 'name', icon: 'i-lucide-keyboard' },
            ]"
            aria-labelledby="coord-mode"
            size="sm"
            :content="false"
            variant="pill"
            class="w-full"
            :ui="{ trigger: 'grow' }"
            :disabled="stage === 'running'"
          />
          <span class="field-hint">
            {{
              mode === 'find'
                ? 'A square is named above the board: click it.'
                : 'A square is marked on the board: type its file and rank (or use the buttons).'
            }}
          </span>
        </div>
        <div class="field">
          <span id="coord-side" class="field-label">Board from the side of</span>
          <UTabs
            v-model="side"
            :items="[
              { label: 'White', value: 'white' },
              { label: 'Black', value: 'black' },
              { label: 'Random', value: 'random' },
            ]"
            aria-labelledby="coord-side"
            size="sm"
            :content="false"
            variant="pill"
            class="w-full"
            :ui="{ trigger: 'grow' }"
            :disabled="stage === 'running'"
          />
        </div>
        <USwitch
          v-model="showCoordinates"
          label="Show coordinates on the board"
          :disabled="stage === 'running'"
        />
      </div>

      <div class="panel-divider" />
      <template v-if="stage === 'running'">
        <div class="run-hud">
          <div class="run-clock" :class="{ low: countdown.left.value < 10_000 }" role="timer">
            {{ formatRunClock(countdown.left.value) }}
          </div>
          <div class="run-score">
            <strong class="tabular">{{ score }}</strong>
            <span>correct</span>
          </div>
        </div>
        <div v-if="mode === 'name'" class="coord-pad" aria-label="Answer">
          <div class="coord-row" role="group" aria-label="File">
            <UButton
              v-for="file in FILES"
              :key="file"
              size="sm"
              :variant="pendingFile === file ? 'solid' : 'outline'"
              :color="pendingFile === file ? 'primary' : 'neutral'"
              @click="press(file)"
              >{{ file }}</UButton
            >
          </div>
          <div class="coord-row" role="group" aria-label="Rank">
            <UButton
              v-for="rank in RANKS"
              :key="rank"
              size="sm"
              variant="outline"
              color="neutral"
              :disabled="!pendingFile"
              @click="press(rank)"
              >{{ rank }}</UButton
            >
          </div>
        </div>
        <UButton variant="outline" color="neutral" icon="i-lucide-flag" @click="finish"
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
            <strong>{{ score }} correct · {{ accuracy }}% accuracy</strong>
            <span class="detail">{{
              saved?.isBest ? 'A new personal best!' : `Best so far: ${best ?? score}`
            }}</span>
          </div>
        </div>
        <p v-else class="section-hint tabular">
          Best ({{ mode === 'find' ? 'find' : 'name' }}): {{ best ?? '—' }}
        </p>
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
.coord-target {
  font-size: 34px;
  letter-spacing: -0.02em;
  font-variant-numeric: tabular-nums;
}
.coord-pad {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.coord-row {
  display: grid;
  grid-template-columns: repeat(8, minmax(0, 1fr));
  gap: 4px;
}
</style>
