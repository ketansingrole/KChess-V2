<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useIntervalFn, useOnline } from '@vueuse/core'
import { useZenSurface } from '../composables/useZenSurface'
import { formatBytes } from '../utils/format'
import { canDirectChallenge, canPlayOnline, perfFor, totalSeconds } from '@kchess/rules/timeControl'
import { CORRESPONDENCE_DAYS } from '@kchess/contracts/types'
import { VARIANT_HINTS, VARIANT_LABELS, VARIANTS } from '@kchess/rules/variant'
import { setupPgn } from '@kchess/rules/chess'
import { useChallengeStore } from '../stores/challenges'
import { useAnalysisStore } from '../stores/analysis'
import { useTournamentStore } from '../stores/tournaments'

const online = useOnline()
const store = useKChessStore()
const challenges = useChallengeStore()
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
  onlineVariant,
  onlineMode,
  onlineDays,
  busy,
  onlineStatus,
  onlineConnection,
  onlineOpponent,
  onlineOpponentRating,
  onlineRating,
  onlineColor,
  onlineHistory,
  onlineMoves,
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
  onlineSetup,
  onlineUnsupported,
  onlineCorrespondence,
  onlineTournament,
  onlineClockConfig,
  drawOffer,
  takebackOffer,
  claimIn,
  firstMoveIn,
  canBerserk,
  lastGame,
  pingStats,
  moveAckMs,
  settings,
  onlineId,
  zenActive,
} = storeToRefs(store)
// The game view (not the lobby or a seek) has the board and its zen toggle.
useZenSurface(
  () =>
    data.value.accounts.some((a) => a.connected) &&
    onlinePhase.value !== 'idle' &&
    onlinePhase.value !== 'seeking',
)
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
onMounted(() => {
  void usage.refresh()
  void challenges.refreshOngoing()
})
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
  rematch,
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
const dayChoices = CORRESPONDENCE_DAYS.map((value) => ({
  label: `${value} ${value === 1 ? 'day' : 'days'}`,
  value,
}))
const modeChoices = [
  { label: 'Real time', value: 'realtime', icon: 'i-lucide-timer' },
  { label: 'Correspondence', value: 'correspondence', icon: 'i-lucide-mail' },
]
const variantItems = VARIANTS.map((value) => ({ label: VARIANT_LABELS[value], value }))
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
const correspondence = computed(() => onlineMode.value === 'correspondence')

const targeted = computed(() => Boolean(onlineTarget.value.trim()))
const perf = computed(() => perfFor(onlineMinutes.value, onlineIncrement.value))
const estimatedMinutes = computed(
  () => totalSeconds(onlineMinutes.value, onlineIncrement.value) / 60,
)
const allowed = computed(
  () =>
    correspondence.value ||
    canPlayOnline(onlineMinutes.value, onlineIncrement.value, targeted.value),
)
const blitzNeedsTarget = computed(
  () =>
    !targeted.value &&
    !canPlayOnline(onlineMinutes.value, onlineIncrement.value, false) &&
    canDirectChallenge(onlineMinutes.value, onlineIncrement.value),
)
const summary = computed(() => {
  const variant =
    onlineVariant.value === 'standard' ? '' : `${VARIANT_LABELS[onlineVariant.value]} · `
  return correspondence.value
    ? `${variant}${onlineDays.value} ${onlineDays.value === 1 ? 'day' : 'days'} per move · ${modeLabel.value}`
    : `${variant}${onlineMinutes.value}+${onlineIncrement.value} · ${perf.value} · ${modeLabel.value}`
})

const playing = computed(() => onlinePhase.value === 'playing')
const canTakeback = computed(() => playing.value && onlineHistory.value.length >= 2)
const confirmResign = ref(false)
const opponentColor = computed(() => (onlineColor.value === 'white' ? 'black' : 'white'))
const hasClock = computed(() => Boolean(onlineClockConfig.value) || onlineCorrespondence.value)
/** The board puts your side at the bottom unless it is flipped. */
const youAtBottom = computed(() => onlineOrientation.value === onlineColor.value)
/** Players follow the board orientation, so flipping never mislabels a side. */
const players = computed(() => {
  const opponent = {
    name: onlineOpponentRating.value
      ? `${onlineOpponent.value} (${onlineOpponentRating.value})`
      : onlineOpponent.value,
    username: store.opponentId ? onlineOpponent.value : undefined,
    // A live game shows the profile in place, so a click never leaves the board.
    card: playing.value,
    icon: 'i-lucide-user',
    clock: hasClock.value ? clockText(opponentColor.value) : undefined,
    presence: onlinePresence.value.opponent,
  }
  const you = {
    name: onlineRating.value ? `You (${onlineRating.value})` : 'You',
    icon: 'i-lucide-user',
    clock: hasClock.value ? clockText(onlineColor.value) : undefined,
    presence: onlinePresence.value.self,
  }
  return youAtBottom.value ? { top: opponent, bottom: you } : { top: you, bottom: opponent }
})
const topPlayer = computed(() => players.value.top)
const bottomPlayer = computed(() => players.value.bottom)
const reviewing = computed(() => playing.value && !onlineCanPlay.value && !onlineUnsupported.value)
const banner = computed(() =>
  onlineUnsupported.value
    ? {
        icon: 'i-lucide-triangle-alert',
        title: `${onlineUnsupported.value} is not supported yet`,
        detail: 'KChess cannot show this variant’s board. You can still resign or abort it here.',
      }
    : reviewing.value
      ? {
          icon: 'i-lucide-history',
          title: 'Reviewing an earlier position',
          detail: 'Go to the last move to keep playing.',
        }
      : playing.value
        ? // Whose move it is already shows on the clocks; announced for screen readers below.
          null
        : onlinePhase.value === 'disconnected'
          ? {
              icon: 'i-lucide-triangle-alert',
              title: 'Connection interrupted',
              detail: onlineStatus.value,
            }
          : // The result already says it all: "You won by resignation".
            { icon: 'i-lucide-flag', title: onlineStatus.value || 'Game over', detail: '' },
)
const variantName = computed(() =>
  onlineSetup.value.variant === 'standard' ? '' : VARIANT_LABELS[onlineSetup.value.variant],
)
/** Finished standard games can go to the analysis board (it plays standard rules). */
const canAnalyse = computed(
  () => onlinePhase.value === 'finished' && onlineSetup.value.variant === 'standard',
)
const analysis = useAnalysisStore()
function analyse(): void {
  const white = onlineColor.value === 'white' ? 'You' : onlineOpponent.value
  const black = onlineColor.value === 'black' ? 'You' : onlineOpponent.value
  if (!analysis.loadPgn(setupPgn(onlineSetup.value, onlineMoves.value), onlineMoves.value.length))
    return
  analysis.orientation = onlineColor.value
  analysis.origin = {
    gameId: onlineId.value,
    account: store.onlineAccount,
    white,
    black,
    player: onlineColor.value,
  }
  analysis.startReview()
  selectPage('analysis')
}
const rematchOffer = computed(() => challenges.rematchOffer)
const tournaments = useTournamentStore()
function backToTournament(): void {
  void tournaments.open(tournaments.systemOf(onlineTournament.value), onlineTournament.value)
  selectPage('tournaments')
}
</script>

<template>
  <div>
    <PageHeader title="Play on Lichess" />
    <p v-if="!online" class="section-hint mb-4" role="status">
      Offline · reconnect to play on Lichess.
    </p>

    <div v-if="!data.accounts.some((a) => a.connected)" class="card">
      <UEmpty
        variant="naked"
        icon="i-lucide-globe"
        title="Connect Lichess to play"
        :actions="[
          {
            label: 'Connect Lichess',
            disabled: !online || store.busy,
            icon: 'i-lucide-log-in',
            onClick: () => store.connect(),
          },
          {
            label: 'Play the computer',
            variant: 'outline',
            color: 'neutral',
            onClick: () => selectPage('computer'),
          },
        ]"
      />
    </div>

    <div v-else-if="onlinePhase === 'idle'" class="online-columns">
      <form class="card online-form form-stack" @submit.prevent="startOnline()">
        <div>
          <h2 class="section-title">Find a game</h2>
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
          <span class="field-label">Kind of game</span>
          <UTabs
            v-model="onlineMode"
            :items="modeChoices"
            aria-label="Kind of game"
            :content="false"
            variant="pill"
            class="w-full"
            :ui="{ trigger: 'grow' }"
          />
        </div>

        <div class="field">
          <span id="variant-label" class="field-label">
            Variant <span class="field-hint">— {{ VARIANT_HINTS[onlineVariant] }}</span>
          </span>
          <USelect
            v-model="onlineVariant"
            :items="variantItems"
            icon="i-lucide-shapes"
            aria-labelledby="variant-label"
            :ui="{ base: 'w-full' }"
          />
        </div>

        <template v-if="!correspondence">
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
              :content="false"
              variant="pill"
              class="w-full"
              :ui="{ trigger: 'grow' }"
            />
          </div>
        </template>
        <div v-else class="field">
          <span class="field-label">Days per move</span>
          <UTabs
            v-model="onlineDays"
            :items="dayChoices"
            aria-label="Days per move"
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
          <span class="field-label"> Game type </span>
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
        <UAlert
          v-else-if="correspondence && !targeted"
          color="neutral"
          variant="subtle"
          icon="i-lucide-info"
          title="A public correspondence seek stays open on Lichess"
          description="Lichess has no way for apps to withdraw it; it closes when someone joins. Add a username to send a challenge you can withdraw instead."
        />

        <div class="flex items-center justify-between gap-3 panel-divider pt-4">
          <div class="text-sm">
            <div class="font-semibold tabular">{{ summary }}</div>
            <div v-if="!correspondence" class="muted text-xs tabular">
              ~{{ estimatedMinutes.toFixed(1) }} min estimated
            </div>
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
            >{{
              targeted ? 'Send challenge' : correspondence ? 'Create seek' : 'Find opponent'
            }}</UButton
          >
        </div>
      </form>
      <div class="flex flex-col gap-5">
        <ChallengeList />
        <OngoingGames />
        <UButton
          variant="outline"
          color="neutral"
          icon="i-lucide-trophy"
          block
          @click="selectPage('tournaments')"
          >Tournaments</UButton
        >
      </div>
    </div>

    <div v-else-if="onlinePhase === 'seeking'" class="seeking-stage">
      <div class="card seeking" role="status">
        <UIcon name="i-lucide-loader-circle" class="animate-spin spinner" />
        <h2 class="section-title">{{ onlineStatus || 'Looking for an opponent…' }}</h2>
        <p class="section-hint">
          {{ summary }}
        </p>
        <UButton variant="outline" color="neutral" icon="i-lucide-x" @click="stopOnline"
          >Cancel</UButton
        >
      </div>
    </div>

    <div v-else v-board-fit class="play-layout">
      <div v-if="onlinePhase === 'disconnected'" class="card col-span-full" role="status">
        <h2 class="section-title">
          {{
            onlineConnection?.phase === 'checking'
              ? 'Checking game status'
              : onlineConnection?.phase === 'auth-required'
                ? 'Sign in again'
                : 'Connection interrupted'
          }}
        </h2>
        <p class="section-hint">
          {{ onlineStatus }} Clocks are paused until a fresh server update arrives.
        </p>
        <div class="mt-3 flex gap-2">
          <UButton :loading="busy" @click="store.reconnectOnline">Reconnect</UButton>
          <UButton variant="outline" color="neutral" @click="store.openSettings('accounts')"
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
        :premove="settings.premove && !onlineCorrespondence"
        :show-dests="settings.showLegalMoves"
        :promotion="settings.promotion"
        :dests="onlineInteractive ? onlineDests : undefined"
        :last-move="onlineLast"
        :check="onlineCheck"
        :turn-color="onlineDisplayTurn"
        :live="playing"
        :top="topPlayer"
        :bottom="bottomPlayer"
        :blindfold="settings.blindfold && playing"
        :variant="onlineSetup.variant"
        @move="onlineMove"
      >
        <template v-if="playing && !onlineCorrespondence" #top-presence>
          <ConnectionPanel
            v-if="!youAtBottom"
            side="you"
            :ping="pingStats"
            :move-ack-ms="moveAckMs"
            :downloaded="playDownloaded"
            :interval-seconds="presenceIntervalSeconds"
          />
          <ConnectionPanel
            v-else
            side="opponent"
            :name="onlineOpponent"
            :presence="onlinePresence.opponent"
          />
        </template>
        <template v-if="playing && !onlineCorrespondence" #bottom-presence>
          <ConnectionPanel
            v-if="youAtBottom"
            side="you"
            :ping="pingStats"
            :move-ack-ms="moveAckMs"
            :downloaded="playDownloaded"
            :interval-seconds="presenceIntervalSeconds"
          />
          <ConnectionPanel
            v-else
            side="opponent"
            :name="onlineOpponent"
            :presence="onlinePresence.opponent"
          />
        </template>
      </PlayBoard>
      <MovePanel
        title="Live game"
        :moves="onlineHistory"
        :ply="onlinePly"
        empty-text="No moves yet."
        @select="viewOnlinePly"
        @flip="onlineFlipped = !onlineFlipped"
      >
        <template #top>
          <div class="flex flex-wrap items-center gap-2 text-xs muted">
            <span class="font-semibold"
              >{{ variantName ? `${variantName} · ` : ''
              }}{{ onlineGameRated ? 'Rated' : 'Casual' }}</span
            >
            <span class="flex-1" />
            <BoardViewToggles />
          </div>
          <p v-if="playing" class="sr-only" role="status">
            {{ onlineInteractive ? 'Your move' : 'Waiting for opponent' }}
          </p>
          <div v-if="banner" class="status-banner" :role="playing ? 'status' : 'alert'">
            <UIcon :name="banner.icon" class="status-icon" />
            <div>
              <strong>{{ banner.title }}</strong>
              <span v-if="banner.detail" class="detail">{{ banner.detail }}</span>
            </div>
          </div>
          <!-- Named only after the game: during it, the name is help from outside the board. -->
          <OpeningName
            v-if="!playing && onlineSetup.variant === 'standard'"
            :setup="onlineSetup"
            :moves="onlineMoves"
            :ply="onlinePly"
          />
          <div v-if="playing && firstMoveIn !== null" class="offer-banner" role="status">
            <UIcon name="i-lucide-timer" />
            <span class="offer-text">Move within {{ firstMoveIn }}s or the game is aborted.</span>
          </div>
          <div v-if="playing && drawOffer === 'theirs'" class="offer-banner" role="alert">
            <UIcon name="i-lucide-handshake" />
            <span class="offer-text">{{ onlineOpponent }} offers a draw.</span>
            <UButton size="xs" @click="onlineAction('acceptDraw')">Accept</UButton>
            <UButton
              size="xs"
              variant="outline"
              color="neutral"
              @click="onlineAction('declineDraw')"
              >Decline</UButton
            >
          </div>
          <div v-else-if="playing && drawOffer === 'mine'" class="offer-banner" role="status">
            <UIcon name="i-lucide-handshake" />
            <span class="offer-text">You offered a draw.</span>
            <UButton size="xs" variant="ghost" color="neutral" @click="onlineAction('declineDraw')"
              >Cancel offer</UButton
            >
          </div>
          <div v-if="playing && takebackOffer === 'theirs'" class="offer-banner" role="alert">
            <UIcon name="i-lucide-undo-2" />
            <span class="offer-text">{{ onlineOpponent }} asks to take back a move.</span>
            <UButton size="xs" @click="onlineAction('takeback')">Accept</UButton>
            <UButton
              size="xs"
              variant="outline"
              color="neutral"
              @click="onlineAction('declineTakeback')"
              >Decline</UButton
            >
          </div>
          <div v-else-if="playing && takebackOffer === 'mine'" class="offer-banner" role="status">
            <UIcon name="i-lucide-undo-2" />
            <span class="offer-text">Takeback requested.</span>
            <UButton
              size="xs"
              variant="ghost"
              color="neutral"
              @click="onlineAction('declineTakeback')"
              >Cancel</UButton
            >
          </div>
          <div v-if="playing && claimIn !== null" class="offer-banner" role="alert">
            <UIcon name="i-lucide-wifi-off" />
            <span class="offer-text">
              {{
                claimIn > 0
                  ? `${onlineOpponent} left the game. You can claim the win in ${claimIn}s.`
                  : `${onlineOpponent} left the game.`
              }}
            </span>
            <template v-if="claimIn === 0">
              <UButton size="xs" icon="i-lucide-trophy" @click="onlineAction('claimVictory')"
                >Claim victory</UButton
              >
              <UButton
                size="xs"
                variant="outline"
                color="neutral"
                @click="onlineAction('claimDraw')"
                >Call it a draw</UButton
              >
            </template>
          </div>
          <div v-if="!playing && rematchOffer" class="offer-banner" role="alert">
            <UIcon name="i-lucide-repeat" />
            <span class="offer-text"
              ><PlayerLink :username="rematchOffer.opponent.name" /> wants a rematch.</span
            >
            <UButton size="xs" @click="challenges.accept(rematchOffer)">Accept</UButton>
            <UButton
              size="xs"
              variant="outline"
              color="neutral"
              @click="challenges.decline(rematchOffer)"
              >Decline</UButton
            >
          </div>
        </template>
        <template #bottom>
          <div class="panel-actions panel-divider pt-3.5">
            <template v-if="playing">
              <UButton
                v-if="canBerserk"
                variant="soft"
                color="warning"
                icon="i-lucide-flame"
                @click="onlineAction('berserk')"
                >Berserk</UButton
              >
              <UButton
                v-if="canTakeback && takebackOffer === 'none'"
                variant="outline"
                color="neutral"
                icon="i-lucide-undo-2"
                @click="onlineAction('takeback')"
                >Takeback</UButton
              >
              <UButton
                v-if="canTakeback && drawOffer === 'none'"
                variant="outline"
                color="neutral"
                icon="i-lucide-handshake"
                @click="onlineAction('offerDraw')"
                >Offer draw</UButton
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
            <template v-else>
              <UButton
                v-if="lastGame && onlinePhase === 'finished' && !onlineTournament"
                icon="i-lucide-repeat"
                :loading="busy"
                @click="rematch"
                >Rematch</UButton
              >
              <UButton
                v-if="canAnalyse"
                variant="outline"
                color="neutral"
                icon="i-lucide-microscope"
                @click="analyse"
                >Analyse</UButton
              >
              <ExportGame
                v-if="onlinePhase === 'finished'"
                :setup="onlineSetup"
                :moves="onlineMoves"
                :orientation="onlineOrientation"
                :white="onlineColor === 'white' ? activeOnlineAccount : onlineOpponent"
                :black="onlineColor === 'black' ? activeOnlineAccount : onlineOpponent"
                :ply="onlinePly"
              />
              <UButton
                v-if="onlinePhase === 'finished' && onlineTournament"
                variant="outline"
                color="neutral"
                icon="i-lucide-trophy"
                @click="backToTournament"
                >Tournament</UButton
              >
              <UButton
                :variant="lastGame ? 'outline' : 'solid'"
                :color="lastGame ? 'neutral' : 'primary'"
                icon="i-lucide-plus"
                @click="stopOnline"
                >New game</UButton
              >
            </template>
            <UButton
              v-if="playing && onlineCorrespondence"
              variant="ghost"
              color="neutral"
              icon="i-lucide-x"
              @click="stopOnline"
              >Close game</UButton
            >
          </div>
        </template>
      </MovePanel>
      <OnlineChat v-if="settings.onlineChat && !zenActive" />
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
