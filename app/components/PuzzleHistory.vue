<script setup lang="ts">
import { computed, onMounted, watch } from 'vue'
import { themeName } from '../utils/puzzleThemes'
import { formatGameDate } from '../utils/games'

/** The account's puzzle attempts as Lichess recorded them (read-only), each of which can be tried again here. */
const kchess = useKChessStore()
const puzzles = usePuzzleStore()
const { activity, activityState, account } = storeToRefs(puzzles)

onMounted(() => void puzzles.loadActivity())
watch(account, () => void puzzles.loadActivity())

const rows = computed(() =>
  activity.value.map((entry) => ({
    ...entry,
    themeText: entry.puzzle.themes
      .filter((theme) => !['short', 'long', 'veryLong', 'oneMove'].includes(theme))
      .slice(0, 3)
      .map(themeName)
      .join(', '),
  })),
)
</script>

<template>
  <div class="card">
    <div class="card-header">
      <div>
        <h2 class="sr-only">Puzzle history</h2>
        <p class="section-hint">Trying one again here is local only.</p>
      </div>
      <div class="toolbar-row">
        <SourceBadge kind="readonly" />
        <UButton
          v-if="account"
          size="sm"
          variant="outline"
          color="neutral"
          icon="i-lucide-refresh-cw"
          :loading="activityState.loading"
          @click="puzzles.loadActivity(true)"
          >Refresh</UButton
        >
      </div>
    </div>
    <UEmpty
      v-if="!account"
      variant="naked"
      icon="i-lucide-user-round-plus"
      title="Connect a Lichess account"
      description="Your puzzle history is read from your Lichess account."
      :actions="[
        { label: 'Connect Lichess', icon: 'i-lucide-log-in', onClick: () => kchess.connect() },
      ]"
    />
    <UEmpty
      v-else-if="activityState.needsReconnect"
      variant="naked"
      icon="i-lucide-key-round"
      title="Lichess needs your permission"
      description="This account was connected before puzzles were added. Connect it again to read its puzzle history."
      :actions="[
        { label: 'Reconnect', icon: 'i-lucide-log-in', onClick: () => puzzles.reconnect() },
      ]"
    />
    <p v-else-if="activityState.error" class="text-error text-sm" role="alert">
      {{ activityState.error }}
    </p>
    <div v-else-if="activityState.loading && !rows.length" class="muted text-sm" role="status">
      <UIcon name="i-lucide-loader-circle" class="animate-spin" /> Loading from Lichess…
    </div>
    <UEmpty
      v-else-if="!rows.length"
      variant="naked"
      icon="i-lucide-puzzle"
      title="No puzzles yet"
      description="Rated puzzles you play here and on Lichess show up in this list."
    />
    <div v-else class="game-list">
      <div class="game-head puzzle-cols" aria-hidden="true">
        <span>Result</span><span>Themes</span><span class="puzzle-rating">Rating</span
        ><span class="puzzle-when">Played</span><span />
      </div>
      <div v-for="row in rows" :key="`${row.puzzle.id}-${row.date}`" class="game-row puzzle-cols">
        <span
          ><UBadge
            :color="row.win ? 'success' : 'error'"
            variant="soft"
            class="w-16 justify-center"
            >{{ row.win ? 'Solved' : 'Missed' }}</UBadge
          ></span
        >
        <span class="puzzle-themes">{{ row.themeText }}</span>
        <span class="puzzle-rating tabular">{{ row.puzzle.rating }}</span>
        <span class="puzzle-when muted">{{ formatGameDate(row.date) }}</span>
        <span class="puzzle-actions">
          <UButton size="xs" variant="soft" color="neutral" @click="puzzles.retry(row.puzzle)"
            >Try again</UButton
          >
          <UTooltip text="Open on Lichess">
            <UButton
              size="xs"
              variant="ghost"
              color="neutral"
              icon="i-lucide-external-link"
              aria-label="Open this puzzle on Lichess"
              :to="`https://lichess.org/training/${row.puzzle.id}`"
              target="_blank"
              rel="noopener"
            />
          </UTooltip>
        </span>
      </div>
    </div>
  </div>
</template>
