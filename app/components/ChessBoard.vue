<script setup lang="ts">
import { parseSan } from 'chessops/san'
import { makeUci, parseUci } from 'chessops/util'
import { usePreferredReducedMotion } from '@vueuse/core'
import { Chessground } from '@lichess-org/chessground'
import type { Api } from '@lichess-org/chessground/api'
import type { Config } from '@lichess-org/chessground/config'
import type { Color, Key } from '@lichess-org/chessground/types'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { SquareName } from 'chessops/types'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { isPromotionMove, type Dests } from '../../src/shared/chess'
import type { CoordinateMode, PieceAnimation, PromotionMode } from '../../src/shared/types'
import { setupStart, type Variant } from '../../src/shared/variant'
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
  /** Arrows and circles the app draws (hints, targets); the player's own right-click drawings are separate. */
  shapes?: DrawShape[]
  /** Change it to put the pieces back where `fen` says (after a move the app rejected). */
  resetKey?: number
  /** Hide the pieces (blindfold play); highlights, arrows and the move input stay. */
  blindfold?: boolean
  /** Rules for typed moves; standard when omitted. */
  variant?: Variant
  /** Show the typed-move row elsewhere (a CSS selector), such as in the player's own line. */
  inputTo?: string
}>()

const emit = defineEmits<{ move: [uci: string]; select: [square: Key] }>()

const el = ref<HTMLElement | null>(null)
const reducedMotion = usePreferredReducedMotion()
const moveInput = ref('')
const typing = computed(
  () => props.interactive !== false && props.movable !== false && Boolean(props.dests?.size),
)
const moveMessage = ref('')
const keyboardInput = ref<HTMLInputElement | null>(null)
let restoreFocus: HTMLElement | null = null
function submitMove(uci: string): void {
  const started = performance.now()
  emit('move', uci)
  requestAnimationFrame(() => {
    void window.kchess
      ?.recordPerformance?.('board.frame', performance.now() - started)
      .catch((error: unknown) => {
        console.warn('[chess-board] Performance record failed:', error)
      })
  })
}
function keyboardMove(): void {
  const pos = setupStart({ variant: props.variant ?? 'standard', fen: props.fen })
  const text = moveInput.value.trim()
  const move = pos && (parseUci(text) ?? parseSan(pos, text))
  if (
    !pos ||
    !move ||
    !pos.isLegal(move) ||
    !props.dests
      ?.get(makeUci(move).slice(0, 2) as SquareName)
      ?.includes(makeUci(move).slice(2, 4) as SquareName)
  ) {
    moveMessage.value = 'Enter a legal move, such as Nf3 or g1f3.'
    return
  }
  const uci = makeUci(move)
  moveMessage.value = `Move submitted: ${text}`
  moveInput.value = ''
  if (uci.length === 4) put(uci.slice(0, 2) as Key, uci.slice(2, 4) as Key)
  else submitMove(uci)
}
let ground: Api | undefined
/** Chessground wipes drawn arrows whenever it is handed a FEN, so it only gets one when the position changed. */
let shownFen: string | undefined
let shownReset: number | undefined

type PromotionRole = 'q' | 'r' | 'b' | 'n'
const pending = ref<{ orig: Key; dest: Key } | null>(null)
const promoDialog = ref<HTMLElement | null>(null)
watch(pending, async (value) => {
  if (!value) {
    await nextTick()
    if (restoreFocus?.isConnected) restoreFocus.focus()
    restoreFocus = null
    return
  }
  restoreFocus =
    document.activeElement instanceof HTMLElement ? document.activeElement : keyboardInput.value
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
    submitMove(`${orig}${dest}`)
    return
  }
  const auto = props.promotion === 'queen' || (props.promotion === 'premove' && metadata?.premove)
  if (auto) submitMove(`${orig}${dest}q`)
  else pending.value = { orig, dest }
}

function promote(role: PromotionRole): void {
  const move = pending.value
  if (!move) return
  pending.value = null
  submitMove(`${move.orig}${move.dest}${role}`)
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
      enabled: reducedMotion.value !== 'reduce' && animationMs[props.animation ?? 'normal'] > 0,
      duration: reducedMotion.value === 'reduce' ? 0 : animationMs[props.animation ?? 'normal'],
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
    drawable: { enabled: true, visible: true, autoShapes: props.shapes ?? [] },
    events: { select: (square) => emit('select', square) },
  }
}

/** Chessground draws coordinate labels only when it rebuilds the board, not on `set`. */
let shownCoordinates = false
onMounted(() => {
  shownReset = props.resetKey
  shownCoordinates = props.coordinates !== 'none'
  if (el.value) ground = Chessground(el.value, configuration())
})

watch(
  () => ({ ...props, reducedMotion: reducedMotion.value }),
  () => {
    if (!ground) return
    if (props.resetKey !== shownReset) {
      shownReset = props.resetKey
      shownFen = undefined
    }
    ground.set(configuration())
    if ((props.coordinates !== 'none') !== shownCoordinates) {
      shownCoordinates = props.coordinates !== 'none'
      ground.redrawAll()
    }
    // A queued premove plays as soon as it is our turn; if nothing can be queued any more, drop it.
    if ((props.interactive ?? true) && props.movable) ground.playPremove()
    else if (!ground.state.premovable.enabled) ground.cancelPremove()
  },
)

/** A plain click on the board clears drawn arrows and circles, like on Lichess. */
function trapPromotion(event: KeyboardEvent): void {
  if (event.key !== 'Tab') return
  const buttons = promoDialog.value?.querySelectorAll<HTMLElement>('button')
  if (!buttons?.length) return
  const first = buttons[0]!,
    last = buttons[buttons.length - 1]!
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault()
    last.focus()
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault()
    first.focus()
  }
}
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
    :class="[
      `board-theme-${theme ?? 'brown'}`,
      `coords-${coordinates ?? 'inside'}`,
      { blindfold: blindfold },
    ]"
    :style="pieceStyle"
    @mousedown.capture="clearShapes"
  >
    <div ref="el" :inert="pending ? true : undefined" role="group" aria-label="Chess board" />
    <!-- The move box, and any controls the page adds (voice), share one row under the board. -->
    <Teleport v-if="typing || $slots.controls" defer :to="inputTo" :disabled="!inputTo">
      <div class="board-input-row" :class="{ inline: inputTo }">
        <form
          v-if="typing"
          class="board-move-form"
          :inert="pending ? true : undefined"
          @submit.prevent="keyboardMove"
        >
          <input
            ref="keyboardInput"
            v-model="moveInput"
            aria-label="Enter a chess move in SAN or UCI"
            placeholder="Type a move: Nf3 or g1f3"
            autocomplete="off"
            class="board-move-input"
          />
          <button
            type="submit"
            class="board-move-submit"
            aria-label="Play move"
            title="Play move (Enter)"
            :disabled="!moveInput.trim()"
          >
            ↵
          </button>
        </form>
        <slot name="controls" />
      </div>
    </Teleport>
    <p class="sr-only" aria-live="polite">
      {{ moveMessage }} ·
      {{
        (turnColor ?? (fen.split(' ')[1] === 'w' ? 'white' : 'black')) === 'white'
          ? 'White'
          : 'Black'
      }}
      to move
    </p>
    <div v-if="pending" class="promo-backdrop" @click.self="cancelPromotion">
      <div
        ref="promoDialog"
        class="promo-choices"
        role="dialog"
        aria-modal="true"
        aria-label="Choose promotion piece"
        @keydown.esc.stop="cancelPromotion"
        @keydown="trapPromotion"
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
