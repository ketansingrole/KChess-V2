<script setup lang="ts">
import { computed, onMounted, watch } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import { storeToRefs } from 'pinia'
import type { Key } from '@lichess-org/chessground/types'
import type { TvPastGame } from '@kchess/contracts/types'
import { CHANNEL_ICONS, useTvPosition } from '../../composables/useTvPosition'
import { useWatchStore } from '../../stores/watch'
import TvViewer from './TvViewer.vue'
import { useKChessStore } from '../../stores/kchess'

const props = defineProps<{
  online: boolean
  playingOwnGame: boolean
}>()

const watcher = useWatchStore()
const store = useKChessStore()
const { settings } = storeToRefs(store)
const tv = useTvPosition()
const { frame, label, openPast } = tv

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
</script>

<template>
  <div class="flex flex-col gap-6">
    <TvViewer :tv="tv">
      <template #header>
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
      </template>
      <div class="channel-list">
        <button
          v-for="channel in watcher.channels"
          :key="channel.key"
          type="button"
          class="channel-row"
          :aria-current="
            watcher.target && 'channel' in watcher.target && watcher.target.channel === channel.key
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
    </TvViewer>

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
@container page (max-width: 1100px) {
  .tv-layout .channel-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
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
