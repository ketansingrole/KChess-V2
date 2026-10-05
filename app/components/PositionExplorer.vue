<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useDebounceFn, useLocalStorage } from '@vueuse/core'
import type {
  ExplorerGame,
  ExplorerSpeed,
  LookupOptions,
  PositionLookup,
  PositionLookupKind,
} from '../../src/shared/types'
import { EXPLORER_RATINGS, EXPLORER_SPEEDS } from '../../src/shared/types'
import { useAnalysisStore } from '../stores/analysis'

const analysis = useAnalysisStore()
const store = useKChessStore()
const result = ref<PositionLookup | null>(null)
const error = ref('')
const busy = ref(false)
const open = useLocalStorage('kchess:explorer-open', false)
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

const KINDS: { value: PositionLookupKind; label: string }[] = [
  { value: 'opening', label: 'Lichess' },
  { value: 'masters', label: 'Masters' },
  { value: 'player', label: 'Player' },
  { value: 'tablebase', label: 'Tablebase' },
]
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

watch(
  () => analysis.node.fen,
  () => {
    epoch++
    result.value = null
    error.value = ''
    busy.value = false
    if (open.value && prefs.value.follow) void followLookup()
  },
)
watch(
  () => prefs.value.kind,
  () => {
    result.value = null
    error.value = ''
  },
)
async function lookup(kind: PositionLookupKind = prefs.value.kind): Promise<void> {
  prefs.value.kind = kind
  const request = ++epoch
  const fen = analysis.node.fen
  busy.value = true
  error.value = ''
  try {
    // A plain copy: Vue's reactive arrays cannot cross IPC.
    const data = await window.kchess.positionLookup(
      kind,
      fen,
      JSON.parse(JSON.stringify(options())),
    )
    if (request === epoch) result.value = data
  } catch (cause) {
    if (request === epoch)
      error.value = cause instanceof Error ? cause.message : 'Lookup failed. Retry.'
  } finally {
    if (request === epoch) busy.value = false
  }
}
const followLookup = useDebounceFn(() => void lookup(), 350)
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
function percent(part = 0, entry: { white?: number; draws?: number; black?: number }): number {
  const total = (entry.white ?? 0) + (entry.draws ?? 0) + (entry.black ?? 0)
  return total ? Math.round((100 * part) / total) : 0
}
function games(entry: { white?: number; draws?: number; black?: number }): number {
  return (entry.white ?? 0) + (entry.draws ?? 0) + (entry.black ?? 0)
}
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
  } catch (cause) {
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
  if (data.opening) return data.opening
  if (data.kind === 'tablebase')
    return `Tablebase · ${category(data.category)} for the side to move`
  if (data.kind === 'masters') return 'Master games (over the board)'
  if (data.kind === 'player') return `Games of ${prefs.value.player} as ${prefs.value.color}`
  return 'Lichess games'
})
</script>
<template>
  <details
    class="panel-divider py-3"
    :open="open"
    @toggle="open = ($event.target as HTMLDetailsElement).open"
  >
    <summary class="cursor-pointer font-semibold">Opening explorer and tablebases</summary>
    <p class="mt-2 text-xs text-muted">
      Lookups send this position to Lichess. The Lichess, Masters and Player databases use a
      connected account. Saved results remain available offline. Statistics describe games, not move
      quality.
    </p>
    <div class="mt-2 flex flex-wrap items-center gap-2">
      <UTabs
        :model-value="prefs.kind"
        :items="KINDS"
        :content="false"
        size="xs"
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
      <USwitch v-model="prefs.follow" size="xs" label="Follow moves" />
    </div>
    <div v-if="showFilters && prefs.kind !== 'tablebase'" class="mt-2 flex flex-col gap-2 text-xs">
      <div v-if="prefs.kind === 'player'" class="flex flex-wrap items-center gap-2">
        <UInput
          v-model="prefs.player"
          size="xs"
          placeholder="Lichess username"
          aria-label="Player"
          class="w-40"
        />
        <UButton
          v-for="name in accounts"
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
          size="xs"
          variant="pill"
        />
      </div>
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
    <div class="mt-2 flex gap-2">
      <UButton size="xs" variant="outline" color="neutral" :loading="busy" @click="lookup()"
        >Look up this position</UButton
      >
    </div>
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
        DTZ {{ result.dtz ?? 'unknown' }} plies to a pawn move or capture; DTZ is not distance to
        mate. Cursed wins/blessed losses account for the fifty-move rule.
      </p>
      <p v-if="result.cached" class="text-xs text-muted">
        {{ result.message || 'Saved lookup' }} ·
        {{ new Date(result.fetchedAt).toLocaleDateString() }}
      </p>
      <p v-if="!result.moves.length" class="mt-2">No moves found for this position.</p>
      <ul class="mt-2 space-y-1">
        <li v-for="move in result.moves" :key="move.uci" class="explorer-row">
          <UButton
            size="xs"
            variant="link"
            class="w-14 justify-start"
            @click="analysis.play(move.uci)"
            >{{ move.san }}</UButton
          >
          <template v-if="result.kind !== 'tablebase'">
            <span class="muted tabular text-xs w-16 text-right">{{
              games(move).toLocaleString()
            }}</span>
            <span
              class="explorer-bar"
              :title="`White ${move.white} · Draw ${move.draws} · Black ${move.black}`"
            >
              <span class="w" :style="{ width: `${percent(move.white, move)}%` }">{{
                percent(move.white, move) > 12 ? `${percent(move.white, move)}%` : ''
              }}</span>
              <span class="d" :style="{ width: `${percent(move.draws, move)}%` }">{{
                percent(move.draws, move) > 12 ? `${percent(move.draws, move)}%` : ''
              }}</span>
              <span class="b" :style="{ width: `${percent(move.black, move)}%` }">{{
                percent(move.black, move) > 12 ? `${percent(move.black, move)}%` : ''
              }}</span>
            </span>
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
  </details>
</template>

<style scoped>
.explorer-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.explorer-bar {
  display: flex;
  flex: 1;
  height: 16px;
  border-radius: 4px;
  overflow: hidden;
  font-size: 10px;
  line-height: 16px;
  border: 1px solid var(--ui-border);
}
.explorer-bar > span {
  text-align: center;
  overflow: hidden;
}
.explorer-bar .w {
  background: #f5f5f5;
  color: #222;
}
.explorer-bar .d {
  background: #9e9e9e;
  color: #fff;
}
.explorer-bar .b {
  background: #333;
  color: #eee;
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
