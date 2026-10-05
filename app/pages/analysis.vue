<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import type { ContextMenuItem, DropdownMenuItem } from '@nuxt/ui'
import type { EngineLine } from '../../src/shared/types'
import { checkColor, destsFor } from '../utils/chess'
import {
  formatEval,
  MOVE_GLYPHS,
  moveGlyph,
  nodeAt,
  onMainline,
  pvSan,
  setMoveGlyph,
} from '../utils/analysisTree'
import { positionProblem, setupFromFen, setupFen } from '../utils/boardEditor'
import {
  ANALYSIS_GRAMMAR,
  spokenAnalysisCommand,
  spokenChoice,
  spokenMove,
  type VoiceMoveChoice,
} from '../utils/voiceCommands'
import type { VoiceResult } from '../utils/voiceCapture'
import { heardFields, logVoice } from '../utils/voiceLog'
import { useAnalysisStore } from '../stores/analysis'

const store = useKChessStore()
const { settings, engineReady } = storeToRefs(store)
const analysis = useAnalysisStore()
const {
  root,
  path,
  node,
  position,
  orientation,
  evaluation,
  engineOn,
  assistanceAllowed,
  engineLines,
  infinite,
  showArrows,
  engineBusy,
  engineError,
  saveError,
  studySaveError,
  gameOver,
  cloud,
  cloudBusy,
  cloudError,
} = storeToRefs(analysis)
const toast = useToast()

onMounted(() => {
  void store.recheckEngine()
  analysis.attach()
  window.addEventListener('keydown', keydown)
})
onUnmounted(() => {
  analysis.detach()
  window.removeEventListener('keydown', keydown)
})

/* ── Board ─────────────────────────────────────────────────────────── */

const resetKey = ref(0)
const dests = computed(() => (position.value ? destsFor(position.value) : new Map()))
const lastMove = computed<Key[] | undefined>(() => {
  const uci = node.value.uci
  return uci ? [uci.slice(0, 2) as Key, uci.slice(2, 4) as Key] : undefined
})
function move(uci: string): void {
  if (!analysis.play(uci)) resetKey.value++
}
function flip(): void {
  orientation.value = orientation.value === 'white' ? 'black' : 'white'
}

/** Lichess's cloud lines for this position, when it has some. */
const cloudLines = computed<EngineLine[]>(() =>
  assistanceAllowed.value && cloud.value && cloud.value.fen === node.value.fen
    ? cloud.value.lines
    : [],
)
/** Without the local engine, the cloud's lines drive the bar, arrows and line list. */
const usingCloud = computed(
  () => (!engineOn.value || !engineReady.value) && cloudLines.value.length > 0,
)
const lines = computed<EngineLine[]>(() =>
  usingCloud.value ? cloudLines.value : (evaluation.value?.lines ?? []),
)
const best = computed(() => lines.value[0])
/** Arrows for the engine's choices: the best one bold, the others thinner. */
const shapes = computed<DrawShape[]>(() => {
  if ((!engineOn.value && !usingCloud.value) || !showArrows.value || gameOver.value) return []
  return lines.value.flatMap((line, index) => {
    const uci = line.pv[0]
    if (!uci) return []
    return [
      {
        orig: uci.slice(0, 2) as Key,
        dest: uci.slice(2, 4) as Key,
        brush: index === 0 ? 'paleBlue' : 'paleGrey',
        modifiers: { lineWidth: index === 0 ? 12 : Math.max(4, 9 - index * 2) },
      },
    ]
  })
})

/** Checkmate and stalemate need no engine: the bar shows the result instead. */
const result = computed(() => {
  const pos = position.value
  if (!gameOver.value || !pos) return ''
  if (pos.isCheckmate()) return pos.turn === 'white' ? '0-1' : '1-0'
  return '½-½'
})

/** Lichess steps through moves with the mouse wheel over the board. */
let wheelTravel = 0
function wheel(event: WheelEvent): void {
  event.preventDefault()
  wheelTravel += event.deltaY
  if (Math.abs(wheelTravel) < 40) return
  if (wheelTravel > 0) analysis.forward()
  else analysis.back()
  wheelTravel = 0
}

/* ── Engine panel ──────────────────────────────────────────────────── */

const engineStatus = computed(() => {
  if (usingCloud.value && cloud.value)
    return `Lichess cloud · depth ${cloud.value.depth} · ${Math.round(cloud.value.knodes / 1000)}M nodes`
  if (!engineReady.value) return 'Stockfish not found'
  if (engineError.value) return engineError.value
  if (!engineOn.value) return 'Engine off'
  if (gameOver.value) return gameOver.value
  if (!assistanceAllowed.value)
    return store.onlineConnection?.phase === 'checking'
      ? 'Checking Lichess game status…'
      : 'Analysis paused until Lichess game status is verified'
  const update = evaluation.value
  if (!update) return 'Starting…'
  const depth = `Depth ${update.depth}${infinite.value ? '' : '/26'}`
  const nps = update.nps ?? 0
  const speed =
    nps >= 1e6
      ? ` · ${(nps / 1e6).toFixed(1)}M nodes/s`
      : nps
        ? ` · ${Math.round(nps / 1000)}k nodes/s`
        : ''
  return `${depth}${speed}${update.reason === 'interrupted' ? ' · Interrupted — retry or change position' : update.done ? '' : '…'}`
})
const lineChoices = [1, 2, 3, 4, 5].map((count) => ({
  label: `${count} ${count === 1 ? 'line' : 'lines'}`,
  value: count,
}))
const pvMoves = computed(() =>
  lines.value.map((line) => ({ line, moves: pvSan(node.value.fen, line.pv) })),
)
/** Play an engine line up to (and including) the move clicked. */
function playLine(pv: readonly string[], upTo = 0): void {
  for (const uci of pv.slice(0, upTo + 1)) if (!analysis.play(uci)) break
}
/** The position after a hovered engine move, shown on a small board beside it. */
const PREVIEW_SIZE = 220
const hovered = ref<{ line: number; move: number; top: number; left: number } | null>(null)
const preview = computed(() => {
  const at = hovered.value
  const pvMove = at && pvMoves.value[at.line]?.moves[at.move]
  if (!at || !pvMove) return null
  return {
    fen: pvMove.fen,
    lastMove: [pvMove.uci.slice(0, 2), pvMove.uci.slice(2, 4)] as Key[],
    style: { top: `${at.top}px`, left: `${at.left}px`, width: `${PREVIEW_SIZE}px` },
  }
})
function showPreview(event: Event, line: number, move: number): void {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const gap = 8
  // Above the move when it fits, otherwise below; kept inside the window sideways.
  const top = rect.top - PREVIEW_SIZE - gap >= 0 ? rect.top - PREVIEW_SIZE - gap : rect.bottom + gap
  const left = Math.min(
    Math.max(gap, rect.left + rect.width / 2 - PREVIEW_SIZE / 2),
    window.innerWidth - PREVIEW_SIZE - gap,
  )
  hovered.value = { line, move, top, left }
}
function hidePreview(): void {
  hovered.value = null
}
watch(path, hidePreview)
function playBest(): void {
  const uci = best.value?.pv[0]
  if (uci) analysis.play(uci)
}

/* ── Moves ─────────────────────────────────────────────────────────── */

const treeList = ref<HTMLElement | null>(null)
watch(
  path,
  async () => {
    await nextTick()
    const container = treeList.value
    const active = container?.querySelector<HTMLElement>('.tree-move.active')
    if (!container || !active) return
    const top = active.offsetTop - container.offsetTop
    if (top < container.scrollTop) container.scrollTop = top - 8
    else if (top + active.offsetHeight > container.scrollTop + container.clientHeight)
      container.scrollTop = top + active.offsetHeight - container.clientHeight + 8
  },
  { flush: 'post' },
)
const hasMoves = computed(() => root.value.children.length > 0)
const onVariation = computed(() => !!path.value && !onMainline(root.value, path.value))

/* ── Tools under the engine: one at a time, so the move list gets the room ── */

const { tool } = storeToRefs(analysis)
const toolTabs = computed(() => {
  const progress = analysis.reviewProgress
  return [
    { value: 'moves' as const, label: 'Moves', icon: 'i-lucide-list-tree', badge: '' },
    {
      value: 'review' as const,
      label: 'Review',
      icon: 'i-lucide-sparkles',
      badge: progress ? `${Math.round((100 * progress.done) / Math.max(1, progress.total))}%` : '',
    },
    { value: 'explorer' as const, label: 'Explorer', icon: 'i-lucide-book-open-text', badge: '' },
  ]
})

/** Right-click a move to comment on it, annotate it, promote or delete it. */
const notes = ref<{ focus: () => Promise<void> } | null>(null)
const menuPath = ref('')
async function comment(): Promise<void> {
  tool.value = 'moves'
  await nextTick()
  await notes.value?.focus()
}
function moveContext(event: MouseEvent): void {
  const at = (event.target as HTMLElement).closest<HTMLElement>('[data-path]')?.dataset.path
  menuPath.value = at ?? ''
  if (at !== undefined) analysis.goTo(at)
}
const moveMenu = computed<ContextMenuItem[][]>(() => {
  const at = menuPath.value
  if (!at)
    return [
      [
        {
          label: 'Copy PGN',
          icon: 'i-lucide-file-text',
          onSelect: () => void copy(analysis.pgn(), 'PGN'),
        },
      ],
    ]
  const target = nodeAt(root.value, at)
  const current = moveGlyph(target)
  return [
    [
      {
        label: target.comments?.length ? 'Edit comment' : 'Add comment',
        icon: 'i-lucide-message-square-text',
        kbds: ['C'],
        onSelect: () => void comment(),
      },
      {
        label: 'Annotate',
        icon: 'i-lucide-highlighter',
        children: [
          MOVE_GLYPHS.map((entry) => ({
            label: `${entry.glyph}  ${entry.label}`,
            type: 'checkbox' as const,
            checked: current?.nag === entry.nag,
            onSelect: () =>
              setMoveGlyph(target, current?.nag === entry.nag ? undefined : entry.nag),
          })),
        ],
      },
    ],
    [
      {
        label: 'Make main line',
        icon: 'i-lucide-arrow-up-to-line',
        disabled: onMainline(root.value, at),
        onSelect: () => analysis.promote(at),
      },
      {
        label: 'Delete from here',
        icon: 'i-lucide-scissors',
        color: 'error' as const,
        onSelect: () => analysis.remove(at),
      },
    ],
    [{ label: 'Copy FEN', icon: 'i-lucide-copy', onSelect: () => void copy(target.fen, 'FEN') }],
  ]
})

/* ── Import and export ─────────────────────────────────────────────── */

const importOpen = ref(false)
const importText = ref('')
const importError = ref('')
function openImport(): void {
  importText.value = ''
  importError.value = ''
  importOpen.value = true
}
function runImport(): void {
  const text = importText.value.trim()
  if (!text) return
  const setup = setupFromFen(text)
  if (setup) {
    const fen = setupFen(setup)
    const problem = positionProblem(fen)
    if (problem) {
      importError.value = problem
      return
    }
    analysis.load(fen)
  } else if (!analysis.loadPgn(text)) {
    importError.value = 'Use a valid FEN or one PGN game with legal moves, under 2 MB.'
    return
  } else if (!root.value.children.length) {
    importError.value = 'No moves could be read from that PGN.'
    return
  }
  importOpen.value = false
}
async function copy(text: string, what: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text)
    toast.add({ title: `${what} copied`, icon: 'i-lucide-clipboard-check' })
  } catch {
    toast.add({ title: `Could not copy the ${what}`, color: 'error', icon: 'i-lucide-clipboard-x' })
  }
}
function editPosition(): void {
  analysis.editPosition()
  void navigateTo('/editor')
}
function newAnalysis(): void {
  analysis.load()
}
const menu = computed<DropdownMenuItem[][]>(() => [
  [
    { label: 'Copy FEN', icon: 'i-lucide-copy', onSelect: () => void copy(node.value.fen, 'FEN') },
    {
      label: 'Copy PGN',
      icon: 'i-lucide-file-text',
      onSelect: () => void copy(analysis.pgn(), 'PGN'),
    },
    { label: 'Import FEN or PGN…', icon: 'i-lucide-import', onSelect: openImport },
  ],
  [
    {
      label: 'Make main line',
      icon: 'i-lucide-arrow-up-to-line',
      disabled: !onVariation.value,
      onSelect: () => analysis.promote(),
    },
    {
      label: 'Delete from here',
      icon: 'i-lucide-scissors',
      disabled: !path.value,
      onSelect: () => analysis.remove(),
    },
  ],
  [
    { label: 'Edit this position', icon: 'i-lucide-pencil-ruler', onSelect: editPosition },
    {
      label: 'Play Stockfish from here',
      icon: 'i-lucide-cpu',
      disabled: Boolean(gameOver.value),
      onSelect: () =>
        store.startComputerFrom(
          { variant: 'standard', fen: node.value.fen },
          position.value?.turn ?? 'white',
        ),
    },
    {
      label: 'Play a friend from here',
      icon: 'i-lucide-users-round',
      disabled: Boolean(gameOver.value),
      onSelect: () => void navigateTo({ path: '/local', query: { fen: node.value.fen } }),
    },
    { label: 'New analysis', icon: 'i-lucide-rotate-ccw', onSelect: newAnalysis },
  ],
])

/* ── Keyboard: ← → ↑ ↓ step, F flips, Space plays the best move, L toggles the engine, C comments ── */

function keydown(event: KeyboardEvent): void {
  if (importOpen.value || event.metaKey || event.ctrlKey || event.altKey) return
  const target = event.target as HTMLElement | null
  if (target?.closest('input, textarea, select, [contenteditable="true"]')) return
  const actions: Record<string, () => void> = {
    ArrowLeft: analysis.back,
    ArrowRight: analysis.forward,
    ArrowUp: analysis.toStart,
    ArrowDown: analysis.toEnd,
    f: flip,
    l: () => (engineOn.value = !engineOn.value),
    c: () => void comment(),
  }
  // Space plays the engine's move, unless a button has focus (then Space presses it).
  if (event.key === ' ' && !target?.closest('button') && !settings.value?.voicePushToTalk)
    actions[' '] = playBest
  const action = actions[event.key]
  if (!action) return
  event.preventDefault()
  action()
}

/* ── Voice: moves, “back”, “next”, “best move”, “flip board” ──────── */

const voiceEnabled = ref(false)
const voiceFeedback = ref('')
const voiceChoices = ref<VoiceMoveChoice[]>([])
watch(path, () => (voiceChoices.value = []))
function log(
  result: VoiceResult,
  outcome: 'played' | 'pending' | 'command' | 'invalid' | 'unclear',
  parsed?: string,
): void {
  void logVoice({
    source: 'analysis',
    ...heardFields(result),
    outcome,
    parsed,
    fen: node.value.fen,
  })
}
function hear(result: VoiceResult): void {
  if (!voiceEnabled.value) return
  const command = spokenAnalysisCommand(result.text)
  if (command) {
    log(result, 'command', command)
    const run = {
      back: analysis.back,
      forward: analysis.forward,
      start: analysis.toStart,
      end: analysis.toEnd,
      best: playBest,
      flip,
      engine: () => (engineOn.value = !engineOn.value),
    }[command]
    run()
    voiceFeedback.value =
      command === 'best' && !best.value ? 'The engine has no move yet.' : `“${result.text}”`
    return
  }
  const index = spokenChoice(result.text)
  if (index && voiceChoices.value.length) {
    const choice = voiceChoices.value[index - 1]
    log(result, 'command', `#${index}`)
    if (choice) {
      analysis.play(choice.uci)
      voiceFeedback.value = `Played ${choice.san}.`
    } else voiceFeedback.value = 'Choose one of the numbered moves.'
    return
  }
  const pos = position.value
  const parsed = pos ? spokenMove(result.text, pos) : ({ kind: 'invalid' } as const)
  if (parsed.kind !== 'move') {
    log(result, result.unclear ? 'unclear' : 'invalid')
    voiceFeedback.value = result.unclear
      ? 'Didn’t catch that. Repeat, or try “Echo four” for E4.'
      : `Heard “${result.text}”: no legal move matches. Try “Knight F three” or “back”.`
    return
  }
  const sans = parsed.choices.map((choice) => choice.san).join('|')
  // Trying a move in analysis loses nothing (it can be stepped back or deleted), so play it.
  if (parsed.choices.length === 1) {
    log(result, 'played', sans)
    analysis.play(parsed.choices[0]!.uci)
    voiceFeedback.value = `Played ${parsed.choices[0]!.san}.`
    return
  }
  log(result, 'pending', sans)
  voiceChoices.value = parsed.choices
  voiceFeedback.value = 'Several moves match: say its number, or select it.'
}
function missed(result: VoiceResult): void {
  if (voiceEnabled.value) log(result, 'unclear')
}
</script>

<template>
  <div>
    <p v-if="studySaveError" role="alert" class="p-3 text-error">{{ studySaveError }}</p>
    <p v-if="saveError" role="alert" class="p-3 text-error">Automatic saving: {{ saveError }}</p>

    <PageHeader title="Analysis board" />
    <StudyBar />

    <div class="play-layout analysis-layout">
      <div class="board-stack">
        <div class="analysis-board">
          <EvalBar
            v-if="assistanceAllowed && (engineOn || usingCloud)"
            class="analysis-eval"
            :line="best"
            :orientation="orientation"
            :result="result"
          />
          <div class="board-shell" @wheel="wheel">
            <ChessBoard
              :fen="node.fen"
              :orientation="orientation"
              :theme="settings?.boardTheme"
              :coordinates="settings?.coordinates"
              :piece-set="settings?.pieceSet"
              :animation="settings?.pieceAnimation"
              movable
              interactive
              :movable-color="position?.turn ?? 'white'"
              :show-dests="settings?.showLegalMoves ?? true"
              :promotion="settings?.promotion === 'queen' ? 'queen' : 'ask'"
              :dests="dests"
              :last-move="lastMove"
              :check="position ? checkColor(position) : false"
              :turn-color="position?.turn"
              :shapes="shapes"
              :reset-key="resetKey"
              @move="move"
            >
              <template #controls>
                <VoiceInput
                  v-model:enabled="voiceEnabled"
                  compact
                  active
                  :context-key="path"
                  :grammar="ANALYSIS_GRAMMAR"
                  hint="Say a move like “Knight F three”, or “back”, “next”, “best move”."
                  :feedback="voiceFeedback"
                  allow-push-talk
                  accept-unclear
                  @result="hear"
                  @unclear="missed"
                >
                  <template v-if="voiceChoices.length">
                    <UButton
                      v-for="(choice, index) in voiceChoices"
                      :key="choice.uci"
                      color="neutral"
                      variant="outline"
                      size="xs"
                      class="shrink-0"
                      @click="analysis.play(choice.uci)"
                      >{{ index + 1 }} · {{ choice.san }}</UButton
                    >
                  </template>
                </VoiceInput>
              </template>
            </ChessBoard>
          </div>
        </div>
      </div>

      <div class="card side-panel analysis-panel">
        <section class="ceval" aria-label="Engine">
          <div class="ceval-head">
            <USwitch
              v-model="engineOn"
              aria-label="Engine analysis"
              :disabled="!engineReady || !assistanceAllowed"
            />
            <strong class="ceval-score" :class="{ dim: !engineOn && !usingCloud }">{{
              !engineOn && !usingCloud ? '—' : result || formatEval(best)
            }}</strong>
            <div class="ceval-info">
              <span class="ceval-name">Stockfish</span>
              <span class="ceval-status" :class="{ 'text-error': engineError }">
                <UIcon
                  v-if="engineOn && engineBusy"
                  name="i-lucide-loader-circle"
                  class="animate-spin"
                />
                {{ engineStatus }}
              </span>
            </div>
            <UPopover :content="{ align: 'end' }">
              <UTooltip text="Engine settings">
                <UButton
                  size="xs"
                  variant="ghost"
                  color="neutral"
                  icon="i-lucide-settings-2"
                  aria-label="Engine settings"
                />
              </UTooltip>
              <template #content>
                <div class="engine-settings">
                  <USwitch
                    v-model="infinite"
                    size="sm"
                    label="Infinite analysis"
                    description="Keep searching past depth 26"
                    :disabled="!engineOn"
                  />
                  <USwitch
                    v-model="showArrows"
                    size="sm"
                    label="Show engine arrows"
                    :disabled="!engineOn"
                  />
                  <USwitch
                    :model-value="settings?.cloudEval ?? false"
                    size="sm"
                    label="Lichess cloud evaluation"
                    description="Sends positions to Lichess"
                    :disabled="!assistanceAllowed"
                    @update:model-value="store.toggleSetting('cloudEval')"
                  />
                  <UFormField label="Engine lines" size="sm" orientation="horizontal">
                    <USelect
                      v-model="engineLines"
                      :items="lineChoices"
                      size="sm"
                      aria-label="Engine lines"
                      class="w-28"
                      :disabled="!engineOn && !settings?.cloudEval"
                    />
                  </UFormField>
                </div>
              </template>
            </UPopover>
          </div>
          <p
            v-if="settings?.cloudEval && assistanceAllowed && !usingCloud && !gameOver"
            class="text-xs muted"
          >
            <UIcon name="i-lucide-cloud" />
            <template v-if="cloudLines.length">
              Lichess cloud: {{ formatEval(cloudLines[0]) }} at depth {{ cloud?.depth }} ·
              <button
                type="button"
                class="underline"
                @click="cloudLines[0] && playLine(cloudLines[0].pv)"
              >
                play {{ pvSan(node.fen, cloudLines[0]!.pv)[0]?.san }}
              </button>
            </template>
            <template v-else>{{
              cloudBusy
                ? 'Asking Lichess cloud…'
                : cloudError || 'Lichess cloud has no evaluation of this position.'
            }}</template>
          </p>
          <div v-if="!engineReady && !usingCloud" class="text-xs muted">
            <UButton
              size="xs"
              variant="link"
              color="neutral"
              class="px-0"
              @click="store.selectPage('settings')"
              >Set up Stockfish</UButton
            >
            to see evaluations.
          </div>
          <ol
            v-else-if="assistanceAllowed && (engineOn || usingCloud) && !gameOver"
            class="pv-list"
          >
            <li v-for="(entry, index) in pvMoves" :key="index" class="pv-row">
              <button
                type="button"
                class="pv-eval"
                :title="`Play ${entry.moves[0]?.san ?? ''}`"
                @click="playLine(entry.line.pv)"
              >
                {{ formatEval(entry.line) }}
              </button>
              <span class="pv-moves">
                <button
                  v-for="(pvMove, moveIndex) in entry.moves"
                  :key="moveIndex"
                  type="button"
                  class="pv-move"
                  @click="playLine(entry.line.pv, moveIndex)"
                  @mouseenter="showPreview($event, index, moveIndex)"
                  @mouseleave="hidePreview"
                  @focus="showPreview($event, index, moveIndex)"
                  @blur="hidePreview"
                >
                  {{ pvMove.label }}
                </button>
              </span>
            </li>
            <li
              v-for="index in usingCloud ? 0 : Math.max(0, engineLines - pvMoves.length)"
              :key="`wait-${index}`"
              class="pv-row pending"
            >
              <span class="pv-eval">…</span>
            </li>
          </ol>
          <Teleport to="body">
            <div v-if="preview" class="pv-preview" :style="preview.style" aria-hidden="true">
              <ChessBoard
                :fen="preview.fen"
                :orientation="orientation"
                :theme="settings?.boardTheme"
                coordinates="none"
                :piece-set="settings?.pieceSet"
                animation="none"
                :interactive="false"
                :last-move="preview.lastMove"
              />
            </div>
          </Teleport>
        </section>

        <div class="panel-tabs" role="tablist" aria-label="Analysis tools">
          <button
            v-for="entry in toolTabs"
            :key="entry.value"
            type="button"
            role="tab"
            class="panel-tab"
            :class="{ active: tool === entry.value }"
            :aria-selected="tool === entry.value"
            @click="tool = entry.value"
          >
            <UIcon :name="entry.icon" />{{ entry.label }}
            <span v-if="entry.badge" class="tab-badge">{{ entry.badge }}</span>
          </button>
        </div>

        <div v-show="tool === 'moves'" class="tool-body moves-tool" role="tabpanel">
          <UContextMenu :items="moveMenu">
            <div class="moves-wrap" @contextmenu.capture="moveContext">
              <div ref="treeList" class="tree-list" role="list" aria-label="Moves">
                <AnalysisLine
                  v-if="hasMoves"
                  :parent="root"
                  parent-path=""
                  :current="path"
                  :depth="0"
                  @select="analysis.goTo"
                />
                <div v-else class="moves-empty">
                  Make a move on the board, or import a game. Variations are kept as you explore.
                </div>
              </div>
            </div>
          </UContextMenu>
          <MoveNotes ref="notes" />
        </div>
        <div v-if="tool === 'review'" class="tool-body tool-scroll" role="tabpanel">
          <AnalysisReview v-if="assistanceAllowed && hasMoves" />
          <p v-else-if="!assistanceAllowed" class="tool-empty">
            Engine assistance is paused until Lichess game status is verified.
          </p>
          <p v-else class="tool-empty">Make some moves or import a game to review it.</p>
        </div>
        <div v-if="tool === 'explorer'" class="tool-body tool-scroll" role="tabpanel">
          <PositionExplorer v-if="assistanceAllowed" />
          <p v-else class="tool-empty">
            Engine assistance is paused until Lichess game status is verified.
          </p>
        </div>

        <div class="move-controls">
          <UTooltip text="Go to start" :kbds="['↑']">
            <UButton
              size="sm"
              variant="ghost"
              color="neutral"
              icon="i-lucide-skip-back"
              aria-label="Go to start"
              :disabled="!path"
              @click="analysis.toStart"
            />
          </UTooltip>
          <UTooltip text="Previous move" :kbds="['←']">
            <UButton
              size="sm"
              variant="ghost"
              color="neutral"
              icon="i-lucide-chevron-left"
              aria-label="Previous move"
              :disabled="!path"
              @click="analysis.back"
            />
          </UTooltip>
          <UTooltip text="Next move" :kbds="['→']">
            <UButton
              size="sm"
              variant="ghost"
              color="neutral"
              icon="i-lucide-chevron-right"
              aria-label="Next move"
              :disabled="!node.children.length"
              @click="analysis.forward"
            />
          </UTooltip>
          <UTooltip text="Go to end of line" :kbds="['↓']">
            <UButton
              size="sm"
              variant="ghost"
              color="neutral"
              icon="i-lucide-skip-forward"
              aria-label="Go to end of line"
              :disabled="!node.children.length"
              @click="analysis.toEnd"
            />
          </UTooltip>
          <UTooltip text="Flip board" :kbds="['F']">
            <UButton
              size="sm"
              variant="ghost"
              color="neutral"
              icon="i-lucide-arrow-up-down"
              aria-label="Flip board"
              @click="flip"
            />
          </UTooltip>
          <ExportGame
            :setup="{ variant: 'standard', fen: root.fen }"
            :moves="analysis.mainline"
            :orientation="orientation"
            :white="root.headers?.White || analysis.origin?.white || 'White'"
            :black="root.headers?.Black || analysis.origin?.black || 'Black'"
            :ply="Math.min(path ? path.split(' ').length : 0, analysis.mainline.length)"
          />
          <UTooltip text="Import a PGN or FEN">
            <UButton
              size="sm"
              variant="ghost"
              color="neutral"
              icon="i-lucide-import"
              aria-label="Import"
              @click="openImport"
            />
          </UTooltip>
          <UDropdownMenu :items="menu">
            <UButton
              size="sm"
              variant="ghost"
              color="neutral"
              icon="i-lucide-ellipsis"
              aria-label="More analysis actions"
            />
          </UDropdownMenu>
        </div>
      </div>
    </div>

    <UModal
      v-model:open="importOpen"
      title="Import a position or game"
      description="Paste a FEN, or a PGN with or without variations."
    >
      <template #body>
        <form class="flex flex-col gap-3" @submit.prevent="runImport">
          <textarea
            v-model="importText"
            class="import-text"
            rows="8"
            spellcheck="false"
            aria-label="FEN or PGN"
            placeholder="rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
          />
          <span v-if="importError" class="text-sm text-error">{{ importError }}</span>
          <div class="flex justify-end gap-2">
            <UButton color="neutral" variant="ghost" @click="importOpen = false">Cancel</UButton>
            <UButton type="submit" :disabled="!importText.trim()">Import</UButton>
          </div>
        </form>
      </template>
    </UModal>
  </div>
</template>

<style scoped>
.analysis-layout {
  --board-chrome: 212px;
  /* The evaluation bar sits beside the board and takes from its width. */
  --board-size: clamp(260px, min(100dvh - var(--board-chrome), 100cqw - 372px), 760px);
  grid-template-columns: calc(var(--board-size) + 22px) minmax(300px, 440px);
}
.analysis-board {
  display: flex;
  gap: 8px;
}
.analysis-eval {
  flex: none;
  width: 14px;
  height: auto;
}
.analysis-board .board-shell {
  flex: 1;
  min-width: 0;
}
.ceval {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.ceval-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.engine-settings {
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: 260px;
  padding: 14px;
}
/* The panel is as tall as the board beside it, never taller: the active tool scrolls inside it. */
.analysis-panel {
  contain: size;
  gap: 12px;
}
.analysis-panel > * + * {
  margin-top: 0;
}
.panel-tabs {
  display: flex;
  flex: none;
  gap: 2px;
  padding: 3px;
  border-radius: 9px;
  background: var(--ui-bg-accented);
}
.panel-tab {
  display: inline-flex;
  flex: 1;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 5px 8px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--ui-text-muted);
  font: inherit;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}
.panel-tab:hover {
  color: var(--ui-text);
}
.panel-tab.active {
  background: var(--ui-bg);
  color: var(--ui-text-highlighted);
}
.tab-badge {
  padding: 0 5px;
  border-radius: 99px;
  background: color-mix(in srgb, var(--ui-primary) 25%, transparent);
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}
.tool-body {
  display: flex;
  flex: 1 1 0;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
}
.tool-scroll {
  overflow-y: auto;
  padding-right: 4px;
}
.tool-empty {
  padding: 24px 8px;
  color: var(--ui-text-muted);
  font-size: 13px;
  text-align: center;
}
.moves-tool .moves-wrap {
  min-height: 140px;
}
.ceval-score {
  min-width: 3.4em;
  font-size: 22px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}
.ceval-score.dim {
  color: var(--ui-text-dimmed);
}
.ceval-info {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  line-height: 1.25;
}
.ceval-name {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-size: 12px;
  font-weight: 600;
}
.ceval-status {
  display: flex;
  align-items: center;
  gap: 4px;
  overflow: hidden;
  color: var(--ui-text-muted);
  font-size: 11px;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.pv-list {
  margin: 0;
  padding: 0;
  list-style: none;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg);
  overflow: hidden;
}
.pv-row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-height: 30px;
  padding: 5px 8px;
  font-size: 12.5px;
}
.pv-row + .pv-row {
  border-top: 1px solid var(--ui-border);
}
.pv-eval {
  flex: none;
  min-width: 3.4em;
  padding: 1px 6px;
  border: 0;
  border-radius: 6px;
  background: var(--ui-bg-accented);
  color: inherit;
  font: inherit;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  text-align: center;
  cursor: pointer;
}
.pv-row.pending .pv-eval {
  cursor: default;
  color: var(--ui-text-dimmed);
}
.pv-moves {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.pv-move {
  padding: 1px 2px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--ui-text-toned);
  font: inherit;
  cursor: pointer;
}
.pv-move:hover,
.pv-eval:hover {
  background: color-mix(in srgb, var(--ui-primary) 24%, transparent);
}
.pv-preview {
  position: fixed;
  z-index: 60;
  padding: 4px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg);
  pointer-events: none;
}
.tree-list {
  position: absolute;
  inset: 0;
  overflow-y: auto;
  padding: 8px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg);
  font-size: 13px;
  line-height: 1.9;
}
.import-text {
  width: 100%;
  padding: 10px;
  border: 1px solid var(--ui-border);
  border-radius: 8px;
  background: var(--ui-bg);
  color: inherit;
  font:
    12px/1.5 ui-monospace,
    SFMono-Regular,
    Menlo,
    monospace;
  resize: vertical;
}
@container page (max-width: 620px) {
  .analysis-panel {
    contain: none;
  }
  .tool-body {
    flex: none;
    max-height: 420px;
  }
  .analysis-layout {
    --board-size: min(100cqw - 22px, 100dvh - 260px);
    grid-template-columns: minmax(0, calc(var(--board-size) + 22px));
  }
}
@media (max-width: 900px) and (orientation: portrait) {
  .analysis-panel {
    contain: none;
  }
  .tool-body {
    flex: none;
    max-height: 420px;
  }
  .analysis-layout {
    --board-size: min(100cqw - 22px, max(260px, 100dvh - 260px), 760px);
    grid-template-columns: minmax(0, calc(var(--board-size) + 22px));
  }
}
</style>

<style>
/* The move tree (AnalysisLine renders recursively, so these are not scoped). */
.tree-move {
  display: inline;
  padding: 2px 4px;
  margin: 0 1px;
  border: 0;
  border-radius: 5px;
  background: transparent;
  color: var(--ui-text-toned);
  font: inherit;
  font-variant-numeric: tabular-nums;
  cursor: pointer;
}
.tree-move.main {
  color: var(--ui-text);
  font-weight: 600;
}
.tree-move:hover {
  background: var(--ui-bg-accented);
}
.tree-move.active {
  background: color-mix(in srgb, var(--ui-primary) 30%, transparent);
  color: var(--ui-text-highlighted);
}
.tree-number {
  margin-right: 2px;
  color: var(--ui-text-dimmed);
  font-weight: 400;
}
.tree-glyph {
  margin-left: 1px;
  font-weight: 700;
}
.tree-glyph.inaccuracy {
  color: var(--judgment-inaccuracy);
}
.tree-glyph.mistake {
  color: var(--judgment-mistake);
}
.tree-glyph.blunder {
  color: var(--judgment-blunder);
}
.tree-variations {
  margin: 2px 0 4px 10px;
  padding-left: 8px;
  border-left: 2px solid var(--ui-border);
  font-size: 12.5px;
}
.tree-variations > .tree-variation {
  display: block;
}
.tree-paren {
  color: var(--ui-text-dimmed);
}
/* Comments run on after their move, like Lichess; main-line ones get their own line. */
.tree-comment {
  margin: 0 4px;
  color: var(--ui-text-muted);
  font-size: 12.5px;
  font-style: italic;
  line-height: 1.5;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  cursor: pointer;
}
.tree-comment.block {
  display: block;
  margin: 2px 0 4px;
  padding: 4px 8px;
  border-left: 2px solid color-mix(in srgb, var(--ui-primary) 50%, transparent);
  border-radius: 0 6px 6px 0;
  background: var(--ui-bg-elevated);
}
</style>
