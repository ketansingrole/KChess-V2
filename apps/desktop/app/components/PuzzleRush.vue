<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import type { Puzzle, RunSaved, RunSummary } from '../../../../core/src/contracts/types'
import {
  RUSH_CHOICES,
  RUSH_CONFIGS,
  accuracy,
  comboProgress,
  correctMove,
  finish,
  formatRunClock,
  mistake,
  newRush,
  puzzleSolved,
  skip,
  tick,
  type RushState,
} from '../../../../core/src/domain/rush'
import { play } from '../utils/sound'
import type PuzzleBoard from './PuzzleBoard.vue'

/**
 * Storm, Streak and Rush. Everything here is local: puzzles come from the database on this computer and
 * scores are kept on it. (Lichess's own Storm and Streak results are shown, read-only, next to it.)
 */
const puzzles = usePuzzleStore()
const { db } = storeToRefs(puzzles)

const choice = ref('storm')
const config = computed(() => RUSH_CONFIGS[choice.value]!)
const stage = ref<'menu' | 'loading' | 'running' | 'over'>('menu')
const error = ref('')

const ladder = ref<Puzzle[]>([])
const index = ref(0)
const state = ref<RushState>(newRush(RUSH_CONFIGS.storm!))
const board = ref<InstanceType<typeof PuzzleBoard> | null>(null)
const saved = ref<RunSaved | null>(null)
const bonusFlash = ref(0)
const startedAt = ref(0)
const elapsedMs = ref(0)
const locked = ref(false)
let generation = 0
let advanceTimer: ReturnType<typeof setTimeout> | undefined
let bonusTimer: ReturnType<typeof setTimeout> | undefined

function clearTimers(): void {
  generation++
  clearTimeout(advanceTimer)
  clearTimeout(bonusTimer)
  locked.value = false
  bonusFlash.value = 0
}

const summaries = ref<Partial<Record<'storm' | 'streak' | 'rush', RunSummary>>>({})
async function loadSummaries(): Promise<void> {
  const [storm, streak, rush] = await Promise.all([
    window.kchess.runSummary('storm'),
    window.kchess.runSummary('streak'),
    window.kchess.runSummary('rush'),
  ])
  summaries.value = { storm, streak, rush }
}
const bestOf = (key: string): number | undefined => {
  const cfg = RUSH_CONFIGS[key]!
  return summaries.value[cfg.mode]?.best[cfg.variant]?.score
}
const recent = computed(() => summaries.value[config.value.mode]?.recent ?? [])

onMounted(() => {
  void puzzles.refreshDb()
  void loadSummaries()
})

let lastTick = 0
let warned = false
const clock = useIntervalFn(
  () => {
    const now = performance.now()
    tick(state.value, now - lastTick)
    lastTick = now
    const left = state.value.timeLeftMs
    if (!warned && left !== undefined && left < 10_000 && left > 0) {
      warned = true
      void play('lowTime')
    }
    if (state.value.over) void endRun()
  },
  100,
  { immediate: false },
)
onBeforeUnmount(() => {
  clock.pause()
  clearTimers()
})

async function start(): Promise<void> {
  clock.pause()
  clearTimers()
  const mine = generation
  const chosen = config.value
  error.value = ''
  stage.value = 'loading'
  try {
    const loaded = await window.kchess.localLadder(chosen.ladder)
    if (mine !== generation) return
    ladder.value = loaded
    if (ladder.value.length < 10)
      throw new Error('The puzzle database is too small; download it again.')
  } catch (cause) {
    if (mine !== generation) return
    console.warn('[puzzle-rush] Could not load ladder:', cause)
    error.value = cause instanceof Error ? cause.message : String(cause)
    stage.value = 'menu'
    return
  }
  index.value = 0
  state.value = newRush(chosen)
  saved.value = null
  warned = false
  startedAt.value = performance.now()
  lastTick = startedAt.value
  stage.value = 'running'
  if (config.value.durationMs) clock.resume()
}

const puzzle = computed(() => ladder.value[index.value])

function advance(): void {
  if (state.value.over) return
  index.value++
  if (index.value >= ladder.value.length) {
    finish(state.value, 'complete')
    void endRun()
  }
}

function onMove(correct: boolean): void {
  if (correct) {
    const bonus = correctMove(state.value, config.value)
    if (bonus) {
      bonusFlash.value = bonus
      clearTimeout(bonusTimer)
      const mine = generation
      bonusTimer = setTimeout(() => {
        if (mine === generation) bonusFlash.value = 0
      }, 1200)
    }
  } else mistake(state.value, config.value)
  if (state.value.over) void endRun()
}

function onDone(): void {
  if (state.value.over) return
  const solved = board.value?.status === 'solved'
  if (solved && puzzle.value) puzzleSolved(state.value, puzzle.value.rating)
  locked.value = true
  const mine = generation
  advanceTimer = setTimeout(
    () => {
      if (mine !== generation || stage.value !== 'running') return
      locked.value = false
      advance()
    },
    solved ? 250 : 750,
  )
}

function skipPuzzle(): void {
  if (!locked.value && skip(state.value)) advance()
}

async function endRun(): Promise<void> {
  if (stage.value !== 'running') return
  clock.pause()
  clearTimers()
  const mine = generation
  const ended = config.value
  elapsedMs.value = performance.now() - startedAt.value
  if (!state.value.over) finish(state.value, 'ended')
  stage.value = 'over'
  try {
    const result = await window.kchess.saveRun({
      kind: ended.mode,
      variant: ended.variant,
      score: state.value.score,
      detail: {
        moves: state.value.moves,
        mistakes: state.value.mistakes,
        accuracy: accuracy(state.value),
        maxCombo: state.value.maxCombo,
        highest: state.value.highest,
        seconds: Math.round(elapsedMs.value / 1000),
      },
    })
    summaries.value = { ...summaries.value, [ended.mode]: result.summary }
    if (mine === generation) saved.value = result
  } catch (cause) {
    if (mine !== generation) return
    console.warn('[puzzle-rush] Could not save run:', cause)
    error.value = cause instanceof Error ? cause.message : String(cause)
  }
}

const overText = computed(() => {
  switch (state.value.over) {
    case 'time':
      return 'Time’s up'
    case 'strikes':
      return config.value.strikes === 1 ? 'Streak over' : 'Out of strikes'
    case 'complete':
      return 'You cleared every puzzle!'
    default:
      return 'Run ended'
  }
})
const lowTime = computed(() => (state.value.timeLeftMs ?? Infinity) < 10_000)
const strikeSlots = computed(() =>
  config.value.strikes && config.value.strikes > 1
    ? Array.from({ length: config.value.strikes }, (_, i) => i < state.value.mistakes)
    : [],
)
const dbReady = computed(() => db.value?.installed === true)
const bonusText = computed(() => (bonusFlash.value ? `+${bonusFlash.value / 1000}s` : ''))
</script>

<template>
  <div>
    <!-- Menu -->
    <div v-if="stage === 'menu' || stage === 'loading'" class="rush-menu">
      <div class="card">
        <div class="card-header">
          <div>
            <h2 class="section-title">Play a run</h2>
          </div>
        </div>
        <div class="rush-choices" role="radiogroup" aria-label="Kind of run">
          <button
            v-for="key in RUSH_CHOICES"
            :key="key"
            type="button"
            role="radio"
            class="rush-choice"
            :class="{ active: choice === key }"
            :aria-checked="choice === key"
            :disabled="stage === 'loading'"
            @click="choice = key"
          >
            <strong>{{ RUSH_CONFIGS[key]!.title }}</strong>
            <span v-if="bestOf(key) !== undefined" class="tabular">Best: {{ bestOf(key) }}</span>
          </button>
        </div>
        <p class="rush-rules">{{ config.rules }}</p>
        <p v-if="error" class="text-error text-sm" role="alert">{{ error }}</p>
        <UButton
          size="lg"
          icon="i-lucide-play"
          :disabled="!dbReady"
          :loading="stage === 'loading'"
          @click="start"
          >Start {{ config.title }}</UButton
        >
        <PuzzleDbCard v-if="!dbReady" class="mt-4" />
        <div v-if="recent.length" class="mt-5">
          <h3 class="section-title">Your recent {{ config.mode }} runs</h3>
          <div class="table-scroll">
            <table class="data-table">
              <thead>
                <tr>
                  <th scope="col">When</th>
                  <th scope="col">Kind</th>
                  <th scope="col" class="num">Score</th>
                  <th scope="col" class="num">Accuracy</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="run in recent.slice(0, 6)" :key="run.id">
                  <td>{{ new Date(run.playedAt).toLocaleString() }}</td>
                  <td>{{ run.variant === 'standard' ? config.title : run.variant }}</td>
                  <td class="num tabular">{{ run.score }}</td>
                  <td class="num tabular">{{ run.detail.accuracy ?? '—' }}%</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
      <StormStats />
    </div>

    <!-- Running and finished -->
    <div v-else class="play-layout">
      <div>
        <div class="run-board">
          <PuzzleBoard
            v-if="puzzle"
            ref="board"
            :key="`${index}-${puzzle.id}`"
            :puzzle="puzzle"
            :retry="false"
            :frozen="stage === 'over' || locked"
            :reply-delay="250"
            @move="onMove"
            @done="onDone"
          />
          <Transition name="bonus">
            <span v-if="bonusText" class="bonus-flash">{{ bonusText }}</span>
          </Transition>
        </div>
      </div>
      <div class="card side-panel puzzle-panel">
        <h2 class="section-title">{{ config.title }}</h2>
        <template v-if="stage === 'running'">
          <div class="run-hud">
            <div
              v-if="state.timeLeftMs !== undefined"
              class="run-clock"
              :class="{ low: lowTime }"
              role="timer"
            >
              {{ formatRunClock(state.timeLeftMs) }}
            </div>
            <div class="run-score">
              <strong class="tabular">{{ state.score }}</strong>
              <span>solved</span>
            </div>
          </div>
          <div v-if="config.combo" class="combo" :aria-label="`Combo ${state.combo}`">
            <div class="combo-label">
              <span>Combo</span><strong class="tabular">{{ state.combo }}</strong>
            </div>
            <div class="combo-bar">
              <span :style="{ width: `${comboProgress(state.combo) * 100}%` }" />
            </div>
          </div>
          <div
            v-if="strikeSlots.length"
            class="strikes"
            :aria-label="`${state.mistakes} of ${config.strikes} strikes`"
          >
            <UIcon
              v-for="(used, i) in strikeSlots"
              :key="i"
              :name="used ? 'i-lucide-circle-x' : 'i-lucide-circle'"
              :class="used ? 'text-error' : 'muted'"
            />
            <span class="muted text-xs">strikes</span>
          </div>
          <p class="section-hint tabular">Puzzle {{ index + 1 }} · rating {{ puzzle?.rating }}</p>
          <div class="panel-actions">
            <UButton
              v-if="config.skips"
              variant="outline"
              color="neutral"
              icon="i-lucide-skip-forward"
              :disabled="state.skipsLeft <= 0 || locked"
              @click="skipPuzzle"
              >Skip ({{ state.skipsLeft }})</UButton
            >
            <UButton variant="outline" color="neutral" icon="i-lucide-flag" @click="endRun"
              >End run</UButton
            >
          </div>
        </template>
        <template v-else>
          <div class="status-banner" :class="saved?.isBest ? 'win' : ''" role="status">
            <UIcon
              :name="saved?.isBest ? 'i-lucide-trophy' : 'i-lucide-flag'"
              class="status-icon"
            />
            <div>
              <strong>{{ overText }}</strong>
              <span class="detail">{{
                saved?.isBest ? 'A new personal best!' : 'Saved on this computer.'
              }}</span>
            </div>
          </div>
          <dl class="run-results">
            <div>
              <dt>Score</dt>
              <dd class="tabular">{{ state.score }}</dd>
            </div>
            <div>
              <dt>Moves</dt>
              <dd class="tabular">{{ state.moves }}</dd>
            </div>
            <div>
              <dt>Accuracy</dt>
              <dd class="tabular">{{ accuracy(state) }}%</dd>
            </div>
            <div v-if="config.combo">
              <dt>Best combo</dt>
              <dd class="tabular">{{ state.maxCombo }}</dd>
            </div>
            <div>
              <dt>Hardest solved</dt>
              <dd class="tabular">{{ state.highest || '—' }}</dd>
            </div>
            <div>
              <dt>Time</dt>
              <dd class="tabular">{{ formatRunClock(elapsedMs) }}</dd>
            </div>
          </dl>

          <div class="panel-actions">
            <UButton icon="i-lucide-rotate-ccw" @click="start">Play again</UButton>
            <UButton variant="outline" color="neutral" @click="stage = 'menu'">Back</UButton>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.rush-menu {
  display: grid;
  grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr);
  gap: 20px;
  align-items: start;
}
@container page (max-width: 900px) {
  .rush-menu {
    grid-template-columns: 1fr;
  }
}
.rush-choices {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 8px;
}
.rush-choice {
  display: flex;
  flex-direction: column;
  gap: 2px;
  align-items: flex-start;
  padding: 10px 12px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg);
  color: inherit;
  text-align: left;
  font-size: 13px;
}
.rush-choice span {
  color: var(--ui-text-muted);
  font-size: 12px;
}
.rush-choice.active {
  border-color: var(--ui-primary);
  background: color-mix(in srgb, var(--ui-primary) 12%, transparent);
}
.rush-rules {
  margin: 14px 0;
  color: var(--ui-text-toned);
  font-size: 13px;
  line-height: 1.55;
}
.run-board {
  position: relative;
}
.bonus-flash {
  position: absolute;
  top: 12px;
  right: 12px;
  padding: 4px 10px;
  border-radius: 999px;
  background: var(--ui-primary);
  color: #fff;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}
.bonus-enter-active,
.bonus-leave-active {
  transition:
    opacity 0.3s,
    transform 0.3s;
}
.bonus-enter-from,
.bonus-leave-to {
  opacity: 0;
  transform: translateY(-6px);
}
.combo-label {
  display: flex;
  justify-content: space-between;
  margin-bottom: 4px;
  font-size: 12px;
  color: var(--ui-text-muted);
}
.combo-bar {
  height: 8px;
  overflow: hidden;
  border-radius: 999px;
  background: var(--ui-bg-accented);
}
.combo-bar span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--ui-primary);
  transition: width 0.2s;
}
.strikes {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 20px;
}
</style>
