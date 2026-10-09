<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes } from '../utils/format'
import type { PlayerPresence } from './PlayerLine.vue'

const props = defineProps<{
  ping: {
    current: number
    average: number
    min: number
    max: number
    samples: number[]
    quality: 1 | 2 | 3 | 4
  } | null
  /** How long Lichess took to accept the last move. */
  moveAckMs: number | null
  opponent?: PlayerPresence
  /** Bytes downloaded from Lichess for this account (play + status checks). */
  downloaded: number
  intervalSeconds: number
}>()

const QUALITY = ['', 'Poor', 'Fair', 'Good', 'Great'] as const
const points = computed(() => {
  const samples = props.ping?.samples ?? []
  if (samples.length < 2) return ''
  const ceiling = Math.max(200, ...samples)
  return samples
    .map((ms, index) => {
      const x = (index / (samples.length - 1)) * 100
      const y = 30 - (Math.min(ms, ceiling) / ceiling) * 28 - 1
      return `${x.toFixed(1)},${y.toFixed(1)}`
    })
    .join(' ')
})
const opponentText = computed(() => {
  const opponent = props.opponent
  if (!opponent) return '—'
  if (opponent.state === 'away') return 'Disconnected'
  if (opponent.state === 'checking') return 'Checking…'
  const base = opponent.state === 'online' ? 'Online' : 'Offline'
  return opponent.signal ? `${base} · signal ${opponent.signal}/4` : base
})
</script>

<template>
  <section class="connection" aria-label="Connection">
    <div class="connection-head">
      <span class="connection-ping tabular">
        <template v-if="ping"> {{ ping.current }}<small>ms ping</small> </template>
        <small v-else>Measuring ping…</small>
      </span>
      <span
        v-if="ping"
        class="presence online"
        :title="`Connection quality: ${QUALITY[ping.quality]}`"
      >
        <span class="signal">
          <i v-for="bar in 4" :key="bar" :class="{ on: bar <= ping.quality }" />
        </span>
        <span class="presence-label">{{ QUALITY[ping.quality] }}</span>
      </span>
    </div>
    <svg
      v-if="points"
      class="connection-graph"
      viewBox="0 0 100 30"
      preserveAspectRatio="none"
      role="img"
      aria-label="Recent ping"
    >
      <polyline
        :points="points"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        vector-effect="non-scaling-stroke"
      />
    </svg>
    <div class="connection-rows tabular">
      <span>Average / worst</span>
      <strong>{{ ping ? `${ping.average} / ${ping.max} ms` : '—' }}</strong>
      <span>Last move accepted in</span>
      <strong>{{ moveAckMs === null ? '—' : `${moveAckMs} ms` }}</strong>
      <span>Opponent</span>
      <strong>{{ opponentText }}</strong>
      <span>Downloaded this account</span>
      <strong>{{ formatBytes(downloaded) }}</strong>
    </div>
    <span class="muted">Measured live against lichess.org every {{ intervalSeconds }} s.</span>
  </section>
</template>
