<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch as watchValue } from 'vue'
import { useIntervalFn, useLocalStorage, useOnline } from '@vueuse/core'
import type { Key } from '@lichess-org/chessground/types'
import type { BroadcastGame, Crosstable, TvPastGame, WatchPlayer } from '../../src/shared/types'
import { isVariant, type GameSetup } from '../../src/shared/variant'
import { formatClock } from '../utils/clock'
import { materialBalance } from '../utils/material'
import {
  lastMoveKeys,
  setupPgn,
  setupPositionAfter,
  setupSanHistory,
  fen as fenOf,
} from '../utils/chess'
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

/* ── Channels ─────────────────────────────────────────────────────────── */
const CHANNEL_ICONS: Record<string, string> = {
  best: 'i-lucide-crown',
  bullet: 'i-lucide-zap',
  blitz: 'i-lucide-flame',
  rapid: 'i-lucide-rabbit',
  classical: 'i-lucide-turtle',
  ultraBullet: 'i-lucide-rocket',
  chess960: 'i-lucide-dices',
  kingOfTheHill: 'i-lucide-mountain',
  threeCheck: 'i-lucide-layers',
  antichess: 'i-lucide-shield-off',
  atomic: 'i-lucide-atom',
  horde: 'i-lucide-grid-3x3',
  racingKings: 'i-lucide-flag',
  crazyhouse: 'i-lucide-house',
  bot: 'i-lucide-bot',
  computer: 'i-lucide-cpu',
}
const SPEEDS: Record<string, { label: string; icon: string }> = {
  ultraBullet: { label: 'UltraBullet', icon: 'i-lucide-rocket' },
  bullet: { label: 'Bullet', icon: 'i-lucide-zap' },
  blitz: { label: 'Blitz', icon: 'i-lucide-flame' },
  rapid: { label: 'Rapid', icon: 'i-lucide-rabbit' },
  classical: { label: 'Classical', icon: 'i-lucide-turtle' },
  correspondence: { label: 'Correspondence', icon: 'i-lucide-mail' },
}
/** Channels that name a variant rather than a speed or a kind of player. */
const NOT_VARIANTS = new Set(['best', 'bot', 'computer', ...Object.keys(SPEEDS)])
const lastChannel = useLocalStorage('kchess:watch-channel', 'best')
function watchChannel(key: string): void {
  lastChannel.value = key
  void watcher.watch({ channel: key })
}
/** TV opens on the channel watched last (Top rated the first time) instead of an empty board. */
function autoWatch(): void {
  if (tab.value !== 'tv' || !online.value || playingOwnGame.value || watcher.target) return
  const known =
    !watcher.channels.length || watcher.channels.some((c) => c.key === lastChannel.value)
  watchChannel(known ? lastChannel.value : 'best')
}

/** Reopens the chosen broadcast round's feed, which stopping the stream left idle. */
function resumeBroadcast(): void {
  if (tab.value === 'broadcasts' && online.value && !playingOwnGame.value) void watcher.resumeTour()
}
onMounted(() => {
  watcher.listen()
  if (online.value) {
    void watcher.loadChannels()
    void friends.refreshStatus()
    void watcher.loadBroadcasts()
    resumeBroadcast()
    autoWatch()
  }
})
onUnmounted(() => {
  // Leaving the page stops the stream: nothing downloads for a board no one sees.
  void watcher.stop()
  watcher.unlisten()
})
watchValue(tab, async () => {
  await watcher.stop()
  resumeBroadcast()
  autoWatch()
})
watchValue(online, (connected) => {
  if (connected) {
    void watcher.loadChannels()
    void friends.refreshStatus()
    void watcher.loadBroadcasts()
    resumeBroadcast()
    autoWatch()
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
const frameLast = computed<Key[] | undefined>(() => {
  const move = frame.value?.lastMove
  return move && !move.includes('@')
    ? [move.slice(0, 2) as Key, move.slice(2, 4) as Key]
    : undefined
})

/* Move list: known once Lichess's delayed export lines up with the feed. */
const tvSetup = computed<GameSetup | null>(() => {
  const current = frame.value
  return current?.moves && current.startFen && isVariant(current.variant)
    ? { variant: current.variant, fen: current.startFen }
    : null
})
const tvMoves = computed(() => (tvSetup.value && frame.value?.moves) || [])
const tvSan = computed(() => (tvSetup.value ? setupSanHistory(tvSetup.value, tvMoves.value) : []))
/** The ply being looked at, or null to follow the live game. */
const tvPly = ref<number | null>(null)
const tvShownPly = computed(() => Math.min(tvPly.value ?? Infinity, tvMoves.value.length))
const tvBrowsing = computed(
  () => tvSetup.value !== null && tvPly.value !== null && tvShownPly.value < tvMoves.value.length,
)
const tvFen = computed(() => {
  if (!frame.value) return ''
  return tvBrowsing.value
    ? fenOf(setupPositionAfter(tvSetup.value!, tvMoves.value.slice(0, tvShownPly.value)))
    : frame.value.fen
})
const tvLast = computed(() =>
  tvBrowsing.value ? lastMoveKeys(tvMoves.value.slice(0, tvShownPly.value)) : frameLast.value,
)
const flipped = ref(false)
watchValue(
  () => frame.value?.gameId,
  () => {
    tvPly.value = null
    flipped.value = false
  },
)
const tvOrientation = computed(() => {
  const side = frame.value?.orientation ?? 'white'
  return flipped.value ? (side === 'white' ? 'black' : 'white') : side
})
function selectTvPly(ply: number): void {
  tvPly.value = ply >= tvMoves.value.length ? null : Math.max(0, ply)
}
const tvEmptyText = computed(() => {
  const current = frame.value
  if (current?.source !== 'tv') return 'Lichess does not send this game’s moves.'
  if (current.variant === 'crazyhouse') return 'No move list for Crazyhouse.'
  return 'Loading moves…'
})

const framePlayers = computed(() => {
  const current = frame.value
  if (!current) return null
  const balance = materialBalance(tvFen.value)
  const white = {
    name: label(current.white),
    icon: 'i-lucide-user',
    clock: clockOf('white'),
    material: balance > 0 ? balance : 0,
  }
  const black = {
    name: label(current.black),
    icon: 'i-lucide-user',
    clock: clockOf('black'),
    material: balance < 0 ? -balance : 0,
  }
  return tvOrientation.value === 'white'
    ? { top: black, bottom: white, topColor: 'black' as const, bottomColor: 'white' as const }
    : { top: white, bottom: black, topColor: 'white' as const, bottomColor: 'black' as const }
})

const ENDINGS: Record<string, string> = {
  mate: 'Checkmate',
  resign: 'Resignation',
  outoftime: 'Time out',
  timeout: 'Opponent left',
  draw: 'Draw',
  stalemate: 'Stalemate',
  aborted: 'Aborted',
  noStart: 'Did not start',
  cheat: 'Cheat detected',
  variantEnd: 'Variant win',
}
/** "Black won · Checkmate", or "Live" while the game goes on. */
const frameResult = computed(() => {
  const current = frame.value
  if (!current || !current.finished) return 'Live'
  const how = ENDINGS[current.status ?? ''] ?? current.status ?? ''
  if (!current.winner) return how || 'Finished'
  return `${current.winner === 'white' ? 'White' : 'Black'} won${how ? ` · ${how}` : ''}`
})

const pastGames = computed<TvPastGame[]>(() =>
  tab.value === 'tv' ? (frame.value?.previous ?? []) : [],
)
function pastResult(entry: TvPastGame): string {
  if (entry.winner) return entry.winner === 'white' ? '1–0' : '0–1'
  if (!entry.status || ['created', 'started'].includes(entry.status)) return ''
  return ['aborted', 'noStart'].includes(entry.status) ? 'Aborted' : '½–½'
}
/** One side's points in a finished game: 1, 0 or ½, as Lichess labels its past TV games. */
function pastScore(entry: TvPastGame, side: 'white' | 'black'): string {
  const result = pastResult(entry)
  if (!result || result === 'Aborted') return ''
  if (result === '½–½') return '½'
  return entry.winner === side ? '1' : '0'
}
const toast = useToast()
async function openPast(entry: TvPastGame): Promise<void> {
  try {
    const pgn = await window.kchess.exportGame(entry.gameId)
    if (!analysis.loadPgn(pgn)) throw new Error('That game could not be read.')
    analysis.orientation = entry.orientation
    analysis.origin = { white: entry.white.name, black: entry.black.name, gameId: entry.gameId }
    store.selectPage('analysis')
  } catch (cause) {
    toast.add({
      title: 'Could not open the game',
      description: cause instanceof Error ? cause.message : String(cause),
      color: 'error',
    })
  }
}
/** Opens the TV game so far in the analysis board, at the move being looked at. */
async function analyseTv(): Promise<void> {
  const current = frame.value
  if (!current) return
  const origin = { white: current.white.name, black: current.black.name, gameId: current.gameId }
  if (tvSetup.value) {
    const pgn = setupPgn(tvSetup.value, tvMoves.value, {
      White: current.white.name,
      Black: current.black.name,
    })
    if (!analysis.loadPgn(pgn, tvShownPly.value)) return
    analysis.orientation = tvOrientation.value
    analysis.origin = origin
    store.selectPage('analysis')
  } else await openPast({ ...current, orientation: tvOrientation.value })
}

/** Lichess-style base time: 60 → "1", 30 → "½", 90 → "1.5". */
function baseMinutes(seconds: number): string {
  const fractions: Record<number, string> = { 15: '¼', 30: '½', 45: '¾' }
  return fractions[seconds] ?? String(Math.round((seconds / 60) * 10) / 10)
}
/** "1+0 · Rated · Bullet", the line Lichess shows above a TV game. */
const frameInfo = computed(() => {
  const current = frame.value
  if (!current) return null
  const speed = current.speed ? SPEEDS[current.speed] : undefined
  const variant =
    current.source === 'tv'
      ? current.channel && !NOT_VARIANTS.has(current.channel)
        ? watcher.channels.find((c) => c.key === current.channel)?.label
        : undefined
      : current.variant !== 'standard'
        ? current.variant
        : undefined
  const parts = [
    current.clock && `${baseMinutes(current.clock.initial)}+${current.clock.increment}`,
    current.rated === undefined ? undefined : current.rated ? 'Rated' : 'Casual',
    speed?.label ?? current.speed,
    variant,
  ].filter(Boolean)
  return {
    text: parts.join(' · '),
    icon: speed?.icon ?? CHANNEL_ICONS[current.channel ?? ''] ?? 'i-lucide-tv',
  }
})

/* ── Head to head ─────────────────────────────────────────────────────── */
const headToHead = ref<{ key: string; table: Crosstable } | null>(null)
const realPlayer = (name: string) =>
  name !== '?' && name !== 'Anonymous' && !name.startsWith('Stockfish level')
const pairing = computed(() =>
  frame.value ? `${frame.value.white.name}|${frame.value.black.name}` : '',
)
watchValue(pairing, async (key) => {
  if (!key || headToHead.value?.key === key) return
  headToHead.value = null
  const [white = '', black = ''] = key.split('|')
  if (!realPlayer(white) || !realPlayer(black)) return
  try {
    const table = await window.kchess.crosstable(white, black)
    if (pairing.value === key && table.nbGames) headToHead.value = { key, table }
  } catch {
    // Optional context: the game is watchable without it.
  }
})
function points(value: number | undefined): string {
  const whole = Math.floor(value ?? 0)
  return (value ?? 0) % 1 ? `${whole || ''}½` : String(whole)
}
const scoreLines = computed(() => {
  const found = headToHead.value
  const current = frame.value
  if (!found || !current) return []
  const score = (users: Record<string, number>) =>
    `${points(users[current.white.name.toLowerCase()])}–${points(users[current.black.name.toLowerCase()])}`
  return [
    { label: 'All time', score: score(found.table.users), games: found.table.nbGames },
    ...(found.table.matchup?.nbGames
      ? [
          {
            label: 'Current match',
            score: score(found.table.matchup.users),
            games: found.table.matchup.nbGames,
          },
        ]
      : []),
  ]
})

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
function resultText(entry: BroadcastGame): string {
  if (entry.ongoing) return 'Live'
  if (entry.result === '1-0') return 'White won · 1–0'
  if (entry.result === '0-1') return 'Black won · 0–1'
  if (entry.result === '1/2-1/2') return 'Draw · ½–½'
  return entry.result
}
function analyse(entry: BroadcastGame): void {
  if (!analysis.loadPgn(entry.pgn, shownPly.value)) return
  analysis.orientation = gameOrientation.value
  analysis.origin = { white: entry.white.name, black: entry.black.name }
  store.selectPage('analysis')
}
/** Arrow keys step through the open broadcast game, or the TV game when its moves are known. */
function keydown(event: KeyboardEvent): void {
  if (event.target instanceof HTMLInputElement) return
  const broadcast = tab.value === 'broadcasts'
  if (broadcast ? !game.value : !tvSetup.value) return
  const max = broadcast ? game.value!.moves.length : tvMoves.value.length
  const at = broadcast ? shownPly.value : tvShownPly.value
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
  if (broadcast) gamePly.value = clamped === max ? null : clamped
  else selectTvPly(clamped)
}
onMounted(() => window.addEventListener('keydown', keydown))
onUnmounted(() => window.removeEventListener('keydown', keydown))
</script>

<template>
  <div :class="{ 'broadcast-page': tab === 'broadcasts' && watcher.tour && game }">
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

    <PublicOnlineNotice v-if="!online" offline-message="Offline · reconnect to watch live games." />
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

    <!-- TV and friends share one board: channels or players | board | moves. -->
    <div v-if="tab !== 'broadcasts'" class="flex flex-col gap-6">
      <div class="tv-layout">
        <section v-if="tab === 'tv'" class="card watch-channels">
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
          <div class="channel-list">
            <button
              v-for="channel in watcher.channels"
              :key="channel.key"
              type="button"
              class="channel-row"
              :aria-current="
                watcher.target &&
                'channel' in watcher.target &&
                watcher.target.channel === channel.key
                  ? 'true'
                  : undefined
              "
              :disabled="playingOwnGame || !online"
              @click="watchChannel(channel.key)"
            >
              <UIcon
                :name="CHANNEL_ICONS[channel.key] ?? 'i-lucide-tv'"
                class="channel-icon"
                aria-hidden="true"
              />
              <span class="row-main">
                <span class="row-title">{{ channel.label }}</span>
                <span class="row-sub">{{
                  channel.player ? label(channel.player) : 'No game'
                }}</span>
              </span>
            </button>
          </div>
        </section>
        <section v-else class="card watch-channels">
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
                @click="
                  entry.status?.playingId && watcher.watch({ gameId: entry.status.playingId })
                "
                >Watch</UButton
              >
            </div>
          </div>
        </section>
        <template v-if="frame && framePlayers">
          <div class="board-stack">
            <PlayerLine :player="framePlayers.top" :color="framePlayers.topColor">
              <template #name>
                <span class="player-name">{{ framePlayers.top.name }}</span>
                <span v-if="framePlayers.top.material" class="material-diff"
                  >+{{ framePlayers.top.material }}</span
                >
              </template>
            </PlayerLine>
            <ChessBoard
              :fen="tvFen"
              :orientation="tvOrientation"
              :theme="settings.boardTheme"
              :coordinates="settings.coordinates"
              :piece-set="settings.pieceSet"
              :animation="settings.pieceAnimation"
              :interactive="false"
              :last-move="tvLast"
            />
            <PlayerLine :player="framePlayers.bottom" :color="framePlayers.bottomColor">
              <template #name>
                <span class="player-name">{{ framePlayers.bottom.name }}</span>
                <span v-if="framePlayers.bottom.material" class="material-diff"
                  >+{{ framePlayers.bottom.material }}</span
                >
              </template>
            </PlayerLine>
          </div>
          <MovePanel
            :moves="tvSan"
            :ply="tvShownPly"
            :empty-text="tvEmptyText"
            @select="selectTvPly"
            @flip="flipped = !flipped"
          >
            <template #top>
              <div class="status-banner" role="status">
                <UIcon
                  :name="frame.finished ? 'i-lucide-flag' : (frameInfo?.icon ?? 'i-lucide-tv')"
                  class="status-icon"
                />
                <div class="banner-text">
                  <strong>{{
                    frame.finished ? frameResult : frameInfo?.text || 'Live game'
                  }}</strong>
                  <span v-if="frame.finished && frameInfo?.text" class="detail">{{
                    frameInfo.text
                  }}</span>
                </div>
                <UBadge
                  v-if="!frame.finished && !tvBrowsing"
                  class="banner-action"
                  label="Live"
                  color="error"
                  variant="soft"
                  icon="i-lucide-radio"
                  size="sm"
                />
                <UButton
                  v-else-if="tvBrowsing"
                  class="banner-action"
                  size="xs"
                  variant="soft"
                  icon="i-lucide-radio"
                  label="Back to live"
                  @click="tvPly = null"
                />
              </div>
              <div v-if="scoreLines.length" class="watch-h2h" aria-label="Head to head">
                <span class="watch-h2h-title">{{ frame.white.name }} – {{ frame.black.name }}</span>
                <span v-for="line in scoreLines" :key="line.label" class="watch-h2h-line">
                  <span class="muted">{{ line.label }}</span>
                  <strong class="tabular">{{ line.score }}</strong>
                  <span class="muted tabular"
                    >{{ line.games }} game{{ line.games === 1 ? '' : 's' }}</span
                  >
                </span>
              </div>
            </template>
            <template #bottom>
              <p v-if="frame.source === 'game'" class="muted text-xs">
                Moves shown with Lichess’s anti-cheat delay.
              </p>
              <div class="panel-actions panel-divider pt-3.5">
                <UButton
                  variant="outline"
                  color="neutral"
                  icon="i-lucide-microscope"
                  @click="analyseTv"
                  >Analyse</UButton
                >
                <UButton
                  variant="outline"
                  color="neutral"
                  icon="i-lucide-square"
                  @click="watcher.stop()"
                  >Stop</UButton
                >
              </div>
            </template>
          </MovePanel>
        </template>
        <div v-else class="card tv-empty">
          <UEmpty
            variant="naked"
            icon="i-lucide-tv"
            :title="watcher.target ? 'Connecting…' : 'Pick something to watch'"
          />
        </div>
      </div>

      <section v-if="pastGames.length" aria-labelledby="tv-past-title">
        <h2 id="tv-past-title" class="section-title mb-3">Previously on this channel</h2>
        <div class="broadcast-grid">
          <button
            v-for="entry in pastGames"
            :key="entry.gameId"
            type="button"
            class="broadcast-cell"
            :title="`Open ${entry.white.name} – ${entry.black.name} in the analysis board`"
            @click="openPast(entry)"
          >
            <div class="past-player">
              <span class="truncate">{{
                label(entry.orientation === 'white' ? entry.black : entry.white)
              }}</span>
              <strong class="tabular">{{
                pastScore(entry, entry.orientation === 'white' ? 'black' : 'white')
              }}</strong>
            </div>
            <ChessBoard
              :fen="entry.fen"
              :orientation="entry.orientation"
              :theme="settings.boardTheme"
              coordinates="none"
              :piece-set="settings.pieceSet"
              animation="none"
              :interactive="false"
              :last-move="
                entry.lastMove && !entry.lastMove.includes('@')
                  ? [entry.lastMove.slice(0, 2) as Key, entry.lastMove.slice(2, 4) as Key]
                  : undefined
              "
            />
            <div class="past-player">
              <span class="truncate">{{
                label(entry.orientation === 'white' ? entry.white : entry.black)
              }}</span>
              <strong class="tabular">{{ pastScore(entry, entry.orientation) }}</strong>
            </div>
            <div class="text-xs muted">{{ pastResult(entry) || 'Result pending' }}</div>
          </button>
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
      <div v-else :class="game ? 'play-layout broadcast-layout' : 'flex flex-col gap-4'">
        <div class="broadcast-toolbar">
          <UButton
            variant="ghost"
            color="neutral"
            icon="i-lucide-arrow-left"
            aria-label="All broadcasts"
            class="-ms-2"
            @click="watcher.closeTour()"
          />
          <h2 class="section-title flex-1 truncate">{{ watcher.tour.name }}</h2>
          <USelect
            v-model="selectedRound"
            :items="roundItems"
            :placeholder="roundItems.length ? 'Choose a round' : 'No rounds yet'"
            :disabled="!roundItems.length"
            aria-label="Round"
            class="w-44"
          />
        </div>
        <p v-if="watcher.roundEnded && !watcher.roundGames.length" class="muted text-sm">
          No games in this round yet.
        </p>
        <template v-if="game">
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
                <div class="banner-text">
                  <strong>{{ resultText(game) }}</strong>
                  <OpeningName
                    class="detail"
                    :setup="gameSetup"
                    :moves="game.moves"
                    :ply="shownPly"
                  />
                </div>
              </div>
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
        </template>
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
.material-diff {
  margin-inline-start: 6px;
  color: var(--ui-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
}
.watch-h2h {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 4px 16px;
  padding: 8px 12px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg-elevated);
  font-size: 0.8125rem;
}
.watch-h2h-title {
  font-weight: 600;
}
.watch-h2h-line {
  display: inline-flex;
  align-items: baseline;
  gap: 6px;
}
.tv-layout {
  --board-chrome: 300px;
  --board-size: clamp(260px, min(100dvh - var(--board-chrome), 100cqw - 620px), 760px);
  display: grid;
  grid-template-columns: 240px var(--board-size) minmax(260px, 340px);
  grid-template-areas: 'list board panel';
  gap: 20px;
  justify-content: center;
}
.tv-layout > .board-stack {
  grid-area: board;
}
.tv-layout > .side-panel {
  grid-area: panel;
}
.tv-empty {
  grid-column: 2 / -1;
  align-self: start;
}
/* Side cards take the board's height and scroll inside it instead of stretching the page. */
.tv-layout > .watch-channels,
.tv-layout > .side-panel {
  height: 0;
  min-height: 100%;
}
.watch-channels {
  grid-area: list;
  display: flex;
  flex-direction: column;
  padding-inline: 12px;
}
.watch-channels .card-header {
  padding-inline: 8px;
}
.watch-channels .list-rows {
  min-height: 0;
  overflow-y: auto;
}
.tv-layout:has(.tv-empty) > .watch-channels {
  height: auto;
  max-height: calc(100dvh - 200px);
}
@container page (max-width: 1100px) {
  .tv-layout {
    --board-size: clamp(260px, min(100dvh - var(--board-chrome), 100cqw - 320px), 760px);
    grid-template-columns: var(--board-size) minmax(260px, 340px);
    grid-template-areas:
      'board panel'
      'list list';
  }
  .tv-empty {
    grid-column: 1 / -1;
  }
  .tv-layout > .watch-channels {
    height: auto;
    min-height: 0;
  }
  .tv-layout .channel-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
  }
}
@container page (max-width: 700px) {
  .tv-layout {
    --board-size: min(100cqw, 100dvh - 260px);
    grid-template-columns: minmax(0, var(--board-size));
    grid-template-areas: 'board' 'panel' 'list';
  }
  .tv-layout > .side-panel {
    height: 360px;
    min-height: 0;
  }
}
.past-player {
  display: flex;
  justify-content: space-between;
  gap: 6px;
  font-size: 0.75rem;
}
.channel-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
}
.channel-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px;
  border-radius: 8px;
  text-align: left;
}
.channel-row:hover:not(:disabled) {
  background: var(--ui-bg-accented);
}
.channel-row[aria-current='true'] {
  background: color-mix(in oklab, var(--ui-primary) 14%, transparent);
}
.channel-row[aria-current='true'] .channel-icon {
  color: var(--ui-primary);
}
.channel-row:disabled {
  opacity: 0.6;
}
.channel-row .row-main {
  display: grid;
  flex: 1 1 auto;
  min-width: 0;
}
.channel-row .row-title {
  overflow: hidden;
  font-weight: 600;
  font-size: 0.875rem;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.channel-row .row-sub {
  overflow: hidden;
  color: var(--ui-text-muted);
  font-size: 0.75rem;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.channel-icon {
  flex: none;
  color: var(--ui-text-muted);
  font-size: 1.25rem;
}
/* While a broadcast game is open the whole page, header tabs included, is as wide as the
   board and panel below it, so every right-hand control shares one edge. */
.broadcast-page {
  --board-chrome: 352px;
  --board-size: clamp(260px, min(100dvh - var(--board-chrome), 100cqw - 344px), 760px);
  max-width: calc(var(--board-size) + 464px);
  margin-inline: auto;
}
.broadcast-layout {
  /* The tour toolbar sits above the board and shares the grid's outer edges. */
  --board-chrome: 352px;
  row-gap: 16px;
}
.broadcast-toolbar {
  grid-column: 1 / -1;
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
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
