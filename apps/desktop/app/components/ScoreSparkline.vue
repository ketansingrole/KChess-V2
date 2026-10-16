<script setup lang="ts">
import { computed } from 'vue'
import { VisArea, VisLine, VisXYContainer } from '@unovis/vue'

/** Recent run scores, oldest first, as a small area chart with no axes (a side panel's trend). */
const props = defineProps<{ scores: number[]; label: string }>()

interface Point {
  index: number
  score: number
}
const points = computed<Point[]>(() => props.scores.map((score, index) => ({ index, score })))
// Headroom above the best score, so equal scores read as a line rather than a filled box.
const domain = computed<[number, number]>(() => [0, Math.max(1, ...props.scores) * 1.25])
const x = (point: Point): number => point.index
const y = (point: Point): number => point.score
const primary = (): string => 'var(--ui-primary)'
</script>

<template>
  <div class="score-sparkline" role="img" :aria-label="label">
    <VisXYContainer :data="points" :height="44" :y-domain="domain" :padding="{ top: 2 }">
      <VisArea :x="x" :y="y" :color="primary" :opacity="0.14" />
      <VisLine :x="x" :y="y" :color="primary" :line-width="2" />
    </VisXYContainer>
  </div>
</template>
