<script setup lang="ts">
import { computed } from 'vue'
import { useOnline } from '@vueuse/core'
import { engineLevelLabel } from '@kchess/rules/engineLevels'
import { useGameArchiveStore } from '../stores/gameArchive'

const online = useOnline()

const store = useKChessStore()
const {
  busy,
  selectedAccount,
  profile,
  chartMode,
  chartRange,
  selectedGameCount,
  recentGames,
  counts,
  chartSeries,
  chartFromGames,
  chartValues,
} = storeToRefs(store)
const { sync, selectPage, openReview } = store
const archive = useGameArchiveStore()
const localGames = computed(() =>
  archive.games
    .filter((game) => game.source !== 'clock')
    .sort((a, b) => b.startedAt - a.startedAt)
    .slice(0, 6),
)
function localResult(result: string): string {
  return result === '1/2-1/2' ? '½–½' : result === '*' ? 'Unfinished' : result.replace('-', '–')
}

const continueComputer = computed(() => store.localMoves.length > 0 && !store.localOver)
const computerDescription = computed(() =>
  continueComputer.value
    ? `Move ${Math.floor(store.localMoves.length / 2) + 1} · ${store.userColor === 'white' ? 'White' : 'Black'} · ${engineLevelLabel(store.level)}`
    : '',
)

function openComputer(): void {
  if (continueComputer.value) store.localPly = store.localMoves.length
  selectPage('computer')
}

const ratingChange = computed(() => {
  const values = chartValues.value
  return values.length >= 2 ? values[values.length - 1]! - values[0]! : null
})
const blitzProg = computed(() => profile.value?.perfs?.blitz?.prog ?? 0)
const trackedCount = computed(() => profile.value?.count?.all ?? selectedGameCount.value)
const accountItems = computed(() => store.connectedAccounts.map((account) => account.username))
/** Rating modes by perf key (`blitz`); labels match the previous display names. */
const chartModes = [
  { label: 'Bullet', value: 'bullet' },
  { label: 'Blitz', value: 'blitz' },
  { label: 'Rapid', value: 'rapid' },
  { label: 'Classical', value: 'classical' },
  { label: 'Correspondence', value: 'correspondence' },
  { label: 'Puzzle', value: 'puzzle' },
]
const chartLabel = computed(
  () => chartModes.find((mode) => mode.value === chartMode.value)?.label ?? chartMode.value,
)
const hasConnectedAccount = computed(() =>
  store.connectedAccounts.some(
    (account) => account.username.toLowerCase() === selectedAccount.value.toLowerCase(),
  ),
)
</script>

<template>
  <div>
    <PageHeader title="Home">
      <USelect
        v-if="accountItems.length > 1"
        v-model="selectedAccount"
        :items="accountItems"
        icon="i-lucide-user"
        aria-label="Account"
        class="min-w-40"
      />
      <UButton
        v-if="hasConnectedAccount"
        icon="i-lucide-refresh-cw"
        variant="outline"
        color="neutral"
        :loading="busy"
        :disabled="!online"
        @click="sync(selectedAccount)"
        >Sync games</UButton
      >
    </PageHeader>

    <section class="home-start" aria-label="Start">
      <div class="home-activities">
        <button type="button" class="home-tile home-primary" @click="openComputer">
          <UIcon name="i-lucide-monitor" class="home-tile-icon" />
          <span class="home-tile-text">
            <span class="home-tile-label">{{
              continueComputer ? 'Continue your game' : 'Play the computer'
            }}</span>
            <span v-if="continueComputer" class="home-tile-sub">{{ computerDescription }}</span>
          </span>
          <UIcon name="i-lucide-arrow-right" class="home-tile-arrow" />
        </button>
        <button type="button" class="home-tile" @click="selectPage('puzzles')">
          <UIcon name="i-lucide-puzzle" class="home-tile-icon" />
          <span class="home-tile-label">Practice puzzles</span>
        </button>
        <button type="button" class="home-tile" @click="selectPage('analysis')">
          <UIcon name="i-lucide-microscope" class="home-tile-icon" />
          <span class="home-tile-label">Open analysis</span>
        </button>
        <button type="button" class="home-tile" @click="selectPage('online')">
          <UIcon name="i-lucide-globe-2" class="home-tile-icon" />
          <span class="home-tile-label">Play on Lichess</span>
        </button>
      </div>
      <div class="toolbar-row">
        <UButton
          variant="ghost"
          color="neutral"
          icon="i-lucide-history"
          @click="selectPage('history')"
          >Your game history</UButton
        >
        <UButton
          variant="ghost"
          color="neutral"
          icon="i-lucide-users-round"
          @click="selectPage('local')"
          >Play over the board</UButton
        >
        <UButton
          variant="ghost"
          color="neutral"
          icon="i-lucide-graduation-cap"
          @click="selectPage('practice')"
          >Practice chess skills</UButton
        >
        <UButton
          variant="ghost"
          color="neutral"
          icon="i-lucide-library-big"
          @click="selectPage('studies')"
          >Your studies</UButton
        >
      </div>
    </section>

    <section v-if="!hasConnectedAccount" class="two-columns home-local">
      <div class="card">
        <div class="card-header">
          <h2 class="section-title">Recent games</h2>
          <UButton
            v-if="localGames.length"
            size="xs"
            variant="link"
            color="neutral"
            trailing-icon="i-lucide-arrow-right"
            @click="selectPage('history')"
            >View all</UButton
          >
        </div>
        <UEmpty
          v-if="!localGames.length"
          variant="naked"
          size="sm"
          icon="i-lucide-history"
          title="No saved games yet"
        />
        <ul v-else class="home-local-games">
          <li v-for="game in localGames" :key="game.id">
            <button type="button" class="home-local-game" @click="selectPage('history')">
              <UIcon
                :name="game.source === 'computer' ? 'i-lucide-monitor' : 'i-lucide-users-round'"
                class="home-local-source"
              />
              <span class="home-local-players">{{ game.white }} vs {{ game.black }}</span>
              <span class="home-local-result tabular">{{ localResult(game.result) }}</span>
              <span class="home-local-date">{{
                new Date(game.startedAt).toLocaleDateString(undefined, {
                  month: 'short',
                  day: 'numeric',
                })
              }}</span>
            </button>
          </li>
        </ul>
      </div>
      <div class="card">
        <h2 class="section-title">Lichess</h2>
        <UEmpty
          variant="naked"
          size="sm"
          icon="i-lucide-chart-line"
          title="Ratings, games and puzzle progress"
          :actions="[
            { label: 'Connect Lichess', icon: 'i-lucide-log-in', onClick: () => store.connect() },
          ]"
        />
      </div>
    </section>

    <section v-if="hasConnectedAccount" class="home-lichess">
      <h2 class="section-title">Your Lichess activity</h2>
      <p v-if="!online" class="section-hint">Offline · showing saved activity.</p>
      <div class="stats-grid">
        <div class="card stat-card">
          <div class="stat-label">Games</div>
          <div class="stat-value">{{ trackedCount }}</div>
          <div class="stat-sub">
            {{ profile?.count?.all ? 'Lichess lifetime' : 'Synced locally' }}
          </div>
        </div>
        <div class="card stat-card">
          <div class="stat-label">Blitz rating</div>
          <div class="stat-value">{{ profile?.perfs?.blitz?.rating ?? '—' }}</div>
          <div class="stat-sub">
            <span v-if="blitzProg" :class="blitzProg > 0 ? 'up' : 'down'"
              >{{ blitzProg > 0 ? '▲' : '▼' }} {{ Math.abs(blitzProg) }}</span
            >
            <template v-if="blitzProg"> recently</template>
            <template v-else>Current rating</template>
          </div>
        </div>
        <div class="card stat-card">
          <div class="stat-label">Rapid rating</div>
          <div class="stat-value">{{ profile?.perfs?.rapid?.rating ?? '—' }}</div>
          <div class="stat-sub">Current rating</div>
        </div>
        <div class="card stat-card">
          <div class="stat-label">Recent record</div>
          <div class="stat-value record" aria-label="Wins, losses and draws">
            <span class="w" title="Wins">{{ counts.win }}<small>W</small></span>
            <span class="l" title="Losses">{{ counts.loss }}<small>L</small></span>
            <span class="d" title="Draws">{{ counts.draw }}<small>D</small></span>
          </div>
          <div class="stat-sub">From synced games</div>
        </div>
      </div>

      <div class="two-columns">
        <div class="card">
          <div class="card-header">
            <div>
              <h2 class="section-title">Rating history</h2>
              <p v-if="ratingChange !== null" class="section-hint tabular">
                {{ chartValues[chartValues.length - 1] }} now ·
                <span :class="ratingChange >= 0 ? 'text-success' : 'text-error'"
                  >{{ ratingChange >= 0 ? '+' : '' }}{{ ratingChange }}</span
                >
                over range
              </p>
            </div>
            <div class="toolbar-row">
              <USelect
                v-model="chartMode"
                :items="chartModes"
                aria-label="Rating mode"
                size="sm"
              /><USelect
                v-model="chartRange"
                :items="['1M', '3M', '6M', 'YTD', '1Y', 'All']"
                aria-label="Rating range"
                size="sm"
              />
            </div>
          </div>
          <LazyRatingChart
            v-if="chartSeries.length >= 2"
            :points="chartSeries"
            :label="chartLabel"
          />
          <p v-if="chartSeries.length >= 2 && chartFromGames" class="section-hint">
            Built from your synced rated games. Lichess has no rating history for this account.
          </p>
          <UEmpty
            v-else
            variant="naked"
            size="sm"
            icon="i-lucide-chart-line"
            title="No rating points"
            description="Try another mode or date range."
          />
        </div>

        <div class="card">
          <div class="card-header">
            <h2 class="section-title">Recent games</h2>
            <UButton
              v-if="recentGames.length"
              size="xs"
              variant="link"
              color="neutral"
              trailing-icon="i-lucide-arrow-right"
              @click="navigateTo('/history?source=lichess')"
              >View all</UButton
            >
          </div>
          <UEmpty
            v-if="!recentGames.length"
            variant="naked"
            size="sm"
            icon="i-lucide-history"
            title="No synced games yet"
            :actions="[
              {
                label: 'Sync games',
                disabled: !online,
                icon: 'i-lucide-refresh-cw',
                variant: 'outline',
                color: 'neutral',
                onClick: () => sync(selectedAccount),
              },
            ]"
          />
          <div v-else class="game-list compact">
            <GameRow
              v-for="game in recentGames"
              :key="`${game.account}-${game.id}`"
              :game="game"
              @select="openReview(game)"
            />
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.home-lichess {
  display: grid;
  gap: 20px;
}
.home-lichess .stats-grid,
.home-lichess .two-columns {
  margin: 0;
}
.home-start {
  display: grid;
  gap: 12px;
  margin-bottom: 32px;
}
.home-start .toolbar-row {
  margin-left: -10px;
}
.home-activities {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 12px;
}
.home-tile {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 72px;
  padding: 14px 16px;
  border: 1px solid var(--ui-border);
  border-radius: 14px;
  background: var(--ui-bg-elevated);
  color: var(--ui-text-highlighted);
  text-align: left;
  cursor: pointer;
  transition:
    border-color 0.15s,
    background-color 0.15s;
}
.home-tile:hover {
  border-color: var(--ui-border-accented);
  background: var(--ui-bg-accented);
}
.home-tile:focus-visible {
  outline: 2px solid var(--ui-primary);
  outline-offset: 2px;
}
.home-tile-icon {
  flex: none;
  width: 24px;
  height: 24px;
  color: var(--ui-primary);
}
.home-tile-text {
  display: grid;
  gap: 2px;
  min-width: 0;
  flex: 1;
}
.home-tile-label {
  font-weight: 600;
}
.home-tile-sub {
  overflow: hidden;
  color: var(--ui-text-muted);
  font-size: 0.8125rem;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.home-tile-arrow {
  flex: none;
  width: 18px;
  height: 18px;
}
.home-primary {
  border-color: var(--ui-primary);
  background: var(--ui-primary);
  color: var(--ui-bg);
}
.home-primary:hover {
  border-color: var(--ui-primary);
  background: color-mix(in oklab, var(--ui-primary) 88%, var(--ui-bg));
}
.home-primary .home-tile-icon,
.home-primary .home-tile-sub {
  color: inherit;
}
.home-primary .home-tile-sub {
  opacity: 0.8;
}
.home-local {
  margin-top: 0;
}
.home-local-games {
  display: grid;
  margin: 0 -8px;
}
.home-local-game {
  display: grid;
  grid-template-columns: 20px minmax(0, 1fr) auto 64px;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 9px 8px;
  border-radius: 8px;
  text-align: left;
  cursor: pointer;
}
.home-local-game:hover {
  background: var(--ui-bg-accented);
}
.home-local-game:focus-visible {
  outline: 2px solid var(--ui-primary);
}
.home-local-source {
  width: 16px;
  height: 16px;
  color: var(--ui-text-muted);
}
.home-local-players {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.home-local-result {
  font-weight: 600;
}
.home-local-date {
  color: var(--ui-text-muted);
  font-size: 0.8125rem;
  text-align: right;
}
@media (max-width: 1100px) {
  .home-activities {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@media (max-width: 560px) {
  .home-activities {
    grid-template-columns: 1fr;
  }
}
</style>
