<script setup lang="ts">
import { computed } from 'vue'
import type { GameRecord } from '../../src/shared/types'

const props = defineProps<{ label: string; record: GameRecord; hint?: string }>()
const parts = computed(() => {
  const { win, draw, loss, total } = props.record
  const pct = (n: number) => (total ? (100 * n) / total : 0)
  return [
    { key: 'win', value: win, pct: pct(win) },
    { key: 'draw', value: draw, pct: pct(draw) },
    { key: 'loss', value: loss, pct: pct(loss) },
  ].filter((part) => part.value > 0)
})
const score = computed(() =>
  props.record.total
    ? Math.round((100 * (props.record.win + props.record.draw / 2)) / props.record.total)
    : 0,
)
const title = computed(
  () =>
    `${props.label}: ${props.record.total} games · ${props.record.win} won, ${props.record.draw} drawn, ${props.record.loss} lost · score ${score.value}%`,
)
</script>

<template>
  <div class="result-row" :title="title">
    <span class="result-label"
      >{{ label }}<span v-if="hint" class="muted"> · {{ hint }}</span></span
    >
    <span class="result-bar" role="img" :aria-label="title">
      <span
        v-for="part in parts"
        :key="part.key"
        class="result-part"
        :class="part.key"
        :style="{ flexBasis: `${part.pct}%` }"
      />
    </span>
    <span class="result-value tabular">{{ record.total ? `${score}%` : '–' }}</span>
    <span class="result-count tabular muted">{{ record.total }}</span>
  </div>
</template>

<style scoped>
.result-row {
  display: grid;
  grid-template-columns: minmax(7rem, 14rem) minmax(0, 1fr) 3rem 3rem;
  align-items: center;
  gap: 10px;
  font-size: 13px;
}
.result-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.result-bar {
  display: flex;
  gap: 2px;
  height: 12px;
}
.result-part {
  min-width: 2px;
  border-radius: 4px;
}
.result-part.win {
  background: var(--ui-success);
}
.result-part.draw {
  background: var(--ui-border-accented);
}
.result-part.loss {
  background: var(--ui-error);
}
.result-value,
.result-count {
  text-align: right;
}
</style>
