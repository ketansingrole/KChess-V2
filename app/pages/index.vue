<script setup lang="ts">
import { computed } from 'vue'
import { useOnline } from '@vueuse/core'
import { engineLevelLabel } from '../../src/shared/engineLevels'

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
  data,
} = storeToRefs(store)
const { sync, selectPage, openReview } = store

const continueComputer = computed(() => store.localMoves.length > 0 && !store.localOver)
const computerDescription = computed(() =>
  continueComputer.value
    ? `Move ${Math.floor(store.localMoves.length / 2) + 1} · Playing as ${store.userColor === 'white' ? 'White' : 'Black'} · ${engineLevelLabel(store.level)}. Your game is saved on this device.`
    : 'Choose your level and play at your own pace. Works offline.',
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

    <section class="home-start" aria-labelledby="home-title">
      <div>
        <h2 id="home-title" class="text-2xl font-semibold">Let's play chess</h2>
        <p class="section-hint">Play, learn and explore. No account needed to get started.</p>
      </div>
      <div class="home-activities">
        <div class="card home-activity home-primary">
          <UIcon name="i-lucide-monitor" class="text-2xl text-primary" />
          <h3 class="section-title">
            {{ continueComputer ? 'Your computer game' : 'Play the computer' }}
          </h3>
          <p class="section-hint">{{ computerDescription }}</p>
          <UButton trailing-icon="i-lucide-arrow-right" @click="openComputer">
            {{ continueComputer ? 'Continue your game' : 'Play the computer' }}
          </UButton>
        </div>
        <div class="card home-activity">
          <UIcon name="i-lucide-puzzle" class="text-2xl text-primary" />
          <h3 class="section-title">Practice puzzles</h3>
          <p class="section-hint">
            Train without an account. Download puzzles to keep playing offline.
          </p>
          <UButton variant="outline" color="neutral" @click="selectPage('puzzles')">
            Practice puzzles
          </UButton>
        </div>
        <div class="card home-activity">
          <UIcon name="i-lucide-microscope" class="text-2xl text-primary" />
          <h3 class="section-title">Analyze a game</h3>
          <p class="section-hint">Explore a position or import a PGN with local engine analysis.</p>
          <UButton variant="outline" color="neutral" @click="selectPage('analysis')">
            Open analysis
          </UButton>
        </div>
        <div class="card home-activity">
          <UIcon name="i-lucide-globe-2" class="text-2xl text-primary" />
          <h3 class="section-title">Play on Lichess</h3>
          <p class="section-hint">
            {{
              store.activeOnlineAccount
                ? `Play as @${store.activeOnlineAccount}.`
                : 'Connect Lichess when you want to play online.'
            }}
            {{
              online
                ? store.activeOnlineAccount
                  ? 'Internet required.'
                  : 'Sign in required.'
                : 'Available when you reconnect to the internet.'
            }}
          </p>
          <UButton variant="outline" color="neutral" @click="selectPage('online')">
            {{ store.activeOnlineAccount ? 'Play on Lichess' : 'Explore online play' }}
          </UButton>
        </div>
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
      <p v-if="!store.connectedAccounts.length && data.accounts.length" class="section-hint">
        No Lichess account connected. Public games from players you follow are still available in
        Following and Game history.
      </p>
      <p v-if="!online" class="section-hint" role="status">
        You're offline. Local play, analysis, saved studies and downloaded puzzles are available.
        <template v-if="store.activeOnlineAccount">Your Lichess account is remembered.</template>
      </p>
    </section>

    <section v-if="hasConnectedAccount" class="home-lichess">
      <h2 class="section-title">Your Lichess activity</h2>
      <p v-if="!online" class="section-hint">
        Showing saved activity. Reconnect to sync the latest games and ratings.
      </p>
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
                :items="['Bullet', 'Blitz', 'Rapid', 'Classical', 'Correspondence', 'Puzzle']"
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
          <RatingChart v-if="chartSeries.length >= 2" :points="chartSeries" :label="chartMode" />
          <p v-if="chartSeries.length >= 2 && chartFromGames" class="section-hint">
            Built from your synced rated games. Lichess has no rating history for this account.
          </p>
          <UEmpty
            v-else
            variant="naked"
            size="sm"
            icon="i-lucide-chart-line"
            title="No rating points"
            description="Nothing recorded for this mode and range. Try another mode or 'All'."
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
            description="Sync your account to pull in your latest games."
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
  gap: 20px;
  margin-bottom: 32px;
}
.home-activities {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
}
.home-activity {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 12px;
}
.home-activity .section-hint {
  margin: 0;
  flex: 1;
}
.home-primary {
  border-color: var(--ui-primary);
}
@media (max-width: 640px) {
  .home-activities {
    grid-template-columns: 1fr;
  }
}
</style>
