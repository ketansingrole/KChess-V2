<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Chess, type Square } from 'chess.js'

const props = defineProps<{ fen: string; orientation: 'white' | 'black'; light: string; dark: string; interactive?: boolean; lastMove?: string }>()
const emit = defineEmits<{ move: [uci: string] }>()
const selected = ref<Square | null>(null)
const highlights = ref<string[]>([])
const arrows = ref<Array<{ from: Square; to: Square }>>([])
const arrowStart = ref<Square | null>(null)
const dragStart = ref<Square | null>(null)
const suppressClick = ref(false)
const game = computed(() => new Chess(props.fen))
const symbols: Record<string, string> = { wk: '♔', wq: '♕', wr: '♖', wb: '♗', wn: '♘', wp: '♙', bk: '♚', bq: '♛', br: '♜', bb: '♝', bn: '♞', bp: '♟' }
const files = 'abcdefgh'
const squares = computed(() => {
  const ranks = props.orientation === 'white' ? [8, 7, 6, 5, 4, 3, 2, 1] : [1, 2, 3, 4, 5, 6, 7, 8]
  const orderedFiles = props.orientation === 'white' ? [...files] : [...files].reverse()
  return ranks.flatMap(rank => orderedFiles.map(file => `${file}${rank}` as Square))
})
const legal = computed(() => selected.value ? game.value.moves({ square: selected.value, verbose: true }).map(m => m.to) : [])
watch(() => props.fen, () => { selected.value = null })

function click(square: Square): void {
  if (suppressClick.value) { suppressClick.value = false; return }
  highlights.value = []; arrows.value = []
  if (!props.interactive) return
  const piece = game.value.get(square)
  if (selected.value && legal.value.includes(square)) {
    const from = selected.value
    const promotion = game.value.get(from)?.type === 'p' && (square.endsWith('8') || square.endsWith('1')) ? 'q' : ''
    selected.value = null
    emit('move', `${from}${square}${promotion}`)
  } else selected.value = piece?.color === game.value.turn() ? square : null
}

function pointerDown(event: PointerEvent, square: Square): void {
  if (event.button === 2) { arrowStart.value = square; (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId) }
  if (event.button === 0 && props.interactive && game.value.get(square)?.color === game.value.turn()) { dragStart.value = square; (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId) }
}
function pointerUp(event: PointerEvent, square: Square): void {
  const target = document.elementFromPoint(event.clientX, event.clientY)?.closest('[data-square]')?.getAttribute('data-square') as Square | null
  const to = target ?? square
  if (event.button === 0 && dragStart.value) {
    const from = dragStart.value
    dragStart.value = null
    if (to !== from && game.value.moves({ square: from, verbose: true }).some(move => move.to === to)) {
      const promotion = game.value.get(from)?.type === 'p' && (to.endsWith('8') || to.endsWith('1')) ? 'q' : ''
      suppressClick.value = true
      setTimeout(() => { suppressClick.value = false }, 0)
      emit('move', `${from}${to}${promotion}`)
    }
    return
  }
  if (event.button !== 2 || !arrowStart.value) return
  if (to === arrowStart.value) highlights.value = highlights.value.includes(to) ? highlights.value.filter(s => s !== to) : [...highlights.value, to]
  else {
    const index = arrows.value.findIndex(arrow => arrow.from === arrowStart.value && arrow.to === to)
    arrows.value = index < 0 ? [...arrows.value, { from: arrowStart.value, to }] : arrows.value.filter((_, i) => i !== index)
  }
  arrowStart.value = null
}
function center(square: Square): { x: number; y: number } {
  const file = files.indexOf(square[0])
  const rank = Number(square[1]) - 1
  return { x: (props.orientation === 'white' ? file : 7 - file) * 12.5 + 6.25, y: (props.orientation === 'white' ? 7 - rank : rank) * 12.5 + 6.25 }
}
</script>

<template>
  <div class="chess-board" role="grid" aria-label="Chess board">
    <button v-for="(square, index) in squares" :key="square" type="button" role="gridcell"
      class="chess-square" :data-square="square" :class="{ selected: selected === square, legal: legal.includes(square), marked: highlights.includes(square), recent: lastMove?.includes(square) }"
      :style="{ backgroundColor: ((Number(square[1]) + files.indexOf(square[0])) % 2 === 0) ? dark : light }"
      :aria-label="`${square} ${game.get(square)?.color === 'w' ? 'white' : 'black'} ${game.get(square)?.type ?? 'empty'}`"
      @click="click(square)" @pointerdown="pointerDown($event, square)" @pointerup="pointerUp($event, square)" @contextmenu.prevent>
      <span v-if="index % 8 === 0" class="coord rank">{{ square[1] }}</span>
      <span v-if="index >= 56" class="coord file">{{ square[0] }}</span>
      <span v-if="game.get(square)" class="piece" :class="game.get(square)?.color === 'w' ? 'white-piece' : 'black-piece'">{{ symbols[`${game.get(square)?.color}${game.get(square)?.type}`] }}</span>
      <span v-else-if="legal.includes(square)" class="move-dot" />
    </button>
    <svg v-if="arrows.length" class="board-arrows" viewBox="0 0 100 100" aria-hidden="true"><defs><marker id="arrowhead" markerWidth="5" markerHeight="5" refX="3.5" refY="2.5" orient="auto"><polygon points="0 0, 5 2.5, 0 5" fill="#d78b28"/></marker></defs><line v-for="arrow in arrows" :key="`${arrow.from}-${arrow.to}`" :x1="center(arrow.from).x" :y1="center(arrow.from).y" :x2="center(arrow.to).x" :y2="center(arrow.to).y" stroke="#d78b28" stroke-width="1.7" stroke-linecap="round" marker-end="url(#arrowhead)" opacity=".85"/></svg>
  </div>
</template>
