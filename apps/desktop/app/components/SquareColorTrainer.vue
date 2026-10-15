<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import type { RunSaved, RunSummary, VoiceOutcome } from '@kchess/contracts/types'
import { randomSquare, squareColor, type Square } from '@kchess/rules/coordinates'
import { useCountdown } from '../utils/countdown'
import { formatRunClock } from '@kchess/rules/rush'
import { play } from '../utils/sound'
import type { VoiceResult } from '../utils/voiceCapture'
import { heardFields, logVoice } from '../utils/voiceLog'
import VoiceInput from './VoiceInput.vue'

/**
 * Is this square light or dark? Answer with the buttons, L / D, or by voice. Thirty seconds; the
 * best score stays on this computer.
 */
/** Only the two answers, so the recognizer never has to choose between near-sounding words. */
const COLOR_GRAMMAR = ['light', 'dark', '[unk]']
const voiceEnabled = ref(false)
const voiceControl = ref<InstanceType<typeof VoiceInput>>()
const voiceFeedback = ref('')
const starting = ref(false)
let mounted = true
const stage = ref<'idle' | 'running' | 'over'>('idle')
const square = ref<Square>('e4')
const score = ref(0)
/** Counts squares asked, so a phrase said again after a miss can point at the first one. */
const question = ref(0)
const mistakes = ref(0)
const flash = ref<'' | 'good' | 'bad'>('')
const saved = ref<RunSaved | null>(null)
const summary = ref<RunSummary | null>(null)
let flashTimer: ReturnType<typeof setTimeout> | undefined

const countdown = useCountdown(30_000, () => void finish())
const best = computed(() => summary.value?.best['standard']?.score)
const accuracy = computed(() =>
  score.value + mistakes.value
    ? Math.round((score.value / (score.value + mistakes.value)) * 100)
    : 0,
)

onMounted(async () => {
  summary.value = await window.kchess.runSummary('squareColor')
  window.addEventListener('keydown', keydown)
})
onBeforeUnmount(() => {
  mounted = false
  window.removeEventListener('keydown', keydown)
  clearTimeout(flashTimer)
})

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
  score.value = 0
  mistakes.value = 0
  saved.value = null
  question.value++
  square.value = randomSquare()
  stage.value = 'running'
  countdown.start()
}

function answer(color: 'light' | 'dark'): void {
  if (stage.value !== 'running') return
  const correct = squareColor(square.value) === color
  flash.value = correct ? 'good' : 'bad'
  clearTimeout(flashTimer)
  flashTimer = setTimeout(() => (flash.value = ''), 300)
  if (correct) score.value++
  else {
    mistakes.value++
    void play('lowTime', 0.5)
  }
  question.value++
  square.value = randomSquare(square.value)
}

/** The asked square is known, so every phrase logged here says exactly what was meant. */
let lastMiss: { asked: number; entry: Promise<number | undefined> } | undefined
function logAnswer(result: VoiceResult, outcome: VoiceOutcome, parsed?: string): void {
  const asked = question.value
  const retryOf = lastMiss?.asked === asked ? lastMiss.entry : undefined
  const expected = squareColor(square.value)
  const entry = (async () =>
    logVoice({
      source: 'squareColor',
      ...heardFields(result),
      outcome,
      parsed,
      expected,
      retryOf: (await retryOf) ?? undefined,
    }))()
  if (outcome !== 'correct') lastMiss = { asked, entry }
}
function voiceMissed(result: VoiceResult): void {
  if (stage.value === 'running') logAnswer(result, 'unclear')
}
function voiceAnswer(result: VoiceResult): void {
  if (stage.value !== 'running') return
  const words = result.text.toLowerCase().split(/\s+/)
  const color = words.includes('light') ? 'light' : words.includes('dark') ? 'dark' : undefined
  if (!color) {
    logAnswer(result, 'invalid')
    voiceFeedback.value = 'Say “light” or “dark”.'
    return
  }
  const asked = square.value
  logAnswer(result, squareColor(asked) === color ? 'correct' : 'wrong', color)
  answer(color)
  voiceFeedback.value = `${color} · ${squareColor(asked) === color ? 'correct' : 'wrong'}`
}

function keydown(event: KeyboardEvent): void {
  if (event.metaKey || event.ctrlKey || event.altKey) return
  const key = event.key.toLowerCase()
  if (key === 'l') answer('light')
  else if (key === 'd') answer('dark')
}

async function finish(): Promise<void> {
  if (stage.value !== 'running') return
  countdown.stop()
  stage.value = 'over'
  try {
    saved.value = await window.kchess.saveRun({
      kind: 'squareColor',
      variant: 'standard',
      score: score.value,
      detail: { mistakes: mistakes.value, accuracy: accuracy.value },
    })
    summary.value = saved.value.summary
  } catch (cause) {
    console.warn('[square-color-trainer] Could not save run:', cause)
    // The score is still on screen; only the record is lost.
  }
}
</script>

<template>
  <div class="drill-center">
    <div class="card drill-card">
      <h2 class="sr-only">Square colours</h2>
      <p class="section-hint">Name each square’s color · 30 seconds</p>

      <div class="drill-stage" :class="flash" role="status" aria-live="polite">
        <template v-if="stage === 'running'">
          <span
            class="drill-clock tabular"
            :class="{ low: countdown.left.value < 10_000 }"
            role="timer"
            >{{ formatRunClock(countdown.left.value) }}</span
          >
          <strong class="drill-square">{{ square }}</strong>
          <span class="muted tabular">{{ score }} correct</span>
        </template>
        <template v-else-if="stage === 'over'">
          <strong class="drill-square small">{{ score }} correct</strong>
          <span class="muted">{{ accuracy }}% accuracy</span>
          <span :class="saved?.isBest ? 'text-success' : 'muted'">{{
            saved?.isBest ? 'A new personal best!' : `Best so far: ${best ?? score}`
          }}</span>
        </template>
        <template v-else>
          <strong class="drill-square small">Ready?</strong>
          <span class="muted tabular">Best: {{ best ?? '—' }}</span>
        </template>
      </div>

      <div v-if="stage === 'running'" class="drill-answers">
        <UButton
          size="xl"
          color="neutral"
          variant="outline"
          class="answer-light"
          @click="answer('light')"
          >Light <UKbd value="L"
        /></UButton>
        <UButton size="xl" color="neutral" variant="solid" @click="answer('dark')"
          >Dark <UKbd value="D"
        /></UButton>
      </div>
      <div v-else class="drill-answers">
        <UButton size="lg" icon="i-lucide-play" :loading="starting" @click="start">{{
          stage === 'over' ? 'Play again' : 'Start'
        }}</UButton>
      </div>

      <VoiceInput
        ref="voiceControl"
        v-model:enabled="voiceEnabled"
        class="mt-4"
        :active="stage === 'running'"
        :context-key="`${stage}:${square}`"
        :grammar="COLOR_GRAMMAR"
        hint="Say “light” or “dark”."
        :feedback="voiceFeedback"
        @result="voiceAnswer"
        @unclear="voiceMissed"
      />
    </div>
  </div>
</template>

<style scoped>
/* Practice has no page header: the card sits in the middle of what is left under the tabs. */
.drill-center {
  display: grid;
  place-items: center;
  min-height: calc(100dvh - var(--topbar-height) - 128px);
}
.drill-card {
  width: 100%;
}
</style>
