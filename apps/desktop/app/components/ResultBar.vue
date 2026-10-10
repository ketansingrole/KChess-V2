<script setup lang="ts">
import { computed } from 'vue'
import type { GameRecord } from '@kchess/contracts/types'
import { formatCount } from '../utils/format'

const props = defineProps<{ label: string; record: GameRecord; hint?: string }>()
const parts = computed(() => {
  const { win, draw, loss, total } = props.record
  const pct = (n: number) => (total ? (100 * n) / total : 0)
  return [
    { key: 'win', label: 'won', value: win, pct: pct(win) },
    { key: 'draw', label: 'drawn', value: draw, pct: pct(draw) },
    { key: 'loss', label: 'lost', value: loss, pct: pct(loss) },
  ]
})
const shown = computed(() => parts.value.filter((part) => part.value > 0))
const score = computed(() =>
  props.record.total
    ? Math.round((100 * (props.record.win + props.record.draw / 2)) / props.record.total)
    : 0,
)
const games = computed(
  () => `${formatCount(props.record.total)} game${props.record.total === 1 ? '' : 's'}`,
)
const title = computed(
  () =>
    `${props.label}: ${games.value} · ${props.record.win} won, ${props.record.draw} drawn, ${props.record.loss} lost · score ${score.value}%`,
)
</script>

<template>
  <div class="result-row">
    <span class="result-label"
      >{{ label }}<span v-if="hint" class="muted"> · {{ hint }}</span></span
    >
    <span class="result-value tabular"
      >{{ record.total ? `${score}%` : '–'
      }}<span v-if="record.total" class="result-unit"> score</span></span
    >
    <span class="result-count tabular muted">{{ games }}</span>
    <UTooltip
      :disabled="!record.total"
      :content="{ side: 'top' }"
      :ui="{ content: 'h-auto px-3 py-2 rounded-md' }"
    >
      <span class="result-bar" role="img" :aria-label="title" tabindex="0">
        <span
          v-for="part in shown"
          :key="part.key"
          class="result-part"
          :class="part.key"
          :style="{ flexBasis: `${part.pct}%` }"
        />
      </span>
      <template #content>
        <div class="result-tip">
          <span v-for="part in parts" :key="part.key" class="result-tip-line">
            <i class="result-swatch" :class="part.key" />
            <span class="tabular">{{ formatCount(part.value) }} {{ part.label }}</span>
            <span class="muted tabular">{{ Math.round(part.pct) }}%</span>
          </span>
          <span class="muted">Score {{ score }}%: wins plus half the draws</span>
        </div>
      </template>
    </UTooltip>
  </div>
</template>

<style scoped>
.result-row {
  /* The bar gets its own line: beside a long label in a narrow card it collapsed to slivers. */
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto 5.5rem;
  grid-template-areas:
    'label value count'
    'bar bar bar';
  align-items: baseline;
  gap: 4px 12px;
  padding-block: 3px;
  font-size: 13px;
}
.result-label {
  grid-area: label;
  min-width: 0;
  overflow-wrap: anywhere;
}
.result-bar {
  grid-area: bar;
  display: flex;
  align-items: center;
  gap: 2px;
  /* Taller than the bar it draws, so it is easy to hover. */
  height: 14px;
  margin-block: -4px;
  border-radius: 4px;
  cursor: default;
}
.result-bar:focus-visible {
  outline-offset: 1px;
}
.result-part {
  height: 6px;
  min-width: 2px;
  border-radius: 3px;
}
.result-unit {
  color: var(--ui-text-muted);
  font-size: 12px;
}
.result-tip {
  display: grid;
  min-width: 11rem;
  gap: 3px;
  font-size: 12px;
}
.result-tip-line {
  display: grid;
  grid-template-columns: 10px auto 1fr;
  align-items: center;
  gap: 6px;
}
.result-tip-line .muted {
  text-align: right;
}
.result-swatch {
  width: 10px;
  height: 10px;
  border-radius: 3px;
}
.result-swatch.win,
.result-part.win {
  background: var(--ui-success);
}
.result-swatch.draw,
.result-part.draw {
  background: var(--ui-border-accented);
}
.result-swatch.loss,
.result-part.loss {
  background: var(--ui-error);
}
.result-value {
  grid-area: value;
  text-align: right;
}
.result-count {
  grid-area: count;
  text-align: right;
}
</style>
