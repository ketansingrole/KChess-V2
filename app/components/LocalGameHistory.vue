<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useGameArchiveStore, type ArchivedGame } from '../stores/gameArchive'
import { useAnalysisStore } from '../stores/analysis'
import { setupPgn } from '../../src/shared/chess'

const props = defineProps<{ source: 'computer' | 'board' }>()
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
  page.value = Math.max(1, Math.min(page.value, Math.ceil(games.value.length / 20)))
}
const games = computed(() =>
  archive.games
    .filter((g) =>
      props.source === 'computer' ? g.source === 'computer' : g.source !== 'computer',
    )
    .sort((a, b) => b.startedAt - a.startedAt),
)
const visible = computed(() => games.value.slice((page.value - 1) * 20, page.value * 20))
/** What the last swipe-back undid, while nothing else changed since. */
const swipeRedo = ref<ArchivedGame | null>(null)
/** A swipe steps out of the open game before leaving the page. */
const hasDetail = computed(() => selected.value !== null)
function goBack(): void {
  swipeRedo.value = selected.value
  selected.value = null
}
const hasRedo = computed(
  () =>
    swipeRedo.value !== null &&
    selected.value === null &&
    games.value.some((game) => game.id === swipeRedo.value!.id),
)
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
    page.value = 1
  },
)
function pgn(game: ArchivedGame): string {
  const clocks = game.timeControl.split(' / ')
  return setupPgn(game.setup, game.moves, {
    ...(clocks.length > 1 && clocks[0] !== clocks[1]
      ? { WhiteTimeControl: clocks[0]!, BlackTimeControl: clocks[1]! }
      : {}),
    Event: game.source === 'computer' ? 'KChess computer game' : 'KChess over-the-board game',
    White: game.white,
    Black: game.black,
    Result: game.result,
    Date: new Date(game.startedAt).toISOString().slice(0, 10).replaceAll('-', '.'),
    TimeControl: clocks.length > 1 && clocks[0] !== clocks[1] ? '-' : clocks[0]!,
    Termination: game.result === '*' ? 'unterminated' : game.reason,
  })
}
function review(): void {
  if (!selected.value || !analysis.loadPgn(pgn(selected.value))) return
  analysis.origin = null
  store.selectPage('analysis')
}
function timeLabel(control: string): string {
  if (control === '-') return 'No clock'
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
        <span>{{ selected.white }} vs {{ selected.black }} · {{ selected.result }}</span>
        <UButton variant="outline" color="neutral" @click="exportGame(selected)"
          >Export PGN</UButton
        >
        <UButton icon="i-lucide-sparkles" @click="review">Open analysis</UButton>
      </div>
      <GameReview
        :pgn="pgn(selected)"
        orientation="white"
        :theme="store.settings.boardTheme"
        :coordinates="store.settings.coordinates"
        :piece-set="store.settings.pieceSet"
        :animation="store.settings.pieceAnimation"
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
    <div v-else class="card local-history-list">
      <article v-for="game in visible" :key="game.id" class="local-history-row">
        <div>
          <strong>{{ game.white }} vs {{ game.black }}</strong>
          <p class="section-hint">
            {{ new Date(game.startedAt).toLocaleString() }} ·
            {{
              game.source === 'clock' ? 'Clock only' : `${Math.ceil(game.moves.length / 2)} moves`
            }}
            · {{ game.setup.variant }} ·
            {{ timeLabel(game.timeControl) }}
          </p>
          <p class="section-hint">
            {{
              game.source === 'clock'
                ? game.finished
                  ? 'Session ended'
                  : 'Clock session'
                : game.result === '*'
                  ? game.finished
                    ? 'Unfinished'
                    : 'In progress'
                  : game.result
            }}
            · {{ game.reason
            }}<template v-if="game.clockSummary"> · {{ game.clockSummary }}</template>
          </p>
        </div>
        <div class="toolbar-row">
          <UButton
            v-if="game.finished"
            variant="ghost"
            color="neutral"
            icon="i-lucide-trash-2"
            :aria-label="`Delete game from ${new Date(game.startedAt).toLocaleString()}`"
            @click="pendingDelete = game"
          />
          <UButton
            v-if="game.source !== 'clock'"
            variant="soft"
            color="neutral"
            @click="selectGame(game)"
            >Replay</UButton
          >
          <UButton variant="ghost" color="neutral" @click="exportGame(game)">{{
            game.source === 'clock' ? 'Copy summary' : 'Export PGN'
          }}</UButton>
        </div>
      </article>
      <UPagination
        v-if="games.length > 20"
        v-model:page="page"
        :total="games.length"
        :items-per-page="20"
      />
    </div>
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
            >Export copy</UButton
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
  display: grid;
  gap: 0;
}
.local-history-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  padding: 16px 0;
  border-bottom: 1px solid var(--ui-border);
}
.local-history-row:first-child {
  padding-top: 0;
}
.local-history-row:last-child {
  border-bottom: 0;
  padding-bottom: 0;
}
@media (max-width: 700px) {
  .local-history-row {
    align-items: flex-start;
    flex-direction: column;
  }
}
</style>
