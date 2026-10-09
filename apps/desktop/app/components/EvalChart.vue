<script setup lang="ts">
import { computed, ref } from 'vue'
import type { Judgment } from '@kchess/core/contracts/types'

/**
 * The game's evaluation over time, Lichess-style: White's winning chances fill from the bottom,
 * the review's labels sit on the curve, and a click jumps to that move.
 */
const props = defineProps<{
  /** White's winning chances (−1…1) per position, the start first; undefined where unknown. */
  points: (number | undefined)[]
  /** The label of move i (shown at position i + 1). */
  judgments: (Judgment | undefined)[]
  /** The position on the board, or −1 when it is off the reviewed line. */
  current: number
}>()
const emit = defineEmits<{ select: [position: number] }>()

const WIDTH = 400
const HEIGHT = 64
const x = (i: number): number =>
  props.points.length > 1 ? (i / (props.points.length - 1)) * WIDTH : 0
/** Kept off the edges so a dot for a won position stays whole. */
const PAD = 4
const y = (chances: number): number => PAD + ((1 - chances) / 2) * (HEIGHT - 2 * PAD)

/** White's area, one polygon per run of known positions. */
const areas = computed(() => {
  const runs: string[] = []
  let run: string[] = []
  const close = (last: number): void => {
    if (run.length > 1)
      runs.push(`${run.join(' ')} ${x(last)},${HEIGHT} ${run[0]!.split(',')[0]},${HEIGHT}`)
    run = []
  }
  props.points.forEach((chances, i) => {
    if (chances === undefined) close(i - 1)
    else run.push(`${x(i)},${y(chances)}`)
  })
  close(props.points.length - 1)
  return runs
})
const dots = computed(() =>
  props.judgments.flatMap((judgment, i) => {
    const chances = props.points[i + 1]
    return judgment && chances !== undefined
      ? [{ i: i + 1, judgment, cx: x(i + 1), cy: y(chances) }]
      : []
  }),
)

const svg = ref<SVGSVGElement | null>(null)
const hover = ref<number | null>(null)
function positionAt(event: MouseEvent): number {
  const box = svg.value!.getBoundingClientRect()
  const ratio = Math.max(0, Math.min(1, (event.clientX - box.left) / box.width))
  return Math.round(ratio * (props.points.length - 1))
}
function keydown(event: KeyboardEvent): void {
  const last = props.points.length - 1
  const from = props.current < 0 ? 0 : props.current
  if (event.key === 'ArrowLeft') emit('select', Math.max(0, from - 1))
  else if (event.key === 'ArrowRight') emit('select', Math.min(last, from + 1))
  else return
  event.preventDefault()
  event.stopPropagation()
}
</script>

<template>
  <svg
    ref="svg"
    class="eval-chart"
    :viewBox="`0 0 ${WIDTH} ${HEIGHT}`"
    preserveAspectRatio="none"
    role="slider"
    tabindex="0"
    aria-label="Evaluation over the game; select a move"
    :aria-valuemin="0"
    :aria-valuemax="Math.max(0, points.length - 1)"
    :aria-valuenow="Math.max(0, current)"
    @mousemove="hover = positionAt($event)"
    @mouseleave="hover = null"
    @click="emit('select', positionAt($event))"
    @keydown="keydown"
  >
    <polygon v-for="(area, index) in areas" :key="index" class="white-area" :points="area" />
    <line class="mid" x1="0" :x2="WIDTH" :y1="HEIGHT / 2" :y2="HEIGHT / 2" />
    <line v-if="hover !== null" class="guide" :x1="x(hover)" :x2="x(hover)" y1="0" :y2="HEIGHT" />
    <line
      v-if="current >= 0"
      class="cursor"
      :x1="x(current)"
      :x2="x(current)"
      y1="0"
      :y2="HEIGHT"
    />
    <circle
      v-for="dot in dots"
      :key="dot.i"
      :class="['dot', dot.judgment]"
      :cx="dot.cx"
      :cy="dot.cy"
      r="3.2"
    />
  </svg>
</template>

<style scoped>
.eval-chart {
  display: block;
  width: 100%;
  height: 64px;
  border-radius: 6px;
  background: #403d39;
  cursor: pointer;
  overflow: hidden;
}
.eval-chart:focus-visible {
  outline: 2px solid var(--ui-primary);
  outline-offset: 2px;
}
.white-area {
  fill: #f2f0ea;
}
.mid {
  stroke: rgb(214 79 0 / 55%);
  stroke-width: 1;
  vector-effect: non-scaling-stroke;
}
.guide,
.cursor {
  stroke-width: 2;
  vector-effect: non-scaling-stroke;
}
.guide {
  stroke: rgb(127 127 127 / 55%);
}
.cursor {
  stroke: var(--ui-primary);
}
.dot {
  stroke: #403d39;
  stroke-width: 1;
  vector-effect: non-scaling-stroke;
}
.dot.inaccuracy {
  fill: var(--judgment-inaccuracy);
}
.dot.mistake {
  fill: var(--judgment-mistake);
}
.dot.blunder {
  fill: var(--judgment-blunder);
}
</style>
