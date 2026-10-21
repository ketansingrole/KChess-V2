<script setup lang="ts">
import { computed } from 'vue'
import { formatRatingDate, ratingYDomain, spansRatingYears } from '../utils/ratingChart'

export interface RatingPoint {
  /** Day of the rating, as a UTC timestamp. */
  date: number
  rating: number
}

/**
 * A value over time, drawn as a rating history by default. Other series (an opening's
 * popularity) pass their own value format, range, height and accessible summary.
 */
const props = defineProps<{
  points: RatingPoint[]
  label: string
  format?: (value: number) => string
  yDomain?: [number, number]
  height?: number
  summary?: string
}>()

const dates = computed(() => props.points.map((point) => point.date))
const ratings = computed(() => props.points.map((point) => point.rating))
const domain = computed<[number, number]>(() => props.yDomain ?? ratingYDomain(ratings.value))
const series = computed(() => [{ values: ratings.value, color: 'var(--ui-primary)', area: true }])
const value = (rating: number): string => props.format?.(rating) ?? String(rating)

/** Short spans need the day; longer ones read better as month and year. */
const spansYears = computed(() =>
  spansRatingYears(Math.min(...dates.value), Math.max(...dates.value)),
)
const formatDate = (timestamp: number, long = false): string =>
  formatRatingDate(timestamp, long, spansYears.value)
</script>

<template>
  <div
    class="rating-chart"
    role="img"
    :aria-label="
      summary ?? `${label} rating history from ${Math.min(...ratings)} to ${Math.max(...ratings)}`
    "
  >
    <LineChart
      :x="dates"
      :series="series"
      :y-domain="domain"
      :height="height ?? 240"
      :x-format="formatDate"
      :y-format="value"
    >
      <template #tooltip="{ index }">
        <strong>{{ value(points[index]!.rating) }}</strong
        ><br />
        <span>{{ formatDate(points[index]!.date, true) }}</span>
      </template>
    </LineChart>
  </div>
</template>

<style>
.rating-chart {
  margin-top: 4px;
}
</style>
