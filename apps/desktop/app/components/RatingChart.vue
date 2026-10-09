<script setup lang="ts">
import { computed } from 'vue'
import { VisArea, VisAxis, VisCrosshair, VisLine, VisTooltip, VisXYContainer } from '@unovis/vue'

export interface RatingPoint {
  /** Day of the rating, as a UTC timestamp. */
  date: number
  rating: number
}

const props = defineProps<{ points: RatingPoint[]; label: string }>()

const x = (point: RatingPoint): number => point.date
const y = (point: RatingPoint): number => point.rating

const PAD = 15
const domain = computed<[number, number]>(() => {
  const ratings = props.points.map(y)
  return [Math.min(...ratings) - PAD, Math.max(...ratings) + PAD]
})

/** Short spans need the day; longer ones read better as month and year. */
const SPAN_FOR_YEARS_MS = 400 * 86_400_000
const spansYears = computed(() => {
  const dates = props.points.map(x)
  return Math.max(...dates) - Math.min(...dates) > SPAN_FOR_YEARS_MS
})
const formatDate = (timestamp: number, long = false): string =>
  new Date(timestamp).toLocaleDateString(
    undefined,
    long
      ? { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' }
      : spansYears.value
        ? { month: 'short', year: '2-digit', timeZone: 'UTC' }
        : { month: 'short', day: 'numeric', timeZone: 'UTC' },
  )

const tooltip = (point: RatingPoint): string =>
  `<strong>${point.rating}</strong><br><span>${formatDate(point.date, true)}</span>`
const primary = (): string => 'var(--ui-primary)'
</script>

<template>
  <div
    class="rating-chart"
    role="img"
    :aria-label="`${label} rating history from ${Math.min(...points.map(y))} to ${Math.max(...points.map(y))}`"
  >
    <VisXYContainer :data="points" :height="240" :y-domain="domain" :padding="{ top: 8 }">
      <VisArea :x="x" :y="y" :color="primary" :opacity="0.14" />
      <VisLine :x="x" :y="y" :color="primary" :line-width="2" />
      <VisAxis
        type="x"
        :tick-format="(tick: number | Date) => formatDate(+tick)"
        :num-ticks="5"
        :grid-line="false"
      />
      <VisAxis type="y" :num-ticks="4" :domain-line="false" />
      <VisCrosshair :x="x" :y="y" :template="tooltip" :color="primary" />
      <VisTooltip />
    </VisXYContainer>
  </div>
</template>

<style>
.rating-chart {
  margin-top: 4px;
  --vis-axis-tick-label-color: var(--ui-text-dimmed);
  --vis-axis-tick-label-font-size: 10px;
  --vis-axis-grid-color: var(--ui-border);
  --vis-axis-tick-color: transparent;
  --vis-crosshair-line-stroke-color: var(--ui-primary);
  --vis-crosshair-circle-stroke-color: var(--ui-primary);
  --vis-tooltip-background-color: var(--ui-bg-elevated);
  --vis-tooltip-border-color: var(--ui-border);
  --vis-tooltip-text-color: var(--ui-text);
  --vis-tooltip-font-size: 12px;
  font-variant-numeric: tabular-nums;
}
</style>
