<script setup lang="ts">
import type { PuzzleTab } from '../stores/puzzles'

const puzzles = usePuzzleStore()
const { tab } = storeToRefs(puzzles)

const tabs = [
  { label: 'Train', value: 'train', icon: 'i-lucide-puzzle' },
  { label: 'Daily', value: 'daily', icon: 'i-lucide-calendar-days' },
  { label: 'Storm · Streak · Rush', value: 'rush', icon: 'i-lucide-zap' },
  { label: 'Lichess stats', value: 'stats', icon: 'i-lucide-chart-column' },
  { label: 'Lichess history', value: 'history', icon: 'i-lucide-history' },
]
</script>

<template>
  <div>
    <PageHeader title="Puzzles" />
    <UTabs
      :model-value="tab"
      :items="tabs"
      :content="false"
      variant="pill"
      size="sm"
      class="mb-5"
      @update:model-value="tab = $event as PuzzleTab"
    />
    <PuzzleTrain v-if="tab === 'train'" />
    <LazyPuzzleDaily v-else-if="tab === 'daily'" />
    <LazyPuzzleRush v-else-if="tab === 'rush'" />
    <LazyPuzzleStats v-else-if="tab === 'stats'" />
    <LazyPuzzleHistory v-else />
  </div>
</template>
