<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { VisAxis, VisCrosshair, VisLine, VisTooltip, VisXYContainer } from '@unovis/vue'
import type { LichessRatingHistory } from '../../src/shared/types'
import { isPuzzleHistory, normalizeRatingKey, ratingDisplayName } from '../../src/shared/ratings'

/** Current rating per Lichess history name ("Blitz"), shown on the toggle chips. */
export interface CurrentRating {
  rating?: number
  provisional?: boolean
  progress?: number
}

const props = defineProps<{
  history: LichessRatingHistory
  current?: Record<string, CurrentRating>
  /** Shown when there is nothing to chart. */
  emptyText?: string
}>()

/**
 * Colour follows the speed, never its rank: each keeps its slot when others are hidden.
 * Slots come from the validated categorical palette; one variant takes the last slot.
 * Keys are Lichess perf keys (`blitz`, not `Blitz`); legacy display names normalize to them.
 */
const FIXED_SLOT_KEYS = [
  'bullet',
  'blitz',
  'rapid',
  'classical',
  'correspondence',
  'ultraBullet',
  'chess960',
]
const VARIANT_SLOT = 8
const DAY_MS = 86_400_000

interface Series {
  /** Perf key (`blitz`); stable identity for hiding, rows and current ratings. */
  key: string
  /** Human label (`Blitz`), shown on chips and tooltips. */
  name: string
  slot: number
  points: { date: number; rating: number }[]
}
const series = computed<Series[]>(() => {
  const withPoints = props.history
    .filter((entry) => entry.name && !isPuzzleHistory(entry.name) && entry.points?.length)
    .map((entry) => {
      const key = normalizeRatingKey(entry.name)
      return {
        key,
        name: ratingDisplayName(entry.name),
        points: entry.points!.map(([year, month, day, rating]) => ({
          date: Date.UTC(year!, month!, day!),
          rating: rating!,
        })),
      }
    })
  const fixed = withPoints
    .filter((entry) => FIXED_SLOT_KEYS.includes(entry.key))
    .map((entry) => ({ ...entry, slot: FIXED_SLOT_KEYS.indexOf(entry.key) + 1 }))
  // More than eight series cannot stay distinguishable; the most played variant gets the last slot.
  const variant = withPoints
    .filter((entry) => !FIXED_SLOT_KEYS.includes(entry.key))
    .sort((a, b) => b.points.length - a.points.length)[0]
  return [...fixed, ...(variant ? [{ ...variant, slot: VARIANT_SLOT }] : [])].sort(
    (a, b) => a.slot - b.slot,
  )
})

const hidden = ref(new Set<string>())
watch(
  () => props.history,
  () => (hidden.value = new Set()),
)
function toggle(key: string): void {
  const next = new Set(hidden.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  hidden.value = next
}
const shown = computed(() => series.value.filter((entry) => !hidden.value.has(entry.key)))

const RANGES = [
  { label: '1M', days: 31 },
  { label: '3M', days: 92 },
  { label: '1Y', days: 366 },
  { label: 'All', days: 0 },
] as const
const range = ref<(typeof RANGES)[number]['label']>('All')
const since = computed(() => {
  const days = RANGES.find((entry) => entry.label === range.value)?.days ?? 0
  return days ? Date.now() - days * DAY_MS : 0
})

/**
 * One row per day any shown speed changed. A rating holds until the next rated game, so each
 * speed carries its last value forward; before its first game it has none (a gap, not zero).
 */
type Row = { date: number } & Record<string, number | undefined>
const rows = computed<Row[]>(() => {
  const dates = new Set<number>()
  for (const entry of shown.value) for (const point of entry.points) dates.add(point.date)
  const sorted = [...dates].sort((a, b) => a - b)
  const cursors = shown.value.map(() => 0)
  const last: (number | undefined)[] = shown.value.map(() => undefined)
  const all = sorted.map((date) => {
    const row: Row = { date }
    shown.value.forEach((entry, index) => {
      while (cursors[index]! < entry.points.length && entry.points[cursors[index]!]!.date <= date)
        last[index] = entry.points[cursors[index]!++]!.rating
      row[entry.key] = last[index]
    })
    return row
  })
  const start = since.value
  if (!start) return all
  // Keep the value each speed had when the range starts, so lines begin at the left edge.
  const before = all.filter((row) => row.date < start).at(-1)
  const inside = all.filter((row) => row.date >= start)
  return before ? [{ ...before, date: start }, ...inside] : inside
})

const x = (row: Row): number => row.date
const accessors = computed(() =>
  shown.value.map((entry) => (row: Row) => row[entry.key] as number | undefined),
)
const colors = computed(() => shown.value.map((entry) => `var(--rating-series-${entry.slot})`))
const color = (_row: Row, index: number): string => colors.value[index] ?? 'var(--ui-primary)'

const PAD = 15
const domain = computed<[number, number]>(() => {
  const values = rows.value.flatMap((row) =>
    shown.value.map((entry) => row[entry.key]).filter((value) => value !== undefined),
  ) as number[]
  if (!values.length) return [0, 1]
  return [Math.min(...values) - PAD, Math.max(...values) + PAD]
})
const spansYears = computed(() => {
  const first = rows.value[0]?.date ?? 0
  const lastDate = rows.value.at(-1)?.date ?? 0
  return lastDate - first > 400 * DAY_MS
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
const escape = (text: string) =>
  text.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]!)
const tooltip = (row: Row): string => {
  const lines = shown.value
    .filter((entry) => row[entry.key] !== undefined)
    .map(
      (entry) =>
        `<span class="rh-tip-line"><i style="background:var(--rating-series-${entry.slot})"></i>${escape(entry.name)}<strong>${row[entry.key]}</strong></span>`,
    )
  return `<div class="rh-tip"><span class="rh-tip-date">${formatDate(row.date, true)}</span>${lines.join('')}</div>`
}
const summary = computed(
  () =>
    `Rating history: ${shown.value
      .map((entry) => `${entry.name} ${entry.points.at(-1)?.rating ?? ''}`)
      .join(', ')}`,
)
</script>

<template>
  <div class="rating-history">
    <div class="rh-toolbar">
      <div class="rh-chips" role="group" aria-label="Show ratings">
        <button
          v-for="entry in series"
          :key="entry.key"
          type="button"
          class="rh-chip"
          :class="{ off: hidden.has(entry.key) }"
          :aria-pressed="!hidden.has(entry.key)"
          :title="hidden.has(entry.key) ? `Show ${entry.name}` : `Hide ${entry.name}`"
          @click="toggle(entry.key)"
        >
          <i class="rh-swatch" :style="{ background: `var(--rating-series-${entry.slot})` }" />
          <span>{{ entry.name }}</span>
          <strong class="tabular">
            {{ current?.[entry.key]?.rating ?? entry.points.at(-1)?.rating
            }}{{ current?.[entry.key]?.provisional ? '?' : '' }}
          </strong>
          <span
            v-if="current?.[entry.key]?.progress"
            class="rh-progress tabular"
            :class="current[entry.key]!.progress! > 0 ? 'up' : 'down'"
            >{{ current[entry.key]!.progress! > 0 ? '+' : ''
            }}{{ current[entry.key]!.progress }}</span
          >
        </button>
      </div>
      <UFieldGroup size="xs">
        <UButton
          v-for="entry in RANGES"
          :key="entry.label"
          :variant="range === entry.label ? 'solid' : 'outline'"
          color="neutral"
          :aria-pressed="range === entry.label"
          @click="range = entry.label"
          >{{ entry.label }}</UButton
        >
      </UFieldGroup>
    </div>
    <div v-if="!series.length" class="rh-none muted text-sm">
      {{ emptyText ?? 'No rated games yet.' }}<slot name="empty" />
    </div>
    <p v-else-if="!shown.length" class="muted text-sm rh-empty">Pick a speed to show.</p>
    <p v-else-if="rows.length < 2" class="muted text-sm rh-empty">
      Not enough rated games in this range.
    </p>
    <div v-else class="rh-chart" role="img" :aria-label="summary">
      <VisXYContainer :data="rows" :height="260" :y-domain="domain" :padding="{ top: 8 }">
        <VisLine :x="x" :y="accessors" :color="color" :line-width="2" />
        <VisAxis
          type="x"
          :tick-format="(tick: number | Date) => formatDate(+tick)"
          :num-ticks="5"
          :grid-line="false"
        />
        <VisAxis type="y" :num-ticks="4" :domain-line="false" />
        <VisCrosshair :x="x" :y="accessors" :color="color" :template="tooltip" />
        <VisTooltip />
      </VisXYContainer>
    </div>
  </div>
</template>

<style>
/* Categorical palette (validated light and dark); colour follows the speed, see FIXED_SLOTS. */
.rating-history {
  --rating-series-1: #2a78d6;
  --rating-series-2: #eb6834;
  --rating-series-3: #1baf7a;
  --rating-series-4: #eda100;
  --rating-series-5: #e87ba4;
  --rating-series-6: #008300;
  --rating-series-7: #4a3aa7;
  --rating-series-8: #e34948;
  display: grid;
  gap: 12px;
}
.dark .rating-history {
  --rating-series-1: #3987e5;
  --rating-series-2: #d95926;
  --rating-series-3: #199e70;
  --rating-series-4: #c98500;
  --rating-series-5: #d55181;
  --rating-series-6: #008300;
  --rating-series-7: #9085e9;
  --rating-series-8: #e66767;
}
.rh-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px 16px;
}
.rh-chips {
  display: flex;
  flex: 1 1 18rem;
  flex-wrap: wrap;
  gap: 6px;
}
.rh-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border: 1px solid var(--ui-border);
  border-radius: 999px;
  background: var(--ui-bg);
  font-size: 0.8125rem;
  transition:
    opacity 0.15s,
    border-color 0.15s;
}
.rh-chip:hover {
  border-color: var(--ui-border-accented);
}
.rh-chip.off {
  opacity: 0.5;
}
.rh-chip.off .rh-swatch {
  background: transparent !important;
  box-shadow: inset 0 0 0 1.5px var(--ui-text-dimmed);
}
.rh-swatch {
  width: 10px;
  height: 10px;
  border-radius: 3px;
}
.rh-progress {
  font-size: 0.75rem;
}
.rh-progress.up {
  color: var(--ui-success);
}
.rh-progress.down {
  color: var(--ui-error);
}
.rh-none {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 8px;
}
.rh-empty {
  display: grid;
  place-items: center;
  height: 260px;
}
.rh-chart {
  --vis-axis-tick-label-color: var(--ui-text-dimmed);
  --vis-axis-tick-label-font-size: 10px;
  --vis-axis-grid-color: var(--ui-border);
  --vis-axis-tick-color: transparent;
  --vis-crosshair-line-stroke-color: var(--ui-border-accented);
  --vis-crosshair-circle-stroke-color: var(--ui-bg-elevated);
  --vis-tooltip-background-color: var(--ui-bg-elevated);
  --vis-tooltip-border-color: var(--ui-border);
  --vis-tooltip-text-color: var(--ui-text);
  --vis-tooltip-font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.rh-tip {
  display: grid;
  gap: 3px;
  min-width: 9rem;
}
.rh-tip-date {
  color: var(--ui-text-muted);
}
.rh-tip-line {
  display: grid;
  grid-template-columns: 10px 1fr auto;
  align-items: center;
  gap: 6px;
}
.rh-tip-line i {
  width: 10px;
  height: 10px;
  border-radius: 3px;
}
</style>
