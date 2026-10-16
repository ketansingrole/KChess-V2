<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { StorageSerializers, useLocalStorage } from '@vueuse/core'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import { treeFromPgn, treeToPgn, type TreeNode } from '@kchess/rules/analysisTree'
import { checkColor, destsFor, positionFromFen } from '@kchess/rules/chess'
import { chapterItems, useStudyStore } from '../stores/studies'
import { playMoveSound } from '../utils/sound'
import { REPERTOIRE_OPENING_KEY, REPERTOIRE_STUDY_KEY } from '../utils/storageKeys'
import { useZenSurface } from '../composables/useZenSurface'
import { useOpeningDrill, type OpeningChoice } from '../composables/useOpeningDrill'
import BoardViewToggles from './BoardViewToggles.vue'

/**
 * Drill a repertoire: any saved study is a tree of moves. KChess plays the other side's moves
 * from the tree (favouring the lines you get wrong) and you must answer with one of yours.
 * One chapter can be drilled, or all of them: chapters from the same starting position are one
 * tree, and each line starts from one of the starting positions. An opening chosen on the
 * Openings page is drilled without a study: its named line, then the explorer's popular moves.
 */
const store = useKChessStore()
const { settings } = storeToRefs(store)
const library = useStudyStore()
const toast = useToast()
/** The repertoire's value when it is the opening below rather than a study. */
const OPENING = 'opening'
const studyId = useLocalStorage(REPERTOIRE_STUDY_KEY, '')
const opening = useLocalStorage<OpeningChoice | null>(REPERTOIRE_OPENING_KEY, null, {
  serializer: StorageSerializers.object,
})
const color = useLocalStorage<'white' | 'black'>('kchess:repertoire-color', 'white')
const stats = ref({ lines: 0, correct: 0, wrong: 0 })
// The board (and its zen toggle) shows once there is a study or an opening to train from.
useZenSurface(() => library.items.length > 0 || !!opening.value)
const drillsOpening = computed(() => studyId.value === OPENING && !!opening.value)
const {
  tree: drillTree,
  preparing,
  note: drillNote,
} = useOpeningDrill(
  computed(() => (drillsOpening.value ? opening.value : null)),
  color,
)

const ALL_CHAPTERS = 'all'
const chapterId = useLocalStorage('kchess:repertoire-chapter', ALL_CHAPTERS)

const study = computed(() => library.items.find((item) => item.id === studyId.value))
const repertoireName = computed(() =>
  drillsOpening.value ? opening.value?.name : study.value?.name,
)
const chapter = computed(() => study.value?.chapters.find((c) => c.id === chapterId.value))
/** The trees drilled: one per starting position among the chosen chapters. */
const roots = computed(() => {
  if (drillsOpening.value) return drillTree.value ? [drillTree.value] : []
  const byStart = new Map<string, TreeNode>()
  for (const { pgn } of chapter.value ? [chapter.value] : (study.value?.chapters ?? [])) {
    const tree = treeFromPgn(pgn)
    if (!tree) continue
    const same = byStart.get(tree.fen)
    if (same) merge(same, tree)
    else byStart.set(tree.fen, tree)
  }
  return [...byStart.values()]
})
/** Adds another chapter's moves (and comments) from the same position to a tree. */
function merge(into: TreeNode, from: TreeNode): void {
  for (const comment of from.comments ?? [])
    if (!into.comments?.includes(comment)) (into.comments ??= []).push(comment)
  for (const child of from.children) {
    const same = into.children.find((entry) => entry.uci === child.uci)
    if (same) merge(same, child)
    else into.children.push(child)
  }
}
const node = ref<TreeNode | null>(null)
const feedback = ref<{ kind: 'ok' | 'wrong' | 'done' | 'info'; text: string } | null>(null)
const hint = ref<DrawShape[]>([])
const resetKey = ref(0)
let reply: ReturnType<typeof setTimeout> | undefined

const missKey = computed(() =>
  drillsOpening.value
    ? `opening:${opening.value?.name.slice(0, 180)}:${color.value}`
    : `${studyId.value}:${color.value}`,
)
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
    const next = stack.pop()
    if (!next) break
    total += table[next.fen] ?? 0
    stack.push(...next.children)
  }
  return total
}
/** One of the nodes, more often those with weak lines below them. */
function pickWeak(nodes: TreeNode[]): TreeNode | undefined {
  const weights = nodes.map((entry) => 1 + 2 * weakness(entry))
  let pick = Math.random() * weights.reduce((sum, w) => sum + w, 0)
  for (let i = 0; i < nodes.length; i++) {
    const weight = weights[i]
    const candidate = nodes[i]
    if (weight === undefined || candidate === undefined) continue
    pick -= weight
    if (pick <= 0) return candidate
  }
  return nodes[0]
}
function opponentMove(): void {
  const current = node.value
  if (!current) return
  if (!current.children.length) return finishLine()
  const next = pickWeak(current.children)
  if (!next) return finishLine()
  node.value = next
  playMoveSound(next.san)
  if (!next.children.length) finishLine()
}
function finishLine(): void {
  stats.value.lines++
  feedback.value = { kind: 'done', text: 'End of this line. Next one in a moment…' }
  reply = setTimeout(startLine, 1400)
}
function startLine(): void {
  clearTimeout(reply)
  hint.value = []
  const start = pickWeak(roots.value)
  node.value = start ?? null
  if (!start) return
  feedback.value = {
    kind: 'info',
    text: `Play the ${color.value} moves of “${repertoireName.value}”.`,
  }
  if (!yourTurn.value) reply = setTimeout(opponentMove, 400)
}
watch([studyId, chapterId, color, drillTree], () => startLine(), { immediate: true })
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
/** A chapter that is gone (or from another study) means all chapters. */
const chapterChoice = computed({
  get: () => (chapter.value ? chapterId.value : ALL_CHAPTERS),
  set: (value: string) => (chapterId.value = value),
})
const chapterChoices = computed(() =>
  study.value ? [{ label: 'All chapters', value: ALL_CHAPTERS }, ...chapterItems(study.value)] : [],
)
const studyItems = computed(() => [
  ...(opening.value ? [{ label: opening.value.name, value: OPENING }] : []),
  ...library.items.map((item) => ({ label: item.name, value: item.id })),
])
const saving = ref(false)
/** Keep the drilled opening, with the replies found for it, as a study to edit or drill later. */
async function saveOpening(): Promise<void> {
  const tree = drillTree.value
  const name = opening.value?.name
  if (!tree || !name) return
  saving.value = true
  try {
    await library.save(library.freshName(name), treeToPgn(tree, { Event: name }))
    toast.add({ title: 'Saved to Studies', icon: 'i-lucide-check' })
  } catch (cause) {
    console.warn('[opening-trainer] Saving the opening failed:', cause)
    toast.add({ title: 'Could not save the study', color: 'error' })
  } finally {
    saving.value = false
  }
}
const accuracy = computed(() => {
  const total = stats.value.correct + stats.value.wrong
  return total ? Math.round((100 * stats.value.correct) / total) : null
})
</script>

<template>
  <div v-if="!library.items.length && !opening" class="card">
    <UEmpty
      variant="naked"
      icon="i-lucide-book-marked"
      title="Choose an opening"
      description="Pick an opening to drill, or save your own lines as a study."
      :actions="[
        { label: 'Browse openings', onClick: () => store.selectPage('openings') },
        {
          label: 'Open the analysis board',
          color: 'neutral',
          variant: 'outline',
          onClick: () => store.selectPage('analysis'),
        },
      ]"
    />
  </div>
  <div v-else v-board-fit class="play-layout">
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
        :blindfold="settings.blindfold"
        @move="play"
      />
    </div>
    <div class="card side-panel">
      <div class="flex items-start justify-between gap-2">
        <h2 class="section-title">Opening trainer</h2>
        <div class="flex gap-1"><BoardViewToggles /></div>
      </div>
      <UFormField label="Repertoire">
        <USelect
          v-model="studyId"
          :items="studyItems"
          placeholder="Choose an opening or study"
          class="w-full"
        />
      </UFormField>
      <UFormField v-if="!drillsOpening && (study?.chapters.length ?? 0) > 1" label="Chapter">
        <USelect v-model="chapterChoice" :items="chapterChoices" class="w-full" />
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
      <p v-if="drillsOpening && preparing" class="text-sm" role="status">
        Loading popular replies…
      </p>
      <p v-else-if="drillsOpening && drillNote" class="text-sm muted" role="status">
        {{ drillNote }}
      </p>
      <div
        v-if="feedback && node"
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
      <p v-if="drillsOpening" class="text-xs muted">
        After the named line you play the most played move on Lichess, and KChess answers with
        popular replies, more often in lines you got wrong.
      </p>
      <p v-else class="text-xs muted">
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
        <UButton
          v-if="drillsOpening"
          variant="ghost"
          color="neutral"
          icon="i-lucide-save"
          :loading="saving"
          :disabled="!drillTree"
          @click="saveOpening"
          >Save as study</UButton
        >
      </div>
    </div>
  </div>
</template>
