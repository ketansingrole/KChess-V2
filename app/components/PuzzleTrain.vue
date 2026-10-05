<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useOnline } from '@vueuse/core'
import { themeItems } from '../utils/puzzleThemes'
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
} = storeToRefs(puzzles)
const { data } = storeToRefs(kchess)

const board = ref<InstanceType<typeof PuzzleBoard> | null>(null)
const flipped = ref(false)

const modes = [
  { label: 'Practice', value: 'practice', icon: 'i-lucide-dumbbell' },
  { label: 'Downloaded', value: 'offline', icon: 'i-lucide-hard-drive' },
  { label: 'Rated', value: 'rated', icon: 'i-lucide-cloud-upload' },
] as const
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
const themes = themeItems()
const accounts = computed(() =>
  data.value.accounts.filter((entry) => entry.connected).map((entry) => entry.username),
)

onMounted(() => {
  void puzzles.refreshDb()
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
function onOutcome(win: boolean): void {
  void puzzles.report(win)
}
</script>

<template>
  <div class="play-layout">
    <div>
      <PuzzleBoard
        v-if="current && phase === 'ready'"
        ref="board"
        :key="puzzleKey"
        :puzzle="current"
        :initial-outcome="attemptOutcome"
        :flipped="flipped"
        @outcome="onOutcome"
      />
      <div v-else class="card puzzle-placeholder">
        <UEmpty
          v-if="phase === 'noaccount'"
          variant="naked"
          icon="i-lucide-user-round-plus"
          title="Connect a Lichess account for rated puzzles"
          description="Rated puzzles change your Lichess puzzle rating, so they need a connected account. Practice and Downloaded work without one."
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
          :description="`@${account} was connected before puzzles were added, or its login expired. Connect it again to allow KChess to read and report puzzles. Sign in to the same account on lichess.org first.`"
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
    </div>

    <div class="card side-panel puzzle-panel">
      <div class="form-stack">
        <div class="field">
          <span id="train-mode-label" class="field-label">Mode</span>
          <UTabs
            v-model="mode"
            :items="[...modes]"
            aria-labelledby="train-mode-label"
            size="sm"
            :content="false"
            variant="pill"
            class="w-full"
            :ui="{ trigger: 'grow' }"
          />
        </div>
        <p class="section-hint" role="status">
          {{
            mode === 'rated'
              ? 'Requires internet and a connected Lichess account. Results affect your Lichess rating.'
              : mode === 'offline'
                ? 'Works offline after downloading puzzles. No account needed.'
                : 'Requires internet. No account needed, and results stay on this device.'
          }}
        </p>
        <SourceBadge v-if="isRetry" kind="local" label="Local only · retry" />
        <SourceBadge
          v-else-if="mode === 'rated'"
          kind="synced"
          :label="account ? `Synced to Lichess · @${account}` : 'Synced to Lichess'"
        />
        <SourceBadge v-else kind="local" label="Local only · nothing is sent" />
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
        <p
          v-if="ratingText"
          class="section-hint"
          :class="{ 'text-error': lastReport?.failed }"
          role="status"
        >
          {{ ratingText }}
        </p>
      </template>

      <div class="panel-divider" />
      <div class="session-line tabular" role="status">
        <span
          ><strong class="text-success">{{ session.solved }}</strong> solved</span
        >
        <span
          ><strong class="text-error">{{ session.failed }}</strong> missed</span
        >
        <span v-if="rating !== undefined && mode === 'rated'"
          >rating <strong>{{ rating }}</strong
          ><template v-if="session.ratingChange">
            ({{ session.ratingChange > 0 ? '+' : '' }}{{ session.ratingChange }})</template
          ></span
        >
        <UButton
          size="xs"
          variant="link"
          color="neutral"
          class="ml-auto"
          @click="puzzles.resetSession()"
          >Reset</UButton
        >
      </div>
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
.train-pair {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}
</style>
