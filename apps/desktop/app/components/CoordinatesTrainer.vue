<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useIntervalFn, useLocalStorage } from '@vueuse/core'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import type { RunSaved, RunSummary, VoiceOutcome } from '@kchess/contracts/types'
import { INITIAL_FEN } from '@kchess/rules/position'
import { EMPTY_FEN, FILES, RANKS, randomSquare, type Square } from '@kchess/rules/coordinates'
import { useCountdown } from '../utils/countdown'
import { play } from '../utils/sound'
import { formatRunClock } from '@kchess/rules/rush'
import { COORDINATE_GRAMMAR, spokenSquare } from '@kchess/rules/voiceCommands'
import type { VoiceResult } from '../utils/voiceCapture'
import { heardFields, logVoice } from '../utils/voiceLog'
import VoiceInput from './VoiceInput.vue'

/**
 * Board coordinates, the way Lichess trains them: name a square shown on the board, or find one that is
 * named for you. A timed run (30 seconds, as on Lichess, or longer) keeps a best score per length on
 * this computer; untimed practice runs until it is ended and keeps none.
 */
const store = useKChessStore()
const { settings } = storeToRefs(store)

/** Run lengths in seconds; 0 is untimed practice. */
const LENGTHS = [
  { label: '30s', value: 30 },
  { label: '1 min', value: 60 },
  { label: '2 min', value: 120 },
  { label: '5 min', value: 300 },
  { label: 'Untimed', value: 0 },
]
const length = useLocalStorage('kchess:coordinates-length', 30)
const untimed = computed(() => length.value === 0)
/** Whether the run on screen (or just ended) was untimed, whatever is chosen since. */
const runUntimed = ref(false)
const lengthLabel = computed(
  () => LENGTHS.find((item) => item.value === length.value)?.label ?? `${length.value}s`,
)
const mode = ref<'find' | 'name' | 'voice'>('find')
const voiceEnabled = ref(false)
const voiceControl = ref<InstanceType<typeof VoiceInput>>()
const voiceFeedback = ref('')
const starting = ref(false)
const runEpoch = ref(0)
const question = ref(0)
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
let mounted = true

const countdown = useCountdown(30_000, () => void finish())
/** Untimed practice counts up instead. */
const startedAt = ref(0)
const elapsed = ref(0)
const stopwatch = useIntervalFn(() => (elapsed.value = performance.now() - startedAt.value), 250, {
  immediate: false,
})
/** 30-second runs keep their original record names, so earlier bests still count. */
const recordVariant = computed(() => {
  const base = mode.value === 'voice' ? 'name-voice' : mode.value
  return length.value === 30 ? base : `${base}-${length.value}s`
})
const best = computed(() => summary.value?.best[recordVariant.value]?.score)
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
// Between runs the board already shows the chosen side; Random picks one when the run starts.
watch(side, (value) => {
  if (stage.value !== 'running' && value !== 'random') orientation.value = value
})
// An empty board looks the same from both sides, so between runs the pieces show which side is chosen.
const boardFen = computed(() => (stage.value === 'running' ? EMPTY_FEN : INITIAL_FEN))
const shapes = computed<DrawShape[]>(() => {
  const list: DrawShape[] = []
  if (stage.value === 'running' && mode.value !== 'find')
    list.push({ orig: target.value, brush: 'yellow' })
  if (flash.value) list.push({ orig: flash.value.square, brush: flash.value.brush })
  return list
})

onMounted(async () => {
  summary.value = await window.kchess.runSummary('coordinates')
  window.addEventListener('keydown', keydown)
})
onBeforeUnmount(() => {
  mounted = false
  window.removeEventListener('keydown', keydown)
  clearTimeout(flashTimer)
})

async function start(): Promise<void> {
  if (starting.value || stage.value === 'running') return
  if (mode.value === 'voice') {
    if (!voiceEnabled.value) return
    starting.value = true
    const ready = await voiceControl.value?.prepare()
    starting.value = false
    if (!mounted || !ready || !voiceEnabled.value || mode.value !== 'voice') return
  }
  runEpoch.value++
  question.value = 0
  voiceFeedback.value = ''
  orientation.value =
    side.value === 'random' ? (Math.random() < 0.5 ? 'white' : 'black') : side.value
  score.value = 0
  mistakes.value = 0
  pendingFile.value = ''
  saved.value = null
  target.value = randomSquare()
  upcoming.value = randomSquare(target.value)
  stage.value = 'running'
  runUntimed.value = untimed.value
  if (untimed.value) {
    startedAt.value = performance.now()
    elapsed.value = 0
    stopwatch.resume()
  } else countdown.start(length.value * 1000)
}

function advance(): void {
  question.value++
  target.value = upcoming.value
  upcoming.value = randomSquare(target.value)
  pendingFile.value = ''
}

/** The asked square is known, so every phrase logged here says exactly what was meant. */
let lastMiss: { asked: string; entry: Promise<number | undefined> } | undefined
function logAnswer(result: VoiceResult, outcome: VoiceOutcome, parsed?: string): void {
  const asked = `${runEpoch.value}:${question.value}`
  const retryOf = lastMiss?.asked === asked ? lastMiss.entry : undefined
  const entry = (async () =>
    logVoice({
      source: 'coordinates',
      ...heardFields(result),
      outcome,
      parsed,
      expected: target.value,
      retryOf: (await retryOf) ?? undefined,
    }))()
  if (outcome !== 'correct') lastMiss = { asked, entry }
}
function voiceMissed(result: VoiceResult): void {
  if (stage.value === 'running' && mode.value === 'voice') logAnswer(result, 'unclear')
}
function voiceAnswer(result: VoiceResult): void {
  if (stage.value !== 'running' || mode.value !== 'voice') return
  const square = spokenSquare(result.text)
  if (!square) {
    logAnswer(result, 'invalid')
    voiceFeedback.value = 'Say one square, such as “E four” or “Echo four”.'
    return
  }
  logAnswer(result, square === target.value ? 'correct' : 'wrong', square)
  voiceFeedback.value = square === target.value ? `${square} · correct` : `${square} · try again`
  answer(square)
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
  stopwatch.pause()
  stage.value = 'over'
  // An untimed score only grows with time, so it is never a record.
  if (runUntimed.value) return
  try {
    saved.value = await window.kchess.saveRun({
      kind: 'coordinates',
      variant: recordVariant.value,
      score: score.value,
      detail: { mistakes: mistakes.value, accuracy: accuracy.value, side: orientation.value },
    })
    summary.value = saved.value.summary
  } catch (cause) {
    console.warn('[coordinates-trainer] Could not save run:', cause)
    // The score is still on screen; only the record is lost.
  }
}
</script>

<template>
  <div v-board-fit class="play-layout">
    <div class="board-stack">
      <p class="sr-only" role="status" aria-live="polite">
        {{
          stage !== 'running'
            ? ''
            : mode === 'find'
              ? `Find ${target}`
              : `Name the marked square${pendingFile ? `: ${pendingFile}` : ''}`
        }}
      </p>
      <div class="board-shell coord-shell">
        <!-- As on Lichess: the square to find over the board, so no row above it takes height. -->
        <div v-if="stage === 'running' && mode === 'find'" class="coord-prompt" aria-hidden="true">
          <strong class="coord-target">{{ target }}</strong>
          <span class="coord-next">{{ upcoming }}</span>
        </div>
        <ChessBoard
          :fen="boardFen"
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
              { label: 'Say the square', value: 'voice', icon: 'i-lucide-mic' },
            ]"
            aria-labelledby="coord-mode"
            :content="false"
            variant="pill"
            class="w-full"
            :ui="{
              // Two modes on top, the third across the whole row: a grid the sliding highlight
              // cannot follow, so the active mode is coloured directly instead.
              list: 'grid grid-cols-2 w-full',
              indicator: 'hidden',
              trigger: 'rounded-md data-[state=active]:bg-primary last:col-span-2',
            }"
            :disabled="stage === 'running' || starting"
          />
        </div>
        <div class="field">
          <span id="coord-length" class="field-label">Length</span>
          <UTabs
            v-model="length"
            :items="LENGTHS"
            aria-labelledby="coord-length"
            :content="false"
            variant="pill"
            class="w-full"
            :ui="{ trigger: 'grow' }"
            :disabled="stage === 'running' || starting"
          />
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

      <VoiceInput
        v-if="mode === 'voice'"
        ref="voiceControl"
        v-model:enabled="voiceEnabled"
        :active="stage === 'running'"
        :context-key="`${runEpoch}:${question}:${stage}`"
        :grammar="COORDINATE_GRAMMAR"
        hint="Say “E four” or “Echo four”."
        :feedback="voiceFeedback"
        @result="voiceAnswer"
        @unclear="voiceMissed"
      />

      <div class="panel-divider" />
      <template v-if="stage === 'running'">
        <div class="run-hud">
          <div
            class="run-clock"
            :class="{ low: !runUntimed && countdown.left.value < 10_000 }"
            role="timer"
          >
            {{ formatRunClock(runUntimed ? elapsed : countdown.left.value) }}
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
          :class="!runUntimed && saved?.isBest ? 'win' : ''"
          role="status"
        >
          <UIcon
            :name="!runUntimed && saved?.isBest ? 'i-lucide-trophy' : 'i-lucide-timer'"
            class="status-icon"
          />
          <div>
            <strong>{{ score }} correct · {{ accuracy }}% accuracy</strong>
            <span class="detail">{{
              runUntimed
                ? `in ${formatRunClock(elapsed)}`
                : saved?.isBest
                  ? 'A new personal best!'
                  : `Best so far: ${best ?? score}`
            }}</span>
          </div>
        </div>
        <p v-else-if="!untimed" class="section-hint tabular">
          Best ({{ mode === 'find' ? 'find' : mode === 'voice' ? 'voice' : 'name' }},
          {{ lengthLabel }}): {{ best ?? '—' }}
        </p>
        <UButton
          size="lg"
          icon="i-lucide-play"
          :loading="starting"
          :disabled="mode === 'voice' && !voiceEnabled"
          @click="start"
          >{{ stage === 'over' ? 'Play again' : 'Start' }}</UButton
        >
      </template>
    </div>
  </div>
</template>

<style scoped>
.coord-shell {
  position: relative;
}
.coord-prompt {
  position: absolute;
  inset: 0;
  z-index: 3;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  /* Clicks go through to the squares underneath. */
  pointer-events: none;
  color: #fff;
  text-shadow:
    0 2px 12px rgb(0 0 0 / 55%),
    0 0 2px rgb(0 0 0 / 70%);
  font-variant-numeric: tabular-nums;
  line-height: 1;
}
.coord-target {
  font-size: calc(var(--board-size) * 0.2);
  letter-spacing: -0.02em;
  opacity: 0.9;
}
.coord-next {
  font-size: calc(var(--board-size) * 0.08);
  opacity: 0.6;
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
