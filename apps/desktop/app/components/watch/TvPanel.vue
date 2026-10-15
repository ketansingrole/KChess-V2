<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import { storeToRefs } from 'pinia'
import type { Key } from '@lichess-org/chessground/types'
import type { TvPastGame } from '@kchess/contracts/types'
import { CHANNEL_ICONS, useTvPosition } from '../../composables/useTvPosition'
import { useWatchStore } from '../../stores/watch'
import { useKChessStore } from '../../stores/kchess'

const props = defineProps<{
  online: boolean
  playingOwnGame: boolean
}>()

const watcher = useWatchStore()
const store = useKChessStore()
const { settings } = storeToRefs(store)
const {
  frame,
  label,
  tvSetup,
  tvMoves,
  tvSan,
  tvPly,
  tvShownPly,
  tvBrowsing,
  tvFen,
  tvLast,
  flipped,
  tvOrientation,
  selectTvPly,
  tvEmptyText,
  framePlayers,
  frameResult,
  frameInfo,
  scoreLines,
  openPast,
  analyseTv,
} = useTvPosition()

/* ── Channels ─────────────────────────────────────────────────────────── */
const lastChannel = useLocalStorage('kchess:watch-channel', 'best')
function watchChannel(key: string): void {
  lastChannel.value = key
  void watcher.watch({ channel: key })
}
/** TV opens on the channel watched last (Top rated the first time) instead of an empty board. */
function autoWatch(): void {
  if (!props.online || props.playingOwnGame || watcher.target) return
  const known =
    !watcher.channels.length || watcher.channels.some((c) => c.key === lastChannel.value)
  watchChannel(known ? lastChannel.value : 'best')
}
onMounted(() => autoWatch())
watch(
  () => props.online,
  (connected) => {
    if (connected) autoWatch()
  },
)

const pastGames = computed<TvPastGame[]>(() => frame.value?.previous ?? [])
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

/** Arrow keys step through the TV game when its moves are known. */
function keydown(event: KeyboardEvent): void {
  const target = event.target as HTMLElement | null
  if (
    target?.closest(
      'input, textarea, select, [contenteditable="true"], [role="combobox"], [role="dialog"]',
    ) ||
    event.metaKey ||
    event.ctrlKey ||
    event.altKey
  )
    return
  if (!tvSetup.value) return
  const max = tvMoves.value.length
  const at = tvShownPly.value
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
  selectTvPly(Math.max(0, Math.min(max, next)))
}
onMounted(() => window.addEventListener('keydown', keydown))
onUnmounted(() => window.removeEventListener('keydown', keydown))
</script>

<template>
  <div class="flex flex-col gap-6">
    <div v-board-fit class="tv-layout">
      <section class="card watch-channels">
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
              <span class="row-sub">{{ channel.player ? label(channel.player) : 'No game' }}</span>
            </span>
          </button>
        </div>
      </section>
      <template v-if="frame && framePlayers">
        <div class="board-stack">
          <PlayerLine
            :player="framePlayers.top"
            :color="framePlayers.topColor"
            :material-fen="frame.variant === 'crazyhouse' ? undefined : tvFen"
          />
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
          <PlayerLine
            :player="framePlayers.bottom"
            :color="framePlayers.bottomColor"
            :material-fen="frame.variant === 'crazyhouse' ? undefined : tvFen"
          />
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
                <strong>{{ frame.finished ? frameResult : frameInfo?.text || 'Live game' }}</strong>
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
              <span class="watch-h2h-title"
                ><PlayerLink :username="frame.white.name" /> –
                <PlayerLink :username="frame.black.name"
              /></span>
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
        <div v-for="entry in pastGames" :key="entry.gameId" class="broadcast-cell">
          <div class="past-player">
            <PlayerLink
              class="truncate"
              :username="(entry.orientation === 'white' ? entry.black : entry.white).name"
              >{{ label(entry.orientation === 'white' ? entry.black : entry.white) }}</PlayerLink
            >
            <strong class="tabular">{{
              pastScore(entry, entry.orientation === 'white' ? 'black' : 'white')
            }}</strong>
          </div>
          <button
            type="button"
            class="past-board"
            :aria-label="`Open ${entry.white.name} – ${entry.black.name} in the analysis board`"
            @click="openPast(entry)"
          >
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
          </button>
          <div class="past-player">
            <PlayerLink
              class="truncate"
              :username="(entry.orientation === 'white' ? entry.white : entry.black).name"
              >{{ label(entry.orientation === 'white' ? entry.white : entry.black) }}</PlayerLink
            >
            <strong class="tabular">{{ pastScore(entry, entry.orientation) }}</strong>
          </div>
          <div class="text-xs muted">{{ pastResult(entry) || 'Result pending' }}</div>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
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
  --board-size: clamp(260px, min(100dvh - var(--board-chrome), 100cqw - 620px), var(--board-max));
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
    --board-size: clamp(260px, min(100dvh - var(--board-chrome), 100cqw - 320px), var(--board-max));
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
.past-board {
  display: block;
  border-radius: 6px;
}
.past-board :deep(.cg-host) {
  pointer-events: none;
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
