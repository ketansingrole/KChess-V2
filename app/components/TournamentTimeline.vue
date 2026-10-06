<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import LichessIcon from './LichessIcon.vue'
import type { TournamentSummary } from '../../src/shared/types'

/** Lichess's arena schedule as a timeline: one row per series, a bar per arena, a line for now. */
const props = defineProps<{
  arenas: TournamentSummary[]
  selectedId?: string
  isJoined: (id: string) => boolean
}>()
const emit = defineEmits<{ select: [id: string] }>()

const MINUTE = 60_000
const PX_PER_MINUTE = 9
const BEFORE_NOW = 60 * MINUTE
const MIN_AHEAD = 3 * 60 * MINUTE
const MAX_AHEAD = 8 * 60 * MINUTE

const now = ref(Date.now())
useIntervalFn(() => (now.value = Date.now()), 30_000)

const SPEEDS = ['bullet', 'blitz', 'rapid', 'classical', 'ultraBullet'] as const
const SPECIAL = new Set([
  'eastern',
  'daily',
  'weekly',
  'weekend',
  'monthly',
  'yearly',
  'shield',
  'marathon',
  'unique',
])
type Category = 'standard' | 'special' | 'capped' | 'variant'
const CATEGORIES: { key: Category; label: string }[] = [
  { key: 'standard', label: 'Hourly & player-made' },
  { key: 'special', label: 'Special events' },
  { key: 'capped', label: 'Rating-limited' },
  { key: 'variant', label: 'Variants' },
]
function categoryOf(t: TournamentSummary): Category {
  if (t.maxRating) return 'capped'
  if (t.perf && !(SPEEDS as readonly string[]).includes(t.perf)) return 'variant'
  return t.freq && SPECIAL.has(t.freq) ? 'special' : 'standard'
}
/** Speeds first (as Lichess orders them), then rating-limited events, then variants. */
function groupOf(t: TournamentSummary): number {
  return categoryOf(t) === 'capped' ? 1 : categoryOf(t) === 'variant' ? 2 : 0
}
function perfOrder(t: TournamentSummary): number {
  const index = (SPEEDS as readonly string[]).indexOf(t.perf ?? '')
  return index < 0 ? 10 : index
}
const endOf = (t: TournamentSummary) => t.finishesAt ?? t.startsAt + (t.minutes ?? 60) * MINUTE

const range = computed(() => {
  const start = Math.floor((now.value - BEFORE_NOW) / (10 * MINUTE)) * 10 * MINUTE
  const latest = Math.max(now.value + MIN_AHEAD, ...props.arenas.map(endOf))
  const end = Math.min(latest, now.value + MAX_AHEAD)
  return { start, end: Math.ceil(end / (10 * MINUTE)) * 10 * MINUTE }
})
// Match Lichess's schedule: second-level differences do not shift bars or split lanes.
const minuteOf = (time: number) => Math.floor(time / MINUTE) * MINUTE
const x = (time: number) => ((minuteOf(time) - range.value.start) / MINUTE) * PX_PER_MINUTE
const width = computed(() => x(range.value.end))

/**
 * Packs arenas into rows the way Lichess does: speeds, then rating-limited events, then variants.
 * Within a group, arenas are placed in start order (which needs the fewest rows), each on the
 * row that last held the same series when it fits, otherwise on the first row with room.
 */
const seriesOf = (t: TournamentSummary) => `${t.perf ?? ''}:${t.maxRating ?? ''}:${t.freq ?? ''}`
const rows = computed(() => {
  const visible = props.arenas
    .filter((t) => endOf(t) > range.value.start && t.startsAt < range.value.end)
    .sort(
      (a, b) =>
        groupOf(a) - groupOf(b) ||
        Math.max(minuteOf(a.startsAt), range.value.start) -
          Math.max(minuteOf(b.startsAt), range.value.start) ||
        perfOrder(a) - perfOrder(b) ||
        (a.maxRating ?? 0) - (b.maxRating ?? 0),
    )
  const lanes: { group: number; end: number; series: string; items: TournamentSummary[] }[] = []
  for (const t of visible) {
    const group = groupOf(t)
    const start = Math.max(minuteOf(t.startsAt), range.value.start)
    const free = lanes.filter((l) => l.group === group && l.end <= start)
    const lane = free.find((l) => l.series === seriesOf(t)) ?? free[0]
    if (lane) {
      lane.items.push(t)
      lane.end = minuteOf(endOf(t))
      lane.series = seriesOf(t)
    } else lanes.push({ group, end: minuteOf(endOf(t)), series: seriesOf(t), items: [t] })
  }
  return lanes.map((lane, index) => ({ key: `${lane.group}-${index}`, items: lane.items }))
})

const ticks = computed(() => {
  const list: { time: number; hour: boolean; label: string }[] = []
  for (let time = range.value.start; time <= range.value.end; time += 10 * MINUTE) {
    const date = new Date(time)
    const hour = date.getMinutes() === 0
    list.push({
      time,
      hour,
      // Hours read in the reader's clock ("2 PM" or "14:00"); minutes stay short (":10").
      label: hour
        ? date.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
        : `:${String(date.getMinutes()).padStart(2, '0')}`,
    })
  }
  return list
})

function control(t: TournamentSummary): string {
  const minutes = t.clock.limit / 60
  const base = { 0.25: '¼', 0.5: '½', 0.75: '¾' }[minutes] ?? String(minutes)
  return `${base}+${t.clock.increment}`
}
function describe(t: TournamentSummary): string {
  const start = new Date(t.startsAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  const state =
    t.status === 'finished' ? 'finished' : t.status === 'started' ? 'running' : `starts ${start}`
  return `${t.name}, ${control(t)} ${t.rated ? 'rated' : 'casual'}, ${t.nbPlayers} players, ${state}${t.playable ? '' : ', watch only in KChess'}`
}

const scroller = ref<HTMLElement | null>(null)
async function scrollToNow(): Promise<void> {
  await nextTick()
  const el = scroller.value
  if (el) el.scrollLeft = Math.max(0, x(now.value) - 30 * PX_PER_MINUTE)
}
onMounted(scrollToNow)
watch(() => props.arenas.length > 0, scrollToNow)
defineExpose({ scrollToNow })
</script>

<template>
  <div class="timeline">
    <div class="timeline-legend text-xs">
      <span v-for="category in CATEGORIES" :key="category.key"
        ><i class="timeline-swatch" :class="category.key" /> {{ category.label }}</span
      >
    </div>
    <div ref="scroller" class="timeline-scroll">
      <div class="timeline-canvas" :style="{ width: `${width}px` }">
        <div class="timeline-axis" aria-hidden="true">
          <span
            v-for="tick in ticks"
            :key="tick.time"
            class="timeline-tick"
            :class="{ hour: tick.hour }"
            :style="{ left: `${x(tick.time)}px` }"
            >{{ tick.label }}</span
          >
        </div>
        <div class="timeline-rows" role="list" aria-label="Arena schedule">
          <div v-for="row in rows" :key="row.key" class="timeline-row">
            <button
              v-for="t in row.items"
              :key="t.id"
              type="button"
              role="listitem"
              class="timeline-bar"
              :class="[
                categoryOf(t),
                t.status,
                { selected: selectedId === t.id, joined: isJoined(t.id) },
              ]"
              :style="{
                left: `${Math.max(0, x(t.startsAt))}px`,
                width: `${Math.max(1, x(Math.min(endOf(t), range.end)) - Math.max(0, x(t.startsAt)))}px`,
              }"
              :aria-label="describe(t)"
              :title="describe(t)"
              @click="emit('select', t.id)"
            >
              <LichessIcon
                :name="t.freq === 'shield' ? 'shield' : (t.perf ?? 'trophy')"
                class="timeline-icon"
              />
              <span class="timeline-text">
                <span class="timeline-name">{{ t.name }}</span>
                <span class="timeline-sub">
                  {{ control(t) }} {{ t.rated ? 'Rated' : 'Casual' }}
                  <template v-if="t.nbPlayers">
                    · <LichessIcon name="user" class="text-xs align-[-2px]" />
                    {{ t.nbPlayers }}</template
                  >
                  <LichessIcon
                    v-if="isJoined(t.id)"
                    name="check"
                    class="text-xs align-[-2px] ms-1"
                  />
                </span>
              </span>
            </button>
          </div>
        </div>
        <div class="timeline-now" :style="{ left: `${x(now)}px` }" aria-hidden="true" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.timeline {
  --tl-standard: #3a8a30;
  --tl-special: #1a6fb8;
  --tl-capped: #6152e0;
  --tl-variant: #7f5232;
  display: grid;
  gap: 10px;
}
.timeline-legend {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 16px;
  color: var(--ui-text-muted);
}
.timeline-swatch {
  display: inline-block;
  width: 10px;
  height: 10px;
  border-radius: 3px;
  vertical-align: -1px;
}
.timeline-swatch.standard,
.timeline-bar.standard {
  background: var(--tl-standard);
}
.timeline-swatch.special,
.timeline-bar.special {
  background: var(--tl-special);
}
.timeline-swatch.capped,
.timeline-bar.capped {
  background: var(--tl-capped);
}
.timeline-swatch.variant,
.timeline-bar.variant {
  background: var(--tl-variant);
}
.timeline-scroll {
  max-height: min(70dvh, 760px);
  overflow: auto;
  overscroll-behavior: contain;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg);
}
.timeline-canvas {
  position: relative;
  min-height: 100%;
}
.timeline-axis {
  position: sticky;
  top: 0;
  z-index: 2;
  height: 30px;
  border-bottom: 1px solid var(--ui-border);
  background: var(--ui-bg);
}
.timeline-tick {
  position: absolute;
  top: 0;
  bottom: 0;
  padding: 7px 0 0 6px;
  border-left: 1px solid var(--ui-border);
  color: var(--ui-text-dimmed);
  font-size: 0.75rem;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.timeline-tick.hour {
  color: var(--ui-text-highlighted);
  font-weight: 600;
  border-left-color: var(--ui-border-accented);
}
.timeline-rows {
  display: grid;
  gap: 8px;
  padding: 8px 0 10px;
}
.timeline-row {
  position: relative;
  height: 46px;
}
.timeline-bar {
  position: absolute;
  top: 0;
  bottom: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  padding: 0 10px;
  overflow: hidden;
  border-radius: 6px;
  color: #fff;
  text-align: left;
  transition:
    filter 0.15s,
    box-shadow 0.15s;
}
.timeline-bar:hover {
  filter: brightness(1.12);
}
.timeline-bar.finished {
  opacity: 0.42;
}
.timeline-bar.selected {
  box-shadow:
    0 0 0 2px var(--ui-bg),
    0 0 0 4px var(--ui-text-highlighted);
}
.timeline-icon {
  flex: none;
  font-size: 1.5rem;
}
.timeline-text {
  display: grid;
  min-width: 0;
  line-height: 1.2;
}
.timeline-name {
  overflow: hidden;
  font-size: 0.875rem;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.timeline-sub {
  overflow: hidden;
  font-size: 0.8125rem;
  opacity: 0.9;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
.timeline-now {
  position: absolute;
  top: 0;
  bottom: 0;
  z-index: 3;
  width: 2px;
  background: var(--ui-error);
  pointer-events: none;
}
.timeline-now::before {
  content: '';
  position: absolute;
  top: 0;
  left: -5px;
  border: 6px solid transparent;
  border-top-color: var(--ui-error);
}
</style>
