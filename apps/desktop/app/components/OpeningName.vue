<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
import type { GameSetup } from '@kchess/rules/variant'
import { loadOpenings, openingAt } from '@kchess/rules/openings'

const props = defineProps<{ setup: GameSetup; moves: readonly string[]; ply?: number }>()
const store = useKChessStore()
const table = shallowRef<Map<string, string> | null>(null)
watch(
  () => store.settings.showOpeningName,
  (show) => {
    if (show && !table.value)
      void loadOpenings()
        .then((value) => (table.value = value))
        .catch((error: unknown) => {
          console.warn('[opening-name] Could not load openings:', error)
          return undefined
        })
  },
  { immediate: true },
)
const opening = computed(() =>
  table.value && store.settings.showOpeningName
    ? openingAt(table.value, props.setup, props.moves, props.ply ?? props.moves.length)
    : undefined,
)
</script>

<template>
  <p v-if="opening" class="opening-name text-sm" :title="`${opening.eco} · ${opening.name}`">
    <span class="muted tabular">{{ opening.eco }}</span> {{ opening.name }}
  </p>
</template>

<style scoped>
.opening-name {
  margin: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
