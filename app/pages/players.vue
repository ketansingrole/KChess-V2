<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useLocalStorage, useOnline } from '@vueuse/core'
import type {
  Crosstable,
  LichessGame,
  LichessRatingHistory,
  LichessUser,
  PerfStats,
  PerfType,
  UserPresence,
} from '../../src/shared/types'
import { PERF_TYPES } from '../../src/shared/types'
import { USERNAME } from '../../src/shared/patterns'
import { useWatchStore } from '../stores/watch'
import { useAnalysisStore } from '../stores/analysis'
import { mergeRatingHistories, ratingHistoryFromGames } from '../../src/shared/ratings'

const online = useOnline()
const store = useKChessStore()
const { activeOnlineAccount, connectedAccounts, data, trackedAccounts } = storeToRefs(store)
const route = useRoute()
const watcher = useWatchStore()
const analysis = useAnalysisStore()
const toast = useToast()

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
const games = ref<LichessGame[] | null>(null)
const history = ref<LichessRatingHistory | null>(null)
/** The chart was rebuilt from recent rated games because Lichess sent no history. */
const historyFromGames = ref(false)
const gamesError = ref('')
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
  games.value = null
  gamesError.value = ''
  history.value = null
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
        .catch((error: unknown) => {
          console.warn('[players] Crosstable unavailable:', error)
          return undefined
        })
    void loadHistory(found.username, current)
    void loadGames(found.username, current)
    void window.kchess
      .presence([found.username])
      .then((report) => {
        if (current === request) status.value = report.users[found.username.toLowerCase()] ?? null
      })
      .catch((error: unknown) => {
        console.warn('[players] Presence unavailable:', error)
        return undefined
      })
  } catch (cause) {
    if (current === request) {
      console.warn('[players] Profile lookup failed:', cause)
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
    if (current === request) {
      console.warn('[players] Performance stats unavailable:', cause)
      statsError.value = cause instanceof Error ? cause.message : String(cause)
    }
  }
}
watch(perf, () => void loadStats())
if (query.value) void lookup()
// A profile link opened while this page is showing changes only the query, not the page.
watch(
  () => route.query.name,
  (name) => {
    if (typeof name !== 'string' || name.toLowerCase() === profile.value?.username.toLowerCase())
      return
    query.value = name
    void lookup(name)
  },
)

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
/** Current rating per speed, keyed by rating-history key (`blitz`, `puzzle`). */
const currentRatings = computed(() =>
  Object.fromEntries(
    perfs.value.map((entry) => [
      entry.key,
      { rating: entry.rating, provisional: entry.prov, progress: entry.prog },
    ]),
  ),
)
const perfItems = computed(() =>
  perfs.value.map((entry) => ({ label: PERF_LABELS[entry.key], value: entry.key })),
)
function hours(seconds: number): string {
  const h = seconds / 3600
  return h >= 1 ? `${Math.round(h)} h` : `${Math.round(seconds / 60)} min`
}
async function loadGames(username: string, current: number): Promise<void> {
  games.value = null
  gamesError.value = ''
  try {
    const list = await window.kchess.recentGames(username)
    if (current === request) games.value = list
  } catch (cause) {
    console.warn('[players] Recent games unavailable:', cause)
    // Lichess sometimes answers "not found" for every player's games; say so plainly.
    if (current === request)
      gamesError.value = 'Lichess isn’t sending this player’s games right now.'
  }
}
/** Asks Lichess again for the history and games it did not send. */
function retryActivity(): void {
  const user = profile.value
  if (!user) return
  history.value = null
  void loadHistory(user.username, request)
  void loadGames(user.username, request)
}
/** Rated games on the profile but nothing to chart: Lichess withheld the data. */
const historyEmptyText = computed(() =>
  perfs.value.length ? 'Lichess isn’t sending this player’s rating history right now.' : undefined,
)

/**
 * Lichess's rating history, or (when it sends none, as its endpoint sometimes does for
 * everyone) one rebuilt from the player's recent rated games.
 */
async function loadHistory(username: string, current: number): Promise<void> {
  const official = await window.kchess.ratingHistory(username).catch((error: unknown) => {
    console.warn('[players] Rating history unavailable:', error)
    return []
  })
  if (current !== request) return
  if (official.some((entry) => entry.points?.length)) {
    history.value = official
    historyFromGames.value = false
    return
  }
  const rated = await window.kchess.recentGames(username, true).catch((error: unknown) => {
    console.warn('[players] Rated games unavailable:', error)
    return []
  })
  if (current !== request) return
  history.value = mergeRatingHistories(official, ratingHistoryFromGames(rated))
  historyFromGames.value = rated.length > 0
}

/** Opens one of the player's games in the analysis board, from their side. */
async function openGame(game: LichessGame): Promise<void> {
  try {
    const pgn = await window.kchess.exportGame(game.id)
    if (!analysis.loadPgn(pgn)) throw new Error('That game could not be read.')
    analysis.orientation = game.color
    analysis.origin = {
      white: game.color === 'white' ? game.account : game.opponent,
      black: game.color === 'white' ? game.opponent : game.account,
      gameId: game.id,
    }
    store.selectPage('analysis')
  } catch (cause) {
    console.warn('[players] Could not open the game:', cause)
    toast.add({
      title: 'Could not open the game',
      description: cause instanceof Error ? cause.message : String(cause),
      color: 'error',
    })
  }
}

/* ── Messages ─────────────────────────────────────────────────────────── */
const canMessage = computed(
  () =>
    connectedAccounts.value.length > 0 &&
    !!profile.value &&
    profile.value.username.toLowerCase() !== activeOnlineAccount.value.toLowerCase(),
)
const messageOpen = ref(false)
const messageText = ref('')
const messageBusy = ref(false)
const messageError = ref('')
/** The account was connected before messaging was requested and must be connected again. */
const messageReconnect = ref(false)
function openMessage(): void {
  messageError.value = ''
  messageReconnect.value = false
  messageOpen.value = true
}
async function sendMessage(): Promise<void> {
  const user = profile.value
  const text = messageText.value.trim()
  if (!user || !text || messageBusy.value) return
  messageBusy.value = true
  messageError.value = ''
  try {
    const result = await window.kchess.sendMessage(activeOnlineAccount.value, user.username, text)
    if ('needsReconnect' in result) {
      messageReconnect.value = true
      return
    }
    messageOpen.value = false
    messageText.value = ''
    toast.add({ title: `Message sent to ${user.username}`, color: 'success' })
  } catch (cause) {
    console.warn('[players] Message send failed:', cause)
    messageError.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    messageBusy.value = false
  }
}
async function reconnect(): Promise<void> {
  messageOpen.value = false
  await store.connect()
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
              v-if="canMessage"
              size="sm"
              variant="outline"
              color="neutral"
              icon="i-lucide-message-circle"
              :disabled="!online"
              @click="openMessage"
              >Message</UButton
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
        <h3 class="section-title text-sm mb-3">
          Rating history
          <span v-if="historyFromGames" class="muted text-xs font-normal">
            · from the last 200 rated games</span
          >
        </h3>
        <RatingHistoryChart
          v-if="history"
          :history="history"
          :current="currentRatings"
          :empty-text="historyEmptyText"
        >
          <template v-if="historyEmptyText" #empty>
            <UButton
              size="xs"
              variant="link"
              icon="i-lucide-rotate-cw"
              :disabled="!online"
              @click="retryActivity"
              >Retry</UButton
            >
          </template>
        </RatingHistoryChart>
        <p v-else class="muted text-sm">Loading…</p>
      </section>
      <section class="card">
        <div class="card-header record-header">
          <div>
            <h2 class="section-title">Record</h2>
            <p v-if="stats" class="section-hint">
              <strong class="tabular record-rating">{{
                stats.rating ? Math.round(stats.rating) : '–'
              }}</strong>
              <span v-if="stats.deviation"> ± {{ Math.round(stats.deviation) }}</span>
              <span v-if="stats.rank"> · rank #{{ stats.rank }}</span>
              <span v-if="stats.percentile"> · better than {{ stats.percentile }}% of players</span>
            </p>
          </div>
          <USelect
            v-if="perfItems.length"
            v-model="perf"
            :items="perfItems"
            size="sm"
            aria-label="Speed"
            class="w-40"
          />
        </div>
        <p v-if="statsError" class="text-error text-sm" role="alert">{{ statsError }}</p>
        <template v-else-if="stats">
          <ResultBar
            label="Results"
            :record="{
              win: stats.count.win,
              draw: stats.count.draw,
              loss: stats.count.loss,
              total: stats.count.all,
            }"
          />
          <dl class="stats-list text-sm">
            <div>
              <dt>Games</dt>
              <dd>{{ stats.count.all }} · {{ stats.count.rated }} rated</dd>
            </div>
            <div>
              <dt>Opponents' average</dt>
              <dd>{{ Math.round(stats.count.opAvg) }}</dd>
            </div>
            <div>
              <dt>Time played</dt>
              <dd>{{ hours(stats.count.seconds) }}</dd>
            </div>
            <div>
              <dt>Tournament games</dt>
              <dd>{{ stats.count.tour }} · berserked {{ stats.count.berserk }}</dd>
            </div>
            <div>
              <dt>Highest</dt>
              <dd>
                <template v-if="stats.highest"
                  >{{ stats.highest.rating }}
                  <span class="muted">{{
                    new Date(stats.highest.at).toLocaleDateString()
                  }}</span></template
                ><template v-else>–</template>
              </dd>
            </div>
            <div>
              <dt>Lowest</dt>
              <dd>
                <template v-if="stats.lowest"
                  >{{ stats.lowest.rating }}
                  <span class="muted">{{
                    new Date(stats.lowest.at).toLocaleDateString()
                  }}</span></template
                ><template v-else>–</template>
              </dd>
            </div>
            <div>
              <dt>Winning streak</dt>
              <dd>{{ stats.winStreak.current }} now · best {{ stats.winStreak.best }}</dd>
            </div>
            <div>
              <dt>Losing streak</dt>
              <dd>{{ stats.lossStreak.current }} now · worst {{ stats.lossStreak.best }}</dd>
            </div>
          </dl>
          <div class="record-opponents text-sm">
            <div>
              <h3 class="section-title text-sm">Best rated wins</h3>
              <p v-if="!stats.bestWins.length" class="muted">None yet.</p>
              <ul v-else class="opponent-list">
                <li v-for="win in stats.bestWins" :key="win.gameId">
                  <PlayerLink :username="win.opponent" class="truncate" />
                  <span class="muted tabular">{{ win.opponentRating }}</span>
                </li>
              </ul>
            </div>
            <div>
              <h3 class="section-title text-sm">Worst rated losses</h3>
              <p v-if="!stats.worstLosses.length" class="muted">None.</p>
              <ul v-else class="opponent-list">
                <li v-for="loss in stats.worstLosses" :key="loss.gameId">
                  <PlayerLink :username="loss.opponent" class="truncate" />
                  <span class="muted tabular">{{ loss.opponentRating }}</span>
                </li>
              </ul>
            </div>
          </div>
        </template>
        <p v-else class="muted text-sm">Loading…</p>
      </section>
      <section class="card player-games">
        <h2 class="section-title mb-3">Recent games</h2>
        <p v-if="gamesError" class="muted text-sm" role="status">
          {{ gamesError }}
          <UButton
            size="xs"
            variant="link"
            icon="i-lucide-rotate-cw"
            :disabled="!online"
            @click="retryActivity"
            >Retry</UButton
          >
        </p>
        <p v-else-if="!games" class="muted text-sm">Loading…</p>
        <p v-else-if="!games.length" class="muted text-sm">No games yet.</p>
        <div v-else class="game-list">
          <div class="game-head" aria-hidden="true">
            <span>Result</span><span>Opponent</span><span>Mode</span><span>Rating</span
            ><span>Accuracy</span><span>Date</span>
          </div>
          <GameRow
            v-for="game in games"
            :key="game.id"
            :game="game"
            detailed
            @select="openGame(game)"
          />
        </div>
      </section>
    </div>
    <UModal
      v-model:open="messageOpen"
      :title="profile ? `Message ${profile.username}` : 'Message'"
      :description="`Sent from @${activeOnlineAccount} as a Lichess private message.`"
    >
      <template #body>
        <UAlert
          v-if="messageReconnect"
          color="warning"
          variant="subtle"
          icon="i-lucide-key-round"
          title="Lichess needs a new permission"
          :description="`Connect @${activeOnlineAccount} again to allow sending messages.`"
          :actions="[{ label: 'Reconnect', icon: 'i-lucide-link', onClick: reconnect }]"
          class="mb-3"
        />
        <UTextarea
          v-model="messageText"
          :rows="5"
          :maxlength="8000"
          autoresize
          placeholder="Write a message"
          aria-label="Message"
          class="w-full"
          @keydown.meta.enter="sendMessage"
          @keydown.ctrl.enter="sendMessage"
        />
        <p v-if="messageError" class="text-error text-sm mt-2" role="alert">{{ messageError }}</p>
      </template>
      <template #footer>
        <div class="flex w-full justify-end gap-2">
          <UButton variant="ghost" color="neutral" @click="messageOpen = false">Cancel</UButton>
          <UButton
            icon="i-lucide-send"
            :loading="messageBusy"
            :disabled="!messageText.trim() || !online"
            @click="sendMessage"
            >Send</UButton
          >
        </div>
      </template>
    </UModal>
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
.player-games {
  grid-column: 1 / -1;
}
.record-header {
  align-items: flex-start;
  margin-bottom: 14px;
}
.record-rating {
  color: var(--ui-text-highlighted);
  font-size: 1rem;
}
/* Label left, value right, on one ruled line each, like the settings lists. */
.stats-list {
  display: grid;
  margin: 14px 0 0;
}
.stats-list > div {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
  padding: 7px 0;
  border-top: 1px solid var(--ui-border);
}
.stats-list dt {
  color: var(--ui-text-muted);
}
.stats-list dd {
  margin: 0;
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.record-opponents {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
  gap: 16px 24px;
  margin-top: 18px;
}
.record-opponents .section-title {
  margin-bottom: 6px;
}
.opponent-list {
  display: grid;
  gap: 4px;
}
.opponent-list li {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  min-width: 0;
}
</style>
