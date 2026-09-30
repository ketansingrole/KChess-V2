<script setup lang="ts">
import { computed } from 'vue'
import type { Color, Key } from '@lichess-org/chessground/types'
import type { Dests } from '../utils/chess'
import type { CoordinateMode, PieceAnimation, PromotionMode } from '../../src/shared/types'
import type { PlayerInfo } from './PlayerLine.vue'

const props = withDefaults(
  defineProps<{
    fen: string
    orientation: Color
    theme: string
    coordinates: CoordinateMode
    pieceSet?: string
    animation?: PieceAnimation
    /** It is the player's turn. */
    movable: boolean
    /** Pieces may be touched (turn or premove); false when reviewing or the game is over. */
    interactive?: boolean
    movableColor: Color
    premove?: boolean
    showDests?: boolean
    promotion?: PromotionMode
    dests?: Dests
    lastMove?: Key[]
    check: Color | false
    turnColor: Color
    /** Player shown at the top of the board (the side facing away from `orientation`). */
    top: PlayerInfo
    /** Player shown at the bottom of the board (the side at `orientation`). */
    bottom: PlayerInfo
    /** Highlight the side to move; turn off once the game is over. */
    live?: boolean
  }>(),
  { live: true, interactive: true, premove: false, showDests: true, promotion: 'ask' },
)

const emit = defineEmits<{ move: [uci: string] }>()
const topColor = computed<Color>(() => (props.orientation === 'white' ? 'black' : 'white'))
</script>

<template>
  <div class="board-stack">
    <PlayerLine :player="top" :color="topColor" :active="live && turnColor === topColor">
      <template v-if="$slots['top-name']" #name><slot name="top-name" /></template>
      <template v-if="$slots['top-aside']" #aside><slot name="top-aside" /></template>
    </PlayerLine>
    <div class="board-shell">
      <ChessBoard
        :fen="fen"
        :orientation="orientation"
        :theme="theme"
        :coordinates="coordinates"
        :piece-set="pieceSet"
        :animation="animation"
        :movable="movable"
        :interactive="interactive"
        :movable-color="movableColor"
        :premove="premove"
        :show-dests="showDests"
        :promotion="promotion"
        :dests="dests"
        :last-move="lastMove"
        :check="check"
        :turn-color="turnColor"
        @move="emit('move', $event)"
      />
    </div>
    <PlayerLine :player="bottom" :color="orientation" :active="live && turnColor === orientation">
      <template v-if="$slots['bottom-name']" #name><slot name="bottom-name" /></template>
      <template v-if="$slots['bottom-aside']" #aside><slot name="bottom-aside" /></template>
    </PlayerLine>
  </div>
</template>
