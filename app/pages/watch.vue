<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch as watchValue } from 'vue'
import { useIntervalFn, useLocalStorage, useOnline } from '@vueuse/core'
import type { Key } from '@lichess-org/chessground/types'
import type { BroadcastGame, WatchPlayer } from '../../src/shared/types'
import { formatClock } from '../utils/clock'
import { lastMoveKeys, setupPositionAfter, setupSanHistory, fen as fenOf } from '../utils/chess'
import { useWatchStore } from '../stores/watch'
import { useFriendsStore } from '../stores/friends'
import { useAnalysisStore } from '../stores/analysis'

const online = useOnline()
const store = useKChessStore()
const watcher = useWatchStore()
const friends = useFriendsStore()
const analysis = useAnalysisStore()
const { settings, onlinePhase } = storeToRefs(store)
const tab = useLocalStorage<'tv' | 'friends' | 'broadcasts'>('kchess:watch-tab', 'tv')
const playingOwnGame = computed(() =>
  ['playing', 'seeking', 'disconnected'].includes(onlinePhase.value),
)

onMounted(() => {
  watcher.listen()
  if (online.value) {
    void watcher.loadChannels()
    void friends.refreshStatus()
    void watcher.loadBroadcasts()
  }
})
onUnmounted(() => {
  // Leaving the page stops the stream: nothing downloads for a board no one sees.
  void watcher.stop()
  watcher.unlisten()
})
watchValue(tab, () => void watcher.stop())
watchValue(online, (connected) => {
  if (connected) {
    void watcher.loadChannels()
    void friends.refreshStatus()
    void watcher.loadBroadcasts()
  } else void watcher.stop()
})

const now = ref(performance.now())
useIntervalFn(() => (now.value = performance.now()), 500)

/* ── TV and single games ────────────────────────────────────────────── */
const frame = computed(() => watcher.frame)
function clockOf(color: 'white' | 'black'): string | undefined {
  const current = frame.value
  if (!current) return undefined
  const base = color === 'white' ? current.whiteClock : current.blackClock
  if (base === undefined) return undefined
  const turn = current.fen.split(' ')[1] === 'b' ? 'black' : 'white'
  const running =
    watcher.connection?.phase === 'connected' &&
    !current.finished &&
    turn === color &&
    current.lastMove !== undefined
  const elapsed = running ? (now.value - watcher.frameAt) / 1000 : 0
  return formatClock(Math.max(0, base - elapsed) * 1000)
}
function label(player: WatchPlayer): string {
  return `${player.title ? `${player.title} ` : ''}${player.name}${player.rating ? ` (${player.rating})` : ''}`
}
const framePlayers = computed(() => {
  const current = frame.value
  if (!current) return null
  const white = { name: label(current.white), icon: 'i-lucide-user', clock: clockOf('white') }
  const black = { name: label(current.black), icon: 'i-lucide-user', clock: clockOf('black') }
  return current.orientation === 'white'
    ? { top: black, bottom: white, topColor: 'black' as const, bottomColor: 'white' as const }
    : { top: white, bottom: black, topColor: 'white' as const, bottomColor: 'black' as const }
})
const frameLast = computed<Key[] | undefined>(() => {
  const move = frame.value?.lastMove
  return move && !move.includes('@')
    ? [move.slice(0, 2) as Key, move.slice(2, 4) as Key]
    : undefined
})
function flipFrame(): void {
  if (frame.value) frame.value.orientation = frame.value.orientation === 'white' ? 'black' : 'white'
}

/* ── Friends ──────────────────────────────────────────────────────────── */
const playingFriends = computed(() =>
  friends.friends
    .map((friend) => ({ friend, status: friends.status[friend.username.toLowerCase()] }))
    .filter((entry) => entry.status?.playing),
)

/* ── Broadcasts ───────────────────────────────────────────────────────── */
const sections = computed(() => {
  const list = watcher.broadcastList ?? []
  return [
    { key: 'active', label: 'Live now', items: list.filter((b) => b.section === 'active') },
    { key: 'upcoming', label: 'Coming up', items: list.filter((b) => b.section === 'upcoming') },
    { key: 'past', label: 'Recently finished', items: list.filter((b) => b.section === 'past') },
  ].filter((section) => section.items.length)
})
const roundItems = computed(
  () =>
    watcher.tour?.rounds.map((round) => ({
      label: `${round.name}${round.ongoing ? ' · live' : round.finished ? '' : ' · upcoming'}`,
      value: round.id,
    })) ?? [],
)
const selectedRound = computed({
  get: () => watcher.roundId,
  set: (id: string) => void watcher.openRound(id),
})
const game = computed(() => watcher.currentGame)
const gamePly = ref<number | null>(null)
watchValue(
  () => watcher.selectedGame,
  () => (gamePly.value = null),
)
const gameSetup = computed(() => ({
  variant: 'standard' as const,
  fen: game.value?.startFen ?? '',
}))
const gameSan = computed(() =>
  game.value ? setupSanHistory(gameSetup.value, game.value.moves) : [],
)
const shownPly = computed(() => Math.min(gamePly.value ?? Infinity, game.value?.moves.length ?? 0))
const gameFen = computed(() =>
  game.value
    ? shownPly.value === game.value.moves.length
      ? game.value.fen
      : fenOf(setupPositionAfter(gameSetup.value, game.value.moves.slice(0, shownPly.value)))
    : '',
)
const gameOrientation = ref<'white' | 'black'>('white')
function boardPlayers(entry: BroadcastGame) {
  const white = {
    name: label(entry.white),
    icon: 'i-lucide-user',
    clock: entry.whiteClock !== undefined ? formatClock(entry.whiteClock * 1000) : undefined,
  }
  const black = {
    name: label(entry.black),
    icon: 'i-lucide-user',
    clock: entry.blackClock !== undefined ? formatClock(entry.blackClock * 1000) : undefined,
  }
  return gameOrientation.value === 'white'
    ? { top: black, bottom: white }
    : { top: white, bottom: black }
}
function analyse(entry: BroadcastGame): void {
  if (!analysis.loadPgn(entry.pgn, shownPly.value)) return
  analysis.orientation = gameOrientation.value
  analysis.origin = { white: entry.white.name, black: entry.black.name }
  store.selectPage('analysis')
}
function keydown(event: KeyboardEvent): void {
  if (!game.value || event.target instanceof HTMLInputElement) return
  const max = game.value.moves.length
  const at = shownPly.value
  const next =
    event.key === 'ArrowLeft'
      ? at - 1
      : event.key === 'ArrowRight'
        ? at + 1
        : event.key === 'ArrowUp'
          ? 0
          : event.key === 'ArrowDown'
            ? max
            : undefined
  if (next === undefined) return
  event.preventDefault()
  const clamped = Math.max(0, Math.min(max, next))
  gamePly.value = clamped === max ? null : clamped
}
onMounted(() => window.addEventListener('keydown', keydown))
onUnmounted(() => window.removeEventListener('keydown', keydown))
</script>

<template>
  <div>
    <PageHeader title="Watch">
      <UTabs
        v-model="tab"
        :items="[
          { label: 'TV', value: 'tv', icon: 'i-lucide-tv' },
          { label: 'Following', value: 'friends', icon: 'i-lucide-users' },
          { label: 'Broadcasts', value: 'broadcasts', icon: 'i-lucide-radio' },
        ]"
        :content="false"
        size="sm"
        variant="pill"
      />
    </PageHeader>

    <PublicOnlineNotice
      offline-message="Live games need an internet connection. Reconnect to watch; local play and saved studies are available offline."
    />
    <UAlert
      v-if="playingOwnGame"
      class="mb-4"
      color="warning"
      variant="subtle"
      icon="i-lucide-swords"
      title="You are in a game"
      description="Watching is paused while you play, so your own game has the connection."
    />
    <UButton
      v-if="
        watcher.target &&
        watcher.connection &&
        ['error', 'ended'].includes(watcher.connection.phase)
      "
      size="xs"
      :disabled="!online"
      @click="watcher.watch(watcher.target)"
      >Retry feed</UButton
    >
    <p v-if="watcher.error" class="text-error text-sm mb-3" role="alert">{{ watcher.error }}</p>

    <!-- TV and friends share one board. -->
    <div v-if="tab !== 'broadcasts'" class="online-columns">
      <div>
        <div v-if="frame && framePlayers" class="board-stack watch-board">
          <PlayerLine :player="framePlayers.top" :color="framePlayers.topColor" />
          <ChessBoard
            :fen="frame.fen"
            :orientation="frame.orientation"
            :theme="settings.boardTheme"
            :coordinates="settings.coordinates"
            :piece-set="settings.pieceSet"
            :animation="settings.pieceAnimation"
            :interactive="false"
            :last-move="frameLast"
          />
          <PlayerLine :player="framePlayers.bottom" :color="framePlayers.bottomColor" />
          <div class="flex flex-wrap items-center gap-2 text-sm">
            <span class="muted flex-1">
              {{ frame.finished ? `Finished · ${frame.status ?? ''}` : 'Live' }}
              {{ frame.source === 'game' ? ' · moves shown with Lichess’s anti-cheat delay' : '' }}
            </span>
            <UButton
              size="xs"
              variant="ghost"
              color="neutral"
              icon="i-lucide-arrow-up-down"
              @click="flipFrame"
              >Flip</UButton
            >
            <UButton
              size="xs"
              variant="ghost"
              color="neutral"
              icon="i-lucide-square"
              @click="watcher.stop()"
              >Stop</UButton
            >
          </div>
        </div>
        <div v-else class="card">
          <UEmpty
            variant="naked"
            icon="i-lucide-tv"
            :title="watcher.target ? 'Connecting…' : 'Pick something to watch'"
            :description="
              tab === 'tv'
                ? 'Choose a channel: the board follows its featured game and moves on to the next.'
                : 'Followed players who are playing right now are listed; watch any of their games live.'
            "
          />
        </div>
      </div>
      <section v-if="tab === 'tv'" class="card">
        <div class="card-header">
          <h2 class="section-title">Channels</h2>
          <UButton
            size="xs"
            variant="ghost"
            color="neutral"
            icon="i-lucide-refresh-cw"
            aria-label="Refresh channels"
            :disabled="!online"
            @click="watcher.loadChannels()"
          />
        </div>
        <div class="list-rows">
          <button
            v-for="channel in watcher.channels"
            :key="channel.key"
            type="button"
            class="list-row text-left"
            :class="{
              'ring-1 ring-primary':
                watcher.target &&
                'channel' in watcher.target &&
                watcher.target.channel === channel.key,
            }"
            :disabled="playingOwnGame || !online"
            @click="watcher.watch({ channel: channel.key })"
          >
            <div class="row-main">
              <div class="row-title">{{ channel.label }}</div>
              <div v-if="channel.player" class="row-sub">{{ label(channel.player) }}</div>
            </div>
          </button>
        </div>
      </section>
      <section v-else class="card">
        <div class="card-header">
          <div>
            <h2 class="section-title">Followed players playing now</h2>
            <p class="section-hint">
              {{
                friends.friends.length
                  ? `${friends.onlineCount} of ${friends.friends.length} online`
                  : 'Follow players on the Following page.'
              }}
            </p>
          </div>
          <UButton
            size="xs"
            variant="ghost"
            color="neutral"
            icon="i-lucide-refresh-cw"
            aria-label="Refresh followed players"
            :disabled="!online"
            @click="friends.refreshStatus()"
          />
        </div>
        <p v-if="!playingFriends.length" class="muted text-sm">
          No followed player is playing right now.
        </p>
        <div class="list-rows">
          <div v-for="entry in playingFriends" :key="entry.friend.username" class="list-row">
            <UIcon name="i-lucide-swords" />
            <div class="row-main">
              <div class="row-title">{{ entry.friend.username }}</div>
              <div class="row-sub">Playing now</div>
            </div>
            <UButton
              size="xs"
              :disabled="!online || !entry.status?.playingId || playingOwnGame"
              @click="entry.status?.playingId && watcher.watch({ gameId: entry.status.playingId })"
              >Watch</UButton
            >
          </div>
        </div>
      </section>
    </div>

    <!-- Broadcasts -->
    <div v-else>
      <div v-if="!watcher.tour" class="flex flex-col gap-5">
        <p v-if="!watcher.broadcastList" class="muted text-sm">Loading broadcasts…</p>
        <section v-for="section in sections" :key="section.key" class="card">
          <h2 class="section-title mb-3">{{ section.label }}</h2>
          <div class="list-rows">
            <button
              v-for="item in section.items"
              :key="item.tourId"
              type="button"
              class="list-row text-left"
              :disabled="playingOwnGame || !online"
              @click="watcher.openTour(item.tourId)"
            >
              <img v-if="item.image" :src="item.image" alt="" class="w-16 rounded" />
              <div class="row-main">
                <div class="row-title">{{ item.tourName }}</div>
                <div class="row-sub">
                  {{ item.roundName }}{{ item.ongoing ? ' · live' : '' }}
                  <span v-if="item.startsAt && section.key === 'upcoming'">
                    · {{ new Date(item.startsAt).toLocaleString() }}</span
                  >
                </div>
              </div>
            </button>
          </div>
        </section>
      </div>
      <div v-else class="flex flex-col gap-4">
        <div class="flex flex-wrap items-center gap-2">
          <UButton
            size="sm"
            variant="ghost"
            color="neutral"
            icon="i-lucide-arrow-left"
            @click="watcher.closeTour()"
            >All broadcasts</UButton
          >
          <h2 class="section-title flex-1">{{ watcher.tour.name }}</h2>
          <USelect
            v-model="selectedRound"
            :items="roundItems"
            size="sm"
            aria-label="Round"
            class="min-w-48"
          />
        </div>
        <p v-if="watcher.roundEnded && !watcher.roundGames.length" class="muted text-sm">
          No games in this round yet.
        </p>
        <div v-if="game" class="play-layout">
          <div class="board-stack">
            <PlayerLine
              :player="boardPlayers(game).top"
              :color="gameOrientation === 'white' ? 'black' : 'white'"
            />
            <ChessBoard
              :fen="gameFen"
              :orientation="gameOrientation"
              :theme="settings.boardTheme"
              :coordinates="settings.coordinates"
              :piece-set="settings.pieceSet"
              :animation="settings.pieceAnimation"
              :interactive="false"
              :last-move="lastMoveKeys(game.moves.slice(0, shownPly))"
            />
            <PlayerLine :player="boardPlayers(game).bottom" :color="gameOrientation" />
          </div>
          <MovePanel
            :title="`${game.white.name} – ${game.black.name}`"
            :moves="gameSan"
            :ply="shownPly"
            empty-text="No moves yet."
            @select="gamePly = $event === game.moves.length ? null : $event"
            @flip="gameOrientation = gameOrientation === 'white' ? 'black' : 'white'"
          >
            <template #top>
              <div class="status-banner" role="status">
                <UIcon
                  :name="game.ongoing ? 'i-lucide-radio' : 'i-lucide-flag'"
                  class="status-icon"
                />
                <div>
                  <strong>{{ game.ongoing ? 'Live' : game.result }}</strong>
                  <span class="detail">{{ game.name }}</span>
                </div>
              </div>
              <OpeningName :setup="gameSetup" :moves="game.moves" :ply="shownPly" />
            </template>
            <template #bottom>
              <div class="panel-actions panel-divider pt-3.5">
                <UButton
                  variant="outline"
                  color="neutral"
                  icon="i-lucide-layout-grid"
                  @click="watcher.selectedGame = ''"
                  >All boards</UButton
                >
                <UButton
                  variant="outline"
                  color="neutral"
                  icon="i-lucide-microscope"
                  @click="analyse(game)"
                  >Analyse</UButton
                >
              </div>
            </template>
          </MovePanel>
        </div>
        <div v-else class="broadcast-grid">
          <button
            v-for="entry in watcher.roundGames"
            :key="entry.id"
            type="button"
            class="broadcast-cell"
            @click="watcher.selectedGame = entry.id"
          >
            <div class="text-xs truncate">{{ label(entry.black) }}</div>
            <ChessBoard
              :fen="entry.fen"
              orientation="white"
              :theme="settings.boardTheme"
              coordinates="none"
              :piece-set="settings.pieceSet"
              animation="none"
              :interactive="false"
              :last-move="lastMoveKeys(entry.moves)"
            />
            <div class="text-xs truncate">{{ label(entry.white) }}</div>
            <div class="text-xs muted">{{ entry.ongoing ? 'Live' : entry.result }}</div>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.watch-board {
  max-width: min(100%, calc(100dvh - 260px));
}
.broadcast-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
  gap: 14px;
}
.broadcast-cell {
  display: flex;
  flex-direction: column;
  gap: 4px;
  text-align: left;
  padding: 8px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg-elevated);
}
.broadcast-cell :deep(.cg-host) {
  pointer-events: none;
}
</style>
