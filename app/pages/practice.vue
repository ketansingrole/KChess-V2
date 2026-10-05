<script setup lang="ts">
import { useLocalStorage } from '@vueuse/core'

const practiceTab = useLocalStorage('kchess:practice-tab', 'coordinates')

const tabs = [
  { label: 'Your mistakes', value: 'mistakes', icon: 'i-lucide-book-open' },
  { label: 'Openings', value: 'openings', icon: 'i-lucide-book-marked' },
  { label: 'Coordinates', value: 'coordinates', icon: 'i-lucide-grid-3x3' },
  { label: 'Square colours', value: 'colors', icon: 'i-lucide-contrast' },
  { label: 'Knight paths', value: 'knight', icon: 'i-lucide-crown' },
  { label: 'Endgames', value: 'endgames', icon: 'i-lucide-swords' },
  { label: 'Puzzle themes', value: 'themes', icon: 'i-lucide-puzzle' },
]
</script>

<template>
  <div>
    <PageHeader title="Practice">
      <SourceBadge kind="local" label="Local only · nothing here is sent to Lichess" />
    </PageHeader>
    <UTabs
      :model-value="practiceTab"
      :items="tabs"
      :content="false"
      variant="pill"
      size="sm"
      class="mb-5"
      @update:model-value="practiceTab = $event as string"
    />
    <MistakeTrainer v-if="practiceTab === 'mistakes'" />
    <OpeningTrainer v-else-if="practiceTab === 'openings'" />
    <CoordinatesTrainer v-else-if="practiceTab === 'coordinates'" />
    <SquareColorTrainer v-else-if="practiceTab === 'colors'" />
    <KnightPath v-else-if="practiceTab === 'knight'" />
    <EndgameDrills v-else-if="practiceTab === 'endgames'" />
    <PuzzleThemeTiles v-else />
  </div>
</template>
