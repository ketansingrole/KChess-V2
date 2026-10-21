<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useId, watch } from 'vue'
import { storeToRefs } from 'pinia'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import type { Puzzle } from '@kchess/contracts/types'
import {
  nextSolutionSquares,
  opponentReply,
  playerColor,
  playerMove,
  startPuzzle,
  type PuzzleState,
} from '@kchess/rules/puzzle'
import { checkColor, destsFor, positionFromFen } from '@kchess/rules/chess'
import type { PuzzleVerdictDetail } from '@kchess/rules/puzzleSession'
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
    /** Show each side's captured material above and below the board (off for timed runs). */
    material?: boolean
  }>(),
  // Without a default, Vue casts an absent `initialOutcome` to false: an attempt already failed.
  { retry: true, replyDelay: 350, initialOutcome: null, material: false },
)

const emit = defineEmits<{
  /**
   * Every attempted move, right or wrong, its number among the player's moves of the solution,
   * and the move itself (UCI).
   */
  move: [correct: boolean, index: number, uci: string]
  /** The first verdict on the puzzle: solved cleanly, or not, and how far it got. Sent once. */
  outcome: [win: boolean, detail: PuzzleVerdictDetail]
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
/** A hint was shown at some point: `hinted` itself clears after each move. */
const hintUsed = ref(false)
const feedback = ref<'' | 'good' | 'wrong'>('')
/** The board's red outline: a brief flash on a wrong move, while the panel keeps the message. */
const flash = ref(false)
let flashTimer: ReturnType<typeof setTimeout> | undefined
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
  hintUsed.value = false
  feedback.value = ''
  flash.value = false
  clearTimeout(flashTimer)
  resetKey.value++
}
watch(() => props.puzzle, begin)
onBeforeUnmount(() => {
  generation++
  clearTimeout(timer)
  clearTimeout(flashTimer)
})

const color = computed(() => playerColor(props.puzzle))
const orientation = computed(() =>
  props.flipped ? (color.value === 'white' ? 'black' : 'white') : color.value,
)
/** The move box joins the bottom material row, as in a game's bottom player line. */
const entry = useId()
/** Where the page puts controls that outlive this board, such as voice input (`PuzzleVoice`). */
const controls = ref<HTMLElement | null>(null)
const topColor = computed(() => (orientation.value === 'white' ? 'black' : 'white'))
const position = computed(() => positionFromFen(state.value.fen))
/** The player may move: not while the opponent replies or the solution plays itself out. */
const playing = computed(
  () => state.value.status === 'playing' && !waiting.value && !revealed.value && !props.frozen,
)
const dests = computed(() =>
  playing.value && position.value ? destsFor(position.value) : undefined,
)
const check = computed(() => (position.value ? checkColor(position.value) : false))

/*
 * The hint: a thick green ring on a soft-white halo, over a faint green tint, under the piece.
 * Chessground's own circle is a thin fixed-width stroke that disappears on busy or green boards;
 * this scales with the square (a 100×100 box) and reads on every board theme.
 */
const HINT_MARK = `<g class="cg-hint">
  <circle cx="50" cy="50" r="42" fill="#16a34a" fill-opacity="0.22" stroke="#fff" stroke-opacity="0.8" stroke-width="14"/>
  <circle cx="50" cy="50" r="42" fill="none" stroke="#16a34a" stroke-width="9"/>
</g>`

const shapes = computed<DrawShape[]>(() => {
  const squares = nextSolutionSquares(props.puzzle, state.value)
  if (!squares || state.value.status !== 'playing') return []
  const [from, to] = squares as [Key, Key]
  if (revealed.value) return [{ orig: from, dest: to, brush: 'green' }]
  return hinted.value ? [{ orig: from, customSvg: { html: HINT_MARK }, below: true }] : []
})

function decide(win: boolean): void {
  if (outcome.value !== null) return
  outcome.value = win
  emit('outcome', win, {
    revealed: revealed.value,
    hinted: hintUsed.value,
    total: Math.ceil(props.puzzle.solution.length / 2),
  })
}

function onMove(uci: string): void {
  if (!playing.value) return
  const index = Math.floor(state.value.index / 2)
  const result = playerMove(props.puzzle, state.value, uci)
  if (!result.correct) {
    mistakes.value++
    feedback.value = 'wrong'
    flash.value = true
    clearTimeout(flashTimer)
    flashTimer = setTimeout(() => (flash.value = false), 900)
    hinted.value = false
    resetKey.value++
    emit('move', false, index, uci)
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
  emit('move', true, index, uci)
  if (result.state.status === 'solved') {
    decide(mistakes.value === 0 && !revealed.value)
    void play('genericNotify')
    emit('done')
    return
  }
  if (result.reply) reply(result.reply)
}

/** A move made off the board (spoken); false when the board takes no move now. */
function playMove(uci: string): boolean {
  if (!playing.value) return false
  onMove(uci)
  return true
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

/** Light up the piece to move, or hide it again. */
function hint(): void {
  if (state.value.status !== 'playing') return
  hinted.value = !hinted.value
  if (hinted.value) hintUsed.value = true
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

defineExpose({
  hint,
  hinted,
  showSolution,
  playMove,
  status,
  mistakes,
  feedback,
  waiting,
  outcome,
  position,
  playing,
  controls,
})
</script>

<template>
  <div class="board-stack" :class="{ 'with-material': material }">
    <div v-if="material" class="puzzle-material" data-side="top">
      <MaterialDiff :fen="state.fen" :color="topColor" />
    </div>
    <div class="board-shell puzzle-board" :class="{ wrong: flash }">
      <ChessBoard
        keep-move-input
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
        :promotion="settings.promotion === 'queen' ? 'queen' : 'ask'"
        :dests="dests"
        :last-move="lastMove"
        :check="check"
        :turn-color="position?.turn"
        :shapes="shapes"
        :reset-key="resetKey"
        :input-to="material ? `[data-move-entry='${entry}']` : undefined"
        @move="onMove"
      />
    </div>
    <div v-if="material" class="player-line puzzle-material" data-side="bottom">
      <MaterialDiff :fen="state.fen" :color="orientation" />
      <div class="player-middle"><div :data-move-entry="entry" class="move-entry" /></div>
      <div ref="controls" class="puzzle-controls" />
    </div>
  </div>
</template>

<style scoped>
.with-material {
  gap: 4px;
}
/* A fixed height, so the board does not shift when a capture adds or removes the row's content. */
.puzzle-material[data-side='top'] {
  display: flex;
  align-items: center;
  height: 18px;
}
.puzzle-controls {
  display: flex;
  min-width: 0;
}
.puzzle-controls:empty {
  display: none;
}
</style>
