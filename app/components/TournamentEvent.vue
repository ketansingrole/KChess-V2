<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useIntervalFn, useOnline } from '@vueuse/core'
import type { Key } from '@lichess-org/chessground/types'
import type { TournamentGame, TournamentSystem } from '../../src/shared/types'
import { formatClock } from '../../src/shared/clock'
import { useTournamentStore } from '../stores/tournaments'
import { useWatchStore } from '../stores/watch'

/** One tournament: joining, its details, the top game, games in progress and the leaderboard. */
const props = defineProps<{ system: TournamentSystem; id: string }>()
const emit = defineEmits<{ connect: [] }>()

const online = useOnline()
const store = useKChessStore()
const { activeOnlineAccount, busy, settings } = storeToRefs(store)
const tournaments = useTournamentStore()
const watcher = useWatchStore()
const password = ref('')

watch(
  () => `${props.system}:${props.id}`,
  () => online.value && void tournaments.open(props.system, props.id),
  { immediate: true },
)
// Standings and games move quickly while it runs.
const ticker = useIntervalFn(() => {
  if (online.value && detail.value?.status !== 'finished')
    void tournaments.open(props.system, props.id)
}, 15_000)
onUnmounted(() => ticker.pause())

const detail = computed(() => (tournaments.detail?.id === props.id ? tournaments.detail : null))
const joined = computed(() =>
  detail.value
    ? detail.value.me
      ? !detail.value.me.withdraw
      : tournaments.isJoined(detail.value.system, detail.value.id)
    : false,
)
async function join(): Promise<void> {
  if (!detail.value || !online.value) return
  if (!activeOnlineAccount.value) {
    emit('connect')
    return
  }
  if (await tournaments.join(detail.value, password.value)) password.value = ''
}

function control(clock: { limit: number; increment: number }): string {
  const minutes = clock.limit / 60
  const base = { 0.25: '¼', 0.5: '½', 0.75: '¾' }[minutes] ?? String(minutes)
  return `${base}+${clock.increment}`
}
const status = computed(() => {
  const d = detail.value
  if (!d) return ''
  if (d.status === 'finished') return 'Finished'
  if (d.secondsToStart) return `Starts in ${Math.ceil(d.secondsToStart / 60)} min`
  if (d.secondsToFinish) return `${Math.ceil(d.secondsToFinish / 60)} min left`
  if (d.nextRoundIn) return `Next round in ${Math.ceil(d.nextRoundIn / 60)} min`
  return 'Under way'
})
const facts = computed(() => {
  const d = detail.value
  if (!d) return []
  return [
    control(d.clock),
    d.variant !== 'standard' ? d.variantName : '',
    d.rated ? 'Rated' : 'Casual',
    d.minutes ? `${d.minutes} min` : d.nbRounds ? `${d.nbRounds} rounds` : '',
    d.berserkable ? 'Berserk allowed' : '',
  ].filter(Boolean)
})

const pages = computed(() => Math.max(1, Math.ceil((detail.value?.nbPlayers ?? 0) / 10)))
function goToPage(next: number): void {
  void tournaments.open(props.system, props.id, Math.min(pages.value, Math.max(1, next)))
}

const featuredLast = computed<Key[] | undefined>(() => {
  const move = detail.value?.featured?.lastMove
  return move && !move.includes('@')
    ? [move.slice(0, 2) as Key, move.slice(2, 4) as Key]
    : undefined
})
const player = (p: TournamentGame['white']) => `${p.name}${p.rating ? ` (${p.rating})` : ''}`
async function watchGame(id: string): Promise<void> {
  localStorage.setItem('kchess:watch-tab', 'friends')
  await watcher.watch({ gameId: id })
  store.selectPage('watch')
}
const playingOwnGame = computed(() =>
  ['playing', 'seeking', 'disconnected'].includes(store.onlinePhase),
)
</script>

<template>
  <div class="event">
    <div class="event-toolbar">
      <UButton
        variant="ghost"
        color="neutral"
        icon="i-lucide-arrow-left"
        to="/tournaments"
        class="-ms-2"
        >All tournaments</UButton
      >
    </div>

    <p v-if="tournaments.detailError && !detail" class="text-error text-sm" role="alert">
      {{ tournaments.detailError }}
    </p>
    <div v-else-if="!detail" class="card"><p class="muted text-sm">Loading…</p></div>

    <template v-else>
      <section class="card event-head">
        <div class="event-title">
          <div class="min-w-0">
            <h2 class="event-name">{{ detail.name }}</h2>
            <p class="section-hint">
              {{ facts.join(' · ') }} · {{ detail.nbPlayers }} players ·
              <strong :class="{ 'text-primary': detail.status === 'started' }">{{ status }}</strong>
            </p>
          </div>
          <div v-if="detail.status !== 'finished'" class="event-actions">
            <template v-if="!joined">
              <UInput
                v-if="activeOnlineAccount"
                v-model="password"
                type="password"
                size="sm"
                placeholder="Entry code (if any)"
                aria-label="Tournament entry code"
                autocomplete="off"
                class="w-44"
              />
              <UButton
                icon="i-lucide-log-in"
                :loading="busy"
                :disabled="!online || !detail.playable || detail.verdicts?.accepted === false"
                @click="join"
                >Join</UButton
              >
            </template>
            <template v-else>
              <UBadge color="success" variant="soft" size="lg">You are in</UBadge>
              <span v-if="detail.me?.rank" class="text-sm tabular">Rank {{ detail.me.rank }}</span>
              <UButton
                v-if="detail.me?.gameId"
                icon="i-lucide-swords"
                @click="store.openOngoing(activeOnlineAccount, detail.me.gameId)"
                >Go to your game</UButton
              >
              <UButton
                variant="outline"
                color="neutral"
                :disabled="!online || busy"
                @click="tournaments.leave(detail)"
                >{{ detail.system === 'arena' ? 'Pause' : 'Withdraw' }}</UButton
              >
            </template>
          </div>
        </div>
        <UAlert
          v-if="detail.problem"
          color="neutral"
          variant="subtle"
          icon="i-lucide-eye"
          :description="detail.problem"
        />
        <p v-if="detail.description" class="event-description">{{ detail.description }}</p>
        <ul v-if="detail.verdicts?.list.length" class="event-conditions">
          <li v-for="entry in detail.verdicts.list" :key="entry.condition">
            <UIcon
              :name="entry.verdict === 'ok' ? 'i-lucide-circle-check' : 'i-lucide-circle-x'"
              :class="entry.verdict === 'ok' ? 'text-success' : 'text-error'"
            />
            {{ entry.condition
            }}<span v-if="entry.verdict !== 'ok'" class="muted"> · {{ entry.verdict }}</span>
          </li>
        </ul>
      </section>

      <div class="event-grid">
        <section class="card" aria-labelledby="standings-title">
          <div class="card-header">
            <h3 id="standings-title" class="section-title">Standings</h3>
            <div v-if="pages > 1" class="flex items-center gap-1">
              <UButton
                size="xs"
                variant="ghost"
                color="neutral"
                icon="i-lucide-chevron-left"
                aria-label="Previous page"
                :disabled="tournaments.page <= 1"
                @click="goToPage(tournaments.page - 1)"
              />
              <span class="muted text-xs tabular">{{ tournaments.page }} / {{ pages }}</span>
              <UButton
                size="xs"
                variant="ghost"
                color="neutral"
                icon="i-lucide-chevron-right"
                aria-label="Next page"
                :disabled="tournaments.page >= pages"
                @click="goToPage(tournaments.page + 1)"
              />
            </div>
          </div>
          <div v-if="detail.podium?.length" class="podium">
            <div
              v-for="entry in detail.podium"
              :key="entry.name"
              class="podium-place"
              :class="`place-${entry.rank}`"
            >
              <UIcon name="i-lucide-trophy" class="podium-trophy" />
              <PlayerLink :username="entry.name" class="podium-name" />
              <span class="muted text-xs tabular">
                {{ entry.score ?? 0 }} pts<template v-if="entry.performance">
                  · perf {{ entry.performance }}</template
                >
              </span>
            </div>
          </div>
          <p v-if="!detail.standing.length" class="muted text-sm">No players yet.</p>
          <ol v-else class="standings">
            <li
              v-for="row in detail.standing"
              :key="row.rank + row.name"
              :class="{ me: row.name.toLowerCase() === activeOnlineAccount.toLowerCase() }"
            >
              <span class="standing-rank tabular">{{ row.rank }}</span>
              <span class="standing-name">
                <template v-if="row.title">{{ row.title }} </template
                ><PlayerLink :username="row.name" />
                <span v-if="row.rating" class="muted tabular">{{ row.rating }}</span>
              </span>
              <span v-if="row.sheet" class="standing-sheet tabular" aria-hidden="true">{{
                row.sheet
              }}</span>
              <span class="standing-score tabular" :class="{ fire: row.fire }">
                <UIcon v-if="row.fire" name="i-lucide-flame" aria-label="On a streak" />
                {{ row.score ?? '' }}
              </span>
            </li>
          </ol>
        </section>

        <div class="flex flex-col gap-5">
          <section v-if="detail.featured" class="card" aria-labelledby="featured-title">
            <h3 id="featured-title" class="section-title mb-3">Top game</h3>
            <div class="featured-player">
              <span>
                <span class="muted tabular"
                  >#{{
                    detail.featured[detail.featured.orientation === 'white' ? 'black' : 'white']
                      .rank
                  }}</span
                >
                <PlayerLink
                  :username="
                    detail.featured[detail.featured.orientation === 'white' ? 'black' : 'white']
                      .name
                  "
                  >{{
                    player(
                      detail.featured[detail.featured.orientation === 'white' ? 'black' : 'white'],
                    )
                  }}</PlayerLink
                >
              </span>
              <span v-if="detail.featured.clocks" class="tabular">{{
                formatClock(
                  detail.featured.clocks[
                    detail.featured.orientation === 'white' ? 'black' : 'white'
                  ] * 1000,
                )
              }}</span>
            </div>
            <button
              type="button"
              class="featured-board"
              :disabled="playingOwnGame || !online"
              :aria-label="`Watch ${detail.featured.white.name} against ${detail.featured.black.name}`"
              @click="watchGame(detail.featured.id)"
            >
              <ChessBoard
                :fen="detail.featured.fen"
                :orientation="detail.featured.orientation"
                :theme="settings.boardTheme"
                coordinates="none"
                :piece-set="settings.pieceSet"
                animation="none"
                :interactive="false"
                :last-move="featuredLast"
              />
            </button>
            <div class="featured-player">
              <span>
                <span class="muted tabular"
                  >#{{ detail.featured[detail.featured.orientation].rank }}</span
                >
                <PlayerLink :username="detail.featured[detail.featured.orientation].name">{{
                  player(detail.featured[detail.featured.orientation])
                }}</PlayerLink>
              </span>
              <span v-if="detail.featured.clocks" class="tabular">{{
                formatClock(detail.featured.clocks[detail.featured.orientation] * 1000)
              }}</span>
            </div>
          </section>

          <section v-if="detail.duels?.length" class="card" aria-labelledby="duels-title">
            <h3 id="duels-title" class="section-title mb-2">Playing now</h3>
            <ul class="duels">
              <li v-for="duel in detail.duels" :key="duel.id">
                <span class="duel-players">
                  <span class="duel-side">
                    <span class="muted tabular">#{{ duel.white.rank }}</span>
                    <PlayerLink :username="duel.white.name" class="truncate" />
                  </span>
                  <span class="duel-side">
                    <span class="muted tabular">#{{ duel.black.rank }}</span>
                    <PlayerLink :username="duel.black.name" class="truncate" />
                  </span>
                </span>
                <UButton
                  size="xs"
                  variant="ghost"
                  color="neutral"
                  icon="i-lucide-tv"
                  :disabled="playingOwnGame || !online"
                  :aria-label="`Watch ${duel.white.name} against ${duel.black.name}`"
                  @click="watchGame(duel.id)"
                />
              </li>
            </ul>
          </section>

          <section v-if="detail.stats" class="card" aria-labelledby="stats-title">
            <h3 id="stats-title" class="section-title mb-2">Totals</h3>
            <dl class="event-stats text-sm">
              <div>
                <dt>Games</dt>
                <dd>{{ detail.stats.games }}</dd>
              </div>
              <div>
                <dt>White · draws · Black</dt>
                <dd>
                  {{ detail.stats.whiteWins }} · {{ detail.stats.draws }} ·
                  {{ detail.stats.blackWins }}
                </dd>
              </div>
              <div>
                <dt>Berserks</dt>
                <dd>{{ detail.stats.berserks }}</dd>
              </div>
              <div>
                <dt>Average rating</dt>
                <dd>{{ detail.stats.averageRating }}</dd>
              </div>
            </dl>
          </section>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.event {
  display: grid;
  gap: 16px;
}
.event-head {
  display: grid;
  gap: 14px;
}
.event-title {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px 20px;
}
.event-name {
  font-size: 1.25rem;
  font-weight: 600;
}
.event-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.event-description {
  font-size: 0.875rem;
  white-space: pre-line;
}
.event-conditions {
  display: grid;
  gap: 4px;
  font-size: 0.8125rem;
}
.event-conditions li {
  display: flex;
  align-items: center;
  gap: 6px;
}
.event-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(280px, 380px);
  gap: 20px;
  align-items: start;
}
@container page (max-width: 900px) {
  .event-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
.podium {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 10px;
  margin-bottom: 14px;
}
.podium-place {
  display: grid;
  justify-items: center;
  gap: 2px;
  padding: 10px 6px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  text-align: center;
}
.podium-trophy {
  font-size: 1.25rem;
}
.place-1 .podium-trophy {
  color: #d4a72c;
}
.place-2 .podium-trophy {
  color: #a8b0b9;
}
.place-3 .podium-trophy {
  color: #b87333;
}
.podium-name {
  max-width: 100%;
  overflow: hidden;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.standings li {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 0;
  border-top: 1px solid var(--ui-border);
  font-size: 0.875rem;
}
.standings li:first-child {
  border-top: 0;
}
.standings li.me {
  margin-inline: -8px;
  padding-inline: 8px;
  border-radius: 6px;
  background: color-mix(in oklab, var(--ui-primary) 12%, transparent);
}
.standing-rank {
  width: 2rem;
  color: var(--ui-text-muted);
  text-align: right;
}
.standing-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.standing-sheet {
  max-width: 40%;
  overflow: hidden;
  color: var(--ui-text-dimmed);
  font-size: 0.75rem;
  letter-spacing: 0.04em;
  text-overflow: ellipsis;
  white-space: nowrap;
  direction: rtl;
}
.standing-score {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  min-width: 2.5rem;
  justify-content: flex-end;
  font-weight: 600;
}
.standing-score.fire {
  color: var(--ui-warning);
}
.featured-player {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 0;
  font-size: 0.8125rem;
}
.featured-board {
  display: block;
  width: 100%;
  border-radius: 8px;
}
.featured-board :deep(.cg-host) {
  pointer-events: none;
}
.duels li {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 0;
  border-top: 1px solid var(--ui-border);
}
.duels li:first-child {
  border-top: 0;
}
.duel-players {
  display: grid;
  flex: 1;
  gap: 1px;
  min-width: 0;
  font-size: 0.8125rem;
}
.duel-side {
  display: flex;
  gap: 6px;
  min-width: 0;
}
.event-stats > div {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 6px 0;
  border-top: 1px solid var(--ui-border);
}
.event-stats > div:first-child {
  border-top: 0;
}
.event-stats dt {
  color: var(--ui-text-muted);
}
.event-stats dd {
  font-variant-numeric: tabular-nums;
}
</style>
