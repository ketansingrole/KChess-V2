<script setup lang="ts">
import { useLocalStorage } from '@vueuse/core'

const practiceTab = useLocalStorage('kchess:practice-tab', 'coordinates')

const tabs = [
  { label: 'Coordinates', value: 'coordinates', icon: 'i-lucide-grid-3x3' },
  { label: 'Square colours', value: 'colors', icon: 'i-lucide-contrast' },
  { label: 'Knight paths', value: 'knight', icon: 'i-lucide-crown' },
  { label: 'Endgames', value: 'endgames', icon: 'i-lucide-swords' },
  { label: 'Openings', value: 'openings', icon: 'i-lucide-book-marked' },
  { label: 'Your mistakes', value: 'mistakes', icon: 'i-lucide-book-open' },
  { label: 'Puzzle themes', value: 'themes', icon: 'i-lucide-puzzle' },
]
</script>

<template>
  <div>
    <PageHeader title="Practice" />
    <UTabs
      :model-value="practiceTab"
      :items="tabs"
      :content="false"
      variant="pill"
      class="mb-5"
      @update:model-value="practiceTab = $event as string"
    />
    <LazyMistakeTrainer v-if="practiceTab === 'mistakes'" />
    <LazyOpeningTrainer v-else-if="practiceTab === 'openings'" />
    <CoordinatesTrainer v-else-if="practiceTab === 'coordinates'" />
    <LazySquareColorTrainer v-else-if="practiceTab === 'colors'" />
    <LazyKnightPath v-else-if="practiceTab === 'knight'" />
    <LazyEndgameDrills v-else-if="practiceTab === 'endgames'" />
    <LazyPuzzleThemeTiles v-else />
  </div>
</template>
