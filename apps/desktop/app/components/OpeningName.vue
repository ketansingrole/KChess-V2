<script setup lang="ts">
import { computed } from 'vue'
import type { GameSetup } from '@kchess/rules/variant'
import { openingAt } from '@kchess/rules/openings'

const props = defineProps<{ setup: GameSetup; moves: readonly string[]; ply?: number }>()
const store = useKChessStore()
const opening = computed(() =>
  store.settings.showOpeningName
    ? openingAt(props.setup, props.moves, props.ply ?? props.moves.length)
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
  /* No margin of its own: it takes the spacing of the panel it sits in. */
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
