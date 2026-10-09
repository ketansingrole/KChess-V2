<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import type { Puzzle } from '../../../../core/src/contracts/types'
import {
  nextSolutionSquares,
  opponentReply,
  playerColor,
  playerMove,
  startPuzzle,
  type PuzzleState,
} from '../../../../core/src/domain/puzzle'
import { checkColor, destsFor, positionFromFen } from '../../../../core/src/domain/chess'
import { play, playMoveSound } from '../utils/sound'

/**
 * One puzzle on a board: the player finds the moves, the board plays the opponent's replies. It
 * reports what happened and leaves everything else (scores, Lichess, the next puzzle) to its parent.
 */
const props = withDefaults(
  defineProps<{
    puzzle: Puzzle
    /** After a wrong move the puzzle stays open to try again (training); off, it ends (Storm, Streak, Rush). */
    retry?: boolean
    /** Nothing can be moved (between puzzles, or after a run ends). */
    frozen?: boolean
    flipped?: boolean
    /** Milliseconds before the opponent replies. */
    replyDelay?: number
    /** A training verdict retained by the parent when this board is remounted. */
    initialOutcome?: boolean | null
  }>(),
  { retry: true, replyDelay: 350 },
)

const emit = defineEmits<{
  /** Every attempted move, right or wrong. */
  move: [correct: boolean]
  /** The first verdict on the puzzle: solved cleanly, or not. Sent once. */
  outcome: [win: boolean]
  /** The puzzle is over: solved, or (without `retry`) failed. */
  done: []
}>()

const store = useKChessStore()
const { settings } = storeToRefs(store)

const state = ref<PuzzleState>(startPuzzle(props.puzzle))
const resetKey = ref(0)
const waiting = ref(false)
const mistakes = ref(props.initialOutcome === false ? 1 : 0)
const outcome = ref<boolean | null>(props.initialOutcome ?? null)
const revealed = ref(false)
const hinted = ref(false)
const feedback = ref<'' | 'good' | 'wrong'>('')
let timer: ReturnType<typeof setTimeout> | undefined
/** Bumped whenever the puzzle changes, so a late timer from the old one does nothing. */
let generation = 0

function begin(): void {
  generation++
  clearTimeout(timer)
  state.value = startPuzzle(props.puzzle)
  waiting.value = false
  mistakes.value = props.initialOutcome === false ? 1 : 0
  outcome.value = props.initialOutcome ?? null
  revealed.value = false
  hinted.value = false
  feedback.value = ''
  resetKey.value++
}
watch(() => props.puzzle, begin)
onBeforeUnmount(() => {
  generation++
  clearTimeout(timer)
})

const color = computed(() => playerColor(props.puzzle))
const orientation = computed(() =>
  props.flipped ? (color.value === 'white' ? 'black' : 'white') : color.value,
)
const position = computed(() => positionFromFen(state.value.fen))
const playing = computed(() => state.value.status === 'playing' && !waiting.value && !props.frozen)
const dests = computed(() =>
  playing.value && position.value ? destsFor(position.value) : undefined,
)
const check = computed(() => (position.value ? checkColor(position.value) : false))

const shapes = computed<DrawShape[]>(() => {
  const squares = nextSolutionSquares(props.puzzle, state.value)
  if (!squares || state.value.status !== 'playing') return []
  const [from, to] = squares as [Key, Key]
  if (revealed.value) return [{ orig: from, dest: to, brush: 'green' }]
  return hinted.value ? [{ orig: from, brush: 'yellow' }] : []
})

function decide(win: boolean): void {
  if (outcome.value !== null) return
  outcome.value = win
  emit('outcome', win)
}

function onMove(uci: string): void {
  if (!playing.value) return
  const result = playerMove(props.puzzle, state.value, uci)
  if (!result.correct) {
    mistakes.value++
    feedback.value = 'wrong'
    hinted.value = false
    resetKey.value++
    emit('move', false)
    decide(false)
    if (!props.retry) {
      state.value = { ...state.value, status: 'failed' }
      emit('done')
    }
    return
  }
  feedback.value = 'good'
  hinted.value = false
  state.value = result.state
  playMoveSound(result.state.sans.at(-1))
  emit('move', true)
  if (result.state.status === 'solved') {
    decide(mistakes.value === 0 && !revealed.value)
    void play('genericNotify')
    emit('done')
    return
  }
  if (result.reply) reply(result.reply)
}

function reply(uci: string): void {
  waiting.value = true
  const mine = generation
  timer = setTimeout(() => {
    if (mine !== generation) return
    state.value = opponentReply(state.value, uci)
    playMoveSound(state.value.sans.at(-1))
    waiting.value = false
  }, props.replyDelay)
}

/** Light up the piece to move. */
function hint(): void {
  if (state.value.status === 'playing') hinted.value = true
}

/** Play out the rest of the solution; the puzzle then counts as failed. */
async function showSolution(): Promise<void> {
  if (state.value.status !== 'playing' || revealed.value) return
  revealed.value = true
  decide(false)
  const mine = generation
  const pause = (ms: number): Promise<void> => new Promise((done) => setTimeout(done, ms))
  while (mine === generation && state.value.status === 'playing') {
    await pause(650)
    if (mine !== generation) return
    const uci = props.puzzle.solution[state.value.index]
    if (!uci) break
    const result = playerMove(props.puzzle, state.value, uci)
    if (!result.correct) break
    state.value = result.state
    playMoveSound(result.state.sans.at(-1))
    if (result.reply) {
      await pause(450)
      if (mine !== generation) return
      state.value = opponentReply(state.value, result.reply)
      playMoveSound(state.value.sans.at(-1))
    }
  }
  if (mine === generation) emit('done')
}

const status = computed(() => (revealed.value ? 'revealed' : state.value.status))
const lastMove = computed(() => state.value.lastMove as Key[] | undefined)

defineExpose({ hint, showSolution, status, mistakes, feedback, waiting, outcome })
</script>

<template>
  <div class="board-stack">
    <div class="board-shell puzzle-board" :class="feedback">
      <ChessBoard
        :fen="state.fen"
        :orientation="orientation"
        :theme="settings.boardTheme"
        :coordinates="settings.coordinates"
        :piece-set="settings.pieceSet"
        :animation="settings.pieceAnimation"
        :movable="playing"
        :interactive="playing"
        :movable-color="color"
        :premove="false"
        :show-dests="settings.showLegalMoves"
        :promotion="settings.promotion === 'premove' ? 'queen' : settings.promotion"
        :dests="dests"
        :last-move="lastMove"
        :check="check"
        :turn-color="position?.turn"
        :shapes="shapes"
        :reset-key="resetKey"
        @move="onMove"
      />
    </div>
  </div>
</template>
