<script setup lang="ts">
import { computed } from 'vue'

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

const ratingChange = computed(() => {
  const values = chartValues.value
  return values.length >= 2 ? values[values.length - 1]! - values[0]! : null
})
const blitzProg = computed(() => profile.value?.perfs?.blitz?.prog ?? 0)
const trackedCount = computed(() => profile.value?.count?.all ?? selectedGameCount.value)
const accountItems = computed(() => data.value.accounts.map((account) => account.username))
</script>

<template>
  <div>
    <PageHeader title="Dashboard" subtitle="Your Lichess activity at a glance">
      <USelect
        v-if="data.accounts.length > 1"
        v-model="selectedAccount"
        :items="accountItems"
        icon="i-lucide-user"
        aria-label="Account"
        class="min-w-40"
      />
      <UButton
        v-if="data.accounts.length"
        icon="i-lucide-refresh-cw"
        variant="outline"
        color="neutral"
        :loading="busy"
        @click="sync()"
        >Sync games</UButton
      >
    </PageHeader>

    <div v-if="!data.accounts.length" class="card">
      <UEmpty
        variant="naked"
        icon="i-lucide-swords"
        title="Connect your chess world"
        description="Add a Lichess username to see ratings, recent games, and your full history."
        :actions="[
          {
            label: 'Open Settings',
            icon: 'i-lucide-settings-2',
            onClick: () => selectPage('settings'),
          },
        ]"
      />
    </div>

    <template v-else>
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
              @click="selectPage('history')"
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
                icon: 'i-lucide-refresh-cw',
                variant: 'outline',
                color: 'neutral',
                onClick: () => sync(),
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
    </template>
  </div>
</template>
