<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import type { Color, Key } from '@lichess-org/chessground/types'
import type { EngineLine, Settings } from '@kchess/core/contracts/types'
import { formatEval, pvSan } from '@kchess/core/domain/analysisTree'

const props = withDefaults(
  defineProps<{
    fen: string
    lines: readonly EngineLine[]
    orientation: Color
    boardTheme?: Settings['boardTheme']
    pieceSet?: Settings['pieceSet']
    pending?: number
    actionLabel?: string
  }>(),
  { pending: 0, actionLabel: 'Play' },
)
const emit = defineEmits<{ select: [pv: readonly string[], moveIndex: number] }>()
const pvMoves = computed(() =>
  props.lines.map((line) => ({ line, moves: pvSan(props.fen, line.pv) })),
)
const PREVIEW_SIZE = 220
const preview = ref<{
  fen: string
  lastMove: Key[]
  style: { top: string; left: string; width: string }
} | null>(null)
function showPreview(event: Event, line: number, move: number): void {
  const pvMove = pvMoves.value[line]?.moves[move]
  if (!pvMove) return
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const gap = 8
  const top =
    rect.top - PREVIEW_SIZE - gap >= 0
      ? rect.top - PREVIEW_SIZE - gap
      : Math.min(rect.bottom + gap, window.innerHeight - PREVIEW_SIZE - gap)
  const left = Math.min(
    Math.max(gap, rect.left + rect.width / 2 - PREVIEW_SIZE / 2),
    window.innerWidth - PREVIEW_SIZE - gap,
  )
  // Keep the hovered position stable when fresh engine lines arrive.
  preview.value = {
    fen: pvMove.fen,
    lastMove: [pvMove.uci.slice(0, 2), pvMove.uci.slice(2, 4)] as Key[],
    style: {
      top: `${Math.max(gap, top)}px`,
      left: `${Math.max(gap, left)}px`,
      width: `${PREVIEW_SIZE}px`,
    },
  }
}
function hidePreview(): void {
  preview.value = null
}
function select(pv: readonly string[], index: number): void {
  hidePreview()
  emit('select', pv, index)
}
watch(() => props.fen, hidePreview)
onMounted(() => {
  window.addEventListener('scroll', hidePreview, true)
  window.addEventListener('resize', hidePreview)
})
onUnmounted(() => {
  window.removeEventListener('scroll', hidePreview, true)
  window.removeEventListener('resize', hidePreview)
})
</script>

<template>
  <ol aria-label="Engine lines" class="pv-list">
    <li v-for="(entry, index) in pvMoves" :key="index" class="pv-row">
      <button
        type="button"
        class="pv-eval"
        :title="`${actionLabel} ${entry.moves[0]?.san ?? ''}`"
        @click="select(entry.line.pv, 0)"
      >
        {{ formatEval(entry.line) }}
      </button>
      <span class="pv-moves">
        <button
          v-for="(pvMove, moveIndex) in entry.moves"
          :key="moveIndex"
          type="button"
          class="pv-move"
          :aria-label="`${actionLabel} ${pvMove.label}`"
          @click="select(entry.line.pv, moveIndex)"
          @mouseenter="showPreview($event, index, moveIndex)"
          @mouseleave="hidePreview"
          @focus="showPreview($event, index, moveIndex)"
          @blur="hidePreview"
        >
          {{ pvMove.label }}
        </button>
      </span>
    </li>
    <li
      v-for="index in Math.max(0, pending - pvMoves.length)"
      :key="`wait-${index}`"
      class="pv-row pending"
    >
      <span class="pv-eval">…</span>
    </li>
  </ol>
  <Teleport to="body">
    <div v-if="preview" class="pv-preview" :style="preview.style" aria-hidden="true">
      <ChessBoard
        :fen="preview.fen"
        :orientation="orientation"
        :theme="boardTheme"
        coordinates="none"
        :piece-set="pieceSet"
        animation="none"
        :interactive="false"
        :last-move="preview.lastMove"
      />
    </div>
  </Teleport>
</template>

<style scoped>
.pv-list {
  margin: 0;
  padding: 0;
  list-style: none;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg);
  overflow: hidden;
}
.pv-row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-height: 30px;
  padding: 5px 8px;
  font-size: 12.5px;
}
.pv-row + .pv-row {
  border-top: 1px solid var(--ui-border);
}
.pv-eval {
  flex: none;
  min-width: 3.4em;
  padding: 1px 6px;
  border: 0;
  border-radius: 6px;
  background: var(--ui-bg-accented);
  color: inherit;
  font: inherit;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  text-align: center;
  cursor: pointer;
}
.pv-row.pending .pv-eval {
  cursor: default;
  color: var(--ui-text-dimmed);
}
.pv-moves {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.pv-move {
  min-width: 24px;
  min-height: 24px;
  padding: 1px 2px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--ui-text-toned);
  font: inherit;
  cursor: pointer;
}
.pv-move:hover,
.pv-eval:hover {
  background: color-mix(in srgb, var(--ui-primary) 24%, transparent);
}
.pv-preview {
  position: fixed;
  z-index: 60;
  padding: 4px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg);
  pointer-events: none;
}
</style>
