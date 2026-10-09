<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import type { RunSummary } from '../../../../core/src/contracts/types'
import {
  ENDGAME_DRILLS,
  evaluateEndgame,
  sanFrom,
  type EndgameDrill,
} from '../../../../core/src/domain/endgames'
import { checkColor, destsFor, positionFromFen } from '../../../../core/src/domain/chess'
import { moveSquares } from '../../../../core/src/domain/puzzle'
import { playMoveSound } from '../utils/sound'

/**
 * Endgames you must convert (or hold) against Stockfish at full strength, from positions checked to be
 * won or drawn. The result is kept on this computer only.
 */
const store = useKChessStore()
const { settings, engineReady } = storeToRefs(store)

const drill = ref<EndgameDrill>(ENDGAME_DRILLS[0]!)
const moves = ref<string[]>([])
const thinking = ref(false)
const hint = ref<string | undefined>()
const flipped = ref(false)
const summary = ref<RunSummary | null>(null)
const engineError = ref('')
let epoch = 0
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
const groups = computed(() =>
  (['Basic mates', 'Pawn endings', 'Rook endings'] as const).map((label) => ({
    label,
    drills: ENDGAME_DRILLS.filter((entry) => entry.group === label),
  })),
)
const solved = (id: string): boolean => (summary.value?.best[id]?.score ?? 0) > 0

onMounted(async () => {
  summary.value = await window.kchess.runSummary('endgame')
  void engine()
})

function restart(next: EndgameDrill = drill.value): void {
  epoch++
  drill.value = next
  moves.value = []
  hint.value = undefined
  thinking.value = false
  engineError.value = ''
  recorded = false
  flipped.value = false
  void engine()
}

/** Stockfish answers whenever it is its turn. */
async function engine(): Promise<void> {
  if (status.value.over || status.value.turn === drill.value.player || thinking.value) return
  const mine = epoch
  thinking.value = true
  try {
    const reply = await window.kchess.bestMove([...moves.value], 'max', {
      fen: drill.value.fen,
      movetime: 350,
    })
    if (mine !== epoch) return
    moves.value = [...moves.value, reply]
    playMoveSound(sanFrom(drill.value.fen, moves.value).at(-1))
  } catch (cause) {
    if (mine === epoch) {
      console.warn('[endgame-drills] Engine move failed:', cause)
      engineError.value = cause instanceof Error ? cause.message : String(cause)
    }
  } finally {
    if (mine === epoch) thinking.value = false
  }
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
  epoch++
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
  const mine = epoch
  try {
    const best = await window.kchess.bestMove([...moves.value], 'max', {
      fen: drill.value.fen,
      movetime: 600,
    })
    if (mine === epoch && yourTurn.value) hint.value = best
  } catch (cause) {
    console.warn('[endgame-drills] Hint failed:', cause)
    engineError.value = cause instanceof Error ? cause.message : String(cause)
  }
}
</script>

<template>
  <div class="play-layout">
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
          :promotion="settings.promotion === 'premove' ? 'queen' : settings.promotion"
          :dests="yourTurn && position ? destsFor(position) : undefined"
          :last-move="lastMove"
          :check="position ? checkColor(position) : false"
          :turn-color="status.turn"
          :shapes="shapes"
          @move="onMove"
        />
      </div>
    </div>

    <div class="card side-panel puzzle-panel">
      <h2 class="section-title">{{ drill.title }}</h2>
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
      <div v-for="group in groups" :key="group.label" class="drill-group">
        <h3 class="section-title">{{ group.label }}</h3>
        <button
          v-for="entry in group.drills"
          :key="entry.id"
          type="button"
          class="drill-item"
          :class="{ active: entry.id === drill.id }"
          :aria-current="entry.id === drill.id ? 'true' : undefined"
          @click="restart(entry)"
        >
          <UIcon
            :name="solved(entry.id) ? 'i-lucide-circle-check' : 'i-lucide-circle'"
            :class="solved(entry.id) ? 'text-success' : 'muted'"
          />
          <span class="drill-title">{{ entry.title }}</span>
          <span class="drill-level muted text-xs">{{ entry.level }}</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.drill-group {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.drill-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: inherit;
  font-size: 13px;
  text-align: left;
}
.drill-level {
  margin-left: auto;
}
.drill-item:hover {
  background: var(--ui-bg-accented);
}
.drill-item.active {
  border-color: var(--ui-primary);
  background: color-mix(in srgb, var(--ui-primary) 10%, transparent);
}
</style>
