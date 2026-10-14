<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch as watchValue } from 'vue'
import { useLocalStorage, useOnline } from '@vueuse/core'
import { storeToRefs } from 'pinia'
import TvPanel from '../components/watch/TvPanel.vue'
import FriendsPanel from '../components/watch/FriendsPanel.vue'
import BroadcastPanel from '../components/watch/BroadcastPanel.vue'
import { useBroadcastNav, type WatchTab } from '../composables/useBroadcastNav'
import { WATCH_TAB_KEY } from '../utils/storageKeys'
import { useWatchStore } from '../stores/watch'
import { useFriendsStore } from '../stores/friends'
import { useKChessStore } from '../stores/kchess'

const online = useOnline()
const store = useKChessStore()
const watcher = useWatchStore()
const friends = useFriendsStore()
const { onlinePhase } = storeToRefs(store)
const tab = useLocalStorage<WatchTab>(WATCH_TAB_KEY, 'tv')
const playingOwnGame = computed(
  () =>
    onlinePhase.value === 'playing' ||
    onlinePhase.value === 'seeking' ||
    (onlinePhase.value === 'disconnected' && Boolean(store.onlineConnection?.gameId)),
)
const nav = useBroadcastNav(tab)
function openBroadcastRound(id: string): void {
  nav.swipeRedo.value = null
  void watcher.openRound(id)
}
const broadcastGameOpen = computed(
  () => tab.value === 'broadcasts' && watcher.tour !== null && watcher.currentGame !== undefined,
)

/** Reopens the chosen broadcast round's feed, which stopping the stream left idle. */
function resumeBroadcast(): void {
  if (tab.value === 'broadcasts' && online.value && !playingOwnGame.value) void watcher.resumeTour()
}
onMounted(() => {
  watcher.listen()
  if (online.value) {
    void watcher.loadChannels()
    void friends.refreshStatus()
    void watcher.loadBroadcasts()
    resumeBroadcast()
  }
})
onUnmounted(() => {
  // Leaving the page stops the stream: nothing downloads for a board no one sees.
  void watcher.stop()
  watcher.unlisten()
})
watchValue(tab, async () => {
  nav.swipeRedo.value = null
  await watcher.stop()
  resumeBroadcast()
})
watchValue(online, (connected) => {
  if (connected) {
    void watcher.loadChannels()
    void friends.refreshStatus()
    void watcher.loadBroadcasts()
    resumeBroadcast()
  } else void watcher.stop()
})
</script>

<template>
  <div :class="{ 'broadcast-page analysis-surface': broadcastGameOpen }">
    <PageHeader title="Watch">
      <!-- An open tournament's toolbar (BroadcastPanel) sits here, beside the tabs. -->
      <template #lead><div data-watch-lead class="watch-lead" /></template>
      <UTabs
        v-model="tab"
        :items="[
          { label: 'TV', value: 'tv', icon: 'i-lucide-tv' },
          { label: 'Following', value: 'friends', icon: 'i-lucide-users' },
          { label: 'Broadcasts', value: 'broadcasts', icon: 'i-lucide-radio' },
        ]"
        :content="false"
        variant="pill"
      />
    </PageHeader>

    <PublicOnlineNotice v-if="!online" offline-message="Offline · reconnect to watch live games." />
    <UAlert
      v-if="playingOwnGame"
      class="mb-4"
      color="warning"
      variant="subtle"
      icon="i-lucide-swords"
      :title="onlinePhase === 'seeking' ? 'Finding a game' : 'You are in a game'"
      description="Watching is paused while your game uses the connection."
    />
    <UAlert
      v-else-if="onlinePhase === 'disconnected'"
      class="mb-4"
      color="warning"
      variant="subtle"
      icon="i-lucide-refresh-cw"
      :title="
        store.onlineConnection?.phase === 'checking'
          ? 'Checking game status…'
          : 'Game status not verified'
      "
      :description="store.onlineConnection?.phase === 'checking' ? undefined : store.onlineStatus"
    >
      <template #actions>
        <UButton
          size="sm"
          variant="outline"
          :disabled="!online || store.busy || store.onlineConnection?.phase === 'checking'"
          @click="store.reconnectOnline()"
          >Check again</UButton
        >
      </template>
    </UAlert>
    <UButton
      v-if="
        watcher.target &&
        watcher.connection &&
        ['error', 'ended'].includes(watcher.connection.phase)
      "
      size="xs"
      :disabled="!online"
      @click="watcher.watch(watcher.target)"
      >Retry feed</UButton
    >
    <p v-if="watcher.error" class="text-error text-sm mb-3" role="alert">{{ watcher.error }}</p>

    <TvPanel v-if="tab === 'tv'" :online="online" :playing-own-game="playingOwnGame" />
    <FriendsPanel
      v-else-if="tab === 'friends'"
      :online="online"
      :playing-own-game="playingOwnGame"
    />
    <BroadcastPanel
      v-else
      :online="online"
      :playing-own-game="playingOwnGame"
      @select-game="nav.selectBroadcastGame"
      @close-game="nav.closeBroadcastGame"
      @leave-tour="nav.leaveTour"
      @open-round="openBroadcastRound"
    />
  </div>
</template>

<style scoped>
.watch-lead {
  display: flex;
  flex: 1;
  min-width: 0;
}
/* While a broadcast game is open the whole page, header tabs included, is as wide as the
   board and panel below it, so every right-hand control shares one edge. */
.broadcast-page {
  max-width: calc(var(--board-size) + 486px);
  margin-inline: auto;
}
</style>
