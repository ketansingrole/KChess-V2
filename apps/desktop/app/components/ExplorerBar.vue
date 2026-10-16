<script setup lang="ts">
import { computed } from 'vue'

/** How the games from a position ended, as the explorer draws them: White, draws, Black. */
const props = defineProps<{ white?: number; draws?: number; black?: number; minLabel?: number }>()
const parts = computed(() => {
  const counts = [props.white ?? 0, props.draws ?? 0, props.black ?? 0]
  const total = counts.reduce((sum, count) => sum + count, 0)
  return (['w', 'd', 'b'] as const).map((key, index) => {
    const pct = total ? Math.round((100 * (counts[index] ?? 0)) / total) : 0
    return { key, pct, label: pct > (props.minLabel ?? 12) ? `${pct}%` : '' }
  })
})
</script>

<template>
  <span class="explorer-bar" :title="`White ${white} · Draw ${draws} · Black ${black}`">
    <span
      v-for="part in parts"
      :key="part.key"
      :class="part.key"
      :style="{ width: `${part.pct}%` }"
      >{{ part.label }}</span
    >
  </span>
</template>

<style scoped>
.explorer-bar {
  display: flex;
  flex: 1;
  height: 16px;
  border-radius: 4px;
  overflow: hidden;
  font-size: 10px;
  line-height: 16px;
  border: 1px solid var(--ui-border);
}
.explorer-bar > span {
  text-align: center;
  overflow: hidden;
}
.explorer-bar .w {
  background: #f5f5f5;
  color: #222;
}
.explorer-bar .d {
  background: #9e9e9e;
  color: #fff;
}
.explorer-bar .b {
  background: #333;
  color: #eee;
}
</style>
