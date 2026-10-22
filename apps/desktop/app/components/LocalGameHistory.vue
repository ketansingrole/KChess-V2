<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useGameArchiveStore, type ArchivedGame } from '../stores/gameArchive'
import { useAnalysisStore } from '../stores/analysis'
import { setupPgn } from '@kchess/rules/chess'
import { useReplaceAnalysis } from '../composables/useReplaceAnalysis'
import {
  archivedOutcome,
  archivedPlayerSide,
  archivedResultBadge,
  formatGameDate,
  moveCount,
  type ArchivedOutcome,
} from '../utils/games'

const props = defineProps<{ source: 'computer' | 'board'; openGame?: string | null }>()
const archive = useGameArchiveStore()
const store = useKChessStore()
const analysis = useAnalysisStore()
const selected = ref<ArchivedGame | null>(null)
const page = ref(1)
const pendingDelete = ref<ArchivedGame | null>(null)
const deleteOpen = computed({
  get: () => Boolean(pendingDelete.value),
  set: (open: boolean) => {
    if (!open) pendingDelete.value = null
  },
})
function deleteGame(): void {
  if (pendingDelete.value) void archive.remove(pendingDelete.value.id)
  pendingDelete.value = null
  page.value = Math.max(1, Math.min(page.value, Math.ceil(shown.value.length / 20)))
}
const games = computed(() =>
  archive.games
    .filter((g) =>
      props.source === 'computer' ? g.source === 'computer' : g.source !== 'computer',
    )
    .sort((a, b) => b.startedAt - a.startedAt),
)
type ResultFilter = 'all' | 'win' | 'loss' | 'draw' | 'white' | 'black' | 'unfinished'
const resultFilter = ref<ResultFilter>('all')
/** The filter a game falls under; clock-only sessions show only under All. */
function filterOf(outcome: ArchivedOutcome): ResultFilter | null {
  if (outcome === 'ongoing') return 'unfinished'
  return outcome === 'clock' ? null : outcome
}
const outcomeCounts = computed(() => {
  const counts = new Map<ResultFilter, number>()
  for (const game of games.value) {
    const key = filterOf(archivedOutcome(game))
    if (key) counts.set(key, (counts.get(key) ?? 0) + 1)
  }
  return counts
})
const resultItems = computed(() =>
  (
    [
      ['all', 'All'],
      ...(props.source === 'computer'
        ? [
            ['win', 'Wins'],
            ['loss', 'Losses'],
          ]
        : [
            ['white', 'White won'],
            ['black', 'Black won'],
          ]),
      ['draw', 'Draws'],
      ['unfinished', 'Unfinished'],
    ] as [ResultFilter, string][]
  ).map(([value, label]) => {
    const count = value === 'all' ? games.value.length : (outcomeCounts.value.get(value) ?? 0)
    return {
      value,
      label,
      // An empty filter cannot lead anywhere; the selected one stays reachable.
      disabled: !count && resultFilter.value !== value,
      badge: { label: String(count), color: 'neutral' as const, variant: 'soft' as const },
    }
  }),
)
const shown = computed(() =>
  resultFilter.value === 'all'
    ? games.value
    : games.value.filter((game) => filterOf(archivedOutcome(game)) === resultFilter.value),
)
watch(resultFilter, () => {
  page.value = 1
})
const visible = computed(() => shown.value.slice((page.value - 1) * 20, page.value * 20))
/** What the last swipe-back undid, while nothing else changed since. */
const swipeRedo = ref<ArchivedGame | null>(null)
/** A swipe steps out of the open game before leaving the page. */
const hasDetail = computed(() => selected.value !== null)
function goBack(): void {
  swipeRedo.value = selected.value
  selected.value = null
}
const hasRedo = computed(() => {
  const redo = swipeRedo.value
  return redo !== null && selected.value === null && games.value.some((game) => game.id === redo.id)
})
function goForward(): void {
  if (!hasRedo.value) return
  selected.value = swipeRedo.value
  swipeRedo.value = null
}
/** Manual drill actions drop the swipe's redo; only a swipe-back sets it. */
function selectGame(game: ArchivedGame): void {
  swipeRedo.value = null
  selected.value = game
}
function closeGame(): void {
  swipeRedo.value = null
  selected.value = null
}
defineExpose({ hasDetail, goBack, hasRedo, goForward })
watch(
  () => props.source,
  () => {
    selected.value = null
    swipeRedo.value = null
    resultFilter.value = 'all'
    page.value = 1
  },
)
// Open the game a link asked for once the archive has it. Clock-only sessions have no replay.
let openedFromLink: string | null = null
watch(
  [() => props.openGame, games],
  ([id]) => {
    if (!id || openedFromLink === id) return
    const game = games.value.find((candidate) => candidate.id === id)
    if (!game) return
    openedFromLink = id
    if (game.source !== 'clock') selectGame(game)
  },
  { immediate: true },
)
function pgn(game: ArchivedGame): string {
  const clocks = game.timeControl.split(' / ')
  const whiteControl = clocks[0]
  const blackControl = clocks[1]
  const splitControl =
    clocks.length > 1 &&
    whiteControl !== undefined &&
    blackControl !== undefined &&
    whiteControl !== blackControl
  return setupPgn(game.setup, game.moves, {
    ...(whiteControl !== undefined &&
    blackControl !== undefined &&
    clocks.length > 1 &&
    whiteControl !== blackControl
      ? { WhiteTimeControl: whiteControl, BlackTimeControl: blackControl }
      : {}),
    Event: game.source === 'computer' ? 'KChess computer game' : 'KChess over-the-board game',
    White: game.white,
    Black: game.black,
    Result: game.result,
    Date: new Date(game.startedAt).toISOString().slice(0, 10).replaceAll('-', '.'),
    TimeControl: splitControl ? '-' : (whiteControl ?? '-'),
    Termination: game.result === '*' ? 'unterminated' : game.reason,
  })
}
const replaceAnalysis = useReplaceAnalysis()
function review(): void {
  const game = selected.value
  if (!game) return
  replaceAnalysis(() => {
    if (!analysis.loadPgn(pgn(game))) return
    analysis.origin = null
    store.selectPage('analysis')
  })
}
/** Who the game was against: the engine for computer games, both names at the board. */
function opponent(game: ArchivedGame): string {
  const you = archivedPlayerSide(game)
  if (you) return you === 'white' ? game.black : game.white
  return `${game.white} vs ${game.black}`
}
function details(game: ArchivedGame): string {
  if (game.source === 'clock') return game.clockSummary ?? game.reason
  return game.result === '*' && !game.finished ? '' : game.reason
}
function gameActions(game: ArchivedGame) {
  return [
    [
      {
        label: game.source === 'clock' ? 'Copy summary' : 'Export PGN',
        icon: game.source === 'clock' ? 'i-lucide-copy' : 'i-lucide-download',
        onSelect: () => void exportGame(game),
      },
    ],
    ...(game.finished
      ? [
          [
            {
              label: 'Delete',
              icon: 'i-lucide-trash-2',
              color: 'error' as const,
              onSelect: () => {
                pendingDelete.value = game
              },
            },
          ],
        ]
      : []),
  ]
}
function timeLabel(control: string): string {
  if (control === '-') return 'Untimed'
  return control
    .split(' / ')
    .map((side) => side.replace(/^(\d+)\+/, (_, seconds: string) => `${Number(seconds) / 60}+`))
    .join(' / ')
}
const exportMessage = ref('')
async function exportGame(game: ArchivedGame): Promise<void> {
  try {
    if (game.source === 'clock') {
      await navigator.clipboard.writeText(
        `${game.white} / ${game.black}\n${game.reason}\n${game.clockSummary}\nTime control: ${game.timeControl}`,
      )
      exportMessage.value = 'Clock summary copied'
    } else if (
      await window.kchess.saveExport({ name: `kchess-${game.id}`, kind: 'pgn', data: pgn(game) })
    ) {
      exportMessage.value = 'Saved the PGN'
    }
  } catch (cause) {
    console.warn('[local-game-history] Export failed:', cause)
    exportMessage.value = cause instanceof Error ? cause.message : 'Export failed'
  }
}
</script>

<template>
  <section class="local-history">
    <p v-if="exportMessage" role="status" class="section-hint">{{ exportMessage }}</p>
    <p v-if="archive.error" role="alert" class="text-error">{{ archive.error }}</p>
    <template v-if="selected">
      <div class="toolbar-row">
        <UButton variant="ghost" color="neutral" icon="i-lucide-arrow-left" @click="closeGame"
          >All games</UButton
        >
        <ResultBadge :badge="archivedResultBadge(selected)" size="sm" />
        <span>{{ selected.white }} vs {{ selected.black }}</span>
        <UButton variant="outline" color="neutral" @click="exportGame(selected)"
          >Export PGN</UButton
        >
        <UButton icon="i-lucide-sparkles" @click="review">Open analysis</UButton>
      </div>
      <GameReview
        :pgn="pgn(selected)"
        orientation="white"
        :player-name="selected.white"
        :opponent-name="selected.black"
      />
    </template>
    <UEmpty
      v-else-if="!games.length"
      class="card"
      icon="i-lucide-history"
      title="No saved games yet"
      :actions="[
        {
          label: source === 'computer' ? 'Play the computer' : 'Play over the board',
          onClick: () => store.selectPage(source === 'computer' ? 'computer' : 'local'),
        },
      ]"
    />
    <template v-else>
      <UTabs
        v-model="resultFilter"
        :items="resultItems"
        :content="false"
        variant="pill"
        size="sm"
        aria-label="Filter by result"
        class="justify-self-start"
      />
      <div class="card local-history-list game-list">
        <div class="game-head local-history-head" aria-hidden="true">
          <span>Result</span><span>{{ source === 'computer' ? 'Opponent' : 'Players' }}</span
          ><span>Game</span><span>Date</span><span />
        </div>
        <div v-for="game in visible" :key="game.id" class="local-history-row">
          <component
            :is="game.source === 'clock' ? 'div' : 'button'"
            :type="game.source === 'clock' ? undefined : 'button'"
            class="game-row local-history-main"
            :aria-label="
              game.source === 'clock'
                ? undefined
                : `${archivedResultBadge(game).label}, ${opponent(game)}, ${formatGameDate(game.startedAt)}. Replay game`
            "
            @click="game.source !== 'clock' && selectGame(game)"
          >
            <span
              ><ResultBadge :badge="archivedResultBadge(game)" class="local-history-result"
            /></span>
            <span class="opponent">
              <strong class="local-history-opponent"
                ><span
                  v-if="archivedPlayerSide(game)"
                  class="side-dot"
                  :class="archivedPlayerSide(game)"
                  :title="`You played ${archivedPlayerSide(game)}`"
                />{{ opponent(game) }}</strong
              >
              <span v-if="details(game)" class="sub" :title="details(game)">{{
                details(game)
              }}</span>
            </span>
            <span class="opponent">
              <span>{{
                game.source === 'clock' ? 'Clock only' : moveCount(game.moves.length)
              }}</span>
              <span class="sub"
                >{{ timeLabel(game.timeControl)
                }}<template v-if="game.setup.variant !== 'standard'">
                  · <span class="capitalize-first">{{ game.setup.variant }}</span></template
                ></span
              >
            </span>
            <span class="muted" :title="new Date(game.startedAt).toLocaleString()">{{
              formatGameDate(game.startedAt)
            }}</span>
          </component>
          <UDropdownMenu :items="gameActions(game)" :content="{ align: 'end' }">
            <UButton
              size="sm"
              variant="ghost"
              color="neutral"
              icon="i-lucide-ellipsis"
              :aria-label="`Actions for the game from ${new Date(game.startedAt).toLocaleString()}`"
            />
          </UDropdownMenu>
        </div>
        <UEmpty
          v-if="!shown.length"
          variant="naked"
          size="sm"
          icon="i-lucide-search-x"
          title="No games match this filter"
          :actions="[
            {
              label: 'Show all games',
              variant: 'outline',
              color: 'neutral',
              onClick: () => (resultFilter = 'all'),
            },
          ]"
        />
        <UPagination
          v-if="shown.length > 20"
          v-model:page="page"
          :total="shown.length"
          :items-per-page="20"
          class="pt-4"
        />
      </div>
    </template>
    <UModal
      v-model:open="deleteOpen"
      title="Delete saved game?"
      description="This removes the game from this device. Export a copy first if you want to keep it."
    >
      <template #body
        ><div class="toolbar-row">
          <UButton
            variant="outline"
            color="neutral"
            @click="pendingDelete && exportGame(pendingDelete)"
            >{{ pendingDelete?.source === 'clock' ? 'Copy summary' : 'Export copy' }}</UButton
          ><UButton color="error" @click="deleteGame">Delete game</UButton
          ><UButton variant="ghost" color="neutral" @click="pendingDelete = null">Cancel</UButton>
        </div></template
      >
    </UModal>
  </section>
</template>

<style scoped>
.local-history {
  display: grid;
  gap: 16px;
}
.local-history-list {
  display: flex;
  flex-direction: column;
}
.local-history-head,
.local-history-row {
  display: grid;
  grid-template-columns: 96px minmax(0, 1fr) 112px 104px 32px;
  gap: 12px;
  align-items: center;
}
.local-history-head {
  padding: 0 8px 8px 12px;
}
.local-history-row {
  padding: 0 8px 0 12px;
  border-bottom: 1px solid var(--ui-border);
}
.local-history-row:has(+ :not(.local-history-row)),
.local-history-row:last-child {
  border-bottom: 0;
}
.local-history-row:hover {
  background: var(--ui-bg-accented);
}
.local-history-main {
  grid-column: 1 / 5;
  grid-template-columns: subgrid;
  padding: 11px 0;
  border-bottom: 0;
}
.local-history-main:hover {
  background: transparent;
}
div.local-history-main {
  cursor: default;
}
.local-history-result {
  width: 96px;
}
.local-history-opponent {
  display: flex;
  align-items: center;
  gap: 6px;
}
@container page (max-width: 620px) {
  .local-history-head,
  .local-history-row {
    grid-template-columns: 96px minmax(0, 1fr) 32px;
  }
  .local-history-main {
    grid-column: 1 / 3;
  }
  .local-history-head > :nth-child(n + 3):not(:last-child),
  .local-history-main > :nth-child(n + 3) {
    display: none;
  }
}
</style>
