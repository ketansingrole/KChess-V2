<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import type { RunSummary } from '@kchess/contracts/types'
import { ENDGAME_DRILLS, evaluateEndgame, sanFrom, type EndgameDrill } from '@kchess/rules/endgames'
import { checkColor, destsFor, positionFromFen } from '@kchess/rules/chess'
import { moveSquares } from '@kchess/rules/puzzle'
import { playMoveSound } from '../utils/sound'
import { useZenSurface } from '../composables/useZenSurface'
import { useEngineMove } from '../composables/useEngineMove'
import BoardViewToggles from './BoardViewToggles.vue'
import TrainingList from './TrainingList.vue'

/**
 * Endgames you must convert (or hold) against Stockfish at full strength, from positions checked to be
 * won or drawn. The result is kept on this computer only.
 */
const emit = defineEmits<{ back: [] }>()
const store = useKChessStore()
const { settings, engineReady } = storeToRefs(store)
useZenSurface(true)
const { thinking, error: engineError, ask, cancel } = useEngineMove('endgame-drills')

const firstDrill = ENDGAME_DRILLS[0]
if (!firstDrill) throw new Error('No endgame drills available.')
const drill = ref<EndgameDrill>(firstDrill)
const moves = ref<string[]>([])
const hint = ref<string | undefined>()
const flipped = ref(false)
const summary = ref<RunSummary | null>(null)
let recorded = false

const status = computed(() =>
  evaluateEndgame(drill.value.fen, moves.value, drill.value.player, drill.value.goal),
)
const position = computed(() => positionFromFen(status.value.fen))
const yourTurn = computed(() => !status.value.over && status.value.turn === drill.value.player)
const orientation = computed(() =>
  flipped.value ? (drill.value.player === 'white' ? 'black' : 'white') : drill.value.player,
)
const sans = computed(() => sanFrom(drill.value.fen, moves.value))
const lastMove = computed(() => {
  const last = moves.value.at(-1)
  if (!last) return undefined
  const before = evaluateEndgame(
    drill.value.fen,
    moves.value.slice(0, -1),
    drill.value.player,
    drill.value.goal,
  )
  return moveSquares(before.fen, last) as Key[] | undefined
})
const shapes = computed<DrawShape[]>(() => {
  if (!hint.value) return []
  const squares = moveSquares(status.value.fen, hint.value)
  return squares ? [{ orig: squares[0] as Key, dest: squares[1] as Key, brush: 'green' }] : []
})
const solved = (id: string): boolean => (summary.value?.best[id]?.score ?? 0) > 0
const groups = computed(() =>
  (['Basic mates', 'Pawn endings', 'Rook endings'] as const).map((label) => ({
    label,
    items: ENDGAME_DRILLS.filter((entry) => entry.group === label).map((entry) => ({
      id: entry.id,
      title: entry.title,
      done: solved(entry.id),
      aside: entry.level,
    })),
  })),
)

onMounted(async () => {
  summary.value = await window.kchess.runSummary('endgame')
  void engine()
})

function restart(next: EndgameDrill = drill.value): void {
  cancel()
  drill.value = next
  moves.value = []
  hint.value = undefined
  recorded = false
  flipped.value = false
  void engine()
}

function pick(id: string): void {
  restart(ENDGAME_DRILLS.find((entry) => entry.id === id))
}

/** Stockfish answers whenever it is its turn. */
async function engine(): Promise<void> {
  if (status.value.over || status.value.turn === drill.value.player || thinking.value) return
  const reply = await ask(drill.value.fen, moves.value, 350)
  if (!reply) return
  moves.value = [...moves.value, reply]
  playMoveSound(sanFrom(drill.value.fen, moves.value).at(-1))
}

function onMove(uci: string): void {
  if (!yourTurn.value) return
  hint.value = undefined
  moves.value = [...moves.value, uci]
  playMoveSound(sans.value.at(-1))
}

watch(moves, () => void engine())
watch(
  () => status.value.over,
  async (over) => {
    if (!over || recorded) return
    recorded = true
    try {
      const saved = await window.kchess.saveRun({
        kind: 'endgame',
        variant: drill.value.id,
        score: status.value.success ? 1 : 0,
        detail: {
          moves: Math.ceil(moves.value.length / 2),
          success: Boolean(status.value.success),
        },
      })
      summary.value = saved.summary
    } catch (cause) {
      console.warn('[endgame-drills] Could not save run:', cause)
      // The result is on screen; only the record is lost.
    }
  },
)

function takeback(): void {
  if (thinking.value) return
  cancel()
  hint.value = undefined
  const kept = moves.value.slice(0, -1)
  while (
    kept.length &&
    evaluateEndgame(drill.value.fen, kept, drill.value.player, drill.value.goal).turn !==
      drill.value.player
  )
    kept.pop()
  moves.value = kept
  recorded = false
}

async function showHint(): Promise<void> {
  if (!yourTurn.value || thinking.value) return
  const best = await ask(drill.value.fen, moves.value, 600, { busy: false })
  if (best && yourTurn.value) hint.value = best
}
</script>

<template>
  <div v-board-fit class="play-layout">
    <div class="board-stack">
      <div class="board-shell">
        <ChessBoard
          :fen="status.fen"
          :orientation="orientation"
          :theme="settings.boardTheme"
          :coordinates="settings.coordinates"
          :piece-set="settings.pieceSet"
          :animation="settings.pieceAnimation"
          :movable="yourTurn"
          :interactive="yourTurn"
          :movable-color="drill.player"
          :premove="false"
          :show-dests="settings.showLegalMoves"
          :promotion="settings.promotion === 'queen' ? 'queen' : 'ask'"
          :dests="yourTurn && position ? destsFor(position) : undefined"
          :last-move="lastMove"
          :check="position ? checkColor(position) : false"
          :turn-color="status.turn"
          :shapes="shapes"
          :blindfold="settings.blindfold && !status.over"
          @move="onMove"
        />
      </div>
    </div>

    <div class="card side-panel puzzle-panel">
      <div class="flex items-start justify-between gap-2">
        <div class="flex items-center gap-1 min-w-0">
          <UButton
            variant="ghost"
            color="neutral"
            size="sm"
            icon="i-lucide-chevron-left"
            aria-label="All courses"
            @click="emit('back')"
          />
          <h2 class="section-title">{{ drill.title }}</h2>
        </div>
        <div class="flex gap-1"><BoardViewToggles /></div>
      </div>
      <p class="section-hint">
        {{ drill.goal === 'win' ? 'Checkmate within 50 moves.' : 'Hold the draw.' }}
        {{ drill.idea }}
      </p>

      <div
        class="status-banner"
        :class="status.over ? (status.success ? 'win' : 'loss') : engineReady ? '' : 'alert'"
        role="status"
      >
        <UIcon
          :name="
            !engineReady
              ? 'i-lucide-triangle-alert'
              : status.over
                ? status.success
                  ? 'i-lucide-trophy'
                  : 'i-lucide-flag'
                : thinking
                  ? 'i-lucide-loader-circle'
                  : 'i-lucide-mouse-pointer-click'
          "
          class="status-icon"
          :class="{ 'animate-spin': thinking }"
        />
        <div>
          <strong>{{
            !engineReady
              ? 'Stockfish not found'
              : thinking
                ? 'Stockfish is thinking…'
                : status.title
          }}</strong>
          <span v-if="!engineReady" class="detail"
            >Install or choose an engine in Settings to use the drills.</span
          >
          <span v-else-if="status.detail" class="detail">{{ status.detail }}</span>
        </div>
      </div>
      <p v-if="engineError" class="text-error text-xs" role="alert">{{ engineError }}</p>

      <div class="panel-actions">
        <UButton
          variant="outline"
          color="neutral"
          icon="i-lucide-lightbulb"
          :disabled="!yourTurn || thinking"
          @click="showHint"
          >Hint</UButton
        >
        <UButton
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
          :variant="status.over ? 'solid' : 'outline'"
          :color="status.over ? 'primary' : 'neutral'"
          icon="i-lucide-rotate-ccw"
          @click="restart()"
          >Restart</UButton
        >
      </div>
      <p v-if="moves.length" class="muted text-xs tabular">
        {{ Math.ceil(moves.length / 2) }} moves · {{ sans.join(' ') }}
      </p>

      <div class="panel-divider" />
      <TrainingList :groups="groups" :active-id="drill.id" @select="pick" />
    </div>
  </div>
</template>
