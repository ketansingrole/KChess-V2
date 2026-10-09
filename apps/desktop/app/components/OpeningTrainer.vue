<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import { treeFromPgn, type TreeNode } from '../../../../core/src/domain/analysisTree'
import { checkColor, destsFor, positionFromFen } from '../../../../core/src/domain/chess'
import { useStudyStore } from '../stores/studies'
import { playMoveSound } from '../utils/sound'

/**
 * Drill a repertoire: any saved study is a tree of moves. KChess plays the other side's moves
 * from the tree (favouring the lines you get wrong) and you must answer with one of yours.
 */
const store = useKChessStore()
const { settings } = storeToRefs(store)
const library = useStudyStore()
const studyId = useLocalStorage('kchess:repertoire-study', '')
const color = useLocalStorage<'white' | 'black'>('kchess:repertoire-color', 'white')
const stats = ref({ lines: 0, correct: 0, wrong: 0 })

const study = computed(() => library.items.find((item) => item.id === studyId.value))
const root = computed(() => (study.value ? treeFromPgn(study.value.pgn) : undefined))
const node = ref<TreeNode | null>(null)
const feedback = ref<{ kind: 'ok' | 'wrong' | 'done' | 'info'; text: string } | null>(null)
const hint = ref<DrawShape[]>([])
const resetKey = ref(0)
let reply: ReturnType<typeof setTimeout> | undefined

const missKey = computed(() => `${studyId.value}:${color.value}`)
const position = computed(() => (node.value ? positionFromFen(node.value.fen) : undefined))
const yourTurn = computed(() => position.value?.turn === color.value)
const lastMove = computed<Key[] | undefined>(() =>
  node.value?.uci
    ? [node.value.uci.slice(0, 2) as Key, node.value.uci.slice(2, 4) as Key]
    : undefined,
)
/** How often the moves below a node were missed: the trainer steers towards weak lines. */
function weakness(start: TreeNode): number {
  const table = library.misses[missKey.value] ?? {}
  let total = 0
  const stack = [start]
  while (stack.length) {
    const next = stack.pop()!
    total += table[next.fen] ?? 0
    stack.push(...next.children)
  }
  return total
}
function opponentMove(): void {
  const current = node.value
  if (!current) return
  if (!current.children.length) return finishLine()
  const weights = current.children.map((child) => 1 + 2 * weakness(child))
  let pick = Math.random() * weights.reduce((sum, w) => sum + w, 0)
  const child = current.children.find((_, i) => (pick -= weights[i]!) <= 0) ?? current.children[0]!
  node.value = child
  playMoveSound(child.san)
  if (!child.children.length) finishLine()
}
function finishLine(): void {
  stats.value.lines++
  feedback.value = { kind: 'done', text: 'End of this line. Next one in a moment…' }
  reply = setTimeout(startLine, 1400)
}
function startLine(): void {
  clearTimeout(reply)
  hint.value = []
  if (!root.value) return
  node.value = root.value
  feedback.value = {
    kind: 'info',
    text: `Play the ${color.value} moves of “${study.value?.name}”.`,
  }
  if (!yourTurn.value) reply = setTimeout(opponentMove, 400)
}
watch([studyId, color], () => startLine(), { immediate: true })
onUnmounted(() => clearTimeout(reply))

function play(uci: string): void {
  const current = node.value
  if (!current || !yourTurn.value) return
  const child = current.children.find((entry) => entry.uci === uci || sameCastle(entry.uci, uci))
  if (!child) {
    stats.value.wrong++
    library.recordMiss(missKey.value, current.fen)
    feedback.value = {
      kind: 'wrong',
      text: `Not in your repertoire. Expected ${current.children.map((c) => c.san).join(' or ') || 'nothing (line ended)'}.`,
    }
    hint.value = current.children.map((entry) => ({
      orig: entry.uci.slice(0, 2) as Key,
      dest: entry.uci.slice(2, 4) as Key,
      brush: 'green',
    }))
    resetKey.value++
    return
  }
  stats.value.correct++
  hint.value = []
  node.value = child
  playMoveSound(child.san)
  feedback.value = { kind: 'ok', text: `${child.san} ✓` }
  if (!child.children.length) finishLine()
  else reply = setTimeout(opponentMove, 450)
}
/** e1g1 and e1h1 are the same castling move. */
function sameCastle(a: string, b: string): boolean {
  const castles: Record<string, string> = { e1h1: 'e1g1', e1a1: 'e1c1', e8h8: 'e8g8', e8a8: 'e8c8' }
  return (castles[a] ?? a) === (castles[b] ?? b)
}
function showAnswer(): void {
  const current = node.value
  if (!current || !yourTurn.value) return
  hint.value = current.children.map((entry) => ({
    orig: entry.uci.slice(0, 2) as Key,
    dest: entry.uci.slice(2, 4) as Key,
    brush: 'blue',
  }))
}
function clearMisses(): void {
  library.clearMisses(missKey.value)
}
const studyItems = computed(() =>
  library.items.map((item) => ({ label: item.name, value: item.id })),
)
const accuracy = computed(() => {
  const total = stats.value.correct + stats.value.wrong
  return total ? Math.round((100 * stats.value.correct) / total) : null
})
</script>

<template>
  <div v-if="!library.items.length" class="card">
    <UEmpty
      variant="naked"
      icon="i-lucide-book-marked"
      title="Save a repertoire first"
      description="Save opening lines as a study to practice them here."
      :actions="[{ label: 'Open the analysis board', onClick: () => store.selectPage('analysis') }]"
    />
  </div>
  <div v-else class="play-layout">
    <div class="board-stack">
      <ChessBoard
        v-if="node"
        :fen="node.fen"
        :orientation="color"
        :theme="settings.boardTheme"
        :coordinates="settings.coordinates"
        :piece-set="settings.pieceSet"
        :animation="settings.pieceAnimation"
        :movable="yourTurn"
        :interactive="yourTurn"
        :movable-color="color"
        :show-dests="settings.showLegalMoves"
        :dests="position && yourTurn ? destsFor(position) : undefined"
        :last-move="lastMove"
        :check="position ? checkColor(position) : false"
        :turn-color="position?.turn"
        :shapes="hint"
        :reset-key="resetKey"
        @move="play"
      />
    </div>
    <div class="card side-panel">
      <h2 class="section-title">Opening trainer</h2>
      <UFormField label="Repertoire (a saved study)">
        <USelect
          v-model="studyId"
          :items="studyItems"
          placeholder="Choose a study"
          class="w-full"
        />
      </UFormField>
      <UFormField label="You play">
        <UTabs
          v-model="color"
          :items="[
            { label: 'White', value: 'white' },
            { label: 'Black', value: 'black' },
          ]"
          :content="false"
          variant="pill"
          class="w-full"
        />
      </UFormField>
      <div
        v-if="feedback"
        class="status-banner"
        :class="{
          win: feedback.kind === 'ok' || feedback.kind === 'done',
          loss: feedback.kind === 'wrong',
        }"
        role="status"
      >
        <UIcon
          :name="
            feedback.kind === 'wrong'
              ? 'i-lucide-x'
              : feedback.kind === 'info'
                ? 'i-lucide-info'
                : 'i-lucide-check'
          "
          class="status-icon"
        />
        <div>
          <strong>{{ feedback.text }}</strong>
          <span v-if="node?.comments?.length" class="detail">{{ node.comments.join(' ') }}</span>
        </div>
      </div>
      <p class="text-sm tabular">
        Lines finished: {{ stats.lines }} · correct {{ stats.correct }} · wrong {{ stats.wrong }}
        <span v-if="accuracy !== null"> · {{ accuracy }}%</span>
      </p>
      <p class="text-xs muted">
        KChess picks the other side's replies from your study, more often in lines you got wrong.
        Comments in the study show as you reach them.
      </p>
      <div class="panel-actions panel-divider pt-3.5">
        <UButton
          variant="outline"
          color="neutral"
          icon="i-lucide-lightbulb"
          :disabled="!yourTurn"
          @click="showAnswer"
          >Show move</UButton
        >
        <UButton variant="outline" color="neutral" icon="i-lucide-rotate-ccw" @click="startLine"
          >Restart line</UButton
        >
        <UButton variant="ghost" color="neutral" icon="i-lucide-eraser" @click="clearMisses"
          >Forget mistakes</UButton
        >
      </div>
    </div>
  </div>
</template>
