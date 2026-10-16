<script setup lang="ts">
import { computed } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import { PRACTICE_TAB_KEY } from '../utils/storageKeys'

const storedTab = useLocalStorage(PRACTICE_TAB_KEY, 'coordinates')

const tabs = [
  { label: 'Coordinates', value: 'coordinates', icon: 'i-lucide-grid-3x3' },
  { label: 'Square colours', value: 'colors', icon: 'i-lucide-contrast' },
  { label: 'Knight paths', value: 'knight', icon: 'i-lucide-crown' },
  { label: 'Endgames', value: 'endgames', icon: 'i-lucide-swords' },
  { label: 'Openings', value: 'openings', icon: 'i-lucide-book-marked' },
  { label: 'Your mistakes', value: 'mistakes', icon: 'i-lucide-book-open' },
]
// A tab saved by an earlier release (the removed puzzle themes) falls back to the first trainer.
const practiceTab = computed({
  get: () => (tabs.some((tab) => tab.value === storedTab.value) ? storedTab.value : 'coordinates'),
  set: (value: string) => (storedTab.value = value),
})
</script>

<template>
  <div>
    <PageHeader title="Practice" />
    <UTabs
      :model-value="practiceTab"
      :items="tabs"
      :content="false"
      variant="pill"
      class="mb-5 zen-hide"
      @update:model-value="practiceTab = $event as string"
    />
    <LazyMistakeTrainer v-if="practiceTab === 'mistakes'" />
    <LazyOpeningTrainer v-else-if="practiceTab === 'openings'" />
    <LazySquareColorTrainer v-else-if="practiceTab === 'colors'" />
    <LazyKnightPath v-else-if="practiceTab === 'knight'" />
    <LazyEndgameTraining v-else-if="practiceTab === 'endgames'" />
    <CoordinatesTrainer v-else />
  </div>
</template>
