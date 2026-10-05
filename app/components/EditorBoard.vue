<script setup lang="ts">
import { Chessground } from '@lichess-org/chessground'
import type { Api } from '@lichess-org/chessground/api'
import type { Config } from '@lichess-org/chessground/config'
import { dragNewPiece } from '@lichess-org/chessground/drag'
import type { Color, Key, MouchEvent, Piece, Role } from '@lichess-org/chessground/types'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { CoordinateMode, PieceAnimation } from '../../src/shared/types'
import { animationMs, DEFAULT_PIECE_SET, pieceVars } from '../utils/pieces'

/**
 * The board editor's board, as on Lichess: drag pieces anywhere (off the board removes them),
 * drag new ones in from the spare rows, or pick a spare piece (or the bin) and click squares.
 */
export type EditorTool = 'pointer' | 'trash' | `${Color}-${Role}`

const props = defineProps<{
  /** Piece placement (the first FEN field). */
  board: string
  orientation: Color
  theme?: string
  coordinates?: CoordinateMode
  pieceSet?: string
  animation?: PieceAnimation
}>()
const tool = defineModel<EditorTool>('tool', { default: 'pointer' })
const emit = defineEmits<{ 'update:board': [board: string] }>()

const ROLES: Role[] = ['king', 'queen', 'rook', 'bishop', 'knight', 'pawn']
const LETTER: Record<Role, string> = {
  king: 'K',
  queen: 'Q',
  rook: 'R',
  bishop: 'B',
  knight: 'N',
  pawn: 'P',
}
const pieceStyle = computed(() => pieceVars(props.pieceSet ?? DEFAULT_PIECE_SET))
/** Spare pieces of the side at the top above the board, the side at the bottom below it. */
const rows = computed<Color[]>(() =>
  props.orientation === 'white' ? ['black', 'white'] : ['white', 'black'],
)

const el = ref<HTMLElement | null>(null)
let ground: Api | undefined

function changed(): void {
  if (!ground) return
  const board = ground.getFen()
  if (board !== props.board) emit('update:board', board)
}

function configuration(): Config {
  const ms = animationMs[props.animation ?? 'normal']
  return {
    fen: props.board,
    orientation: props.orientation,
    coordinates: props.coordinates !== 'none',
    animation: { enabled: ms > 0, duration: ms },
    disableContextMenu: true,
    highlight: { lastMove: false, check: false },
    movable: { free: true, color: 'both', showDests: false },
    premovable: { enabled: false },
    draggable: { enabled: true, showGhost: true, deleteOnDropOff: true },
    selectable: { enabled: false },
    drawable: { enabled: true, visible: true },
    events: { change: changed },
  }
}

onMounted(() => {
  if (el.value) ground = Chessground(el.value, configuration())
})
onBeforeUnmount(() => {
  ground?.destroy()
  ground = undefined
})
watch(
  () => props.board,
  (board) => {
    if (ground && ground.getFen() !== board) ground.set({ fen: board })
  },
)
watch(
  () => [props.orientation, props.coordinates, props.animation] as const,
  () => {
    if (!ground) return
    const ms = animationMs[props.animation ?? 'normal']
    ground.set({
      orientation: props.orientation,
      coordinates: props.coordinates !== 'none',
      animation: { enabled: ms > 0, duration: ms },
    })
  },
)

function pieceOfTool(value: EditorTool): Piece | undefined {
  if (value === 'pointer' || value === 'trash') return undefined
  const [color, role] = value.split('-') as [Color, Role]
  return { color, role }
}

/**
 * With a spare piece or the bin chosen, a click puts that piece down (or takes it off again
 * when it is already there) instead of picking up what stands on the square.
 */
function paint(event: MouseEvent | TouchEvent): void {
  if (!ground || tool.value === 'pointer') return
  if ('button' in event && (event.button !== 0 || event.shiftKey)) return
  const point = 'touches' in event ? event.touches[0] : event
  if (!point) return
  const key = ground.getKeyAtDomPos([point.clientX, point.clientY])
  if (!key) return
  event.preventDefault()
  event.stopPropagation()
  const piece = pieceOfTool(tool.value)
  const there = ground.state.pieces.get(key)
  const same = piece && there && there.color === piece.color && there.role === piece.role
  ground.setPieces(new Map<Key, Piece | undefined>([[key, same ? undefined : piece]]))
  changed()
}

/** Pressing a spare piece chooses it and starts dragging a new one onto the board. */
function spareDown(color: Color, role: Role, event: MouseEvent | TouchEvent): void {
  if ('button' in event && event.button !== 0) return
  tool.value = `${color}-${role}`
  if (!ground) return
  event.preventDefault()
  dragNewPiece(ground.state, { color, role }, event as MouchEvent, true)
}

function spareLabel(color: Color, role: Role): string {
  return `${color === 'white' ? 'White' : 'Black'} ${role}`
}
</script>

<template>
  <div class="editor-board" :style="pieceStyle">
    <div
      v-for="(color, index) in rows"
      :key="color"
      class="spare-row"
      :style="{ order: index * 2 }"
      role="toolbar"
      :aria-label="`${color === 'white' ? 'White' : 'Black'} pieces`"
    >
      <UTooltip text="Move pieces">
        <button
          type="button"
          class="spare tool"
          :class="{ selected: tool === 'pointer' }"
          aria-label="Move pieces"
          :aria-pressed="tool === 'pointer'"
          @click="tool = 'pointer'"
        >
          <UIcon name="i-lucide-mouse-pointer-2" />
        </button>
      </UTooltip>
      <button
        v-for="role in ROLES"
        :key="role"
        type="button"
        class="spare"
        :class="{ selected: tool === `${color}-${role}` }"
        :aria-label="spareLabel(color, role)"
        :aria-pressed="tool === `${color}-${role}`"
        :title="spareLabel(color, role)"
        @mousedown="spareDown(color, role, $event)"
        @touchstart="spareDown(color, role, $event)"
        @keydown.enter.space.prevent="tool = `${color}-${role}`"
      >
        <span
          class="spare-piece"
          :style="{
            backgroundImage: `var(--piece-${color[0]}${LETTER[role]})`,
          }"
        />
      </button>
      <UTooltip text="Remove pieces">
        <button
          type="button"
          class="spare tool"
          :class="{ selected: tool === 'trash' }"
          aria-label="Remove pieces"
          :aria-pressed="tool === 'trash'"
          @click="tool = 'trash'"
        >
          <UIcon name="i-lucide-trash-2" />
        </button>
      </UTooltip>
    </div>
    <div
      class="cg-host"
      style="order: 1"
      :class="[
        `board-theme-${theme ?? 'brown'}`,
        `coords-${coordinates ?? 'inside'}`,
        { painting: tool !== 'pointer' },
      ]"
      @mousedown.capture="paint"
      @touchstart.capture="paint"
    >
      <div ref="el" />
    </div>
  </div>
</template>

<style scoped>
.editor-board {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.spare-row {
  display: grid;
  grid-template-columns: repeat(8, minmax(0, 1fr));
  gap: 4px;
  padding: 4px;
  border-radius: 10px;
  border: 1px solid var(--ui-border);
  background: var(--ui-bg-elevated);
}
.spare {
  display: grid;
  place-items: center;
  aspect-ratio: 1;
  max-height: 56px;
  width: 100%;
  border-radius: 8px;
  border: 0;
  background: transparent;
  color: var(--ui-text-muted);
  font-size: 20px;
  cursor: pointer;
  touch-action: none;
}
.spare:hover {
  background: var(--ui-bg-accented);
}
.spare.selected {
  background: color-mix(in srgb, var(--ui-primary) 30%, transparent);
  outline: 2px solid var(--ui-primary);
  outline-offset: -2px;
  color: var(--ui-text);
}
.spare-piece {
  width: 86%;
  height: 86%;
  /* A stable light square keeps both piece colors legible in every app theme. */
  background-color: #f0d9b5;
  border-radius: 5px;
  background-size: contain;
  background-repeat: no-repeat;
  background-position: center;
  pointer-events: none;
}
.cg-host.painting :deep(cg-board) {
  cursor: crosshair;
}
</style>
