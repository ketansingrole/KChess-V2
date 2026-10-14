<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes } from '../utils/format'
import type { PlayerPresence } from './PlayerLine.vue'

/** One player's connection, shown when their signal bars are pointed at. */
const props = defineProps<
  | {
      side: 'you'
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
      /** Bytes downloaded from Lichess for this account (play + status checks). */
      downloaded: number
      intervalSeconds: number
    }
  | { side: 'opponent'; name: string; presence?: PlayerPresence }
>()

const QUALITY = ['', 'Poor', 'Fair', 'Good', 'Great'] as const
const points = computed(() => {
  const samples = props.side === 'you' ? (props.ping?.samples ?? []) : []
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
const opponentState = computed(() => {
  if (props.side !== 'opponent' || !props.presence) return '—'
  const state = props.presence.state
  if (state === 'away') return 'Disconnected'
  if (state === 'checking') return 'Checking…'
  return state === 'online' ? 'Online' : 'Offline'
})
const opponentSignal = computed(() => {
  const signal = props.side === 'opponent' ? props.presence?.signal : undefined
  return signal ? `${QUALITY[signal as 1 | 2 | 3 | 4]} · ${signal} of 4` : '—'
})
</script>

<template>
  <section v-if="props.side === 'you'" class="connection" aria-label="Your connection">
    <div class="connection-head">
      <span class="connection-ping tabular">
        <template v-if="props.ping"> {{ props.ping.current }}<small>ms ping</small> </template>
        <small v-else>Measuring ping…</small>
      </span>
      <span v-if="props.ping" class="presence online">
        <span class="presence-label">{{ QUALITY[props.ping.quality] }}</span>
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
      <strong>{{ props.ping ? `${props.ping.average} / ${props.ping.max} ms` : '—' }}</strong>
      <span>Last move accepted in</span>
      <strong>{{ props.moveAckMs === null ? '—' : `${props.moveAckMs} ms` }}</strong>
      <span>Downloaded this account</span>
      <strong>{{ formatBytes(props.downloaded) }}</strong>
    </div>
    <span class="muted">Measured against lichess.org every {{ props.intervalSeconds }} s.</span>
  </section>
  <section v-else class="connection" :aria-label="`${props.name}'s connection`">
    <strong class="connection-name">{{ props.name }}</strong>
    <div class="connection-rows tabular">
      <span>Status</span>
      <strong>{{ opponentState }}</strong>
      <span>Signal</span>
      <strong>{{ opponentSignal }}</strong>
    </div>
    <span class="muted">As reported by Lichess.</span>
  </section>
</template>
