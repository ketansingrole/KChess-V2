<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useLocalStorage, useOnline } from '@vueuse/core'
import type {
  Crosstable,
  LichessUser,
  PerfStats,
  PerfType,
  UserPresence,
} from '../../src/shared/types'
import { PERF_TYPES } from '../../src/shared/types'
import { USERNAME } from '../../src/shared/patterns'
import { useWatchStore } from '../stores/watch'

const online = useOnline()
const store = useKChessStore()
const { activeOnlineAccount, connectedAccounts, data, trackedAccounts } = storeToRefs(store)
const route = useRoute()
const watcher = useWatchStore()

const query = ref(typeof route.query.name === 'string' ? route.query.name : '')
const recent = useLocalStorage<string[]>('kchess:players-recent', [])
/** Recent lookups first, then followed players, for one-click lookups. */
const suggestions = computed(() => {
  const seen = new Set<string>()
  return [...recent.value, ...trackedAccounts.value.map((a) => a.username)]
    .filter((name) => {
      const key = name.toLowerCase()
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
    .slice(0, 12)
})
const profile = ref<LichessUser | null>(null)
const status = ref<UserPresence | null>(null)
const perf = ref<PerfType>('blitz')
const stats = ref<PerfStats | null>(null)
const statsError = ref('')
const record = ref<Crosstable | null>(null)
const error = ref('')
const loading = ref(false)
let request = 0

const PERF_LABELS: Record<PerfType, string> = {
  ultraBullet: 'UltraBullet',
  bullet: 'Bullet',
  blitz: 'Blitz',
  rapid: 'Rapid',
  classical: 'Classical',
  correspondence: 'Correspondence',
  chess960: 'Chess960',
  kingOfTheHill: 'King of the Hill',
  threeCheck: 'Three-check',
  antichess: 'Antichess',
  atomic: 'Atomic',
  horde: 'Horde',
  racingKings: 'Racing Kings',
  crazyhouse: 'Crazyhouse',
}
/** Categories the player has games in, most played first. */
const perfs = computed(() => {
  const all = (profile.value?.perfs ?? {}) as Record<
    string,
    { games?: number; rating?: number; prov?: boolean; prog?: number }
  >
  return PERF_TYPES.filter((key) => (all[key]?.games ?? 0) > 0)
    .map((key) => ({ key, ...all[key]! }))
    .sort((a, b) => (b.games ?? 0) - (a.games ?? 0))
})

async function lookup(name = query.value.trim()): Promise<void> {
  if (!online.value) {
    error.value = 'Reconnect to look up a player.'
    return
  }
  if (!USERNAME.test(name)) {
    error.value = 'Enter a Lichess username.'
    return
  }
  const current = ++request
  loading.value = true
  error.value = ''
  stats.value = null
  record.value = null
  status.value = null
  try {
    const found = await window.kchess.profile(name)
    if (current !== request) return
    profile.value = found
    query.value = found.username
    recent.value = [
      found.username,
      ...recent.value.filter((n) => n.toLowerCase() !== found.username.toLowerCase()),
    ].slice(0, 8)
    perf.value = (perfs.value[0]?.key as PerfType | undefined) ?? 'blitz'
    void loadStats()
    const me = activeOnlineAccount.value
    if (me && me.toLowerCase() !== found.username.toLowerCase())
      void window.kchess
        .crosstable(me, found.username)
        .then((value) => {
          if (current === request) record.value = value
        })
        .catch(() => undefined)
    void window.kchess
      .presence([found.username])
      .then((report) => {
        if (current === request) status.value = report.users[found.username.toLowerCase()] ?? null
      })
      .catch(() => undefined)
  } catch (cause) {
    if (current === request) {
      profile.value = null
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
  } finally {
    if (current === request) loading.value = false
  }
}
async function loadStats(): Promise<void> {
  const user = profile.value
  if (!user) return
  const current = request
  statsError.value = ''
  try {
    const next = await window.kchess.playerPerf(user.username, perf.value)
    if (current === request) stats.value = next
  } catch (cause) {
    if (current === request)
      statsError.value = cause instanceof Error ? cause.message : String(cause)
  }
}
watch(perf, () => void loadStats())
if (query.value) void lookup()

const isFriend = computed(() =>
  data.value.accounts.some(
    (a) => a.username.toLowerCase() === profile.value?.username.toLowerCase(),
  ),
)
const score = computed(() => {
  const value = record.value
  const user = profile.value
  const me = activeOnlineAccount.value.toLowerCase()
  if (!value || !user || !value.nbGames) return null
  return {
    mine: value.users[me] ?? 0,
    theirs: value.users[user.username.toLowerCase()] ?? 0,
    games: value.nbGames,
  }
})
function percent(part: number, whole: number): string {
  return whole ? `${Math.round((100 * part) / whole)}%` : '–'
}
function hours(seconds: number): string {
  const h = seconds / 3600
  return h >= 1 ? `${Math.round(h)} h` : `${Math.round(seconds / 60)} min`
}
async function watchGame(): Promise<void> {
  if (!status.value?.playingId) return
  localStorage.setItem('kchess:watch-tab', 'friends')
  await watcher.watch({ gameId: status.value.playingId })
  store.selectPage('watch')
}
</script>

<template>
  <div>
    <PageHeader title="Players" />
    <PublicOnlineNotice offline-message="Offline · reconnect to look up players." />
    <form
      class="card player-search flex flex-wrap items-center gap-2 mb-5"
      :class="{ 'player-search-empty': !profile }"
      @submit.prevent="lookup()"
    >
      <UEmpty
        v-if="!profile"
        variant="naked"
        size="sm"
        icon="i-lucide-user-search"
        title="Look up a player"
        class="w-full"
      />
      <UInput
        v-model="query"
        icon="i-lucide-user-search"
        placeholder="Lichess username"
        autocomplete="off"
        aria-label="Lichess username"
        class="min-w-48"
        :class="profile ? 'flex-1' : 'player-search-input'"
      />
      <UButton type="submit" :loading="loading" :disabled="!online">Look up</UButton>
      <div v-if="suggestions.length" class="player-suggestions flex flex-wrap gap-1 w-full">
        <UButton
          v-for="name in suggestions"
          :key="name"
          size="xs"
          variant="soft"
          color="neutral"
          @click="lookup(name)"
          >{{ name }}</UButton
        >
      </div>
    </form>
    <p v-if="error" class="text-error text-sm" role="alert">{{ error }}</p>

    <div v-if="profile" class="online-columns">
      <section class="card">
        <div class="card-header">
          <div>
            <h2 class="section-title">
              {{ profile.title ? `${profile.title} ` : '' }}{{ profile.username }}
              <UBadge v-if="status?.online" size="sm" color="success" variant="soft">{{
                status.playing ? 'Playing' : 'Online'
              }}</UBadge>
            </h2>
            <p class="section-hint">
              {{ profile.count?.all ?? 0 }} games · joined
              {{ profile.createdAt ? new Date(profile.createdAt).toLocaleDateString() : '?' }}
              <span v-if="profile.seenAt">
                · seen {{ new Date(profile.seenAt).toLocaleDateString() }}</span
              >
            </p>
          </div>
          <div class="flex flex-wrap gap-2">
            <UButton
              v-if="!isFriend"
              size="sm"
              variant="outline"
              color="neutral"
              icon="i-lucide-user-plus"
              :disabled="!online"
              @click="store.importFriends([profile.username])"
              >Follow player</UButton
            >
            <UButton
              v-if="
                connectedAccounts.length &&
                profile.username.toLowerCase() !== activeOnlineAccount.toLowerCase()
              "
              size="sm"
              icon="i-lucide-swords"
              :disabled="!online"
              @click="store.challengeFriend(profile.username)"
              >Challenge</UButton
            >
            <UButton
              v-if="status?.playingId"
              size="sm"
              variant="outline"
              color="neutral"
              icon="i-lucide-tv"
              :disabled="!online"
              @click="watchGame"
              >Watch</UButton
            >
          </div>
        </div>
        <p v-if="profile.profile?.bio" class="text-sm whitespace-pre-line mb-3">
          {{ profile.profile.bio }}
        </p>
        <div v-if="score" class="offer-banner mb-4">
          <UIcon name="i-lucide-scale" />
          <span class="offer-text">
            You (@{{ activeOnlineAccount }}) <strong class="tabular">{{ score.mine }}</strong> –
            <strong class="tabular">{{ score.theirs }}</strong> {{ profile.username }} over
            {{ score.games }} games<span v-if="record?.matchup?.nbGames">
              · today {{ record.matchup.users[activeOnlineAccount.toLowerCase()] ?? 0 }} –
              {{ record.matchup.users[profile.username.toLowerCase()] ?? 0 }}</span
            >
          </span>
        </div>
        <h3 class="section-title text-sm mb-2">Ratings</h3>
        <div class="list-rows">
          <button
            v-for="entry in perfs"
            :key="entry.key"
            type="button"
            class="list-row text-left"
            :class="{ 'ring-1 ring-primary': perf === entry.key }"
            @click="perf = entry.key"
          >
            <div class="row-main">
              <div class="row-title">{{ PERF_LABELS[entry.key] }}</div>
              <div class="row-sub">{{ entry.games }} games</div>
            </div>
            <span class="tabular font-semibold">{{ entry.rating }}{{ entry.prov ? '?' : '' }}</span>
            <span
              v-if="entry.prog"
              class="tabular text-xs"
              :class="entry.prog > 0 ? 'text-success' : 'text-error'"
              >{{ entry.prog > 0 ? '+' : '' }}{{ entry.prog }}</span
            >
          </button>
        </div>
      </section>
      <section class="card">
        <h2 class="section-title mb-3">{{ PERF_LABELS[perf] }} record</h2>
        <p v-if="statsError" class="text-error text-sm" role="alert">{{ statsError }}</p>
        <template v-else-if="stats">
          <p class="text-sm mb-3">
            <strong class="tabular">{{ stats.rating ? Math.round(stats.rating) : '–' }}</strong>
            <span v-if="stats.deviation" class="muted"> ± {{ Math.round(stats.deviation) }}</span>
            <span v-if="stats.rank"> · rank #{{ stats.rank }}</span>
            <span v-if="stats.percentile"> · better than {{ stats.percentile }}% of players</span>
          </p>
          <dl class="stats-grid text-sm">
            <dt>Games</dt>
            <dd>{{ stats.count.all }} ({{ stats.count.rated }} rated)</dd>
            <dt>Wins</dt>
            <dd>{{ stats.count.win }} · {{ percent(stats.count.win, stats.count.all) }}</dd>
            <dt>Draws</dt>
            <dd>{{ stats.count.draw }} · {{ percent(stats.count.draw, stats.count.all) }}</dd>
            <dt>Losses</dt>
            <dd>{{ stats.count.loss }} · {{ percent(stats.count.loss, stats.count.all) }}</dd>
            <dt>Opponents' average</dt>
            <dd>{{ Math.round(stats.count.opAvg) }}</dd>
            <dt>Time played</dt>
            <dd>{{ hours(stats.count.seconds) }}</dd>
            <dt>Tournament games</dt>
            <dd>{{ stats.count.tour }} · berserked {{ stats.count.berserk }}</dd>
            <dt>Highest</dt>
            <dd>
              {{
                stats.highest
                  ? `${stats.highest.rating} (${new Date(stats.highest.at).toLocaleDateString()})`
                  : '–'
              }}
            </dd>
            <dt>Lowest</dt>
            <dd>
              {{
                stats.lowest
                  ? `${stats.lowest.rating} (${new Date(stats.lowest.at).toLocaleDateString()})`
                  : '–'
              }}
            </dd>
            <dt>Winning streak</dt>
            <dd>{{ stats.winStreak.current }} now · best {{ stats.winStreak.best }}</dd>
            <dt>Losing streak</dt>
            <dd>{{ stats.lossStreak.current }} now · worst {{ stats.lossStreak.best }}</dd>
          </dl>
          <div class="grid sm:grid-cols-2 gap-4 mt-4 text-sm">
            <div>
              <h3 class="section-title text-sm mb-1">Best rated wins</h3>
              <p v-if="!stats.bestWins.length" class="muted">None yet.</p>
              <p v-for="win in stats.bestWins" :key="win.gameId">
                {{ win.opponent }} <span class="muted tabular">{{ win.opponentRating }}</span>
              </p>
            </div>
            <div>
              <h3 class="section-title text-sm mb-1">Worst rated losses</h3>
              <p v-if="!stats.worstLosses.length" class="muted">None.</p>
              <p v-for="loss in stats.worstLosses" :key="loss.gameId">
                {{ loss.opponent }} <span class="muted tabular">{{ loss.opponentRating }}</span>
              </p>
            </div>
          </div>
        </template>
        <p v-else class="muted text-sm">Loading…</p>
      </section>
    </div>
  </div>
</template>

<style scoped>
.player-search-empty {
  justify-content: center;
  padding-block: 28px 32px;
}
.player-search-input {
  flex: 0 1 420px;
}
.player-search-empty .player-suggestions {
  justify-content: center;
  margin-top: 4px;
}
.stats-grid {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: 4px 16px;
}
.stats-grid dt {
  color: var(--ui-text-muted);
}
</style>
