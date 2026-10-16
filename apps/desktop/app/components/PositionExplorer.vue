<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useDebounceFn, useLocalStorage } from '@vueuse/core'
import type {
  ExplorerGame,
  ExplorerSpeed,
  LookupOptions,
  PositionLookup,
  PositionLookupKind,
} from '@kchess/contracts/types'
import { EXPLORER_RATINGS, EXPLORER_SPEEDS } from '@kchess/contracts/types'
import { positionFromFen } from '@kchess/rules/chess'
import { openingAt } from '@kchess/rules/openings'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import { uciSquares } from '@kchess/rules/position'
import type { Key } from '@lichess-org/chessground/types'
import { useAnalysisStore } from '../stores/analysis'
import { errorText } from '../utils/format'

const props = defineProps<{
  /** The position to look up; the analysis board's when omitted. */
  fen?: string
  /** The moves from the starting position to `fen`: each continuation is named after its opening. */
  moves?: readonly string[]
  /** Continuations as a grid of boards, looked up whenever the position changes. */
  grid?: boolean
}>()
/** A move chosen from the results, when the explorer is given its own position. */
const emit = defineEmits<{ play: [uci: string] }>()

const analysis = useAnalysisStore()
const store = useKChessStore()
const fen = computed(() => props.fen ?? analysis.node.fen)
const result = ref<PositionLookup | null>(null)
const error = ref('')
const busy = ref(false)
/** Explorer preferences, remembered on this device like Lichess's explorer settings. */
const prefs = useLocalStorage<{
  kind: PositionLookupKind
  speeds: ExplorerSpeed[]
  ratings: number[]
  player: string
  color: 'white' | 'black'
  since: string
  follow: boolean
}>(
  'kchess:explorer',
  {
    kind: 'opening',
    speeds: ['blitz', 'rapid', 'classical'],
    ratings: [],
    player: '',
    color: 'white',
    since: '',
    follow: false,
  },
  { mergeDefaults: true },
)
const showFilters = ref(false)
let epoch = 0

const ALL_KINDS: { value: PositionLookupKind; label: string }[] = [
  { value: 'opening', label: 'Lichess' },
  { value: 'masters', label: 'Masters' },
  { value: 'player', label: 'Player' },
  { value: 'tablebase', label: 'Tablebase' },
]
// Opening positions have too many pieces for the tablebase.
const KINDS = props.grid ? ALL_KINDS.filter((kind) => kind.value !== 'tablebase') : ALL_KINDS
if (!KINDS.some((kind) => kind.value === prefs.value.kind)) prefs.value.kind = 'opening'
/** The grid looks up every position it is shown; the list does when asked, or when following. */
const following = computed(() => props.grid || prefs.value.follow)
const SPEED_LABELS: Record<ExplorerSpeed, string> = {
  ultraBullet: 'UltraBullet',
  bullet: 'Bullet',
  blitz: 'Blitz',
  rapid: 'Rapid',
  classical: 'Classical',
  correspondence: 'Corr.',
}
const accounts = computed(() => store.connectedAccounts.map((a) => a.username))
if (!prefs.value.player && accounts.value[0]) prefs.value.player = accounts.value[0]

function options(): LookupOptions {
  const p = prefs.value
  if (p.kind === 'opening')
    return { speeds: p.speeds, ratings: p.ratings, since: p.since || undefined }
  if (p.kind === 'masters') return { since: p.since ? p.since.slice(0, 4) : undefined }
  if (p.kind === 'player')
    return {
      player: p.player.trim(),
      color: p.color,
      speeds: p.speeds,
      since: p.since || undefined,
    }
  return {}
}

watch(fen, () => {
  epoch++
  result.value = null
  error.value = ''
  busy.value = false
  if (following.value) void followLookup()
})
watch(
  () => prefs.value.kind,
  () => {
    result.value = null
    error.value = ''
  },
)
/** Lichess usernames: what the player database accepts. */
const USERNAME = /^[a-zA-Z0-9_-]{2,30}$/
const playerInput = ref<{ inputRef?: HTMLInputElement } | null>(null)
const needsPlayer = computed(
  () => prefs.value.kind === 'player' && !USERNAME.test(prefs.value.player.trim()),
)
async function lookup(kind: PositionLookupKind = prefs.value.kind, asked = true): Promise<void> {
  prefs.value.kind = kind
  if (needsPlayer.value) {
    result.value = null
    error.value = ''
    // Ask for the name, but never steal focus from the board while following moves.
    if (asked) playerInput.value?.inputRef?.focus()
    return
  }
  const request = ++epoch
  const position = fen.value
  busy.value = true
  error.value = ''
  try {
    // A plain copy: Vue's reactive arrays cannot cross IPC.
    const data = await window.kchess.positionLookup(
      kind,
      position,
      JSON.parse(JSON.stringify(options())),
    )
    if (request === epoch) result.value = data
  } catch (cause) {
    console.warn('[position-explorer] Lookup failed:', cause)
    if (request === epoch) error.value = errorText(cause) || 'Lookup failed. Retry.'
  } finally {
    if (request === epoch) busy.value = false
  }
}
const followLookup = useDebounceFn(() => void lookup(prefs.value.kind, false), 350)
// Opening the explorer while following moves looks up the position shown.
onMounted(() => {
  if (following.value) void lookup(prefs.value.kind, false)
})
// The grid follows its position: choosing a database looks it up straight away.
watch(
  () => [prefs.value.speeds, prefs.value.ratings, prefs.value.since, prefs.value.color],
  () => {
    if (props.grid) void followLookup()
  },
  { deep: true },
)
function playMove(uci: string): void {
  if (props.fen === undefined) analysis.play(uci)
  else emit('play', uci)
}
function toggle<T>(list: T[], value: T): T[] {
  return list.includes(value) ? list.filter((entry) => entry !== value) : [...list, value]
}
function category(value: string | undefined, opponent = false): string {
  if (!value) return ''
  if (opponent)
    value = value
      .replace(/win|loss/g, (part) => (part === 'win' ? 'loss' : 'win'))
      .replace('blessed-win', 'cursed-win')
      .replace('cursed-loss', 'blessed-loss')
  return value.replace(/-/g, ' ')
}
function games(entry: { white?: number; draws?: number; black?: number }): number {
  return (entry.white ?? 0) + (entry.draws ?? 0) + (entry.black ?? 0)
}
/** Each continuation's board, share of the games and opening name, for the grid. */
const tiles = computed(() => {
  const data = result.value
  if (!props.grid || !data || data.kind === 'tablebase') return []
  const start = positionFromFen(data.fen)
  const total = data.total || data.moves.reduce((sum, move) => sum + games(move), 0)
  return data.moves.map((move) => {
    const squares = uciSquares(move.uci)
    return {
      move,
      fen: start?.play(move.uci)?.position.fen ?? data.fen,
      lastMove: squares as Key[] | undefined,
      share: total ? Math.round((100 * games(move)) / total) : 0,
      opening: props.moves
        ? openingAt(STANDARD_SETUP, [...props.moves, move.uci])?.name
        : undefined,
    }
  })
})
const toast = useToast()
async function openGame(game: ExplorerGame): Promise<void> {
  try {
    const pgn =
      result.value?.kind === 'masters'
        ? await window.kchess.mastersGame(game.id)
        : await window.kchess.exportGame(game.id)
    if (!analysis.loadPgn(pgn)) throw new Error('That game could not be read.')
    analysis.origin = {
      white: game.white,
      black: game.black,
      gameId: result.value?.kind === 'masters' ? undefined : game.id,
    }
    // Away from the analysis board, the game opens on it.
    if (props.fen !== undefined) store.selectPage('analysis')
  } catch (cause) {
    console.warn('[position-explorer] Could not open the game:', cause)
    toast.add({
      title: 'Could not open the game',
      description: cause instanceof Error ? cause.message : String(cause),
      color: 'error',
    })
  }
}
const heading = computed(() => {
  const data = result.value
  if (!data) return ''
  // The grid's page already names the opening.
  if (data.opening && !props.grid) return data.opening
  if (data.kind === 'tablebase')
    return `Tablebase · ${category(data.category)} for the side to move`
  if (data.kind === 'masters') return 'Master games (over the board)'
  if (data.kind === 'player') return `Games of ${prefs.value.player} as ${prefs.value.color}`
  return 'Lichess games'
})
</script>
<template>
  <section aria-label="Opening explorer and tablebases">
    <div class="flex flex-wrap items-center gap-2">
      <UTabs
        :model-value="prefs.kind"
        :items="KINDS"
        :content="false"
        variant="pill"
        @update:model-value="lookup($event as PositionLookupKind)"
      />
      <UButton
        v-if="prefs.kind !== 'tablebase' && prefs.kind !== 'masters'"
        size="xs"
        variant="ghost"
        color="neutral"
        icon="i-lucide-sliders-horizontal"
        :aria-pressed="showFilters"
        aria-label="Explorer filters"
        @click="showFilters = !showFilters"
      />
      <span class="flex-1" />
      <UTooltip
        text="Lookups send this position to Lichess. The Lichess, Masters and Player databases use a connected account; saved results stay available offline. Statistics describe games, not move quality."
      >
        <UIcon name="i-lucide-info" class="muted" aria-label="About the explorer" />
      </UTooltip>
    </div>
    <div v-if="prefs.kind === 'player'" class="mt-2 flex flex-wrap items-center gap-2">
      <UInput
        ref="playerInput"
        v-model="prefs.player"
        size="xs"
        placeholder="Lichess username"
        aria-label="Player"
        class="w-36"
        @keydown.enter="lookup()"
      />
      <UButton
        v-for="name in accounts.filter((a) => a !== prefs.player)"
        :key="name"
        size="xs"
        variant="soft"
        color="neutral"
        @click="prefs.player = name"
        >{{ name }}</UButton
      >
      <UTabs
        v-model="prefs.color"
        :items="[
          { label: 'as White', value: 'white' },
          { label: 'as Black', value: 'black' },
        ]"
        :content="false"
        variant="pill"
      />
    </div>
    <div v-if="showFilters && prefs.kind !== 'tablebase'" class="mt-2 flex flex-col gap-2 text-xs">
      <div class="flex flex-wrap gap-1">
        <UButton
          v-for="speed in EXPLORER_SPEEDS"
          :key="speed"
          size="xs"
          :variant="prefs.speeds.includes(speed) ? 'soft' : 'ghost'"
          :color="prefs.speeds.includes(speed) ? 'primary' : 'neutral'"
          @click="prefs.speeds = toggle(prefs.speeds, speed)"
          >{{ SPEED_LABELS[speed] }}</UButton
        >
      </div>
      <div v-if="prefs.kind === 'opening'" class="flex flex-wrap gap-1">
        <UButton
          v-for="rating in EXPLORER_RATINGS"
          :key="rating"
          size="xs"
          :variant="prefs.ratings.includes(rating) ? 'soft' : 'ghost'"
          :color="prefs.ratings.includes(rating) ? 'primary' : 'neutral'"
          @click="prefs.ratings = toggle(prefs.ratings, rating)"
          >{{ rating }}</UButton
        >
        <span class="muted self-center">(none selected: all ratings)</span>
      </div>
      <label class="flex items-center gap-2">
        Since
        <UInput
          v-model="prefs.since"
          size="xs"
          placeholder="YYYY-MM"
          class="w-24"
          aria-label="Since"
        />
      </label>
    </div>
    <div v-if="!grid" class="mt-2 flex items-center gap-3">
      <UButton
        size="xs"
        variant="outline"
        color="neutral"
        icon="i-lucide-search"
        :loading="busy"
        :disabled="needsPlayer"
        @click="lookup()"
        >Look up this position</UButton
      >
      <USwitch v-model="prefs.follow" size="xs" label="Follow moves" />
    </div>
    <p v-if="needsPlayer" class="mt-2 text-xs muted">
      Enter a Lichess username to see the moves they played here.
    </p>
    <p v-if="busy" role="status" class="mt-2 text-sm">
      {{
        prefs.kind === 'player'
          ? 'Indexing the player’s games — this can take a while the first time…'
          : 'Loading position lookup…'
      }}
    </p>
    <p v-if="error" role="alert" class="mt-2 text-sm">{{ error }}</p>
    <div v-if="result" class="mt-2 text-sm">
      <p>
        {{ heading }}
        <span v-if="result.total" class="muted tabular">
          · {{ result.total.toLocaleString() }} games</span
        >
      </p>
      <p v-if="result.kind === 'tablebase'" class="text-xs text-muted">
        <UTooltip
          text="Plies to a pawn move or capture; not distance to mate. Cursed wins and blessed losses account for the fifty-move rule."
        >
          <span class="underline decoration-dotted">DTZ {{ result.dtz ?? 'unknown' }}</span>
        </UTooltip>
      </p>
      <p v-if="result.cached" class="text-xs text-muted">
        {{ result.message || 'Saved lookup' }} ·
        {{ new Date(result.fetchedAt).toLocaleDateString() }}
      </p>
      <p v-if="!result.moves.length" class="mt-2">No moves found for this position.</p>
      <ul v-if="grid" class="explorer-grid mt-2" aria-label="Popular continuations">
        <li v-for="tile in tiles" :key="tile.move.uci">
          <button
            type="button"
            class="explorer-tile"
            :aria-label="`${tile.move.san}${tile.opening ? `, ${tile.opening}` : ''}`"
            @click="playMove(tile.move.uci)"
          >
            <span class="explorer-tile-board" aria-hidden="true">
              <ChessBoard
                :fen="tile.fen"
                :theme="store.settings.boardTheme"
                coordinates="none"
                :piece-set="store.settings.pieceSet"
                animation="none"
                :interactive="false"
                :last-move="tile.lastMove"
              />
            </span>
            <span class="flex items-baseline justify-between gap-2">
              <strong>{{ tile.move.san }}</strong>
              <span class="muted tabular text-xs">{{ tile.share }}%</span>
            </span>
            <span v-if="tile.opening" class="explorer-tile-name text-xs muted">{{
              tile.opening
            }}</span>
            <ExplorerBar
              :white="tile.move.white"
              :draws="tile.move.draws"
              :black="tile.move.black"
              :min-label="20"
            />
          </button>
        </li>
      </ul>
      <ul v-else class="mt-2 space-y-1">
        <li v-for="move in result.moves" :key="move.uci" class="explorer-row">
          <UButton
            size="xs"
            variant="link"
            class="w-14 justify-start"
            @click="playMove(move.uci)"
            >{{ move.san }}</UButton
          >
          <template v-if="result.kind !== 'tablebase'">
            <span class="muted tabular text-xs w-16 text-right">{{
              games(move).toLocaleString()
            }}</span>
            <ExplorerBar :white="move.white" :draws="move.draws" :black="move.black" />
          </template>
          <span v-else class="text-xs"
            >{{ category(move.category, true) }} for the current player</span
          >
        </li>
      </ul>
      <div v-if="result.games?.length" class="mt-3">
        <h4 class="text-xs font-semibold muted mb-1">
          {{ result.kind === 'masters' ? 'Top games' : 'Recent games' }}
        </h4>
        <ul class="space-y-1">
          <li v-for="game in result.games" :key="game.id">
            <button type="button" class="explorer-game" @click="openGame(game)">
              <span class="truncate"
                >{{ game.white }} <span class="muted tabular">{{ game.whiteRating ?? '' }}</span> –
                {{ game.black }}
                <span class="muted tabular">{{ game.blackRating ?? '' }}</span></span
              >
              <span class="muted tabular text-xs"
                >{{ game.winner === 'white' ? '1-0' : game.winner === 'black' ? '0-1' : '½-½' }}
                {{ game.year ?? game.month ?? '' }}</span
              >
            </button>
          </li>
        </ul>
      </div>
    </div>
  </section>
</template>

<style scoped>
.explorer-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.explorer-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(120px, 100%), 1fr));
  gap: 10px;
}
.explorer-tile {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 100%;
  padding: 6px;
  border: 1px solid var(--ui-border);
  border-radius: 8px;
  text-align: left;
  font-size: 13px;
}
.explorer-tile:hover {
  background: var(--ui-bg-accented);
}
.explorer-tile-board {
  display: block;
  pointer-events: none;
}
.explorer-tile-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.explorer-tile > :deep(.explorer-bar) {
  flex: none;
}
.explorer-game {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  text-align: left;
  font-size: 12px;
  padding: 2px 4px;
  border-radius: 6px;
}
.explorer-game:hover {
  background: var(--ui-bg-accented);
}
</style>
