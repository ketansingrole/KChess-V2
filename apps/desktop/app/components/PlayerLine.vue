<script setup lang="ts">
import { createReusableTemplate } from '@vueuse/core'
import type { Color } from '@lichess-org/chessground/types'

export interface PlayerInfo {
  name: string
  /** Lichess username; when set the name opens the player's profile. */
  username?: string
  /** Show the profile as a card in place instead of leaving the page (during a live game). */
  card?: boolean
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

defineProps<{
  player: PlayerInfo
  color: Color
  active?: boolean
  /** The position whose material difference to show beside the name; omit to show none. */
  materialFen?: string
}>()

/** The dot, bars and latency, drawn the same with or without a details popover. */
const [DefinePresence, PresenceBadge] = createReusableTemplate<{ presence: PlayerPresence }>()

function presenceTitle(presence: PlayerPresence): string {
  const parts: string[] = [presence.label ?? STATE_LABEL[presence.state]]
  if (presence.state === 'online' && presence.signal) parts.push(`signal ${presence.signal} of 4`)
  if (presence.latencyMs !== undefined) parts.push(`${presence.latencyMs} ms`)
  return parts.join(' · ')
}
</script>

<template>
  <DefinePresence v-slot="{ presence }">
    <span class="presence-dot" />
    <span v-if="presence.label" class="presence-label">{{ presence.label }}</span>
    <span v-if="presence.state === 'online' && presence.signal" class="signal">
      <i v-for="bar in 4" :key="bar" :class="{ on: bar <= (presence.signal ?? 0) }" />
    </span>
    <span v-if="presence.latencyMs !== undefined" class="presence-ms tabular"
      >{{ presence.latencyMs }} ms</span
    >
  </DefinePresence>
  <div class="player-line" :class="{ active }">
    <div class="player-id">
      <span class="player-avatar" :class="color" aria-hidden="true"
        ><UIcon :name="player.icon"
      /></span>
      <slot name="name"
        ><UPopover v-if="player.username && player.card">
          <button
            type="button"
            class="player-name player-link"
            :aria-label="`${player.name}: show profile`"
          >
            {{ player.name }}
          </button>
          <template #content><PlayerCard :username="player.username" full /></template> </UPopover
        ><PlayerLink v-else-if="player.username" :username="player.username" class="player-name">{{
          player.name
        }}</PlayerLink
        ><span v-else class="player-name">{{ player.name }}</span></slot
      >
      <!-- With details, pointing at this player's own bars shows that player's connection. -->
      <UPopover
        v-if="player.presence && $slots.presence"
        mode="hover"
        :open-delay="150"
        :close-delay="150"
      >
        <button
          type="button"
          class="presence"
          :class="player.presence.state"
          :aria-label="presenceTitle(player.presence)"
        >
          <PresenceBadge :presence="player.presence" />
        </button>
        <template #content><slot name="presence" /></template>
      </UPopover>
      <span
        v-else-if="player.presence"
        class="presence"
        :class="player.presence.state"
        :title="presenceTitle(player.presence)"
        role="img"
        :aria-label="presenceTitle(player.presence)"
      >
        <PresenceBadge :presence="player.presence" />
      </span>
      <MaterialDiff v-if="materialFen" :fen="materialFen" :color="color" />
      <span class="sr-only">plays {{ color }}</span>
      <span v-if="player.status" class="player-status" role="status">
        <UIcon v-if="player.busy" name="i-lucide-loader-circle" class="animate-spin" />
        {{ player.status }}
      </span>
    </div>
    <div v-if="$slots.middle" class="player-middle"><slot name="middle" /></div>
    <span v-if="player.clock" class="player-clock" :aria-label="`${color} clock`">{{
      player.clock
    }}</span>
    <slot name="aside" />
  </div>
</template>
