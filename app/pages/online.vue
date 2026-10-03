<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import { formatBytes } from '../utils/format'
import {
  canDirectChallenge,
  canPlayOnline,
  perfFor,
  totalSeconds,
} from '../../src/shared/timeControl'

const store = useKChessStore()
const {
  data,
  connectedAccounts,
  activeOnlineAccount,
  onlinePhase,
  onlineMinutes,
  onlineIncrement,
  onlineChoice,
  onlineRated,
  onlineGameRated,
  onlineTarget,
  busy,
  onlineStatus,
  onlineConnection,
  onlineOpponent,
  onlineColor,
  onlineHistory,
  onlineLast,
  onlineDests,
  onlineCheck,
  onlineInteractive,
  onlineCanPlay,
  onlinePly,
  onlineDisplay,
  onlineDisplayTurn,
  onlineOrientation,
  onlineFlipped,
  onlinePresence,
  pingStats,
  moveAckMs,
  settings,
  onlineId,
} = storeToRefs(store)
const usage = useUsageStore()
/** Bytes this account has downloaded for playing and status checks. */
const playDownloaded = computed(() => {
  const byKind = usage.usageOf(activeOnlineAccount.value).byKind
  return (byKind.play?.bytesIn ?? 0) + (byKind.presence?.bytesIn ?? 0)
})
// Keep the data figure moving while a game is on; it is a cheap local read.
useIntervalFn(() => {
  if (onlinePhase.value === 'playing') void usage.refresh()
}, 10_000)
onMounted(() => void usage.refresh())
// Not reactive, so read from the store itself rather than through storeToRefs.
const { presenceIntervalSeconds } = store
const {
  viewOnlinePly,
  selectPage,
  startOnline,
  stopOnline,
  clockText,
  onlineAction,
  onlineMove,
  fen,
  setOnlineAccount,
} = store

/** Which connected account plays; only shown when there is a choice. */
const playingAccount = computed({
  get: () => activeOnlineAccount.value,
  set: (username: string) => setOnlineAccount(username),
})
const accountItems = computed(() =>
  connectedAccounts.value.map((account) => ({
    label: `@${account.username}`,
    value: account.username,
  })),
)

const timeControls = [
  { minutes: 3, label: 'Blitz' },
  { minutes: 5, label: 'Blitz' },
  { minutes: 10, label: 'Rapid' },
  { minutes: 15, label: 'Rapid' },
  { minutes: 30, label: 'Classical' },
  { minutes: 60, label: 'Classical' },
]
const increments = [0, 1, 2, 3, 5, 10, 20].map((value) => ({ label: `${value}s`, value }))
const colorChoices = [
  { label: 'Random', value: 'random', icon: 'i-lucide-shuffle' },
  { label: 'White', value: 'white', icon: 'i-lucide-circle' },
  { label: 'Black', value: 'black', icon: 'i-lucide-circle-dot' },
]

const ratedChoices = [
  { label: 'Casual', value: 'casual', icon: 'i-lucide-coffee' },
  { label: 'Rated', value: 'rated', icon: 'i-lucide-trophy' },
]
const ratedMode = computed({
  get: () => (onlineRated.value ? 'rated' : 'casual'),
  set: (value: string) => {
    onlineRated.value = value === 'rated'
  },
})
const modeLabel = computed(() => (onlineRated.value ? 'Rated' : 'Casual'))

const targeted = computed(() => Boolean(onlineTarget.value.trim()))
const perf = computed(() => perfFor(onlineMinutes.value, onlineIncrement.value))
const estimatedMinutes = computed(
  () => totalSeconds(onlineMinutes.value, onlineIncrement.value) / 60,
)
const allowed = computed(() =>
  canPlayOnline(onlineMinutes.value, onlineIncrement.value, targeted.value),
)
const blitzNeedsTarget = computed(
  () =>
    !targeted.value &&
    !canPlayOnline(onlineMinutes.value, onlineIncrement.value, false) &&
    canDirectChallenge(onlineMinutes.value, onlineIncrement.value),
)

const playing = computed(() => onlinePhase.value === 'playing')
const canTakeback = computed(() => playing.value && onlineHistory.value.length >= 2)
const confirmResign = ref(false)
const opponentColor = computed(() => (onlineColor.value === 'white' ? 'black' : 'white'))
/** Players follow the board orientation, so flipping never mislabels a side. */
const players = computed(() => {
  const opponent = {
    name: onlineOpponent.value,
    icon: 'i-lucide-user',
    clock: clockText(opponentColor.value),
    presence: onlinePresence.value.opponent,
  }
  const you = {
    name: 'You',
    icon: 'i-lucide-user',
    clock: clockText(onlineColor.value),
    presence: onlinePresence.value.self,
  }
  return onlineOrientation.value === onlineColor.value
    ? { top: opponent, bottom: you }
    : { top: you, bottom: opponent }
})
const topPlayer = computed(() => players.value.top)
const bottomPlayer = computed(() => players.value.bottom)
const reviewing = computed(() => playing.value && !onlineCanPlay.value)
const banner = computed(() =>
  reviewing.value
    ? {
        icon: 'i-lucide-history',
        title: 'Reviewing an earlier position',
        detail: 'Go to the last move to keep playing.',
      }
    : playing.value
      ? {
          icon: onlineInteractive.value ? 'i-lucide-mouse-pointer-click' : 'i-lucide-hourglass',
          title: onlineInteractive.value ? 'Your move' : 'Waiting for opponent',
          detail: onlineStatus.value,
        }
      : onlinePhase.value === 'disconnected'
        ? {
            icon: 'i-lucide-triangle-alert',
            title: 'Connection interrupted',
            detail: onlineStatus.value,
          }
        : { icon: 'i-lucide-flag', title: 'Game over', detail: onlineStatus.value },
)
</script>

<template>
  <div>
    <PageHeader
      title="Play Online"
      subtitle="Play live games through your connected Lichess accounts"
    />

    <div v-if="!data.accounts.some((a) => a.connected)" class="card">
      <UEmpty
        variant="naked"
        icon="i-lucide-globe"
        title="Connect Lichess to play"
        description="Online play uses Lichess OAuth and the Board API. Connect your account in Settings to get started."
        :actions="[
          {
            label: 'Open Settings',
            icon: 'i-lucide-settings-2',
            onClick: () => selectPage('settings'),
          },
        ]"
      />
    </div>

    <form
      v-else-if="onlinePhase === 'idle'"
      class="card online-form form-stack"
      @submit.prevent="startOnline"
    >
      <div>
        <h2 class="section-title">Find a game</h2>
        <p class="section-hint">
          Lichess limits third-party clients: public seeks allow Rapid and slower, direct challenges
          allow Blitz and slower, and Bullet is always blocked.
        </p>
      </div>

      <div v-if="connectedAccounts.length > 1" class="field">
        <span id="account-label" class="field-label">Account</span>
        <USelect
          v-model="playingAccount"
          :items="accountItems"
          icon="i-lucide-user"
          aria-labelledby="account-label"
          :ui="{ base: 'w-full' }"
        />
      </div>

      <div class="field">
        <span id="tc-label" class="field-label">Time control</span>
        <div class="tc-grid" role="radiogroup" aria-labelledby="tc-label">
          <button
            v-for="tc in timeControls"
            :key="tc.minutes"
            type="button"
            role="radio"
            class="tc-option"
            :aria-checked="onlineMinutes === tc.minutes"
            @click="onlineMinutes = tc.minutes"
          >
            <span class="tc-main">{{ tc.minutes }} min</span>
            <span class="tc-sub">{{ tc.label }}</span>
          </button>
        </div>
      </div>

      <div class="field">
        <span class="field-label">Increment per move</span>
        <UTabs
          v-model="onlineIncrement"
          :items="increments"
          aria-label="Increment per move"
          size="sm"
          :content="false"
          variant="pill"
          class="w-full"
          :ui="{ trigger: 'grow' }"
        />
      </div>

      <div class="field">
        <span class="field-label">Play as</span>
        <UTabs
          v-model="onlineChoice"
          :items="colorChoices"
          aria-label="Play as"
          :content="false"
          variant="pill"
          class="w-full"
          :ui="{ trigger: 'grow' }"
        />
      </div>

      <div class="field">
        <span class="field-label">
          Game type
          <span class="field-hint"
            >— {{ onlineRated ? 'changes your Lichess rating' : 'no rating change' }}</span
          >
        </span>
        <UTabs
          v-model="ratedMode"
          :items="ratedChoices"
          aria-label="Game type"
          :content="false"
          variant="pill"
          class="w-full"
          :ui="{ trigger: 'grow' }"
        />
      </div>

      <label class="field">
        <span class="field-label">
          Challenge a specific player
          <span class="field-hint">— optional, unlocks Blitz</span>
        </span>
        <UInput
          v-model="onlineTarget"
          icon="i-lucide-user-search"
          placeholder="Lichess username"
          autocomplete="off"
          :ui="{ root: 'w-full' }"
        />
      </label>

      <UAlert
        v-if="!allowed"
        color="warning"
        variant="subtle"
        icon="i-lucide-triangle-alert"
        :title="`Lichess blocks ${perf} (${onlineMinutes}+${onlineIncrement}) here`"
        :description="
          blitzNeedsTarget
            ? 'Add a username above to send it as a direct challenge, or pick a longer control.'
            : 'Pick Blitz or slower for a direct challenge, Rapid or slower for the public pool.'
        "
      />

      <div class="flex items-center justify-between gap-3 panel-divider pt-4">
        <div class="text-sm">
          <div class="font-semibold tabular">
            {{ onlineMinutes }}+{{ onlineIncrement }} · {{ perf }} · {{ modeLabel }}
          </div>
          <div class="muted text-xs tabular">~{{ estimatedMinutes.toFixed(1) }} min estimated</div>
          <div class="muted text-xs tabular">
            Data used so far by @{{ activeOnlineAccount }}: {{ formatBytes(playDownloaded) }}
          </div>
        </div>
        <UButton
          type="submit"
          size="lg"
          :icon="targeted ? 'i-lucide-send' : 'i-lucide-search'"
          :loading="busy"
          :disabled="!allowed"
          >{{ targeted ? 'Send challenge' : 'Find opponent' }}</UButton
        >
      </div>
    </form>

    <div v-else-if="onlinePhase === 'seeking'" class="seeking-stage">
      <div class="card seeking" role="status">
        <UIcon name="i-lucide-loader-circle" class="animate-spin spinner" />
        <h2 class="section-title">{{ onlineStatus || 'Looking for an opponent…' }}</h2>
        <p class="section-hint">
          {{ onlineMinutes }}+{{ onlineIncrement }} · {{ perf }} · {{ modeLabel }}. The game starts
          automatically when someone accepts.
        </p>
        <UButton variant="outline" color="neutral" icon="i-lucide-x" @click="stopOnline"
          >Cancel</UButton
        >
      </div>
    </div>

    <div v-else class="play-layout">
      <div v-if="onlinePhase === 'disconnected'" class="card col-span-full" role="status">
        <h2 class="section-title">
          {{
            onlineConnection?.phase === 'auth-required' ? 'Sign in again' : 'Connection interrupted'
          }}
        </h2>
        <p class="section-hint">
          {{ onlineStatus }} Clocks are paused until a fresh server update arrives.
        </p>
        <div class="mt-3 flex gap-2">
          <UButton :loading="busy" @click="store.reconnectOnline">Reconnect</UButton>
          <UButton variant="outline" color="neutral" @click="store.selectPage('settings')"
            >Account settings</UButton
          >
          <UButton v-if="!onlineId" variant="ghost" color="neutral" @click="stopOnline"
            >Find another game</UButton
          >
        </div>
      </div>
      <PlayBoard
        :fen="fen(onlineDisplay)"
        :orientation="onlineOrientation"
        :theme="settings.boardTheme"
        :coordinates="settings.coordinates"
        :piece-set="settings.pieceSet"
        :animation="settings.pieceAnimation"
        :movable="onlineInteractive"
        :interactive="onlineCanPlay"
        :movable-color="onlineColor"
        :premove="settings.premove"
        :show-dests="settings.showLegalMoves"
        :promotion="settings.promotion"
        :dests="onlineInteractive ? onlineDests : undefined"
        :last-move="onlineLast"
        :check="onlineCheck"
        :turn-color="onlineDisplayTurn"
        :live="playing"
        :top="topPlayer"
        :bottom="bottomPlayer"
        @move="onlineMove"
      />
      <MovePanel
        title="Live game"
        :moves="onlineHistory"
        :ply="onlinePly"
        empty-text="No moves yet."
        @select="viewOnlinePly"
        @flip="onlineFlipped = !onlineFlipped"
      >
        <template #top>
          <div class="status-banner" :role="playing ? 'status' : 'alert'">
            <UIcon :name="banner.icon" class="status-icon" />
            <div>
              <strong>{{ banner.title }}</strong>
              <span class="detail">{{ banner.detail }}</span>
            </div>
          </div>
          <ConnectionPanel
            v-if="playing"
            :ping="pingStats"
            :move-ack-ms="moveAckMs"
            :opponent="onlinePresence.opponent"
            :downloaded="playDownloaded"
            :interval-seconds="presenceIntervalSeconds"
          />
        </template>
        <template #bottom>
          <div class="panel-actions panel-divider pt-3.5">
            <template v-if="playing">
              <UButton
                v-if="canTakeback"
                variant="outline"
                color="neutral"
                icon="i-lucide-undo-2"
                @click="onlineAction('takeback')"
                >Takeback</UButton
              >
              <UButton
                v-if="canTakeback"
                variant="outline"
                color="error"
                icon="i-lucide-flag"
                @click="confirmResign = true"
                >Resign</UButton
              >
              <UButton
                v-else
                variant="outline"
                color="error"
                icon="i-lucide-ban"
                @click="onlineAction('abort')"
                >Abort</UButton
              >
            </template>
            <UButton v-else icon="i-lucide-plus" @click="stopOnline">New game</UButton>
          </div>
          <p class="muted text-xs tabular">
            {{ onlineGameRated ? 'Rated' : 'Casual' }} · Game {{ onlineId }}
          </p>
        </template>
      </MovePanel>
    </div>

    <ConfirmDialog
      v-model:open="confirmResign"
      title="Resign this game?"
      description="Your opponent will be awarded the win."
      confirm-label="Resign"
      color="error"
      @confirm="onlineAction('resign')"
    />
  </div>
</template>
