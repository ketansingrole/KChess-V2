<script setup lang="ts">
import { computed } from 'vue'
import { useTvPosition } from '../../composables/useTvPosition'
import { useWatchStore } from '../../stores/watch'
import TvViewer from './TvViewer.vue'
import { useFriendsStore } from '../../stores/friends'

defineProps<{
  online: boolean
  playingOwnGame: boolean
}>()

const watcher = useWatchStore()
const friends = useFriendsStore()
const tv = useTvPosition()

const playingFriends = computed(() =>
  friends.friends
    .map((friend) => ({ friend, status: friends.status[friend.username.toLowerCase()] }))
    .filter((entry) => entry.status?.playing),
)
</script>

<template>
  <div class="flex flex-col gap-6">
    <TvViewer :tv="tv">
      <template #header>
        <div>
          <h2 class="section-title">Followed players playing now</h2>
          <p class="section-hint">
            {{
              friends.friends.length
                ? `${friends.onlineCount} of ${friends.friends.length} online`
                : 'Follow players on the Following page.'
            }}
          </p>
        </div>
        <UButton
          size="xs"
          variant="ghost"
          color="neutral"
          icon="i-lucide-refresh-cw"
          aria-label="Refresh followed players"
          :disabled="!online"
          @click="friends.refreshStatus()"
        />
      </template>
      <p v-if="!playingFriends.length" class="muted text-sm">
        No followed player is playing right now.
      </p>
      <div class="list-rows">
        <div v-for="entry in playingFriends" :key="entry.friend.username" class="list-row">
          <UIcon name="i-lucide-swords" />
          <div class="row-main">
            <div class="row-title"><PlayerLink :username="entry.friend.username" /></div>
            <div class="row-sub">Playing now</div>
          </div>
          <UButton
            size="xs"
            :disabled="!online || !entry.status?.playingId || playingOwnGame"
            @click="entry.status?.playingId && watcher.watch({ gameId: entry.status.playingId })"
            >Watch</UButton
          >
        </div>
      </div>
    </TvViewer>
  </div>
</template>

<style scoped>
.list-rows {
  min-height: 0;
  overflow-y: auto;
}
</style>
