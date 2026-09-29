<script setup lang="ts">
import { Chessground } from '@lichess-org/chessground'
import type { Api } from '@lichess-org/chessground/api'
import type { Config } from '@lichess-org/chessground/config'
import type { Color, Key } from '@lichess-org/chessground/types'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { isPromotionMove, type Dests } from '../utils/chess'
import type { CoordinateMode, PieceAnimation, PromotionMode } from '../../src/shared/types'
import { animationMs, DEFAULT_PIECE_SET, pieceVars } from '../utils/pieces'

const props = defineProps<{
  fen: string
  orientation?: Color
  /** It is the player's turn: `dests` are playable now. */
  movable?: boolean
  /** Pieces may be touched at all (false while reviewing, or once the game is over). */
  interactive?: boolean
  movableColor?: Color | 'both'
  /** Allow queuing one move while it is the opponent's turn. */
  premove?: boolean
  /** Dot the squares a selected piece can reach. */
  showDests?: boolean
  promotion?: PromotionMode
  dests?: Dests
  lastMove?: Key[]
  check?: Color | false
  turnColor?: Color
  theme?: string
  coordinates?: CoordinateMode
  pieceSet?: string
  animation?: PieceAnimation
}>()

const emit = defineEmits<{ move: [uci: string] }>()

const el = ref<HTMLElement | null>(null)
let ground: Api | undefined
/** Chessground wipes drawn arrows whenever it is handed a FEN, so it only gets one when the position changed. */
let shownFen: string | undefined

type PromotionRole = 'q' | 'r' | 'b' | 'n'
const pending = ref<{ orig: Key; dest: Key } | null>(null)
const promoDialog = ref<HTMLElement | null>(null)
watch(pending, async (value) => {
  if (!value) return
  await nextTick()
  promoDialog.value?.querySelector<HTMLElement>('button')?.focus()
})
const pieceStyle = computed(() => pieceVars(props.pieceSet ?? DEFAULT_PIECE_SET))
const promotionChoices = computed<{ role: PromotionRole; label: string; image: string }[]>(() => {
  const color = props.movableColor === 'black' ? 'b' : 'w'
  return [
    { role: 'q', label: 'Queen', image: `var(--piece-${color}Q)` },
    { role: 'r', label: 'Rook', image: `var(--piece-${color}R)` },
    { role: 'b', label: 'Bishop', image: `var(--piece-${color}B)` },
    { role: 'n', label: 'Knight', image: `var(--piece-${color}N)` },
  ]
})

function put(orig: Key, dest: Key, metadata?: { premove?: boolean }): void {
  if (!isPromotionMove(props.fen, orig, dest)) {
    emit('move', `${orig}${dest}`)
    return
  }
  const auto = props.promotion === 'queen' || (props.promotion === 'premove' && metadata?.premove)
  if (auto) emit('move', `${orig}${dest}q`)
  else pending.value = { orig, dest }
}

function promote(role: PromotionRole): void {
  const move = pending.value
  if (!move) return
  pending.value = null
  emit('move', `${move.orig}${move.dest}${role}`)
}

/** Dismissing the picker puts the pawn back where the position says it is. */
function cancelPromotion(): void {
  pending.value = null
  shownFen = undefined
  ground?.set(configuration())
}

function configuration(): Config {
  const interactive = props.interactive ?? true
  const movable = interactive && (props.movable ?? true)
  const fen = props.fen === shownFen ? undefined : props.fen
  shownFen = props.fen
  return {
    fen,
    orientation: props.orientation ?? 'white',
    turnColor: props.turnColor,
    check: props.check ?? false,
    lastMove: props.lastMove,
    coordinates: props.coordinates !== 'none',
    highlight: { lastMove: true, check: true },
    animation: {
      enabled: animationMs[props.animation ?? 'normal'] > 0,
      duration: animationMs[props.animation ?? 'normal'],
    },
    disableContextMenu: true,
    // Never view-only: Chessground ignores drawing and premoves entirely in that mode.
    viewOnly: false,
    movable: {
      free: false,
      color: interactive ? (props.movableColor ?? 'both') : undefined,
      dests: movable ? (props.dests ?? new Map<Key, Key[]>()) : new Map<Key, Key[]>(),
      showDests: props.showDests ?? true,
      rookCastle: true,
      events: { after: put },
    },
    premovable: {
      enabled: Boolean(props.premove) && interactive && !movable,
      showDests: props.showDests ?? true,
      castle: true,
    },
    draggable: { enabled: true, showGhost: true },
    selectable: { enabled: true },
    drawable: { enabled: true, visible: true },
  }
}

onMounted(() => {
  if (el.value) ground = Chessground(el.value, configuration())
})

watch(
  () => ({ ...props }),
  () => {
    if (!ground) return
    ground.set(configuration())
    // A queued premove plays as soon as it is our turn; if nothing can be queued any more, drop it.
    if ((props.interactive ?? true) && props.movable) ground.playPremove()
    else if (!ground.state.premovable.enabled) ground.cancelPremove()
  },
)

/** A plain click on the board clears drawn arrows and circles, like on Lichess. */
function clearShapes(event: MouseEvent): void {
  if (event.button === 0 && !event.shiftKey && ground?.state.drawable.shapes.length)
    ground.setShapes([])
}

onBeforeUnmount(() => {
  ground?.destroy()
  ground = undefined
})
</script>

<template>
  <div
    class="cg-host"
    :class="[`board-theme-${theme ?? 'brown'}`, `coords-${coordinates ?? 'inside'}`]"
    :style="pieceStyle"
    @mousedown.capture="clearShapes"
  >
    <div ref="el" />
    <div v-if="pending" class="promo-backdrop" @click.self="cancelPromotion">
      <div
        ref="promoDialog"
        class="promo-choices"
        role="dialog"
        aria-modal="true"
        aria-label="Choose promotion piece"
        @keydown.esc.stop="cancelPromotion"
      >
        <button
          v-for="choice in promotionChoices"
          :key="choice.role"
          type="button"
          class="promo-choice"
          :aria-label="choice.label"
          :title="choice.label"
          @click="promote(choice.role)"
        >
          <span class="promo-piece" :style="{ backgroundImage: choice.image }" />
        </button>
      </div>
    </div>
  </div>
</template>
