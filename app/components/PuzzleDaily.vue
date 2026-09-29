<script setup lang="ts">
import { onMounted, ref } from 'vue'
import type PuzzleBoard from './PuzzleBoard.vue'

const puzzles = usePuzzleStore()
const { daily, dailyError, dailyLoading } = storeToRefs(puzzles)
const board = ref<InstanceType<typeof PuzzleBoard> | null>(null)
const flipped = ref(false)
onMounted(() => void puzzles.loadDaily())
</script>

<template>
  <div class="play-layout">
    <div>
      <PuzzleBoard v-if="daily" ref="board" :key="daily.id" :puzzle="daily" :flipped="flipped" />
      <div v-else class="card daily-placeholder">
        <UEmpty
          v-if="dailyError"
          variant="naked"
          icon="i-lucide-triangle-alert"
          title="Couldn’t get the daily puzzle"
          :description="dailyError"
          :actions="[
            {
              label: 'Try again',
              icon: 'i-lucide-rotate-cw',
              onClick: () => puzzles.loadDaily(true),
            },
          ]"
        />
        <div v-else class="muted" role="status">
          <UIcon name="i-lucide-loader-circle" class="animate-spin" />
          {{ dailyLoading ? 'Fetching today’s puzzle…' : '' }}
        </div>
      </div>
    </div>
    <div class="card side-panel puzzle-panel">
      <h2 class="sr-only">Daily puzzle</h2>
      <SourceBadge kind="local" label="Local only · result is not sent" />
      <template v-if="daily">
        <PuzzleStatusPanel
          :puzzle="daily"
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
          </template>
        </PuzzleStatusPanel>
      </template>
    </div>
  </div>
</template>

<style scoped>
.daily-placeholder {
  display: grid;
  place-items: center;
  aspect-ratio: 1;
  width: 100%;
  padding: 24px;
}
</style>
