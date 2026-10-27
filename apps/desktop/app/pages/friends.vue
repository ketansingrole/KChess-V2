<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useIntervalFn, useOnline } from '@vueuse/core'
import type { LichessAccount, TimelineEntry } from '@kchess/contracts/types'
import { formatBytes, formatCount } from '../utils/format'
import { timelineGroups } from '../utils/activity'

const online = useOnline()
const store = useKChessStore()
const usage = useUsageStore()
const friendsStore = useFriendsStore()
const { busy, usernameInput, trackedAccounts, connectedAccounts, libraryOverview, historyAccount } =
  storeToRefs(store)
const { profiles, status, loading, onlineCount } = storeToRefs(friendsStore)
const { addAccount, removeAccount, sync, selectPage, challengeFriend } = store

const key = (name: string): string => name.toLowerCase()

/** How the connected accounts have fared against a friend. */
function versusOf(username: string): { win: number; loss: number; draw: number } | undefined {
  return libraryOverview.value.versus[key(username)]
}

const stored = (username: string): number => {
  const entry = usage.storageOf(username)
  return entry.bytes + entry.cacheBytes
}
/** Who can be watched or challenged right now comes first: playing, then online, then the rest. */
const sortedFriends = computed(() => {
  const rank = (friend: LichessAccount): number => {
    const presence = status.value[key(friend.username)]
    return presence?.playing ? 0 : presence?.online ? 1 : 2
  }
  return trackedAccounts.value
    .map((friend, index) => ({ friend, index }))
    .sort((a, b) => rank(a.friend) - rank(b.friend) || a.index - b.index)
    .map(({ friend }) => friend)
})

const friendsStored = computed(() =>
  trackedAccounts.value.reduce((sum, friend) => sum + stored(friend.username), 0),
)
const friendsDownloaded = computed(() =>
  trackedAccounts.value.reduce(
    (sum, friend) => sum + usage.usageOf(friend.username).total.bytesIn,
    0,
  ),
)

async function add(): Promise<void> {
  await addAccount()
  void friendsStore.loadProfiles()
  void friendsStore.refreshStatus()
}

watch(online, (connected) => {
  if (connected) {
    void friendsStore.loadProfiles()
    void friendsStore.refreshStatus()
  }
})

const importOpen = ref(false)

const pendingRemoval = ref<LichessAccount | null>(null)
const confirmRemove = ref(false)
function askRemove(friend: LichessAccount): void {
  pendingRemoval.value = friend
  confirmRemove.value = true
}
async function removePending(): Promise<void> {
  const friend = pendingRemoval.value
  if (!friend) return
  await removeAccount(friend.username)
  friendsStore.forget(friend.username)
}
// Kept after closing so the dialog text doesn't flicker while it animates out.
const pendingClear = ref<LichessAccount | null>(null)
const confirmClear = ref(false)
function askClear(friend: LichessAccount): void {
  pendingClear.value = friend
  confirmClear.value = true
}
async function clearPending(): Promise<void> {
  const friend = pendingClear.value
  if (friend) await store.clearSyncedData(friend.username)
}
function showGames(friend: LichessAccount): void {
  historyAccount.value = friend.username
  selectPage('history', { source: 'lichess' })
}

/** What the players the online account follows on Lichess did lately. */
const timeline = ref<TimelineEntry[] | null>(null)
const timelineError = ref('')
let timelineRequest = 0
async function loadTimeline(): Promise<void> {
  const account = store.activeOnlineAccount
  const current = ++timelineRequest
  timelineError.value = ''
  if (!account || !online.value) return
  try {
    const answer = await window.kchess.timeline(account)
    if (current !== timelineRequest) return
    timeline.value = Array.isArray(answer) ? answer : []
  } catch (cause) {
    if (current !== timelineRequest) return
    console.warn('[friends] Timeline unavailable:', cause)
    timelineError.value = cause instanceof Error ? cause.message : String(cause)
  }
}
const timelineList = computed(() => timelineGroups(timeline.value ?? []))
watch(
  () => [store.activeOnlineAccount, online.value],
  () => {
    timeline.value = null
    void loadTimeline()
  },
  { immediate: true },
)

onMounted(() => {
  void usage.refresh()
  void friendsStore.loadProfiles()
  void friendsStore.refreshStatus()
})
// Cheap (one small request for everyone), and stops with the page.
useIntervalFn(() => void friendsStore.refreshStatus(), 20_000)
watch(
  () => trackedAccounts.value.length,
  () => void friendsStore.refreshStatus(),
)
</script>

<template>
  <div>
    <PageHeader title="Following">
      <template #lead>
        <form class="friends-add" @submit.prevent="add">
          <UInput
            v-model="usernameInput"
            icon="i-lucide-user-plus"
            placeholder="Lichess username"
            aria-label="Lichess username to follow"
            autocomplete="off"
            class="friends-add-input"
          />
          <UButton
            type="submit"
            :title="
              connectedAccounts.length
                ? `Follows on Lichess as @${store.activeOnlineAccount} and downloads public games.`
                : 'Follows in KChess and downloads public games.'
            "
            :disabled="!online || !usernameInput.trim() || busy"
            icon="i-lucide-plus"
            >Follow player</UButton
          >
          <UButton
            v-if="connectedAccounts.length"
            variant="outline"
            color="neutral"
            icon="i-lucide-download"
            :disabled="!online"
            title="Adds players without downloading their games"
            @click="importOpen = true"
            >Import from Lichess</UButton
          >
        </form>
      </template>
      <UButton
        v-if="trackedAccounts.length"
        variant="outline"
        color="neutral"
        icon="i-lucide-refresh-cw"
        :loading="busy"
        :disabled="!online"
        @click="sync()"
        >Sync all</UButton
      >
    </PageHeader>

    <PublicOnlineNotice offline-message="Offline · showing saved players." />
    <UEmpty
      v-if="!trackedAccounts.length"
      variant="naked"
      icon="i-lucide-users"
      title="Follow your first player"
    />
    <template v-else>
      <p class="friends-summary tabular" role="status">
        {{ formatCount(trackedAccounts.length) }}
        {{ trackedAccounts.length === 1 ? 'player' : 'players' }} · {{ onlineCount }}
        {{ online ? 'online' : 'last known online' }} · {{ formatBytes(friendsStored) }} on this
        computer · {{ formatBytes(friendsDownloaded) }} downloaded
        <UIcon
          v-if="loading"
          name="i-lucide-loader-circle"
          class="animate-spin align-[-2px] ms-1"
          aria-label="Refreshing profiles"
        />
      </p>
      <div class="friends-frame">
        <table class="friends-table">
          <thead>
            <tr>
              <th scope="col">Player</th>
              <th scope="col" class="friend-num col-minor">Bullet</th>
              <th scope="col" class="friend-num col-rating">Blitz</th>
              <th scope="col" class="friend-num col-rating">Rapid</th>
              <th scope="col" class="friend-num col-minor">Classical</th>
              <th scope="col" class="friend-num col-versus">You vs them</th>
              <th scope="col" class="col-synced">Synced</th>
              <th scope="col"><span class="sr-only">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            <FriendRow
              v-for="friend in sortedFriends"
              :key="friend.username"
              :account="friend"
              :profile="profiles[key(friend.username)]"
              :presence="status[key(friend.username)]"
              :versus="versusOf(friend.username)"
              :storage="usage.storageOf(friend.username)"
              :usage="usage.usageOf(friend.username)"
              :busy="busy"
              @sync="sync(friend.username)"
              @remove="askRemove(friend)"
              @clear="askClear(friend)"
              @games="showGames(friend)"
              @challenge="challengeFriend(friend.username)"
            />
          </tbody>
        </table>
      </div>
    </template>

    <section v-if="store.activeOnlineAccount && online" class="card mt-6">
      <h2 class="section-title mb-3">Timeline</h2>
      <p v-if="timelineError" class="text-error text-sm" role="alert">{{ timelineError }}</p>
      <p v-else-if="!timeline" class="muted text-sm">Loading…</p>
      <p v-else-if="!timeline.length" class="muted text-sm">Nothing new.</p>
      <ActivityList v-else :groups="timelineList" />
    </section>

    <FriendImportDialog v-model:open="importOpen" />
    <ConfirmDialog
      v-model:open="confirmClear"
      title="Clear this player's synced data?"
      :description="`Deletes @${pendingClear?.username}'s ${formatCount(usage.storageOf(pendingClear?.username ?? '').games)} stored games and cached profile (${formatBytes(stored(pendingClear?.username ?? ''))}) from this device. They stay in Following, and syncing again downloads everything afresh.`"
      confirm-label="Clear data"
      color="error"
      @confirm="clearPending"
    />

    <ConfirmDialog
      v-model:open="confirmRemove"
      title="Stop following this player?"
      :description="`@${pendingRemoval?.username} and their ${formatCount(usage.storageOf(pendingRemoval?.username ?? '').games)} stored games (${formatBytes(stored(pendingRemoval?.username ?? ''))}) will be deleted from this device. They stay removed until you add them again.${connectedAccounts.length ? ` @${store.activeOnlineAccount} stops following them on Lichess.` : ''}`"
      confirm-label="Remove"
      color="error"
      @confirm="removePending"
    />
  </div>
</template>
