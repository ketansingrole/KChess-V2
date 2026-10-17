<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'
import { storeToRefs } from 'pinia'
import type { useTvPosition } from '../../composables/useTvPosition'
import { useWatchStore } from '../../stores/watch'
import { useKChessStore } from '../../stores/kchess'

/**
 * The watched game beside a list of what can be watched (TV channels, followed players): board,
 * move list, live status and head-to-head. The parent owns the `useTvPosition()` state it shows.
 */
const props = defineProps<{ tv: ReturnType<typeof useTvPosition> }>()

const watcher = useWatchStore()
const { settings } = storeToRefs(useKChessStore())
const {
  frame,
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
  analyseTv,
} = props.tv

/** Arrow keys step through the game when its moves are known. */
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
  <div v-board-fit class="tv-layout">
    <section class="card watch-channels">
      <div class="card-header"><slot name="header" /></div>
      <slot />
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
            <UButton variant="outline" color="neutral" icon="i-lucide-microscope" @click="analyseTv"
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
</style>
