<script setup lang="ts">
import { computed } from 'vue'
import type {
  AccountStorage,
  AccountUsage,
  LichessAccount,
  LichessUser,
  UserPresence,
} from '../../src/shared/types'
import { formatBytes, formatCount, timeAgo } from '../utils/format'
import { useWatchStore } from '../stores/watch'

const props = defineProps<{
  account: LichessAccount
  profile?: LichessUser | null
  presence?: UserPresence
  /** Results of the friend's games that are stored here. */
  record: { win: number; loss: number; draw: number }
  /** How the connected account has fared against them, when they have met. */
  versus?: { win: number; loss: number; draw: number }
  storage: AccountStorage
  usage: AccountUsage
  /** This friend's share (0–1) of everything KChess stores. */
  storageShare: number
  busy?: boolean
}>()
defineEmits<{ sync: []; remove: []; clear: []; games: []; challenge: [] }>()
async function watchGame(): Promise<void> {
  const id = props.presence?.playingId
  if (!id) return
  localStorage.setItem('kchess:watch-tab', 'friends')
  await useWatchStore().watch({ gameId: id })
  void navigateTo('/watch')
}

const perfs = computed(() => [
  { label: 'Bullet', value: props.profile?.perfs?.bullet?.rating },
  { label: 'Blitz', value: props.profile?.perfs?.blitz?.rating },
  { label: 'Rapid', value: props.profile?.perfs?.rapid?.rating },
  { label: 'Classical', value: props.profile?.perfs?.classical?.rating },
])
const state = computed(() => {
  if (!props.presence) return { label: 'Status unknown', className: 'unknown' }
  if (props.presence.playing) return { label: 'Playing now', className: 'online' }
  return props.presence.online
    ? { label: 'Online', className: 'online' }
    : { label: 'Offline', className: 'offline' }
})
const stored = computed(() => props.storage.bytes + props.storage.cacheBytes)
const totalGames = computed(() => props.profile?.count?.all)
const memberSince = computed(() =>
  props.profile?.createdAt ? new Date(props.profile.createdAt).toLocaleDateString() : undefined,
)
const lastSeen = computed(() => props.profile?.seenAt)
const kinds = computed(() => [
  { label: 'Games', cell: props.usage.byKind.games },
  { label: 'Profile & ratings', cell: props.usage.byKind.profile },
  { label: 'Online status', cell: props.usage.byKind.presence },
])
const formatRecord = (r: { win: number; loss: number; draw: number }): string =>
  `${r.win}W ${r.loss}L ${r.draw}D`
</script>

<template>
  <article class="card friend-card" :aria-label="`@${account.username}`">
    <header class="friend-head">
      <UAvatar :text="account.username.slice(0, 2).toUpperCase()" size="lg" />
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <strong class="truncate">@{{ account.username }}</strong>
          <UBadge v-if="profile?.title" color="primary" variant="soft" size="sm">{{
            profile.title
          }}</UBadge>
        </div>
        <div class="friend-state" :class="state.className">
          <span class="presence-dot" /> {{ state.label }}
          <span v-if="presence?.online && presence.signal" class="signal" title="Connection signal">
            <i v-for="bar in 4" :key="bar" :class="{ on: bar <= (presence.signal ?? 0) }" />
          </span>
          <span v-if="!presence?.online && lastSeen" class="muted"
            >· seen {{ timeAgo(lastSeen) }}</span
          >
        </div>
      </div>
    </header>

    <div class="friend-ratings">
      <div v-for="perf in perfs" :key="perf.label" class="friend-rating">
        <span class="stat-label">{{ perf.label }}</span>
        <strong class="tabular">{{ perf.value ?? '—' }}</strong>
      </div>
    </div>

    <dl class="friend-facts">
      <div>
        <dt>Lichess games</dt>
        <dd class="tabular">{{ totalGames !== undefined ? formatCount(totalGames) : '—' }}</dd>
      </div>
      <div>
        <dt>Member since</dt>
        <dd>{{ memberSince ?? '—' }}</dd>
      </div>
      <div>
        <dt>Their synced games</dt>
        <dd class="tabular">{{ storage.games ? formatRecord(record) : 'Not synced' }}</dd>
      </div>
      <div>
        <dt>You vs them</dt>
        <dd class="tabular">{{ versus ? formatRecord(versus) : 'Never played' }}</dd>
      </div>
    </dl>

    <section class="friend-data" :aria-label="`Data used by @${account.username}`">
      <div class="friend-data-head">
        <span class="stat-label">Data</span>
        <span class="muted text-xs"
          >Last synced {{ account.lastSyncedAt ? timeAgo(account.lastSyncedAt) : 'never' }}</span
        >
      </div>
      <div
        class="meter"
        role="img"
        :aria-label="`${Math.round(storageShare * 100)}% of the space KChess uses`"
      >
        <span :style="{ width: `${Math.max(storageShare * 100, stored ? 2 : 0)}%` }" />
      </div>
      <div class="friend-data-line">
        <span>On this computer</span>
        <strong class="tabular"
          >{{ formatBytes(stored) }} · {{ formatCount(storage.games) }} games</strong
        >
      </div>
      <div class="friend-data-line">
        <span>Downloaded from Lichess</span>
        <strong class="tabular"
          >{{ formatBytes(usage.total.bytesIn) }} ·
          {{ formatCount(usage.total.requests) }} requests</strong
        >
      </div>
      <ul v-if="usage.total.requests" class="friend-data-kinds">
        <li v-for="kind in kinds.filter((k) => k.cell)" :key="kind.label">
          <span>{{ kind.label }}</span>
          <span class="tabular">{{ formatBytes(kind.cell!.bytesIn) }}</span>
        </li>
      </ul>
    </section>

    <footer class="friend-actions">
      <UButton
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-history"
        :disabled="!storage.games"
        @click="$emit('games')"
        >Games</UButton
      >
      <UButton
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-swords"
        @click="$emit('challenge')"
        >Challenge</UButton
      >
      <UButton
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-chart-column"
        @click="navigateTo({ path: '/players', query: { name: account.username } })"
        >Stats</UButton
      >
      <UButton
        v-if="presence?.playingId"
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-tv"
        @click="watchGame"
        >Watch</UButton
      >
      <UButton
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-refresh-cw"
        :disabled="busy"
        @click="$emit('sync')"
        >Sync</UButton
      >
      <UButton
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-eraser"
        :disabled="busy || (!storage.games && !storage.cacheBytes)"
        :title="`Delete the ${formatBytes(storage.bytes + storage.cacheBytes)} downloaded for this friend`"
        @click="$emit('clear')"
        >Clear data</UButton
      >
      <UButton
        size="sm"
        variant="ghost"
        color="error"
        icon="i-lucide-user-minus"
        class="ml-auto"
        :disabled="busy"
        @click="$emit('remove')"
        >Remove</UButton
      >
    </footer>
  </article>
</template>
