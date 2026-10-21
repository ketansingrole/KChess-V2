<script setup lang="ts">
import { computed, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { areaPath, linePath, linearScale, nearestIndex, niceTicks } from '../utils/lineChart'

export interface LineSeries {
  /** One value per x; undefined leaves a gap. */
  values: readonly (number | undefined)[]
  color: string
  /** Fill the space under the line. */
  area?: boolean
}

/**
 * The app's line chart: one or more series over a shared ascending x, with optional axes and a
 * crosshair whose tooltip content comes from the `tooltip` slot (`{ index }` into `x`).
 */
const props = withDefaults(
  defineProps<{
    x: readonly number[]
    series: readonly LineSeries[]
    yDomain: readonly [number, number]
    height: number
    padTop?: number
    axes?: boolean
    xFormat?: (value: number) => string
    yFormat?: (value: number) => string
  }>(),
  { padTop: 8, axes: true, xFormat: String, yFormat: String },
)
defineSlots<{ tooltip?(props: { index: number }): unknown }>()

const root = ref<HTMLElement | null>(null)
const { width } = useElementSize(root)

const TICK_FONT_PX = 10
const yTicks = computed(() => (props.axes ? niceTicks(props.yDomain, 4) : []))
const yLabels = computed(() => yTicks.value.map((tick) => props.yFormat(tick)))
// Room for the widest y label (tabular digits are about 0.62em wide).
const left = computed(() =>
  props.axes
    ? Math.ceil(Math.max(0, ...yLabels.value.map((label) => label.length)) * TICK_FONT_PX * 0.62) +
      8
    : 0,
)
const bottom = computed(() => (props.axes ? 20 : 0))
const plotRight = computed(() => Math.max(left.value + 1, width.value - (props.axes ? 8 : 0)))
const plotBottom = computed(() => props.height - bottom.value)

const xDomain = computed<[number, number]>(() => [props.x[0] ?? 0, props.x.at(-1) ?? 0])
const xScale = computed(() => linearScale(xDomain.value, [left.value, plotRight.value]))
const yScale = computed(() =>
  linearScale(props.yDomain, [plotBottom.value, Math.min(props.padTop, plotBottom.value)]),
)
const xTicks = computed(() => (props.axes ? niceTicks(xDomain.value, 5) : []))

const paths = computed(() =>
  width.value
    ? props.series.map((entry) => ({
        color: entry.color,
        line: linePath(props.x, entry.values, xScale.value, yScale.value),
        area: entry.area
          ? areaPath(props.x, entry.values, xScale.value, yScale.value, plotBottom.value)
          : '',
      }))
    : [],
)

const active = ref(-1)
function onPointer(event: PointerEvent): void {
  if (!root.value || !props.x.length) return
  const px = event.clientX - root.value.getBoundingClientRect().left
  const span = plotRight.value - left.value
  const target =
    xDomain.value[0] + ((px - left.value) / span) * (xDomain.value[1] - xDomain.value[0])
  active.value = nearestIndex(props.x, target)
}
const crosshair = computed(() => {
  const index = active.value
  const xValue = props.x[index]
  if (xValue === undefined) return null
  const at = xScale.value(xValue)
  return {
    index,
    at,
    dots: props.series.flatMap((entry) => {
      const value = entry.values[index]
      return value === undefined ? [] : [{ color: entry.color, y: yScale.value(value) }]
    }),
    // The tooltip sits beside the crosshair, on whichever side has more room.
    flip: at > (left.value + plotRight.value) / 2,
  }
})
</script>

<template>
  <div
    ref="root"
    class="line-chart"
    :style="{ height: `${height}px` }"
    @pointermove="onPointer"
    @pointerleave="active = -1"
  >
    <svg v-if="width" :width="width" :height="height" aria-hidden="true">
      <g v-if="axes" class="line-chart-axis">
        <template v-for="(tick, index) in yTicks" :key="`y${tick}`">
          <line :x1="left" :x2="plotRight" :y1="yScale(tick)" :y2="yScale(tick)" class="grid" />
          <text :x="left - 6" :y="yScale(tick)" text-anchor="end" dominant-baseline="middle">
            {{ yLabels[index] }}
          </text>
        </template>
        <line :x1="left" :x2="plotRight" :y1="plotBottom" :y2="plotBottom" class="domain" />
        <text
          v-for="tick in xTicks"
          :key="`x${tick}`"
          :x="xScale(tick)"
          :y="plotBottom + 14"
          text-anchor="middle"
        >
          {{ xFormat(tick) }}
        </text>
      </g>
      <template v-for="(path, index) in paths" :key="index">
        <path v-if="path.area" :d="path.area" :fill="path.color" fill-opacity="0.14" />
        <path :d="path.line" fill="none" :stroke="path.color" stroke-width="2" />
      </template>
      <g v-if="crosshair" class="line-chart-crosshair">
        <line :x1="crosshair.at" :x2="crosshair.at" :y1="padTop" :y2="plotBottom" />
        <circle
          v-for="(dot, index) in crosshair.dots"
          :key="index"
          :cx="crosshair.at"
          :cy="dot.y"
          r="4"
          :fill="dot.color"
        />
      </g>
    </svg>
    <div
      v-if="crosshair && $slots.tooltip"
      class="line-chart-tooltip"
      :style="
        crosshair.flip
          ? { right: `${width - crosshair.at + 12}px` }
          : { left: `${crosshair.at + 12}px` }
      "
    >
      <slot name="tooltip" :index="crosshair.index" />
    </div>
  </div>
</template>

<style>
.line-chart {
  position: relative;
  width: 100%;
  font-variant-numeric: tabular-nums;
}
.line-chart svg {
  display: block;
  overflow: visible;
}
.line-chart-axis text {
  fill: var(--ui-text-dimmed);
  font-size: 10px;
}
.line-chart-axis .grid {
  stroke: var(--ui-border);
}
.line-chart-axis .domain {
  stroke: var(--ui-border-accented);
}
.line-chart-crosshair line {
  stroke: var(--line-chart-crosshair, var(--ui-primary));
}
.line-chart-crosshair circle {
  stroke: var(--line-chart-ring, var(--ui-primary));
  stroke-width: 1.5;
}
.line-chart-tooltip {
  position: absolute;
  top: 0;
  padding: 6px 10px;
  border: 1px solid var(--ui-border);
  border-radius: 6px;
  background: var(--ui-bg-elevated);
  color: var(--ui-text);
  font-size: 12px;
  white-space: nowrap;
  pointer-events: none;
  box-shadow: 0 2px 8px rgb(0 0 0 / 0.08);
}
</style>
