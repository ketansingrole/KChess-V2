<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { SquareName } from '@kchess/rules/position'
import type { RunSaved, RunSummary, VoiceOutcome } from '@kchess/contracts/types'
import { knightChallenge, knightFen, knightMoves } from '@kchess/rules/knight'
import type { Square } from '@kchess/rules/coordinates'
import { useRunLength } from '../composables/useRunLength'
import RunLengthPicker from './RunLengthPicker.vue'
import { useZenSurface } from '../composables/useZenSurface'
import BoardViewToggles from './BoardViewToggles.vue'
import { playMoveSound } from '../utils/sound'
import { COORDINATE_GRAMMAR, spokenSquare } from '@kchess/rules/voiceCommands'
import type { VoiceResult } from '../utils/voiceCapture'
import { heardFields, logVoice } from '../utils/voiceLog'
import VoiceInput from './VoiceInput.vue'

/**
 * Knight vision: get the knight to the marked square in the fewest jumps. The shortest path is worked out
 * for you and told up front; the skill is finding it. A timed run (a minute, or another length) keeps a
 * best per difficulty and length on this computer; untimed practice runs until it is ended and keeps none.
 * Jumps are made on the board or by saying the square to land on.
 */
const store = useKChessStore()
const { settings } = storeToRefs(store)
useZenSurface(true)

const LEVELS = {
  easy: { label: 'Easy · 2–3 jumps', min: 2, max: 3 },
  medium: { label: 'Medium · 3–4 jumps', min: 3, max: 4 },
  hard: { label: 'Hard · 4–6 jumps', min: 4, max: 6 },
} as const
const level = ref<keyof typeof LEVELS>('medium')
const voiceEnabled = ref(false)
const voiceControl = ref<InstanceType<typeof VoiceInput>>()
const voiceFeedback = ref('')
const starting = ref(false)
/** Counts rounds, so a phrase said again after a miss can point at the first one. */
const rounds = ref(0)
let mounted = true
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

const run = useRunLength('kchess:knight-length', 60, () => void finish())
const { length, untimed, runUntimed, clock, low, label: lengthLabel } = run
const bestScore = computed(() => summary.value?.best[run.recordVariant(level.value)]?.score)
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
onBeforeUnmount(() => (mounted = false))

function nextRound(): void {
  rounds.value++
  const { min, max } = LEVELS[level.value]
  const round = knightChallenge(min, max)
  setBoard(round.from, round.to, round.best)
}

async function start(): Promise<void> {
  if (starting.value || stage.value === 'running') return
  if (voiceEnabled.value) {
    // The microphone and model are ready before the clock starts, as in Coordinates.
    starting.value = true
    const ready = await voiceControl.value?.prepare()
    starting.value = false
    if (!mounted || !ready) return
  }
  voiceFeedback.value = ''
  optimal.value = 0
  reached.value = 0
  missedRounds.value = 0
  notice.value = ''
  saved.value = null
  nextRound()
  stage.value = 'running'
  run.start()
}

function onMove(uci: string): void {
  jump(uci.slice(2, 4) as Square)
}

/** Takes the jump when it is one; says whether it was. */
function jump(to: Square): boolean {
  if (stage.value !== 'running' || !knightMoves(knight.value).includes(to)) return false
  knight.value = to
  jumps.value++
  playMoveSound()
  if (to !== goal.value) return true
  reached.value++
  if (jumps.value <= best.value) {
    optimal.value++
    notice.value = 'Shortest path!'
  } else {
    missedRounds.value++
    notice.value = `Reached it in ${jumps.value}; the shortest was ${best.value}.`
  }
  setTimeout(() => stage.value === 'running' && nextRound(), 450)
  return true
}

/** The goal is known, so every phrase logged here says what was being aimed for. */
let lastMiss: { asked: string; entry: Promise<number | undefined> } | undefined
function logAnswer(result: VoiceResult, outcome: VoiceOutcome, parsed?: string): void {
  const asked = `${rounds.value}:${jumps.value}`
  const retryOf = lastMiss?.asked === asked ? lastMiss.entry : undefined
  const entry = (async () =>
    logVoice({
      source: 'knightPath',
      ...heardFields(result),
      outcome,
      parsed,
      expected: goal.value,
      retryOf: (await retryOf) ?? undefined,
    }))()
  if (outcome !== 'played') lastMiss = { asked, entry }
}
function voiceMissed(result: VoiceResult): void {
  if (stage.value === 'running') logAnswer(result, 'unclear')
}
/** A spoken square is where the knight jumps to. */
function voiceAnswer(result: VoiceResult): void {
  if (stage.value !== 'running') return
  const square = spokenSquare(result.text)
  if (!square) {
    logAnswer(result, 'invalid')
    voiceFeedback.value = 'Say a square, such as “F three” or “Foxtrot three”.'
    return
  }
  if (!knightMoves(knight.value).includes(square)) {
    logAnswer(result, 'invalid', square)
    voiceFeedback.value = `${square} · not a knight jump from ${knight.value}`
    return
  }
  logAnswer(result, 'played', square)
  voiceFeedback.value = square
  jump(square)
}

function skipRound(): void {
  missedRounds.value++
  notice.value = `Skipped — the shortest path was ${best.value} jumps.`
  nextRound()
}

async function finish(): Promise<void> {
  if (stage.value !== 'running') return
  run.stop()
  stage.value = 'over'
  // An untimed score only grows with time, so it is never a record.
  if (runUntimed.value) return
  try {
    saved.value = await window.kchess.saveRun({
      kind: 'knightPath',
      variant: run.recordVariant(level.value),
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
  <div v-board-fit class="play-layout">
    <div class="board-stack">
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
          :show-dests="!settings.blindfold"
          :blindfold="settings.blindfold"
          square-input
          :dests="stage === 'running' ? dests : undefined"
          :shapes="shapes"
          @move="onMove"
        />
      </div>
    </div>

    <div class="card side-panel puzzle-panel">
      <!-- The view toggles share the first row rather than taking one of their own. -->
      <div class="flex items-center justify-between gap-2">
        <h2 class="sr-only">Knight paths</h2>
        <p class="section-hint">Reach the green square in the fewest moves.</p>
        <div class="flex gap-1"><BoardViewToggles /></div>
      </div>
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
      <RunLengthPicker v-model="length" :disabled="stage === 'running' || starting" />
      <VoiceInput
        ref="voiceControl"
        v-model:enabled="voiceEnabled"
        :active="stage === 'running'"
        :context-key="`${rounds}:${jumps}:${stage}`"
        :grammar="COORDINATE_GRAMMAR"
        hint="Say the square to jump to, such as “F three”."
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
            <strong class="tabular">{{ optimal }}</strong>
            <span>shortest paths</span>
          </div>
        </div>
        <!-- The target lives here rather than above the board, so the board keeps that height. -->
        <p class="knight-target" role="status" aria-live="polite">
          <strong>Reach {{ goal }} in {{ best }} {{ best === 1 ? 'jump' : 'jumps' }}</strong>
          <span class="muted tabular">{{ jumps }} so far</span>
        </p>
        <p v-if="notice" class="section-hint" role="status">{{ notice }}</p>
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
          :class="!runUntimed && saved?.isBest ? 'win' : ''"
          role="status"
        >
          <UIcon
            :name="!runUntimed && saved?.isBest ? 'i-lucide-trophy' : 'i-lucide-timer'"
            class="status-icon"
          />
          <div>
            <strong>{{ optimal }} shortest paths · {{ reached }} reached</strong>
            <span class="detail">{{
              runUntimed
                ? `in ${clock}`
                : saved?.isBest
                  ? 'A new personal best!'
                  : `Best so far: ${bestScore ?? optimal}`
            }}</span>
          </div>
        </div>
        <p v-else-if="!untimed" class="section-hint tabular">
          Best ({{ level }}, {{ lengthLabel }}): {{ bestScore ?? '—' }}
        </p>
        <UButton size="lg" icon="i-lucide-play" :loading="starting" @click="start">{{
          stage === 'over' ? 'Play again' : 'Start'
        }}</UButton>
      </template>
    </div>
  </div>
</template>

<style scoped>
.knight-target {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  font-size: 18px;
}
</style>
