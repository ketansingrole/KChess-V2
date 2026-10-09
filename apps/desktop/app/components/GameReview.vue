<script setup lang="ts">
import type { Color, Key } from '@lichess-org/chessground/types'
import { Chess, type Position } from 'chessops/chess'
import { makeFen } from 'chessops/fen'
import { parsePgn, startingPosition } from 'chessops/pgn'
import { parseSan } from 'chessops/san'
import { makeSquare } from 'chessops/util'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import type { CoordinateMode, PieceAnimation } from '../../../../core/src/contracts/types'

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

type ReviewStep = { position: Position; lastMove?: Key[] }

const replay = computed(() => {
  const game = parsePgn(props.pgn)[0]
  const starting = game && startingPosition(game.headers)
  const position = starting && starting.isOk ? starting.value : Chess.default()
  const steps: ReviewStep[] = [{ position: position.clone() }]
  const moves: string[] = []

  if (game) {
    for (const node of game.moves.mainlineNodes()) {
      const move = parseSan(position, node.data.san)
      if (!move || !position.isLegal(move)) break
      moves.push(node.data.san)
      const lastMove =
        'from' in move ? ([makeSquare(move.from), makeSquare(move.to)] as Key[]) : undefined
      position.play(move)
      steps.push({ position: position.clone(), lastMove })
    }
  }
  return { steps, moves }
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
const check = computed<Color | false>(() =>
  step.value.position.isCheck() ? step.value.position.turn : false,
)

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
      :fen="makeFen(step.position.toSetup())"
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
      :turn-color="step.position.turn"
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
