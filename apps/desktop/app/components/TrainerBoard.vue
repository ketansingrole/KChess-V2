<script setup lang="ts">
import { computed } from 'vue'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Color, Key } from '@lichess-org/chessground/types'
import { checkColor, destsFor, positionFromFen } from '@kchess/rules/chess'

/**
 * The board of a drill played against the app (endgames, lessons, opening repertoires): the player
 * moves `color` while `active`, with their legal-move, promotion and blindfold settings. Nothing
 * is premoved here, so queening only premoves asks, as on puzzle boards.
 */
const props = defineProps<{
  fen: string
  orientation: Color
  /** The side the player moves. */
  color: Color
  /** The player's turn: moves are taken now. */
  active: boolean
  lastMove?: Key[]
  shapes?: DrawShape[]
  resetKey?: number
  /** Show the pieces while blindfolded, such as once the drill is over. */
  reveal?: boolean
}>()
const emit = defineEmits<{ move: [uci: string] }>()
const { settings } = storeToRefs(useKChessStore())
const position = computed(() => positionFromFen(props.fen))
</script>

<template>
  <ChessBoard
    keep-move-input
    :fen="fen"
    :orientation="orientation"
    :movable="active"
    :interactive="active"
    :movable-color="color"
    :show-dests="settings?.showLegalMoves"
    :promotion="settings?.promotion === 'queen' ? 'queen' : 'ask'"
    :dests="active && position ? destsFor(position) : undefined"
    :last-move="lastMove"
    :check="position ? checkColor(position) : false"
    :turn-color="position?.turn"
    :shapes="shapes"
    :reset-key="resetKey"
    :blindfold="Boolean(settings?.blindfold) && !reveal"
    @move="emit('move', $event)"
  />
</template>
