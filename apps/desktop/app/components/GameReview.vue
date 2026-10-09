<script setup lang="ts">
import type { Color, Key } from '@lichess-org/chessground/types'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import type { CoordinateMode, PieceAnimation } from '@kchess/core/contracts/types'
import { INITIAL_FEN, uciSquares } from '@kchess/core/domain/position'
import { pgnMainline } from '@kchess/core/domain/pgn'

const props = defineProps<{
  pgn: string
  orientation?: Color
  theme?: string
  coordinates?: CoordinateMode
  pieceSet?: string
  animation?: PieceAnimation
  playerName?: string
  opponentName?: string
  /** The names are Lichess usernames, so they open the players' profiles. */
  linkNames?: boolean
}>()

type ReviewStep = { fen: string; check: boolean; lastMove?: Key[] }

const replay = computed(() => {
  const game = pgnMainline(props.pgn)
  if (!game)
    return { steps: [{ fen: INITIAL_FEN, check: false }] as ReviewStep[], moves: [] as string[] }
  const steps: ReviewStep[] = [{ fen: game.start ?? INITIAL_FEN, check: game.startCheck }]
  for (const move of game.moves) {
    steps.push({
      fen: move.fen,
      check: move.check,
      lastMove: uciSquares(move.uci) as Key[] | undefined,
    })
  }
  return { steps, moves: game.moves.map((move) => move.san) }
})

const ply = ref(0)
const flipped = ref(false)
watch(
  () => props.pgn,
  () => {
    ply.value = replay.value.moves.length
  },
  { immediate: true },
)
watch(
  () => props.orientation,
  () => {
    flipped.value = false
  },
)

const orientation = computed<Color>(() => {
  const color = props.orientation ?? 'white'
  return flipped.value ? (color === 'white' ? 'black' : 'white') : color
})
const step = computed(() => replay.value.steps[ply.value] ?? replay.value.steps.at(-1)!)
const topName = computed(() =>
  orientation.value === props.orientation ? props.opponentName : props.playerName,
)
const bottomName = computed(() =>
  orientation.value === props.orientation ? props.playerName : props.opponentName,
)
const turn = computed<Color>(() => (step.value.fen.split(' ')[1] === 'w' ? 'white' : 'black'))
const check = computed<Color | false>(() => (step.value.check ? turn.value : false))

function keydown(event: KeyboardEvent): void {
  if (
    event.target instanceof HTMLInputElement ||
    event.target instanceof HTMLSelectElement ||
    event.target instanceof HTMLTextAreaElement
  )
    return
  const max = replay.value.moves.length
  if (event.key === 'ArrowLeft') {
    ply.value = Math.max(0, ply.value - 1)
    event.preventDefault()
  } else if (event.key === 'ArrowRight') {
    ply.value = Math.min(max, ply.value + 1)
    event.preventDefault()
  } else if (event.key === 'ArrowUp') {
    ply.value = 0
    event.preventDefault()
  } else if (event.key === 'ArrowDown') {
    ply.value = max
    event.preventDefault()
  }
}
onMounted(() => window.addEventListener('keydown', keydown))
/** The move being looked at, so “Analyse” can open the game there. */
defineExpose({ ply })
onUnmounted(() => window.removeEventListener('keydown', keydown))
</script>

<template>
  <div class="play-layout review">
    <PlayBoard
      :fen="step.fen"
      :orientation="orientation"
      :theme="theme ?? 'brown'"
      :coordinates="coordinates ?? 'inside'"
      :piece-set="pieceSet"
      :animation="animation"
      :movable="false"
      :interactive="false"
      :movable-color="orientation"
      :last-move="step.lastMove"
      :check="check"
      :turn-color="turn"
      :live="false"
      :top="{
        name: topName ?? 'Opponent',
        username: linkNames ? topName : undefined,
        icon: 'i-lucide-user',
      }"
      :bottom="{
        name: bottomName ?? 'Player',
        username: linkNames ? bottomName : undefined,
        icon: 'i-lucide-user',
      }"
    />
    <MovePanel :moves="replay.moves" :ply="ply" @select="ply = $event" @flip="flipped = !flipped" />
  </div>
</template>
