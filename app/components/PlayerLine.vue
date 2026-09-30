<script setup lang="ts">
import type { Color } from '@lichess-org/chessground/types'

export interface PlayerInfo {
  name: string
  icon: string
  /** Short secondary text, e.g. "Thinking…". */
  status?: string
  /** Shows a spinner next to the status. */
  busy?: boolean
  clock?: string
  /** Connection state as Lichess reports it; leave unset for players with no such state (the engine). */
  presence?: PlayerPresence
}

export interface PlayerPresence {
  /** `checking` and `unavailable` are for the engine, the rest for people on Lichess. */
  state: 'online' | 'offline' | 'away' | 'checking' | 'unavailable'
  /** Text shown beside the dot (e.g. "Ready"); people show signal bars instead. */
  label?: string
  /** Lichess signal strength, 1 (poor) to 4 (great). */
  signal?: number
  /** Round trip to Lichess in milliseconds. */
  latencyMs?: number
}

const STATE_LABEL = {
  online: 'Online',
  offline: 'Offline',
  away: 'Disconnected',
  checking: 'Checking…',
  unavailable: 'Unavailable',
} as const

defineProps<{ player: PlayerInfo; color: Color; active?: boolean }>()

function presenceTitle(presence: PlayerPresence): string {
  const parts: string[] = [presence.label ?? STATE_LABEL[presence.state]]
  if (presence.state === 'online' && presence.signal) parts.push(`signal ${presence.signal} of 4`)
  if (presence.latencyMs !== undefined) parts.push(`${presence.latencyMs} ms`)
  return parts.join(' · ')
}
</script>

<template>
  <div class="player-line" :class="{ active }">
    <div class="player-id">
      <span class="player-avatar" :class="color" aria-hidden="true"
        ><UIcon :name="player.icon"
      /></span>
      <slot name="name"
        ><span class="player-name">{{ player.name }}</span></slot
      >
      <span
        v-if="player.presence"
        class="presence"
        :class="player.presence.state"
        :title="presenceTitle(player.presence)"
        role="img"
        :aria-label="presenceTitle(player.presence)"
      >
        <span class="presence-dot" />
        <span v-if="player.presence.label" class="presence-label">{{ player.presence.label }}</span>
        <span v-if="player.presence.state === 'online' && player.presence.signal" class="signal">
          <i v-for="bar in 4" :key="bar" :class="{ on: bar <= (player.presence.signal ?? 0) }" />
        </span>
        <span v-if="player.presence.latencyMs !== undefined" class="presence-ms tabular"
          >{{ player.presence.latencyMs }} ms</span
        >
      </span>
      <span class="sr-only">plays {{ color }}</span>
      <span v-if="player.status" class="player-status" role="status">
        <UIcon v-if="player.busy" name="i-lucide-loader-circle" class="animate-spin" />
        {{ player.status }}
      </span>
    </div>
    <span v-if="player.clock" class="player-clock" :aria-label="`${color} clock`">{{
      player.clock
    }}</span>
    <slot name="aside" />
  </div>
</template>
