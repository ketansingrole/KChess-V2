<script setup lang="ts">
import { computed } from 'vue'
import type { Key, Color } from '@lichess-org/chessground/types'
import type { MoveClassification } from '@kchess/rules/review'
import { MOVE_CLASSIFICATIONS } from '../utils/moveClassification'

const props = defineProps<{
  square: Key
  classification: MoveClassification
  orientation?: Color
}>()
const verdict = computed(() => MOVE_CLASSIFICATIONS[props.classification])
// Text marks reuse the move-list glyphs; the rest are drawn so they stay legible when small.
const ICONS: Partial<Record<MoveClassification, string>> = {
  best: 'M12 5.6l1.9 4.1 4.5.5-3.4 3 1 4.4L12 15.3l-4 2.3 1-4.4-3.4-3 4.5-.5z',
  excellent:
    'M6.4 11h2.5v6.8H6.4zM10.2 11.2l2.5-4.5c.7-1.2 2.4-.6 2.1.8l-.5 2.5h2.7c1 0 1.7.9 1.5 1.9l-.9 4.4c-.2.8-.9 1.4-1.7 1.4h-5.7z',
  good: 'M7.4 12.4l3.1 3.1 6.1-6.6',
}
const icon = computed(() => ICONS[props.classification])
const placement = computed(() => {
  const file = props.square.charCodeAt(0) - 97
  const rank = Number(props.square[1]) - 1
  const flipped = props.orientation === 'black'
  const column = flipped ? 7 - file : file
  const row = flipped ? rank : 7 - rank
  return {
    left: `${column * 12.5}%`,
    top: `${row * 12.5}%`,
    '--move-color': verdict.value.color,
    // The badge overhangs the top-right corner, except where that would leave the board.
    '--badge-x': column === 7 ? '0%' : '32%',
    '--badge-y': row === 0 ? '0%' : '-32%',
  }
})
</script>

<template>
  <div class="board-feedback" aria-live="polite" aria-atomic="true">
    <div
      class="classified-square"
      :style="placement"
      :data-classification="classification"
      :data-square="square"
    >
      <svg
        class="move-badge"
        viewBox="0 0 24 24"
        role="img"
        :aria-label="`${verdict.label} move on ${square}`"
      >
        <circle class="badge-disc" cx="12" cy="12" r="11" />
        <path
          v-if="icon"
          class="badge-icon"
          :class="{ stroked: classification === 'good' }"
          :d="icon"
        />
        <text v-else class="badge-text" x="12" y="12" dy="0.36em">{{ verdict.glyph }}</text>
      </svg>
    </div>
  </div>
</template>

<style scoped>
.board-feedback {
  position: absolute;
  inset: 0;
  z-index: 5;
  pointer-events: none;
  overflow: hidden;
  border-radius: var(--board-radius);
  container-type: inline-size;
}
.classified-square {
  position: absolute;
  width: 12.5%;
  height: 12.5%;
}
.move-badge {
  position: absolute;
  top: 0;
  right: 0;
  width: 40%;
  height: 40%;
  overflow: visible;
  transform: translate(var(--badge-x), var(--badge-y));
  filter: drop-shadow(0 1px 1.5px #0006);
}
.badge-disc {
  fill: var(--move-color);
  stroke: #fff;
  stroke-width: 1.4;
}
.badge-icon {
  fill: #fff;
  stroke: #fff;
  stroke-width: 0.6;
  stroke-linejoin: round;
}
.badge-icon.stroked {
  fill: none;
  stroke-width: 2.6;
  stroke-linecap: round;
}
.badge-text {
  fill: #fff;
  font-family: system-ui, sans-serif;
  font-size: 12.5px;
  font-weight: 800;
  letter-spacing: -0.3px;
  text-anchor: middle;
}
</style>
