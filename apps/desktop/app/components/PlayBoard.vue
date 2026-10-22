<script setup lang="ts">
import { computed, useId } from 'vue'
import type { Color, Key } from '@lichess-org/chessground/types'
import type { Dests } from '@kchess/rules/chess'
import type { PromotionMode } from '@kchess/contracts/types'
import type { PlayerInfo } from './PlayerLine.vue'
import type { Variant } from '@kchess/rules/variant'
import { opposite } from '@kchess/rules/position'

const props = withDefaults(
  defineProps<{
    fen: string
    orientation: Color
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
    blindfold?: boolean
    variant?: Variant
    /** Change it to put the pieces back after a move the app rejected. */
    resetKey?: number
    /** A game played here: the typed-move box stays between moves (see ChessBoard). */
    keepMoveInput?: boolean
  }>(),
  { live: true, interactive: true, premove: false, showDests: true, promotion: 'ask' },
)

const emit = defineEmits<{ move: [uci: string] }>()
/** The typed-move box sits in the bottom player's line rather than on a row of its own. */
const entry = useId()
const topColor = computed<Color>(() => opposite(props.orientation))
/** Hidden while blindfolded: counting the pieces would give the position away. */
const materialFen = computed(() => (props.blindfold ? undefined : props.fen))
</script>

<template>
  <div class="board-stack">
    <PlayerLine
      :player="top"
      :color="topColor"
      :active="live && turnColor === topColor"
      :material-fen="materialFen"
    >
      <template v-if="$slots['top-name']" #name><slot name="top-name" /></template>
      <template v-if="$slots['top-aside']" #aside><slot name="top-aside" /></template>
      <template v-if="$slots['top-presence']" #presence><slot name="top-presence" /></template>
    </PlayerLine>
    <div class="board-shell">
      <ChessBoard
        :fen="fen"
        :orientation="orientation"
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
        :blindfold="blindfold"
        :variant="variant"
        :reset-key="resetKey"
        :input-to="`[data-move-entry='${entry}']`"
        :keep-move-input="keepMoveInput"
        @move="emit('move', $event)"
      />
    </div>
    <PlayerLine
      :player="bottom"
      :color="orientation"
      :active="live && turnColor === orientation"
      :material-fen="materialFen"
    >
      <template v-if="$slots['bottom-name']" #name><slot name="bottom-name" /></template>
      <template v-if="$slots['bottom-aside']" #aside><slot name="bottom-aside" /></template>
      <template v-if="$slots['bottom-presence']" #presence
        ><slot name="bottom-presence"
      /></template>
      <template #middle><div :data-move-entry="entry" class="move-entry" /></template>
    </PlayerLine>
  </div>
</template>
