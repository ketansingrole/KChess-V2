<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import type { RunSummary } from '@kchess/contracts/types'
import {
  lessonHint,
  lessonMove,
  lessonStatus,
  type LessonCourse,
  type LessonReason,
  type LessonStep,
} from '@kchess/rules/lessons'
import { sanFrom } from '@kchess/rules/endgames'
import { checkColor, destsFor, positionFromFen } from '@kchess/rules/chess'
import { moveSquares } from '@kchess/rules/puzzle'
import { playMoveSound } from '../utils/sound'
import { useZenSurface } from '../composables/useZenSurface'
import { useEngineMove } from '../composables/useEngineMove'
import BoardViewToggles from './BoardViewToggles.vue'
import TrainingList from './TrainingList.vue'

/**
 * One course: its chapters, each a few steps to read, follow or solve. Finished chapters are saved
 * as `lesson` runs in the core.
 */
const props = defineProps<{ course: LessonCourse; summary: RunSummary | null }>()
const emit = defineEmits<{ back: []; saved: [summary: RunSummary] }>()
const store = useKChessStore()
const { settings, engineReady } = storeToRefs(store)
useZenSurface(true)
const { thinking, error: engineError, ask, cancel } = useEngineMove('endgame-lesson')

const done = (id: string): boolean => (props.summary?.best[id]?.score ?? 0) > 0
const firstChapter = props.course.chapters[0]
const firstStep = firstChapter?.steps[0]
if (!firstChapter || !firstStep) throw new Error(`Course ${props.course.id} has no steps.`)
const chapterId = ref((props.course.chapters.find((c) => !done(c.id)) ?? firstChapter).id)
const stepIndex = ref(0)
const moves = ref<string[]>([])
const hint = ref<string | undefined>()
const wrong = ref(false)
const flipped = ref(false)
const resetKey = ref(0)
let replyTimer: ReturnType<typeof setTimeout> | undefined
let saved = ''

const chapter = computed(
  () => props.course.chapters.find((c) => c.id === chapterId.value) ?? firstChapter,
)
const step = computed<LessonStep>(() => chapter.value.steps[stepIndex.value] ?? firstStep)
const lastStep = computed(() => stepIndex.value === chapter.value.steps.length - 1)
const chapterIndex = computed(() => props.course.chapters.indexOf(chapter.value))
const status = computed(() => lessonStatus(step.value.fen, step.value.task, moves.value))
const position = computed(() => positionFromFen(status.value.fen))
const playable = computed(() => step.value.task.kind !== 'read')
const yourTurn = computed(
  () => playable.value && !status.value.over && status.value.turn === status.value.player,
)
const side = computed(() => (status.value.player === 'white' ? 'White' : 'Black'))
const other = computed(() => (status.value.player === 'white' ? 'Black' : 'White'))
const orientation = computed(() =>
  flipped.value ? (status.value.player === 'white' ? 'black' : 'white') : status.value.player,
)
const paragraphs = computed(() => step.value.text.split(/\n{2,}/))
const lastMove = computed(() => {
  const last = moves.value.at(-1)
  if (!last) return undefined
  const before = lessonStatus(step.value.fen, step.value.task, moves.value.slice(0, -1))
  return moveSquares(before.fen, last) as Key[] | undefined
})
const shapes = computed<DrawShape[]>(() => {
  const drawn: DrawShape[] = (step.value.shapes ?? []).map((shape) => ({
    orig: shape.orig as Key,
    dest: shape.dest as Key | undefined,
    brush: shape.brush,
  }))
  const squares = hint.value ? moveSquares(status.value.fen, hint.value) : undefined
  if (squares) drawn.push({ orig: squares[0] as Key, dest: squares[1] as Key, brush: 'blue' })
  return drawn
})
const chapters = computed(() => [
  {
    items: props.course.chapters.map((c) => ({ id: c.id, title: c.title, done: done(c.id) })),
  },
])

/** What the step asks, while it is yours to play. */
const goal = computed(() => {
  const task = step.value.task
  if (task.kind === 'line') return `Find the best move for ${side.value}.`
  if (task.kind === 'mate')
    return task.within === 1 ? 'Checkmate in one move.' : `Checkmate in ${task.within} moves.`
  if (task.kind !== 'play') return ''
  const moves = `${task.within} ${task.within === 1 ? 'move' : 'moves'}`
  switch (task.goal) {
    case 'mate':
      return `Checkmate the engine within ${moves}.`
    case 'win':
      return `Win against the engine within ${moves}.`
    case 'draw':
      return `Hold the draw for ${moves}.`
    case 'reach':
      return `Bring your king to a marked square within ${moves}.`
    case 'confine':
      return `Box the king into the marked area within ${moves}.`
  }
  return ''
})

const REASONS: Record<LessonReason, string> = {
  read: '',
  lineDone: 'Well done',
  checkmate: 'Checkmate!',
  mated: 'You were checkmated',
  stalemate: 'Stalemate',
  insufficient: 'Insufficient material',
  repetition: 'Threefold repetition',
  fiftyMoves: 'Fifty-move rule',
  promoted: 'Promoted — the rest is technique',
  bareKing: 'Only a bare king is left',
  reached: 'Key square reached',
  confined: 'The king is boxed in',
  held: 'Draw held',
  outOfMoves: 'Out of moves',
  materialLost: 'Material lost',
  lost: 'The position slipped away',
}
const outcome = computed(() => {
  const s = status.value
  if (!s.over || !s.reason || s.reason === 'read') return ''
  const title = REASONS[s.reason]
  return s.success && ['stalemate', 'insufficient', 'repetition', 'fiftyMoves'].includes(s.reason)
    ? `Draw held — ${title.toLowerCase()}`
    : title
})

function clearReply(): void {
  if (replyTimer) clearTimeout(replyTimer)
  replyTimer = undefined
}

function openStep(index: number, id = chapterId.value): void {
  cancel()
  clearReply()
  chapterId.value = id
  stepIndex.value = index
  moves.value = []
  hint.value = undefined
  wrong.value = false
  flipped.value = false
  resetKey.value++
}

function openChapter(id: string): void {
  openStep(0, id)
}

function next(): void {
  if (!lastStep.value) return openStep(stepIndex.value + 1)
  const following = props.course.chapters[chapterIndex.value + 1]
  if (following) openChapter(following.id)
}

function play(uci: string): void {
  moves.value = [...moves.value, uci]
  playMoveSound(sanFrom(step.value.fen, moves.value).at(-1))
}

function onMove(uci: string): void {
  if (!yourTurn.value) return
  hint.value = undefined
  const verdict = lessonMove(step.value.fen, step.value.task, moves.value, uci)
  if (!verdict.accepted || !verdict.uci) {
    wrong.value = true
    resetKey.value++
    return
  }
  wrong.value = false
  play(verdict.uci)
}

/** The opponent answers: a scripted or solved reply after a beat, else the engine. */
async function respond(): Promise<void> {
  const s = status.value
  if (s.over || s.turn === s.player || !playable.value || thinking.value || replyTimer) return
  if (s.reply) {
    const reply = s.reply
    const at = moves.value.length
    replyTimer = setTimeout(() => {
      replyTimer = undefined
      if (moves.value.length === at) play(reply)
    }, 450)
    return
  }
  if (!s.engine) return
  const reply = await ask(step.value.fen, moves.value, 350)
  if (reply) play(reply)
}

async function showHint(): Promise<void> {
  if (!yourTurn.value || thinking.value) return
  const known = lessonHint(step.value.fen, step.value.task, moves.value)
  if (known) {
    hint.value = known
    return
  }
  const best = await ask(step.value.fen, moves.value, 600, { busy: false })
  if (best && yourTurn.value) hint.value = best
}

function takeback(): void {
  if (thinking.value) return
  cancel()
  clearReply()
  hint.value = undefined
  wrong.value = false
  const kept = moves.value.slice(0, -1)
  while (
    kept.length &&
    lessonStatus(step.value.fen, step.value.task, kept).turn !== status.value.player
  )
    kept.pop()
  moves.value = kept
}

watch(moves, () => void respond())
watch(
  () => [status.value.over, status.value.success] as const,
  async ([over, success]) => {
    if (!over || !success || !lastStep.value) return
    const id = chapter.value.id
    if (saved === id) return
    saved = id
    try {
      const result = await window.kchess.saveRun({
        kind: 'lesson',
        variant: id,
        score: 1,
        detail: { course: props.course.id, steps: chapter.value.steps.length },
      })
      emit('saved', result.summary)
    } catch (cause) {
      console.warn('[endgame-lesson] Could not save progress:', cause)
      saved = ''
    }
  },
  { immediate: true },
)
watch(chapterId, () => (saved = ''))
onBeforeUnmount(() => {
  clearReply()
})
</script>

<template>
  <div v-board-fit class="play-layout lesson-layout">
    <nav class="card lesson-chapters" aria-label="Chapters">
      <div class="flex items-center gap-1">
        <UButton
          variant="ghost"
          color="neutral"
          size="sm"
          icon="i-lucide-chevron-left"
          aria-label="All courses"
          @click="emit('back')"
        />
        <h2 class="section-title truncate">{{ course.title }}</h2>
      </div>
      <TrainingList :groups="chapters" :active-id="chapter.id" @select="openChapter" />
    </nav>

    <div class="board-stack">
      <div class="board-shell">
        <ChessBoard
          keep-move-input
          :fen="status.fen"
          :orientation="orientation"
          :theme="settings.boardTheme"
          :coordinates="settings.coordinates"
          :piece-set="settings.pieceSet"
          :animation="settings.pieceAnimation"
          :movable="yourTurn"
          :interactive="yourTurn"
          :movable-color="status.player"
          :premove="false"
          :show-dests="settings.showLegalMoves"
          :promotion="settings.promotion === 'queen' ? 'queen' : 'ask'"
          :dests="yourTurn && position ? destsFor(position) : undefined"
          :last-move="lastMove"
          :check="position ? checkColor(position) : false"
          :turn-color="status.turn"
          :shapes="shapes"
          :reset-key="resetKey"
          @move="onMove"
        />
      </div>
    </div>

    <div class="card side-panel puzzle-panel">
      <div class="flex items-start justify-between gap-2">
        <div class="min-w-0">
          <h2 class="section-title">{{ chapter.title }}</h2>
          <p v-if="chapter.steps.length > 1" class="muted text-xs tabular">
            {{ stepIndex + 1 }} / {{ chapter.steps.length }}
          </p>
        </div>
        <div class="flex gap-1"><BoardViewToggles /></div>
      </div>

      <div class="lesson-text">
        <p v-for="(paragraph, index) in paragraphs" :key="index">{{ paragraph }}</p>
      </div>

      <div
        v-if="playable"
        class="status-banner steady"
        :class="
          status.over
            ? status.success
              ? 'win'
              : 'loss'
            : wrong || (status.engine && !engineReady)
              ? 'alert'
              : ''
        "
        role="status"
      >
        <UIcon
          :name="
            status.over
              ? status.success
                ? 'i-lucide-trophy'
                : 'i-lucide-flag'
              : thinking
                ? 'i-lucide-loader-circle'
                : wrong
                  ? 'i-lucide-x'
                  : 'i-lucide-mouse-pointer-click'
          "
          class="status-icon"
          :class="{ 'animate-spin': thinking }"
        />
        <div>
          <template v-if="status.over">
            <strong>{{ outcome }}</strong>
          </template>
          <template v-else-if="status.engine && !engineReady">
            <strong>Stockfish not found</strong>
            <span class="detail">Install or choose an engine in Settings to play this out.</span>
          </template>
          <template v-else-if="yourTurn">
            <strong>{{ wrong ? 'Not the move — try again' : 'Your turn' }}</strong>
            <span class="detail">{{ goal }}</span>
          </template>
          <template v-else>
            <strong>{{ thinking ? 'Stockfish is thinking…' : `${other} to move` }}</strong>
          </template>
        </div>
      </div>
      <p v-if="engineError" class="text-error text-xs" role="alert">{{ engineError }}</p>

      <div class="panel-actions">
        <template v-if="playable">
          <UButton
            variant="outline"
            color="neutral"
            icon="i-lucide-lightbulb"
            :disabled="!yourTurn || thinking"
            @click="showHint"
            >Hint</UButton
          >
          <UButton
            v-if="step.task.kind === 'play'"
            variant="outline"
            color="neutral"
            icon="i-lucide-undo-2"
            :disabled="!moves.length || thinking"
            @click="takeback"
            >Take back</UButton
          >
          <UButton
            variant="outline"
            color="neutral"
            icon="i-lucide-arrow-up-down"
            aria-label="Flip board"
            @click="flipped = !flipped"
          />
          <UButton
            :variant="status.over && !status.success ? 'solid' : 'outline'"
            :color="status.over && !status.success ? 'primary' : 'neutral'"
            icon="i-lucide-rotate-ccw"
            :disabled="!moves.length"
            @click="openStep(stepIndex)"
            >Retry</UButton
          >
        </template>
        <UButton
          v-if="stepIndex > 0"
          variant="outline"
          color="neutral"
          icon="i-lucide-chevron-left"
          aria-label="Previous step"
          @click="openStep(stepIndex - 1)"
        />
        <UButton
          v-if="!lastStep || chapterIndex < course.chapters.length - 1"
          :variant="status.over && status.success ? 'solid' : 'outline'"
          :color="status.over && status.success ? 'primary' : 'neutral'"
          trailing-icon="i-lucide-chevron-right"
          @click="next"
          >{{ lastStep ? 'Next chapter' : 'Next' }}</UButton
        >
      </div>
      <p v-if="moves.length" class="muted text-xs tabular">
        {{ sanFrom(step.fen, moves).join(' ') }}
      </p>
    </div>
  </div>
</template>

<style scoped>
.play-layout.lesson-layout {
  --chapters-width: 250px;
  --board-size: clamp(
    260px,
    min(100dvh - var(--board-chrome), 100cqw - 368px - var(--chapters-width)),
    var(--board-max)
  );
  grid-template-columns: var(--chapters-width) var(--board-size) minmax(300px, var(--panel-max));
}
.lesson-chapters {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
  max-height: var(--board-size);
  overflow-y: auto;
  padding: 12px;
}
.lesson-text {
  display: flex;
  flex-direction: column;
  gap: 10px;
  font-size: 14px;
  line-height: 1.5;
}
@container page (max-width: 860px) {
  .play-layout.lesson-layout {
    --chapters-width: 0px;
    grid-template-columns: var(--board-size) minmax(300px, var(--panel-max));
  }
  .lesson-chapters {
    grid-column: 1 / -1;
    grid-row: 2;
    max-height: none;
  }
}
</style>
