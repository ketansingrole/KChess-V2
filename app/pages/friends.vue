<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useIntervalFn, useOnline } from '@vueuse/core'
import type { LichessAccount } from '../../src/shared/types'
import { formatBytes, formatCount, timeAgo } from '../utils/format'

const online = useOnline()
const store = useKChessStore()
const usage = useUsageStore()
const friendsStore = useFriendsStore()
const { busy, usernameInput, trackedAccounts, connectedAccounts, libraryOverview, historyAccount } =
  storeToRefs(store)
const { profiles, status, loading, onlineCount } = storeToRefs(friendsStore)
const { addAccount, removeAccount, sync, selectPage, challengeFriend } = store

const key = (name: string): string => name.toLowerCase()

/** Results of a friend's own stored games, from their point of view. */
function recordOf(username: string): { win: number; loss: number; draw: number } {
  return libraryOverview.value.byAccount[key(username)] ?? { win: 0, loss: 0, draw: 0 }
}
/** How the connected accounts have fared against a friend. */
function versusOf(username: string): { win: number; loss: number; draw: number } | undefined {
  return libraryOverview.value.versus[key(username)]
}

const stored = (username: string): number => {
  const entry = usage.storageOf(username)
  return entry.bytes + entry.cacheBytes
}
const storageShare = (username: string): number =>
  usage.totalStored ? stored(username) / usage.totalStored : 0

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
  selectPage('history')
}

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

    <PublicOnlineNotice
      offline-message="Showing saved players. Reconnect to follow someone new, refresh profiles or check who is playing."
    />
    <div class="card friends-add">
      <form class="toolbar-row flex-nowrap" @submit.prevent="add">
        <UInput
          v-model="usernameInput"
          icon="i-lucide-user-plus"
          placeholder="Lichess username"
          aria-label="Lichess username to follow"
          autocomplete="off"
          class="flex-1 max-w-sm"
        />
        <UButton
          type="submit"
          :disabled="!online || !usernameInput.trim() || busy"
          icon="i-lucide-plus"
          >Follow player</UButton
        >
        <UButton
          variant="outline"
          color="neutral"
          icon="i-lucide-download"
          :disabled="!online || !connectedAccounts.length"
          :title="connectedAccounts.length ? undefined : 'Connect a Lichess account first'"
          @click="importOpen = true"
          >Import from Lichess</UButton
        >
      </form>
      <p class="section-hint">
        Following a player by name downloads their public games (up to 5,000) so you can browse them
        offline; importing from Lichess adds them without games until you sync each one. Every
        download is counted below. Following here is saved on this device and does not change who
        you follow on Lichess.
      </p>
    </div>

    <div v-if="trackedAccounts.length" class="friends-summary" role="status">
      <div>
        <span class="stat-label">Following</span>
        <strong class="tabular">{{ trackedAccounts.length }}</strong>
      </div>
      <div>
        <span class="stat-label">{{ online ? 'Online now' : 'Last known online' }}</span>
        <strong class="tabular">{{ onlineCount }}</strong>
      </div>
      <div>
        <span class="stat-label">Stored for followed players</span>
        <strong class="tabular">{{ formatBytes(friendsStored) }}</strong>
      </div>
      <div>
        <span class="stat-label">Downloaded for followed players</span>
        <strong class="tabular">{{ formatBytes(friendsDownloaded) }}</strong>
      </div>
      <span v-if="loading" class="muted text-xs friends-loading"
        ><UIcon name="i-lucide-loader-circle" class="animate-spin" /> Refreshing profiles…</span
      >
    </div>

    <div v-if="!trackedAccounts.length" class="card">
      <UEmpty
        variant="naked"
        icon="i-lucide-users"
        title="Follow your first player"
        description="Enter a Lichess username above to follow their public ratings and games. No sign-in is required. They stay here until you remove them."
      />
    </div>
    <div v-else class="friends-grid">
      <FriendCard
        v-for="friend in trackedAccounts"
        :key="friend.username"
        :account="friend"
        :profile="profiles[key(friend.username)]"
        :presence="status[key(friend.username)]"
        :record="recordOf(friend.username)"
        :versus="versusOf(friend.username)"
        :storage="usage.storageOf(friend.username)"
        :usage="usage.usageOf(friend.username)"
        :storage-share="storageShare(friend.username)"
        :busy="busy"
        @sync="sync(friend.username)"
        @remove="askRemove(friend)"
        @clear="askClear(friend)"
        @games="showGames(friend)"
        @challenge="challengeFriend(friend.username)"
      />
    </div>

    <p v-if="status && trackedAccounts.length" class="muted text-xs mt-3">
      When online, status refreshes every 20 seconds. Last checked
      {{ timeAgo(friendsStore.statusAt) }}. {{ formatCount(usage.totalRequests) }} requests made to
      Lichess in total; see Settings → Data &amp; storage.
    </p>

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
      :description="`@${pendingRemoval?.username} and their ${formatCount(usage.storageOf(pendingRemoval?.username ?? '').games)} stored games (${formatBytes(stored(pendingRemoval?.username ?? ''))}) will be deleted from this device. They stay removed until you add them again. Nothing changes on Lichess.`"
      confirm-label="Remove"
      color="error"
      @confirm="removePending"
    />
  </div>
</template>
