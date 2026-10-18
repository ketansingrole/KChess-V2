<script setup lang="ts">
import { useOnline } from '@vueuse/core'
import { computed } from 'vue'
import type {
  AccountStorage,
  AccountUsage,
  LichessAccount,
  LichessUser,
  UserPresence,
} from '@kchess/contracts/types'
import { formatBytes, formatCount, timeAgo } from '../utils/format'
import { useWatchStore } from '../stores/watch'

/** One followed player in the Following table: who they are, how they're rated, what's synced. */
const online = useOnline()
const app = useKChessStore()
const props = defineProps<{
  account: LichessAccount
  profile?: LichessUser | null
  presence?: UserPresence
  /** How the connected account has fared against them, when they have met. */
  versus?: { win: number; loss: number; draw: number }
  storage: AccountStorage
  usage: AccountUsage
  busy?: boolean
}>()
const emit = defineEmits<{ sync: []; remove: []; clear: []; games: []; challenge: [] }>()

async function watchGame(): Promise<void> {
  const id = props.presence?.playingId
  if (id) await useWatchStore().watchGame(id)
}

const ratings = computed(() => [
  props.profile?.perfs?.bullet?.rating,
  props.profile?.perfs?.blitz?.rating,
  props.profile?.perfs?.rapid?.rating,
  props.profile?.perfs?.classical?.rating,
])
const state = computed(() => {
  if (!online.value) return { label: 'Saved profile', className: 'unknown' }
  if (!props.presence) return { label: 'Status unknown', className: 'unknown' }
  if (props.presence.playing) return { label: 'Playing now', className: 'online' }
  return props.presence.online
    ? { label: 'Online', className: 'online' }
    : { label: 'Offline', className: 'offline' }
})
const lastSeen = computed(() =>
  !props.presence?.online && props.profile?.seenAt ? timeAgo(props.profile.seenAt) : '',
)
const stored = computed(() => props.storage.bytes + props.storage.cacheBytes)
const downloaded = computed(
  () =>
    `Downloaded ${formatBytes(props.usage.total.bytesIn)} from Lichess in ${formatCount(props.usage.total.requests)} requests`,
)
const versus = computed(() =>
  props.versus ? `${props.versus.win}W ${props.versus.loss}L ${props.versus.draw}D` : '',
)

const more = computed(() => [
  [
    {
      label: 'Sync',
      icon: 'i-lucide-refresh-cw',
      disabled: props.busy || !online.value,
      onSelect: () => emit('sync'),
    },
    {
      label: 'Clear data',
      icon: 'i-lucide-eraser',
      disabled: !props.storage.games && !props.storage.cacheBytes,
      onSelect: () => emit('clear'),
    },
  ],
  [
    {
      label: 'Remove',
      icon: 'i-lucide-user-minus',
      color: 'error' as const,
      onSelect: () => emit('remove'),
    },
  ],
])
</script>

<template>
  <tr class="friend-row">
    <th scope="row">
      <div class="friend-player">
        <UAvatar :text="account.username.slice(0, 2).toUpperCase()" size="sm" />
        <div class="min-w-0">
          <div class="friend-name">
            <PlayerLink :username="account.username" class="truncate"
              >@{{ account.username }}</PlayerLink
            >
            <UBadge v-if="profile?.title" color="primary" variant="soft" size="sm">{{
              profile.title
            }}</UBadge>
          </div>
          <div class="friend-state" :class="state.className">
            <span class="presence-dot" />
            {{ state.label }}
            <span
              v-if="online && presence?.online && presence.signal"
              class="signal"
              title="Connection signal"
            >
              <i v-for="bar in 4" :key="bar" :class="{ on: bar <= (presence.signal ?? 0) }" />
            </span>
            <template v-if="lastSeen">· seen {{ lastSeen }}</template>
          </div>
        </div>
      </div>
    </th>
    <td
      v-for="(rating, index) in ratings"
      :key="index"
      class="friend-num tabular"
      :class="index === 0 || index === 3 ? 'col-minor' : 'col-rating'"
    >
      {{ rating ?? '—' }}
    </td>
    <td class="friend-num tabular col-versus">{{ versus || '—' }}</td>
    <td class="col-synced" :title="usage.total.requests ? downloaded : undefined">
      <template v-if="storage.games">
        <div class="tabular">{{ formatCount(storage.games) }} games</div>
        <div class="friend-sub tabular">
          {{ formatBytes(stored)
          }}<template v-if="account.lastSyncedAt"> · {{ timeAgo(account.lastSyncedAt) }}</template>
        </div>
      </template>
      <span v-else class="friend-sub">Not synced</span>
    </td>
    <td class="friend-actions">
      <UButton
        v-if="presence?.playingId"
        size="xs"
        variant="soft"
        icon="i-lucide-tv"
        :disabled="!online"
        @click="watchGame"
        >Watch</UButton
      >
      <UButton
        size="xs"
        variant="ghost"
        color="neutral"
        icon="i-lucide-swords"
        :disabled="!online || !app.activeOnlineAccount"
        :title="!app.activeOnlineAccount ? 'Connect Lichess to challenge a player' : undefined"
        @click="emit('challenge')"
        >Challenge</UButton
      >
      <UButton
        size="xs"
        variant="ghost"
        color="neutral"
        icon="i-lucide-history"
        :disabled="!storage.games"
        @click="emit('games')"
        >Games</UButton
      >
      <UDropdownMenu :items="more" :content="{ align: 'end' }">
        <UButton
          size="xs"
          variant="ghost"
          color="neutral"
          icon="i-lucide-ellipsis"
          :aria-label="`More for @${account.username}`"
        />
      </UDropdownMenu>
    </td>
  </tr>
</template>
