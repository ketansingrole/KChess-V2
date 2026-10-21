<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useOnline } from '@vueuse/core'
import type { PuzzleAttempt } from '@kchess/contracts/types'
import type { PuzzleVerdictDetail } from '@kchess/rules/puzzleSession'
import { themeItems } from '../utils/puzzleThemes'
import { ATTEMPT_BADGES, moveAccuracy, movesText } from '../utils/puzzleAttempts'
import { usePuzzleAttemptActions } from '../composables/usePuzzleAttemptActions'
import type PuzzleBoard from './PuzzleBoard.vue'

const online = useOnline()
const kchess = useKChessStore()
const puzzles = usePuzzleStore()
const {
  phase,
  current,
  puzzleKey,
  attemptOutcome,
  isRetry,
  trainError,
  rating,
  session,
  lastReport,
  mode,
  angle,
  difficulty,
  color,
  account,
  openingAngle,
  progress,
} = storeToRefs(puzzles)
const attemptActions = usePuzzleAttemptActions()
const { data } = storeToRefs(kchess)

const board = ref<InstanceType<typeof PuzzleBoard> | null>(null)
const flipped = ref(false)

const modes = [
  { label: 'Practice', value: 'practice', icon: 'i-lucide-dumbbell' },
  { label: 'Downloaded', value: 'offline', icon: 'i-lucide-hard-drive' },
  { label: 'Rated', value: 'rated', icon: 'i-lucide-cloud-upload' },
] as const
/** What each mode means for the result, where the choice is made. */
const MODE_HINTS = {
  practice: 'Lichess puzzles, unrated.',
  offline: 'Puzzles downloaded to this computer.',
  rated: 'Results affect your Lichess rating.',
} as const
const difficulties = [
  { label: 'Easiest', value: 'easiest' },
  { label: 'Easier', value: 'easier' },
  { label: 'Normal', value: 'normal' },
  { label: 'Harder', value: 'harder' },
  { label: 'Hardest', value: 'hardest' },
]
const colors = [
  { label: 'Random', value: 'random' },
  { label: 'White', value: 'white' },
  { label: 'Black', value: 'black' },
]
/** The themes, and the opening an opening page chose puzzles of. */
const themes = computed(() => {
  const [mix, ...rest] = themeItems()
  const opening = openingAngle.value
  return opening
    ? [
        ...(mix ? [mix] : []),
        { type: 'separator' as const },
        { type: 'label' as const, label: 'Opening' },
        { label: opening.name, value: opening.key },
        ...rest,
      ]
    : [...(mix ? [mix] : []), ...rest]
})
/** What could still have puzzles once a theme runs out at this difficulty, best first. */
const exhaustedActions = computed(() => {
  const actions = []
  const index = difficulties.findIndex((entry) => entry.value === difficulty.value)
  const easier = index > 0 ? difficulties[index - 1] : undefined
  if (easier)
    actions.push({
      label: `Try ${easier.label}`,
      icon: 'i-lucide-chevrons-down',
      onClick: () => (difficulty.value = easier.value as typeof difficulty.value),
    })
  if (angle.value !== 'mix')
    actions.push({
      label: 'Try Healthy mix',
      icon: 'i-lucide-shuffle',
      onClick: () => (angle.value = 'mix'),
    })
  if (mode.value !== 'offline')
    actions.push({
      label: 'Use downloaded puzzles',
      icon: 'i-lucide-hard-drive',
      onClick: () => (mode.value = 'offline'),
    })
  return actions.map((action, order) =>
    order ? { ...action, variant: 'outline' as const, color: 'neutral' as const } : action,
  )
})
const difficultyLabel = computed(
  () => difficulties.find((entry) => entry.value === difficulty.value)?.label ?? difficulty.value,
)
const accounts = computed(() =>
  data.value.accounts.filter((entry) => entry.connected).map((entry) => entry.username),
)

onMounted(() => {
  void puzzles.refreshDb()
  void puzzles.loadProgress()
  if (phase.value === 'idle') void puzzles.loadNext()
})
// A different theme, difficulty, colour, account or mode means a different puzzle right away.
watch([mode, angle, difficulty, color, account], () => void puzzles.loadNext())

const finished = computed(() =>
  ['solved', 'revealed', 'failed'].includes(board.value?.status ?? ''),
)
const ratingText = computed(() => {
  const report = lastReport.value
  if (!report) return ''
  if (report.failed) return report.failed
  if (report.ratingDiff === undefined) return 'Sent to Lichess.'
  return `Lichess puzzle rating ${report.ratingDiff >= 0 ? '+' : ''}${report.ratingDiff}${rating.value ? ` → ${rating.value}` : ''}`
})
function onMove(correct: boolean, index: number, uci: string): void {
  puzzles.move(index, correct, uci)
}
function onOutcome(win: boolean, detail: PuzzleVerdictDetail): void {
  void puzzles.report(win, detail)
}

/** The session by outcome; solved and missed always show, the rest once they happen. */
const sessionParts = computed(() => {
  const tally = session.value
  return [
    { count: tally.solved, label: 'solved', tone: 'text-success', always: true },
    { count: tally.hinted, label: 'with hint', tone: 'text-success' },
    { count: tally.partial, label: 'partial', tone: 'text-info' },
    { count: tally.missed, label: 'missed', tone: 'text-error', always: true },
    { count: tally.revealed, label: 'revealed', tone: '' },
  ].filter((part) => part.always || part.count > 0)
})
const sessionAccuracy = computed(() =>
  moveAccuracy(session.value.movesFound, session.value.movesTried),
)
const dayStreak = computed(() => progress.value?.dayStreak ?? 0)
/** The latest attempts, oldest first, so the newest sits at the end of the strip. */
const recentAttempts = computed(() => [...(progress.value?.recent ?? [])].reverse())
const ATTEMPT_ICONS = {
  solved: 'i-lucide-check',
  hinted: 'i-lucide-lightbulb',
  partial: 'i-lucide-chart-no-axes-column',
  missed: 'i-lucide-x',
  revealed: 'i-lucide-eye',
} as const
const attemptLabel = (attempt: PuzzleAttempt): string =>
  `${ATTEMPT_BADGES[attempt.outcome].label}, ${movesText(attempt)}, rated ${attempt.puzzle.rating}`

/** A pause on the final position before the next puzzle replaces it. */
const AUTO_NEXT_DELAY = 800
let autoNext: ReturnType<typeof setTimeout> | undefined
function onDone(): void {
  clearTimeout(autoNext)
  puzzles.end()
  if (board.value?.status !== 'solved' || !kchess.settings?.puzzleAutoNext) return
  const key = puzzleKey.value
  autoNext = setTimeout(() => {
    // Moving on by hand, or changing what to train, already replaced the puzzle.
    if (puzzleKey.value === key && phase.value === 'ready') void puzzles.loadNext()
  }, AUTO_NEXT_DELAY)
}
watch(puzzleKey, () => clearTimeout(autoNext))
onBeforeUnmount(() => clearTimeout(autoNext))
</script>

<template>
  <div v-board-fit class="play-layout puzzle-wide">
    <div>
      <PuzzleBoard
        v-if="current && phase === 'ready'"
        ref="board"
        :key="puzzleKey"
        :puzzle="current"
        :initial-outcome="attemptOutcome"
        :flipped="flipped"
        material
        @move="onMove"
        @outcome="onOutcome"
        @done="onDone"
      />
      <div v-else class="card puzzle-placeholder">
        <UEmpty
          v-if="phase === 'noaccount'"
          variant="naked"
          icon="i-lucide-user-round-plus"
          title="Connect a Lichess account for rated puzzles"
          description="Connect Lichess or switch to Practice."
          :actions="[
            {
              label: 'Connect Lichess',
              icon: 'i-lucide-log-in',
              disabled: !online || kchess.busy,
              onClick: () => kchess.connect(),
            },
            {
              label: online ? 'Practice instead' : 'Use downloaded puzzles',
              variant: 'outline',
              color: 'neutral',
              onClick: () => (mode = online ? 'practice' : 'offline'),
            },
          ]"
        />
        <UEmpty
          v-else-if="phase === 'reconnect'"
          variant="naked"
          icon="i-lucide-key-round"
          title="Lichess needs your permission for puzzles"
          :description="`Sign in to @${account} on lichess.org, then reconnect to enable rated puzzles.`"
          :actions="[
            {
              label: 'Reconnect',
              icon: 'i-lucide-log-in',
              disabled: !online || kchess.busy,
              onClick: () => puzzles.reconnect(),
            },
            {
              label: online ? 'Practice instead' : 'Use downloaded puzzles',
              variant: 'outline',
              color: 'neutral',
              onClick: () => (mode = online ? 'practice' : 'offline'),
            },
          ]"
        />
        <div v-else-if="phase === 'nodb'" class="puzzle-db">
          <PuzzleDbCard />
        </div>
        <UEmpty
          v-else-if="phase === 'exhausted'"
          variant="naked"
          icon="i-lucide-puzzle"
          :title="`No more ${difficultyLabel} puzzles for this theme`"
          :actions="exhaustedActions"
        />
        <UEmpty
          v-else-if="phase === 'error'"
          variant="naked"
          icon="i-lucide-triangle-alert"
          title="Couldn’t get a puzzle"
          :description="trainError"
          :actions="[
            { label: 'Try again', icon: 'i-lucide-rotate-cw', onClick: () => puzzles.loadNext() },
            ...(mode !== 'offline'
              ? [
                  {
                    label: 'Use downloaded puzzles',
                    variant: 'outline' as const,
                    color: 'neutral' as const,
                    onClick: () => (mode = 'offline'),
                  },
                ]
              : []),
          ]"
        />
        <div v-else class="puzzle-loading" role="status">
          <UIcon name="i-lucide-loader-circle" class="animate-spin" /> Finding a puzzle…
        </div>
      </div>
      <PuzzleVoice :board="board" :context-key="String(puzzleKey)" />
    </div>

    <div class="card side-panel puzzle-panel">
      <div class="form-stack">
        <div class="field">
          <span id="train-mode-label" class="field-label">Mode</span>
          <UTabs
            v-model="mode"
            :items="[...modes]"
            aria-labelledby="train-mode-label"
            :content="false"
            variant="pill"
            class="w-full"
            :ui="{ trigger: 'grow' }"
          />
        </div>
        <p v-if="!isRetry" class="section-hint">{{ MODE_HINTS[mode] }}</p>
        <SourceBadge v-if="isRetry" kind="local" label="Retry · unrated" />
        <SourceBadge
          v-else-if="mode === 'rated'"
          kind="synced"
          :label="account ? `Rated · @${account}` : 'Rated'"
        />
        <div v-if="mode === 'rated' && accounts.length > 1" class="field">
          <span id="train-account-label" class="field-label">Account</span>
          <USelect
            :model-value="account"
            :items="accounts"
            icon="i-lucide-user"
            size="sm"
            aria-labelledby="train-account-label"
            :ui="{ base: 'w-full' }"
            @update:model-value="puzzles.setAccount($event as string)"
          />
        </div>
        <div class="field">
          <span id="train-theme-label" class="field-label">Theme</span>
          <USelect
            v-model="angle"
            :items="themes"
            size="sm"
            aria-labelledby="train-theme-label"
            :ui="{ base: 'w-full' }"
          />
        </div>
        <div class="train-pair">
          <div class="field">
            <span id="train-difficulty-label" class="field-label">Difficulty</span>
            <USelect
              v-model="difficulty"
              :items="difficulties"
              size="sm"
              aria-labelledby="train-difficulty-label"
              :ui="{ base: 'w-full' }"
            />
          </div>
          <div class="field">
            <span id="train-color-label" class="field-label">Play as</span>
            <USelect
              v-model="color"
              :items="colors"
              size="sm"
              aria-labelledby="train-color-label"
              :ui="{ base: 'w-full' }"
              :disabled="mode === 'offline'"
            />
          </div>
        </div>
      </div>

      <template v-if="current && phase === 'ready'">
        <div class="panel-divider" />
        <PuzzleStatusPanel
          :puzzle="current"
          :status="board?.status ?? 'playing'"
          :outcome="board?.outcome ?? null"
          :feedback="board?.feedback ?? ''"
          :waiting="board?.waiting ?? false"
          :mistakes="board?.mistakes ?? 0"
          :hinted="board?.hinted ?? false"
          @hint="board?.hint()"
          @solution="board?.showSolution()"
        >
          <template #actions>
            <UButton
              icon="i-lucide-arrow-up-down"
              variant="ghost"
              color="neutral"
              aria-label="Flip board"
              @click="flipped = !flipped"
            />
            <UButton
              :variant="finished ? 'solid' : 'outline'"
              :color="finished ? 'primary' : 'neutral'"
              trailing-icon="i-lucide-arrow-right"
              @click="puzzles.loadNext()"
              >Next</UButton
            >
          </template>
        </PuzzleStatusPanel>
      </template>
      <p
        v-if="current && phase === 'ready' && ratingText"
        class="section-hint"
        :class="{ 'text-error': lastReport?.failed }"
        role="status"
      >
        {{ ratingText }}
      </p>

      <div class="panel-divider" />
      <div class="session-line tabular" role="status">
        <span v-for="part in sessionParts" :key="part.label"
          ><strong :class="part.tone">{{ part.count }}</strong> {{ part.label }}</span
        >
        <span
          v-if="sessionAccuracy !== undefined"
          :title="`${sessionAccuracy}% of moves tried were right at the first try`"
          ><strong>{{ session.movesFound }}</strong
          >/{{ session.movesTried }} moves right</span
        >
        <span v-if="rating !== undefined && mode === 'rated'"
          >rating <strong>{{ rating }}</strong
          ><template v-if="session.ratingChange">
            ({{ session.ratingChange > 0 ? '+' : '' }}{{ session.ratingChange }})</template
          ></span
        >
        <span class="session-end">
          <span
            v-if="dayStreak >= 2"
            class="day-streak"
            :title="`Days in a row with a solved puzzle · best ${progress?.bestDayStreak}`"
            :aria-label="`${dayStreak} days in a row with a solved puzzle`"
            ><UIcon name="i-lucide-flame" aria-hidden="true" /><strong>{{
              dayStreak
            }}</strong></span
          >
          <UButton size="xs" variant="link" color="neutral" @click="puzzles.resetSession()"
            >Reset</UButton
          >
        </span>
      </div>
      <ul v-if="recentAttempts.length" class="attempt-strip" aria-label="Recent puzzles">
        <li v-for="attempt in recentAttempts" :key="attempt.id">
          <UDropdownMenu :items="attemptActions.menu(attempt.puzzle, attempt.wrongMoves)">
            <UButton
              size="xs"
              square
              :variant="ATTEMPT_BADGES[attempt.outcome].variant"
              :color="ATTEMPT_BADGES[attempt.outcome].color"
              :icon="ATTEMPT_ICONS[attempt.outcome]"
              :aria-label="attemptLabel(attempt)"
              :title="attemptLabel(attempt)"
            />
          </UDropdownMenu>
        </li>
      </ul>
    </div>
  </div>
</template>

<style scoped>
.puzzle-placeholder {
  display: grid;
  place-items: center;
  aspect-ratio: 1;
  width: 100%;
  padding: 24px;
}
.puzzle-loading {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--ui-text-muted);
}
.puzzle-db {
  width: 100%;
  max-width: 440px;
}
.attempt-strip {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 8px;
}
.session-end {
  display: inline-flex;
  align-items: baseline;
  gap: 8px;
  margin-left: auto;
}
.day-streak {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  color: var(--ui-warning);
}
.day-streak strong {
  color: var(--ui-text);
}
.train-pair {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}
</style>
