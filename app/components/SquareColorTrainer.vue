<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import type { RunSaved, RunSummary } from '../../src/shared/types'
import { randomSquare, squareColor, type Square } from '../utils/coordinates'
import { useCountdown } from '../utils/countdown'
import { formatRunClock } from '../utils/rush'
import { play } from '../utils/sound'

/** Is this square light or dark? Thirty seconds; the best score stays on this computer. */
const stage = ref<'idle' | 'running' | 'over'>('idle')
const square = ref<Square>('e4')
const score = ref(0)
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
  window.removeEventListener('keydown', keydown)
  clearTimeout(flashTimer)
})

function start(): void {
  score.value = 0
  mistakes.value = 0
  saved.value = null
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
  square.value = randomSquare(square.value)
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
  } catch {
    // The score is still on screen; only the record is lost.
  }
}
</script>

<template>
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
      <UButton size="lg" icon="i-lucide-play" @click="start">{{
        stage === 'over' ? 'Play again' : 'Start'
      }}</UButton>
    </div>
  </div>
</template>
