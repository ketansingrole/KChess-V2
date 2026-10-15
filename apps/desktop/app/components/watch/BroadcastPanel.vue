<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useDocumentVisibility, useIntervalFn, useLocalStorage } from '@vueuse/core'
import { storeToRefs } from 'pinia'
import type { DrawShape } from '@lichess-org/chessground/draw'
import type { Key } from '@lichess-org/chessground/types'
import { useBroadcastEvaluations } from '../../composables/useBroadcastEvaluations'
import { formatEval } from '@kchess/rules/analysisTree'
import { RequestScope } from '@kchess/rules/requestScope'
import type { BroadcastGame, BroadcastSummary, WatchPlayer } from '@kchess/contracts/types'
import {
  lastMoveKeys,
  setupPositionAfter,
  setupSanHistory,
  fen as fenOf,
} from '@kchess/rules/chess'
import { formatClock } from '@kchess/rules/clock'
import { useWatchStore } from '../../stores/watch'
import { useAnalysisStore } from '../../stores/analysis'
import { useKChessStore } from '../../stores/kchess'

defineProps<{
  online: boolean
  playingOwnGame: boolean
}>()

const emit = defineEmits<{
  'select-game': [id: string]
  'close-game': []
  'leave-tour': []
  'open-round': [id: string]
}>()

const store = useKChessStore()
const watcher = useWatchStore()
const analysis = useAnalysisStore()
const { settings, onlinePhase } = storeToRefs(store)

const now = ref(performance.now())
useIntervalFn(() => (now.value = performance.now()), 500)

function label(player: WatchPlayer): string {
  return `${player.title ? `${player.title} ` : ''}${player.name}${player.rating ? ` (${player.rating})` : ''}`
}

/* ── Broadcasts ───────────────────────────────────────────────────────── */
const broadcastQuery = ref('')
const broadcastResults = ref<BroadcastSummary[]>([])
const broadcastSearching = ref(false)
const broadcastSearchError = ref('')
const broadcastRequests = new RequestScope()
let broadcastSearchTimer: ReturnType<typeof setTimeout> | undefined
watch(broadcastQuery, (query) => {
  clearTimeout(broadcastSearchTimer)
  const request = broadcastRequests.next()
  broadcastSearchError.value = ''
  broadcastResults.value = []
  broadcastSearching.value = Boolean(query.trim())
  if (!query.trim()) return
  broadcastSearchTimer = setTimeout(async () => {
    try {
      const found = await window.kchess.broadcasts(query.trim())
      if (request.current()) broadcastResults.value = found
    } catch (cause) {
      if (request.current()) {
        console.warn('[watch] Broadcast search failed:', cause)
        broadcastSearchError.value = cause instanceof Error ? cause.message : String(cause)
      }
    } finally {
      if (request.current()) broadcastSearching.value = false
    }
  }, 350)
})
onUnmounted(() => {
  clearTimeout(broadcastSearchTimer)
  broadcastRequests.invalidate()
})
const sections = computed(() => {
  if (broadcastQuery.value.trim())
    return [{ key: 'search', label: 'Search results', items: broadcastResults.value }]
  const list = watcher.broadcastList ?? []
  return [
    { key: 'active', label: 'Live now', items: list.filter((b) => b.section === 'active') },
    { key: 'upcoming', label: 'Coming up', items: list.filter((b) => b.section === 'upcoming') },
    { key: 'past', label: 'Recently finished', items: list.filter((b) => b.section === 'past') },
  ].filter((section) => section.items.length)
})
const roundItems = computed(
  () =>
    watcher.tour?.rounds.map((round) => ({
      label: `${round.name}${round.ongoing ? ' · live' : round.finished ? '' : ' · upcoming'}`,
      value: round.id,
    })) ?? [],
)
const selectedRound = computed({
  get: () => watcher.roundId,
  set: (id: string) => {
    emit('open-round', id)
  },
})
const game = computed(() => watcher.currentGame)
const gamePly = ref<number | null>(null)
const variationBase = ref<string[] | null>(null)
const variationMoves = ref<string[]>([])
watch(
  () => watcher.selectedGame,
  () => {
    gamePly.value = null
    returnToBroadcast()
  },
)
const gameSetup = computed(() => ({
  variant: 'standard' as const,
  fen: game.value?.startFen ?? '',
}))
const gameSan = computed(() =>
  game.value ? setupSanHistory(gameSetup.value, game.value.moves) : [],
)
const shownPly = computed(() => Math.min(gamePly.value ?? Infinity, game.value?.moves.length ?? 0))
const gameFen = computed(() =>
  variationBase.value
    ? fenOf(setupPositionAfter(gameSetup.value, [...variationBase.value, ...variationMoves.value]))
    : game.value
      ? shownPly.value === game.value.moves.length
        ? game.value.fen
        : fenOf(setupPositionAfter(gameSetup.value, game.value.moves.slice(0, shownPly.value)))
      : '',
)
const gameOrientation = ref<'white' | 'black'>('white')
const broadcastRetry = ref(0)
const broadcastEngineOn = useLocalStorage('kchess:broadcast-engine', false)
const broadcastEngineLines = useLocalStorage('kchess:broadcast-engine-lines', 3)
const broadcastInfinite = useLocalStorage('kchess:broadcast-engine-infinite', false)
const broadcastArrows = useLocalStorage('kchess:broadcast-engine-arrows', true)
const broadcastBar = useLocalStorage('kchess:broadcast-engine-bar', true)
const gridEvaluations = useLocalStorage('kchess:broadcast-grid-evaluations', true)
const visibility = useDocumentVisibility()
const broadcastAssistance = computed(
  () => !['playing', 'seeking', 'disconnected'].includes(onlinePhase.value),
)
const broadcastHistory = computed(() =>
  variationBase.value
    ? [...variationBase.value, ...variationMoves.value]
    : (game.value?.moves.slice(0, shownPly.value) ?? []),
)
const broadcastTerminal = computed(() => {
  if (!game.value) return ''
  const position = setupPositionAfter(gameSetup.value, broadcastHistory.value)
  if (position.isCheckmate()) return position.turn === 'white' ? '0-1' : '1-0'
  if (position.isStalemate() || position.isInsufficientMaterial()) return '½-½'
  return ''
})
const broadcastEngineActive = computed(
  () =>
    broadcastAssistance.value &&
    Boolean(game.value) &&
    broadcastEngineOn.value &&
    store.engineReady,
)
const {
  update: broadcastEval,
  error: broadcastEngineError,
  busy: broadcastEngineBusy,
  evaluation: gridEvaluation,
  unavailable: gridEvaluationUnavailable,
} = useBroadcastEvaluations(
  () => {
    void broadcastRetry.value
    return broadcastEngineActive.value && !broadcastTerminal.value
      ? {
          fen: gameFen.value,
          rootFen: gameSetup.value.fen,
          moves: broadcastHistory.value,
          lines: Math.max(1, Math.min(5, broadcastEngineLines.value)),
          infinite: broadcastInfinite.value,
        }
      : null
  },
  () =>
    watcher.tour &&
    !game.value &&
    gridEvaluations.value &&
    broadcastAssistance.value &&
    store.engineReady &&
    visibility.value === 'visible'
      ? watcher.roundGames
      : [],
  () => settings.value.enginePath,
)
const broadcastBest = computed(() =>
  broadcastEngineActive.value ? broadcastEval.value?.lines[0] : undefined,
)
const broadcastShapes = computed<DrawShape[]>(() =>
  !broadcastEngineActive.value || !broadcastArrows.value
    ? []
    : (broadcastEval.value?.lines ?? []).flatMap((line, index) => {
        const move = line.pv[0]
        if (!move || move.length < 4) return []
        return [
          {
            orig: move.slice(0, 2) as Key,
            dest: move.slice(2, 4) as Key,
            brush: index === 0 ? 'paleBlue' : 'paleGrey',
          },
        ]
      }),
)
const broadcastEngineStatus = computed(() => {
  if (!broadcastAssistance.value)
    return store.onlineConnection?.gameId || onlinePhase.value === 'playing'
      ? 'Analysis paused during your game'
      : 'Game status not verified'
  if (!store.engineReady) return 'Engine unavailable'
  if (broadcastEngineError.value) return broadcastEngineError.value
  if (!broadcastEngineOn.value) return 'Off'
  if (broadcastTerminal.value)
    return broadcastTerminal.value === '½-½' ? 'Drawn position' : 'Checkmate'
  if (!broadcastEval.value) return 'Starting…'
  return `Depth ${broadcastEval.value.depth}${broadcastEngineBusy.value ? '…' : ''}`
})
function returnToBroadcast(): void {
  variationBase.value = null
  variationMoves.value = []
}
function selectBroadcastPly(ply: number): void {
  returnToBroadcast()
  gamePly.value = ply === game.value?.moves.length ? null : ply
}
function previewBroadcastLine(moves: string[]): void {
  if (!broadcastAssistance.value) return
  if (!variationBase.value) variationBase.value = game.value?.moves.slice(0, shownPly.value) ?? []
  variationMoves.value = [...variationMoves.value, ...moves]
}
watch(gamePly, returnToBroadcast)
watch(broadcastAssistance, (allowed) => {
  if (!allowed) returnToBroadcast()
})

function boardPlayers(entry: BroadcastGame) {
  const whiteClock = watcher.broadcastClock(entry, 'white', now.value)
  const blackClock = watcher.broadcastClock(entry, 'black', now.value)
  const white = {
    name: label(entry.white),
    icon: 'i-lucide-user',
    clock: whiteClock !== undefined ? formatClock(whiteClock) : undefined,
  }
  const black = {
    name: label(entry.black),
    icon: 'i-lucide-user',
    clock: blackClock !== undefined ? formatClock(blackClock) : undefined,
  }
  return gameOrientation.value === 'white'
    ? { top: black, bottom: white }
    : { top: white, bottom: black }
}
function resultText(entry: BroadcastGame): string {
  if (entry.ongoing) return 'Live'
  if (entry.result === '1-0') return 'White won · 1–0'
  if (entry.result === '0-1') return 'Black won · 0–1'
  if (entry.result === '1/2-1/2') return 'Draw · ½–½'
  return entry.result
}
function analyse(entry: BroadcastGame): void {
  if (!analysis.loadPgn(entry.pgn, shownPly.value)) return
  analysis.orientation = gameOrientation.value
  analysis.origin = { white: entry.white.name, black: entry.black.name }
  store.selectPage('analysis')
}
/** Arrow keys step through the open broadcast game. */
function keydown(event: KeyboardEvent): void {
  const target = event.target as HTMLElement | null
  if (
    target?.closest(
      'input, textarea, select, [contenteditable="true"], [role="combobox"], [role="dialog"]',
    ) ||
    event.metaKey ||
    event.ctrlKey ||
    event.altKey
  )
    return
  if (!game.value) return
  if (event.key.toLowerCase() === 'l' && store.engineReady && broadcastAssistance.value) {
    event.preventDefault()
    broadcastEngineOn.value = !broadcastEngineOn.value
    return
  }
  if (event.key.toLowerCase() === 'f') {
    event.preventDefault()
    gameOrientation.value = gameOrientation.value === 'white' ? 'black' : 'white'
    return
  }
  const max = game.value.moves.length
  const at = shownPly.value
  const next =
    event.key === 'ArrowLeft'
      ? at - 1
      : event.key === 'ArrowRight'
        ? at + 1
        : event.key === 'ArrowUp'
          ? 0
          : event.key === 'ArrowDown'
            ? max
            : undefined
  if (next === undefined) return
  event.preventDefault()
  selectBroadcastPly(Math.max(0, Math.min(max, next)))
}
onMounted(() => window.addEventListener('keydown', keydown))
onUnmounted(() => window.removeEventListener('keydown', keydown))
</script>

<template>
  <div>
    <div v-if="!watcher.tour" class="flex flex-col gap-5">
      <div class="broadcast-directory-header">
        <h1 class="text-2xl font-semibold">Tournament broadcasts</h1>
        <UInput
          v-model="broadcastQuery"
          icon="i-lucide-search"
          placeholder="Search broadcasts"
          aria-label="Search broadcasts"
          :maxlength="100"
          class="broadcast-search"
        />
      </div>
      <p v-if="broadcastSearchError" role="alert" class="text-error">
        {{ broadcastSearchError }}
      </p>
      <p
        v-if="broadcastSearching || (!broadcastQuery.trim() && watcher.broadcastLoading)"
        role="status"
        class="muted text-sm"
      >
        {{ broadcastSearching ? 'Searching…' : 'Loading broadcasts…' }}
      </p>
      <p
        v-else-if="broadcastQuery.trim() && !broadcastResults.length && !broadcastSearchError"
        role="status"
        class="muted"
      >
        No broadcasts found.
      </p>
      <div
        v-if="!broadcastQuery.trim() && watcher.broadcastError"
        role="alert"
        class="flex items-center gap-3 text-sm"
      >
        <span class="text-error">{{ watcher.broadcastError }}</span>
        <UButton
          size="sm"
          variant="outline"
          :disabled="!online || watcher.broadcastLoading"
          @click="watcher.loadBroadcasts()"
          >Retry broadcasts</UButton
        >
      </div>
      <section v-for="section in sections" :key="section.key">
        <h2 v-if="section.items.length" class="section-title mb-3">{{ section.label }}</h2>
        <div class="broadcast-directory-grid">
          <BroadcastCard
            v-for="item in section.items"
            :key="item.tourId"
            :item="item"
            :disabled="playingOwnGame || !online"
            @select="watcher.openTour(item.tourId)"
          />
        </div>
      </section>
    </div>
    <div
      v-else
      v-board-fit
      :class="game ? 'play-layout broadcast-layout analysis-surface' : 'flex flex-col gap-4'"
    >
      <!-- In the page header's free space beside the tabs, not a row of its own. -->
      <Teleport defer to="[data-watch-lead]">
        <div class="broadcast-toolbar">
          <UButton
            variant="ghost"
            color="neutral"
            icon="i-lucide-arrow-left"
            aria-label="All broadcasts"
            class="-ms-2"
            @click="emit('leave-tour')"
          />
          <h2 class="section-title flex-1 truncate">{{ watcher.tour.name }}</h2>
          <USwitch
            v-if="!game"
            v-model="gridEvaluations"
            label="Evaluation bars"
            :disabled="!store.engineReady || !broadcastAssistance"
          />
          <USelect
            v-model="selectedRound"
            :items="roundItems"
            :placeholder="roundItems.length ? 'Choose a round' : 'No rounds yet'"
            :disabled="!roundItems.length"
            aria-label="Round"
            class="w-44"
          />
        </div>
      </Teleport>
      <p v-if="watcher.roundEnded && !watcher.roundGames.length" class="muted text-sm">
        No games in this round yet.
      </p>
      <template v-if="game">
        <div class="board-stack">
          <PlayerLine
            :player="boardPlayers(game).top"
            :material-fen="gameFen"
            :color="gameOrientation === 'white' ? 'black' : 'white'"
            :active="
              watcher.broadcastClockRunning(game, gameOrientation === 'white' ? 'black' : 'white')
            "
          />
          <div
            class="broadcast-board"
            :class="{ 'with-eval': broadcastEngineActive && broadcastBar }"
          >
            <EvalBar
              v-if="broadcastEngineActive && broadcastBar"
              :line="broadcastBest"
              :result="broadcastTerminal"
              :orientation="gameOrientation"
            />
            <ChessBoard
              :fen="gameFen"
              :orientation="gameOrientation"
              :theme="settings.boardTheme"
              :coordinates="settings.coordinates"
              :piece-set="settings.pieceSet"
              :animation="settings.pieceAnimation"
              :interactive="false"
              :last-move="lastMoveKeys(broadcastHistory)"
              :shapes="broadcastShapes"
            />
          </div>
          <PlayerLine
            :player="boardPlayers(game).bottom"
            :material-fen="gameFen"
            :color="gameOrientation"
            :active="watcher.broadcastClockRunning(game, gameOrientation)"
          />
        </div>
        <MovePanel
          :moves="gameSan"
          :ply="shownPly"
          empty-text="No moves yet."
          @select="selectBroadcastPly"
          @flip="gameOrientation = gameOrientation === 'white' ? 'black' : 'white'"
        >
          <template #top>
            <section class="broadcast-engine" aria-label="Broadcast engine analysis">
              <div class="broadcast-engine-head">
                <USwitch
                  v-model="broadcastEngineOn"
                  aria-label="Broadcast engine analysis"
                  title="Toggle engine (L)"
                  :disabled="!store.engineReady || !broadcastAssistance"
                />
                <strong>{{
                  broadcastEngineActive ? broadcastTerminal || formatEval(broadcastBest) : '—'
                }}</strong>
                <div class="min-w-0 flex-1 text-sm">
                  <span>{{ store.engineName || 'Stockfish' }}</span>
                  <div class="muted text-xs" :class="{ 'text-error': broadcastEngineError }">
                    {{ broadcastEngineStatus }}
                  </div>
                </div>
                <UPopover :content="{ align: 'end' }">
                  <UButton
                    icon="i-lucide-settings-2"
                    variant="ghost"
                    color="neutral"
                    aria-label="Broadcast analysis settings"
                  />
                  <template #content>
                    <div class="broadcast-engine-settings">
                      <UFormField label="Lines"
                        ><USelect
                          v-model="broadcastEngineLines"
                          :items="[1, 2, 3, 4, 5]"
                          aria-label="Broadcast engine lines"
                          class="w-full"
                      /></UFormField>
                      <USwitch v-model="broadcastBar" label="Evaluation bar" />
                      <USwitch v-model="broadcastArrows" label="Best move arrows" />
                      <USwitch v-model="broadcastInfinite" label="Infinite analysis" />
                    </div>
                  </template>
                </UPopover>
              </div>
              <UButton
                v-if="broadcastEngineError && broadcastEngineActive"
                size="sm"
                variant="outline"
                icon="i-lucide-refresh-cw"
                @click="broadcastRetry++"
                >Retry engine</UButton
              >
              <EngineLines
                v-if="broadcastEngineActive && !broadcastTerminal"
                :fen="gameFen"
                :lines="broadcastEval?.lines ?? []"
                :orientation="gameOrientation"
                :board-theme="settings.boardTheme"
                :piece-set="settings.pieceSet"
                :pending="broadcastEngineLines"
                action-label="Explore"
                @select="(pv, at) => previewBroadcastLine(pv.slice(0, at + 1))"
              />
              <UButton
                v-if="variationBase"
                size="sm"
                variant="soft"
                icon="i-lucide-undo-2"
                @click="returnToBroadcast"
                >Return to game</UButton
              >
            </section>
            <div class="status-banner" role="status">
              <UIcon
                :name="game.ongoing ? 'i-lucide-radio' : 'i-lucide-flag'"
                class="status-icon"
              />
              <div class="banner-text">
                <strong>{{ resultText(game) }}</strong>
                <OpeningName
                  class="detail"
                  :setup="gameSetup"
                  :moves="game.moves"
                  :ply="shownPly"
                />
              </div>
            </div>
          </template>
          <template #bottom>
            <div class="panel-actions panel-divider pt-3.5">
              <UButton
                variant="outline"
                color="neutral"
                icon="i-lucide-layout-grid"
                @click="emit('close-game')"
                >All boards</UButton
              >
              <UButton
                variant="outline"
                color="neutral"
                icon="i-lucide-microscope"
                @click="analyse(game)"
                >Analyse</UButton
              >
            </div>
          </template>
        </MovePanel>
      </template>
      <div v-else class="broadcast-grid">
        <button
          v-for="entry in watcher.roundGames"
          :key="entry.id"
          type="button"
          class="broadcast-cell"
          @click="emit('select-game', entry.id)"
        >
          <div class="text-xs truncate">{{ label(entry.black) }}</div>
          <div
            class="broadcast-mini-board"
            :class="{ 'with-eval': gridEvaluations && broadcastAssistance }"
          >
            <EvalBar
              v-if="gridEvaluations && broadcastAssistance"
              orientation="white"
              :line="gridEvaluation(entry)"
              :result="entry.ongoing ? undefined : entry.result"
            />
            <ChessBoard
              :fen="entry.fen"
              orientation="white"
              :theme="settings.boardTheme"
              coordinates="none"
              :piece-set="settings.pieceSet"
              animation="none"
              :interactive="false"
              :last-move="lastMoveKeys(entry.moves)"
            />
          </div>
          <div class="text-xs truncate">{{ label(entry.white) }}</div>
          <div class="broadcast-mini-status text-xs muted">
            <span>{{ entry.ongoing ? 'Live' : entry.result }}</span>
            <span
              v-if="gridEvaluations && broadcastAssistance && entry.ongoing"
              class="tabular"
              :title="
                gridEvaluation(entry)
                  ? `Stockfish · depth ${gridEvaluation(entry)!.depth}`
                  : gridEvaluationUnavailable(entry)
                    ? 'Evaluation unavailable'
                    : 'Waiting for evaluation'
              "
              >{{
                gridEvaluationUnavailable(entry) ? '—' : formatEval(gridEvaluation(entry))
              }}</span
            >
          </div>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.broadcast-board {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 8px;
  padding-left: 22px;
}
.broadcast-mini-board {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 6px;
}
.broadcast-mini-board.with-eval {
  grid-template-columns: 8px minmax(0, 1fr);
}
.broadcast-mini-status {
  display: flex;
  justify-content: space-between;
}
.broadcast-board.with-eval {
  padding-left: 0;
  grid-template-columns: 14px minmax(0, 1fr);
}
.broadcast-engine {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-bottom: 14px;
}
.broadcast-engine-head {
  display: flex;
  align-items: center;
  gap: 10px;
}
.broadcast-engine-settings {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 16px;
  width: 230px;
}
.broadcast-directory-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 16px;
}
.broadcast-search {
  width: 300px;
  max-width: 100%;
}
.broadcast-directory-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 280px), 1fr));
  gap: 20px;
}
/* While a broadcast game is open the whole page, header tabs included, is as wide as the
   board and panel below it, so every right-hand control shares one edge. */
.broadcast-layout {
  /* The tour toolbar sits above the board and shares the grid's outer edges. */
  row-gap: 16px;
}
.broadcast-toolbar {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.broadcast-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
  gap: 14px;
}
.broadcast-cell {
  display: flex;
  flex-direction: column;
  gap: 4px;
  text-align: left;
  padding: 8px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg-elevated);
}
.broadcast-cell :deep(.cg-host) {
  pointer-events: none;
}
</style>
