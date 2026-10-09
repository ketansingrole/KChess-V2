<script setup lang="ts">
import { computed } from 'vue'
import type { DropdownMenuItem } from '@nuxt/ui'
import type { ChallengeInfo, DeclineReason } from '@kchess/core/contracts/types'
import { describeControl, useChallengeStore } from '../stores/challenges'

const challenges = useChallengeStore()
const store = useKChessStore()

const DECLINE: { reason: DeclineReason; label: string }[] = [
  { reason: 'generic', label: 'Decline' },
  { reason: 'later', label: 'Not now, maybe later' },
  { reason: 'tooFast', label: 'Too fast for me' },
  { reason: 'tooSlow', label: 'Too slow for me' },
  { reason: 'timeControl', label: 'Not this time control' },
  { reason: 'rated', label: 'Send a rated challenge instead' },
  { reason: 'casual', label: 'Send a casual challenge instead' },
  { reason: 'standard', label: 'Standard chess only, please' },
  { reason: 'variant', label: 'Not this variant' },
]
function declineMenu(challenge: ChallengeInfo): DropdownMenuItem[][] {
  return [
    DECLINE.map((entry) => ({
      label: entry.label,
      onSelect: () => void challenges.decline(challenge, entry.reason),
    })),
  ]
}
const status = computed(() => {
  const lobby = challenges.lobby
  if (!store.settings.receiveChallenges)
    return { tone: 'muted', text: 'Not listening for challenges (Settings → Online play).' }
  if (!lobby) return { tone: 'muted', text: 'Listening for challenges while a game is open.' }
  if (lobby.phase === 'connected')
    return { tone: 'ok', text: `Listening for challenges to @${lobby.account}.` }
  if (lobby.phase === 'auth-required')
    return { tone: 'error', text: lobby.message ?? 'Sign in to Lichess again in Settings.' }
  if (lobby.phase === 'disconnected')
    return { tone: 'error', text: lobby.message ?? 'Not connected to Lichess.' }
  return { tone: 'muted', text: 'Connecting to Lichess…' }
})
function colorText(challenge: ChallengeInfo): string {
  if (challenge.color === 'random') return 'random colours'
  // The colour is the challenger's; for an incoming one you get the other.
  const yours =
    challenge.direction === 'out'
      ? challenge.color
      : challenge.color === 'white'
        ? 'black'
        : 'white'
  return `you play ${yours}`
}
</script>

<template>
  <section class="card" aria-labelledby="challenges-title">
    <div class="card-header">
      <div>
        <h2 id="challenges-title" class="section-title">Challenges</h2>
        <p
          class="section-hint"
          :class="{ 'text-error': status.tone === 'error', 'text-success': status.tone === 'ok' }"
          role="status"
        >
          {{ status.text }}
        </p>
      </div>
    </div>
    <p v-if="!challenges.list.length" class="muted text-sm">No open challenges.</p>
    <div class="list-rows">
      <div v-for="challenge in challenges.incoming" :key="challenge.id" class="list-row">
        <UIcon
          :name="challenge.rematchOf ? 'i-lucide-repeat' : 'i-lucide-swords'"
          class="shrink-0"
        />
        <div class="row-main">
          <div class="row-title">
            {{ challenge.opponent.title ? `${challenge.opponent.title} ` : ''
            }}<PlayerLink :username="challenge.opponent.name" />
            <span v-if="challenge.opponent.rating" class="muted tabular"
              >({{ challenge.opponent.rating
              }}{{ challenge.opponent.provisional ? '?' : '' }})</span
            >
          </div>
          <div class="row-sub">
            {{ challenge.rematchOf ? 'Rematch · ' : '' }}{{ challenge.variantName }} ·
            {{ describeControl(challenge) }} · {{ challenge.rated ? 'Rated' : 'Casual' }} ·
            {{ colorText(challenge) }}
            <span v-if="store.connectedAccounts.length > 1"> · to @{{ challenge.account }}</span>
          </div>
          <div v-if="challenge.problem" class="row-sub text-warning">{{ challenge.problem }}</div>
        </div>
        <div class="row-actions">
          <UButton
            size="xs"
            icon="i-lucide-check"
            :disabled="!challenge.playable"
            :loading="challenges.busyId === challenge.id"
            @click="challenges.accept(challenge)"
            >Accept</UButton
          >
          <UDropdownMenu :items="declineMenu(challenge)">
            <UButton
              size="xs"
              variant="outline"
              color="neutral"
              icon="i-lucide-x"
              trailing-icon="i-lucide-chevron-down"
              :disabled="challenges.busyId === challenge.id"
              >Decline</UButton
            >
          </UDropdownMenu>
        </div>
      </div>
      <div v-for="challenge in challenges.outgoing" :key="challenge.id" class="list-row">
        <UIcon name="i-lucide-send" class="shrink-0" />
        <div class="row-main">
          <div class="row-title">To <PlayerLink :username="challenge.opponent.name" /></div>
          <div class="row-sub">
            {{ challenge.variantName }} · {{ describeControl(challenge) }} ·
            {{ challenge.rated ? 'Rated' : 'Casual' }} · {{ colorText(challenge) }} · waiting
          </div>
        </div>
        <div class="row-actions">
          <UButton
            size="xs"
            variant="outline"
            color="neutral"
            icon="i-lucide-x"
            :loading="challenges.busyId === challenge.id"
            @click="challenges.withdraw(challenge)"
            >Withdraw</UButton
          >
        </div>
      </div>
    </div>
  </section>
</template>
