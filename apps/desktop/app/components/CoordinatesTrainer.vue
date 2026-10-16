<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import type { RunSaved, RunSummary, VoiceOutcome } from '@kchess/contracts/types'
import { INITIAL_FEN } from '@kchess/rules/position'
import {
  EMPTY_FEN,
  FILES,
  RANKS,
  randomSquare,
  squaresOn,
  type Square,
} from '@kchess/rules/coordinates'
import { useRunLength } from '../composables/useRunLength'
import RunLengthPicker from './RunLengthPicker.vue'
import { useZenSurface } from '../composables/useZenSurface'
import BoardViewToggles from './BoardViewToggles.vue'
import { play } from '../utils/sound'
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
useZenSurface(true)

const mode = ref<'find' | 'name' | 'voice'>('find')
const voiceEnabled = ref(false)
const voiceControl = ref<InstanceType<typeof VoiceInput>>()
const voiceFeedback = ref('')
const starting = ref(false)
const runEpoch = ref(0)
const question = ref(0)
const side = ref<'white' | 'black' | 'random'>('white')
const showCoordinates = useLocalStorage('kchess:coordinates-show', false)
const onSquares = useLocalStorage('kchess:coordinates-on-squares', false)
const showPieces = useLocalStorage('kchess:coordinates-pieces', false)
/** Practice only some files and ranks; such runs are not records. */
const limited = useLocalStorage('kchess:coordinates-limited', false)
const storedFiles = useLocalStorage<string[]>('kchess:coordinates-files', [...FILES])
const storedRanks = useLocalStorage<string[]>('kchess:coordinates-ranks', [...RANKS])
/** The stored choice, in board order; all of them when nothing valid was stored. */
function chosen<T extends string>(all: readonly T[], stored: unknown): T[] {
  const list = all.filter((item) => Array.isArray(stored) && stored.includes(item))
  return list.length ? list : [...all]
}
const files = computed(() => chosen(FILES, storedFiles.value))
const ranks = computed(() => chosen(RANKS, storedRanks.value))
/** Turn one file or rank on or off, always keeping at least one. */
function toggle(list: readonly string[], item: string): string[] {
  if (!list.includes(item)) return [...list, item]
  return list.length > 1 ? list.filter((other) => other !== item) : [...list]
}
const allowed = computed(() => (limited.value ? squaresOn(files.value, ranks.value) : undefined))
const runLimited = ref(false)
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

const run = useRunLength('kchess:coordinates-length', 30, () => void finish())
const { length, untimed, runUntimed, clock, low, label: lengthLabel } = run
const recordVariant = computed(() =>
  run.recordVariant(mode.value === 'voice' ? 'name-voice' : mode.value),
)
const best = computed(() => summary.value?.best[recordVariant.value]?.score)
const accuracy = computed(() =>
  score.value + mistakes.value
    ? Math.round((score.value / (score.value + mistakes.value)) * 100)
    : 0,
)
const boardCoordinates = computed(() =>
  showCoordinates.value || onSquares.value
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
const boardFen = computed(() =>
  stage.value === 'running' && !showPieces.value ? EMPTY_FEN : INITIAL_FEN,
)
/** Each side's recent timed runs of this mode and length, oldest first, and their average. */
const sideScores = computed(() => {
  const variant = recordVariant.value
  const runs = (summary.value?.recent ?? []).filter((entry) => entry.variant === variant)
  return (['white', 'black'] as const)
    .map((color) => {
      const scores = runs
        .filter((entry) => entry.detail.side === color)
        .map((entry) => entry.score)
        .reverse()
      const average = scores.reduce((sum, value) => sum + value, 0) / (scores.length || 1)
      return { color, scores, average: Math.round(average * 100) / 100 }
    })
    .filter((side) => side.scores.length)
})
const shapes = computed<DrawShape[]>(() => {
  const list: DrawShape[] = []
  if (stage.value === 'running' && mode.value !== 'find')
    list.push({ orig: target.value, brush: 'yellow' })
  if (flash.value) list.push({ orig: flash.value.square, brush: flash.value.brush })
  return list
})

onMounted(async () => {
  // Keys work at once; the saved scores are optional and may arrive after the drill has closed.
  window.addEventListener('keydown', keydown)
  try {
    const loaded = await window.kchess.runSummary('coordinates')
    if (mounted) summary.value = loaded
  } catch (cause) {
    console.warn('[coordinates-trainer] Could not load saved runs:', cause)
  }
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
  runLimited.value = limited.value
  target.value = randomSquare(undefined, Math.random, allowed.value)
  upcoming.value = randomSquare(target.value, Math.random, allowed.value)
  stage.value = 'running'
  run.start()
}

function advance(): void {
  question.value++
  target.value = upcoming.value
  upcoming.value = randomSquare(target.value, Math.random, allowed.value)
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
  run.stop()
  stage.value = 'over'
  // An untimed score only grows with time, and some files and ranks are easier: never records.
  if (runUntimed.value || runLimited.value) return
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
          :coordinates-on-squares="onSquares"
          :piece-set="settings.pieceSet"
          :animation="settings.pieceAnimation"
          :movable="false"
          :interactive="false"
          :shapes="shapes"
          :blindfold="settings.blindfold"
          @select="onSelect"
        />
      </div>
    </div>

    <div class="card side-panel puzzle-panel">
      <div class="form-stack">
        <div class="field">
          <!-- The view toggles share the first label's row rather than taking one of their own. -->
          <div class="flex items-center justify-between gap-2">
            <h2 class="sr-only">Board coordinates</h2>
            <span id="coord-mode" class="field-label">Mode</span>
            <div class="flex gap-1"><BoardViewToggles /></div>
          </div>
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
        <RunLengthPicker v-model="length" :disabled="stage === 'running' || starting" />
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
          v-model="limited"
          label="Practice only some files & ranks"
          :disabled="stage === 'running'"
        />
        <div v-if="limited" class="coord-pad">
          <div class="coord-row" role="group" aria-label="Files to practice">
            <UButton
              v-for="file in FILES"
              :key="file"
              size="sm"
              :variant="files.includes(file) ? 'solid' : 'outline'"
              :color="files.includes(file) ? 'primary' : 'neutral'"
              :aria-pressed="files.includes(file)"
              :disabled="stage === 'running'"
              @click="storedFiles = toggle(files, file)"
              >{{ file }}</UButton
            >
          </div>
          <div class="coord-row" role="group" aria-label="Ranks to practice">
            <UButton
              v-for="rank in RANKS"
              :key="rank"
              size="sm"
              :variant="ranks.includes(rank) ? 'solid' : 'outline'"
              :color="ranks.includes(rank) ? 'primary' : 'neutral'"
              :aria-pressed="ranks.includes(rank)"
              :disabled="stage === 'running'"
              @click="storedRanks = toggle(ranks, rank)"
              >{{ rank }}</UButton
            >
          </div>
        </div>
        <USwitch
          v-model="showCoordinates"
          label="Show coordinates"
          :disabled="stage === 'running'"
        />
        <USwitch
          v-model="onSquares"
          label="Coordinates on every square"
          :disabled="stage === 'running'"
        />
        <USwitch v-model="showPieces" label="Show pieces" :disabled="stage === 'running'" />
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
          <div class="run-clock" :class="{ low }" role="timer">
            {{ clock }}
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
              runLimited
                ? 'Not saved: some files & ranks only'
                : runUntimed
                  ? `in ${clock}`
                  : saved?.isBest
                    ? 'A new personal best!'
                    : `Best so far: ${best ?? score}`
            }}</span>
          </div>
        </div>
        <div v-if="!untimed && sideScores.length" class="coord-averages">
          <div v-for="row in sideScores" :key="row.color">
            <p class="text-sm">
              Average as {{ row.color === 'white' ? 'White' : 'Black' }}:
              <strong class="tabular">{{ row.average }}</strong>
            </p>
            <ScoreSparkline
              v-if="row.scores.length > 1"
              :scores="row.scores"
              :label="`Recent scores as ${row.color === 'white' ? 'White' : 'Black'}: ${row.scores.join(', ')}`"
            />
          </div>
        </div>
        <p v-if="stage !== 'over' && !untimed" class="section-hint tabular">
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
/* Every square named the way the drill asks for it (e4, not E4). */
.coord-shell :deep(coords.squares) {
  text-transform: none;
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
.coord-averages {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.coord-row {
  display: grid;
  grid-template-columns: repeat(8, minmax(0, 1fr));
  gap: 4px;
}
</style>
