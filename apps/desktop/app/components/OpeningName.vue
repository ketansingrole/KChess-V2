<script setup lang="ts">
import { computed } from 'vue'
import type { GameSetup } from '@kchess/rules/variant'
import { openingAt } from '@kchess/rules/openings'

const props = defineProps<{ setup: GameSetup; moves: readonly string[]; ply?: number }>()
const store = useKChessStore()
const shown = computed(() => store.settings.showOpeningName && props.setup.variant === 'standard')
const opening = computed(() =>
  shown.value ? openingAt(props.setup, props.moves, props.ply ?? props.moves.length) : undefined,
)
</script>

<template>
  <!-- The line stays before an opening is named, so naming one moves nothing below it. -->
  <p
    v-if="shown"
    class="opening-name text-sm"
    :title="opening ? `${opening.eco} · ${opening.name}` : undefined"
  >
    <template v-if="opening"
      ><span class="muted tabular">{{ opening.eco }}</span> {{ opening.name }}</template
    >
    <template v-else>&nbsp;</template>
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
