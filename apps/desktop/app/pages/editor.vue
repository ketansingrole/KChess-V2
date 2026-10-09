<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { EditorTool } from '../components/EditorBoard.vue'
import {
  castlingAvailable,
  EMPTY_BOARD,
  enPassantSquares,
  positionProblem,
  setupFen,
  setupFromFen,
  START_SETUP,
  withPiece,
} from '../../../../core/src/domain/boardEditor'
import {
  EDITOR_GRAMMAR,
  spokenEdit,
  type PieceColor,
} from '../../../../core/src/domain/voiceCommands'
import type { VoiceResult } from '../utils/voiceCapture'
import { heardFields, logVoice } from '../utils/voiceLog'
import { useAnalysisStore } from '../stores/analysis'

const { settings } = storeToRefs(useKChessStore())
const analysis = useAnalysisStore()
const { editor, editorOrientation } = storeToRefs(analysis)
const toast = useToast()

const tool = ref<EditorTool>('pointer')
const fen = computed(() => setupFen(editor.value))
const problem = computed(() => positionProblem(fen.value))
const available = computed(() => castlingAvailable(editor.value.board))
const epSquares = computed(() => enPassantSquares(editor.value.board, editor.value.turn))

/* The FEN box shows the position; entering a valid FEN there sets the board. */
const fenInput = ref(fen.value)
const fenError = ref('')
watch(fen, (value) => {
  fenInput.value = value
  fenError.value = ''
})
function applyFen(): void {
  const setup = setupFromFen(fenInput.value)
  if (!setup) {
    fenError.value = 'That is not a valid FEN.'
    return
  }
  fenError.value = ''
  editor.value = setup
}

function setBoard(board: string): void {
  editor.value = { ...editor.value, board }
}
function startPosition(): void {
  editor.value = structuredClone(START_SETUP)
}
function clearBoard(): void {
  editor.value = { ...editor.value, board: EMPTY_BOARD, ep: '' }
}
function flip(): void {
  editorOrientation.value = editorOrientation.value === 'white' ? 'black' : 'white'
}
const turn = computed({
  get: () => editor.value.turn,
  set: (value: 'white' | 'black') => (editor.value = { ...editor.value, turn: value, ep: '' }),
})
const ep = computed({
  get: () => editor.value.ep || 'none',
  set: (value: string) => (editor.value = { ...editor.value, ep: value === 'none' ? '' : value }),
})
const castlingRights = [
  { key: 'K', label: 'White O-O' },
  { key: 'Q', label: 'White O-O-O' },
  { key: 'k', label: 'Black O-O' },
  { key: 'q', label: 'Black O-O-O' },
] as const
function toggleCastling(key: (typeof castlingRights)[number]['key'], value: boolean): void {
  editor.value = { ...editor.value, castling: { ...editor.value.castling, [key]: value } }
}

/** Play this position: against Stockfish, or over the board with a friend. */
function playComputer(): void {
  if (problem.value) return
  const turn = fen.value.split(' ')[1] === 'b' ? 'black' : 'white'
  useKChessStore().startComputerFrom({ variant: 'standard', fen: fen.value }, turn)
}
function playLocal(): void {
  if (problem.value) return
  void navigateTo({ path: '/local', query: { fen: fen.value } })
}
function analyse(): void {
  if (problem.value) return
  analysis.analyseSetup(fen.value)
  void navigateTo('/analysis')
}
async function copyFen(): Promise<void> {
  try {
    await navigator.clipboard.writeText(fen.value)
    toast.add({ title: 'FEN copied', icon: 'i-lucide-clipboard-check' })
  } catch (cause) {
    console.warn('[editor] Copy FEN failed:', cause)
    toast.add({ title: 'Could not copy the FEN', color: 'error', icon: 'i-lucide-clipboard-x' })
  }
}

/* ── Voice: “white knight F three”, “remove E four”, “clear board”, “black to move” ───── */

const voiceEnabled = ref(false)
const voiceFeedback = ref('')
/** A piece said without a colour takes the colour said last. */
const lastColor = ref<PieceColor>('white')
function hear(result: VoiceResult): void {
  if (!voiceEnabled.value) return
  const action = spokenEdit(result.text, lastColor.value)
  void logVoice({
    source: 'editor',
    ...heardFields(result),
    outcome: action ? 'command' : result.unclear ? 'unclear' : 'invalid',
    parsed: action ? JSON.stringify(action) : undefined,
    fen: problem.value ? undefined : fen.value,
  })
  if (!action) {
    voiceFeedback.value = result.unclear
      ? 'Didn’t catch that. Try “white knight F three”.'
      : `Heard “${result.text}”. Try “white knight F three”, “remove E four” or “clear board”.`
    return
  }
  switch (action.kind) {
    case 'place': {
      lastColor.value = action.color
      setBoard(withPiece(editor.value.board, action.square, action))
      voiceFeedback.value = `${action.color === 'white' ? 'White' : 'Black'} ${action.role} on ${action.square.toUpperCase()}.`
      break
    }
    case 'remove':
      setBoard(withPiece(editor.value.board, action.square, undefined))
      voiceFeedback.value = `Cleared ${action.square.toUpperCase()}.`
      break
    case 'clear':
      clearBoard()
      voiceFeedback.value = 'Board cleared.'
      break
    case 'start':
      startPosition()
      voiceFeedback.value = 'Starting position.'
      break
    case 'turn':
      turn.value = action.color
      voiceFeedback.value = `${action.color === 'white' ? 'White' : 'Black'} to move.`
      break
    case 'flip':
      flip()
      voiceFeedback.value = 'Board flipped.'
      break
    case 'analyze':
      if (problem.value) voiceFeedback.value = `Can’t analyse yet: ${problem.value}`
      else analyse()
      break
  }
}
function missed(result: VoiceResult): void {
  if (voiceEnabled.value)
    void logVoice({ source: 'editor', ...heardFields(result), outcome: 'unclear' })
}
</script>

<template>
  <div>
    <PageHeader title="Board editor" />

    <div class="play-layout editor-layout">
      <div class="board-stack">
        <EditorBoard
          v-model:tool="tool"
          :board="editor.board"
          :orientation="editorOrientation"
          :theme="settings?.boardTheme"
          :coordinates="settings?.coordinates"
          :piece-set="settings?.pieceSet"
          :animation="settings?.pieceAnimation"
          @update:board="setBoard"
        />
        <VoiceInput
          v-model:enabled="voiceEnabled"
          compact
          active
          context-key="editor"
          :grammar="EDITOR_GRAMMAR"
          hint="Say “white knight F three”, “remove E four” or “clear board”."
          :feedback="voiceFeedback"
          allow-push-talk
          accept-unclear
          @result="hear"
          @unclear="missed"
        />
      </div>

      <div class="card side-panel editor-panel">
        <div class="status-banner" :class="{ alert: problem }" role="status">
          <UIcon
            :name="problem ? 'i-lucide-triangle-alert' : 'i-lucide-circle-check'"
            class="status-icon"
          />
          <div>
            <strong>{{ problem ? 'Not a legal position yet' : 'Ready to analyse' }}</strong>
            <span class="detail">{{
              problem ?? `${turn === 'white' ? 'White' : 'Black'} to move`
            }}</span>
          </div>
        </div>

        <div class="editor-field">
          <span id="editor-turn" class="editor-label">Side to move</span>
          <UTabs
            v-model="turn"
            :items="[
              { label: 'White', value: 'white' },
              { label: 'Black', value: 'black' },
            ]"
            aria-labelledby="editor-turn"
            :content="false"
            variant="pill"
          />
        </div>

        <div class="editor-field">
          <span class="editor-label">Castling</span>
          <div class="castling-grid">
            <label
              v-for="right in castlingRights"
              :key="right.key"
              class="castling-option"
              :class="{ unavailable: !available[right.key] }"
              :title="available[right.key] ? '' : 'The king or rook is not on its starting square'"
            >
              <input
                type="checkbox"
                :checked="editor.castling[right.key] && available[right.key]"
                :disabled="!available[right.key]"
                @change="toggleCastling(right.key, ($event.target as HTMLInputElement).checked)"
              />
              {{ right.label }}
            </label>
          </div>
        </div>

        <div v-if="epSquares.length" class="editor-field">
          <span id="editor-ep" class="editor-label">En passant</span>
          <USelect
            v-model="ep"
            :items="[
              { label: 'None', value: 'none' },
              ...epSquares.map((square) => ({ label: square, value: square })),
            ]"
            aria-labelledby="editor-ep"
            size="sm"
            class="w-32"
          />
        </div>

        <form class="editor-field" @submit.prevent="applyFen">
          <label for="editor-fen" class="editor-label">FEN</label>
          <div class="flex gap-2">
            <UInput
              id="editor-fen"
              v-model="fenInput"
              class="flex-1 min-w-0 fen-input"
              size="sm"
              autocomplete="off"
              spellcheck="false"
              :color="fenError ? 'error' : undefined"
              @blur="applyFen"
              @keydown.enter.prevent="applyFen"
            />
            <UTooltip text="Copy FEN">
              <UButton
                color="neutral"
                variant="outline"
                size="sm"
                icon="i-lucide-copy"
                aria-label="Copy FEN"
                @click="copyFen"
              />
            </UTooltip>
          </div>
          <span v-if="fenError" class="text-xs text-error">{{ fenError }}</span>
        </form>

        <div class="editor-actions">
          <UButton
            color="neutral"
            variant="outline"
            icon="i-lucide-rotate-ccw"
            @click="startPosition"
            >Starting position</UButton
          >
          <UButton color="neutral" variant="outline" icon="i-lucide-eraser" @click="clearBoard"
            >Clear board</UButton
          >
          <UButton color="neutral" variant="outline" icon="i-lucide-arrow-up-down" @click="flip"
            >Flip board</UButton
          >
        </div>

        <div class="panel-divider pt-3.5 mt-auto">
          <UButton
            block
            icon="i-lucide-microscope"
            :disabled="!!problem"
            :title="problem"
            @click="analyse"
            >Analyse this position</UButton
          >
          <div class="mt-2 grid grid-cols-2 gap-2">
            <UButton
              variant="outline"
              color="neutral"
              icon="i-lucide-cpu"
              :disabled="!!problem"
              @click="playComputer"
              >Play Stockfish</UButton
            >
            <UButton
              variant="outline"
              color="neutral"
              icon="i-lucide-users-round"
              :disabled="!!problem"
              @click="playLocal"
              >Play a friend here</UButton
            >
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.editor-layout {
  /* Spare-piece rows above and below the board, and the voice bar, take height from it. */
  --board-chrome: 300px;
}
.editor-panel > * + * {
  margin-top: 16px;
}
.editor-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.editor-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--ui-text-muted);
}
.castling-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 6px 12px;
  font-size: 13px;
}
.castling-option {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
}
.castling-option input {
  accent-color: var(--ui-primary);
}
.castling-option.unavailable {
  color: var(--ui-text-dimmed);
  cursor: not-allowed;
}
.fen-input :deep(input) {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
}
.editor-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
</style>
