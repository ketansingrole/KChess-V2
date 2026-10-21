<script setup lang="ts">
import { computed } from 'vue'

/** Recent run scores, oldest first, as a small area chart with no axes (a side panel's trend). */
const props = defineProps<{ scores: number[]; label: string }>()

const x = computed(() => props.scores.map((_, index) => index))
// Headroom above the best score, so equal scores read as a line rather than a filled box.
const domain = computed<[number, number]>(() => [0, Math.max(1, ...props.scores) * 1.25])
const series = computed(() => [{ values: props.scores, color: 'var(--ui-primary)', area: true }])
</script>

<template>
  <div class="score-sparkline" role="img" :aria-label="label">
    <LineChart :x="x" :series="series" :y-domain="domain" :height="44" :pad-top="2" :axes="false" />
  </div>
</template>
