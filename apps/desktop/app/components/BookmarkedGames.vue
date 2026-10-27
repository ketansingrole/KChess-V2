<script setup lang="ts">
import { ref, watch } from 'vue'
import { useOnline } from '@vueuse/core'
import type { LichessGame } from '@kchess/contracts/types'
import { useOpenLichessGame } from '../composables/useOpenLichessGame'

/** The games the online account bookmarked on Lichess. */
const store = useKChessStore()
const { activeOnlineAccount } = storeToRefs(store)
const online = useOnline()
const openGame = useOpenLichessGame()

const games = ref<LichessGame[] | null>(null)
const error = ref('')
const needsReconnect = ref(false)
let request = 0
async function load(): Promise<void> {
  const account = activeOnlineAccount.value
  const current = ++request
  games.value = null
  error.value = ''
  needsReconnect.value = false
  if (!account || !online.value) return
  try {
    const answer = await window.kchess.bookmarkedGames(account)
    if (current !== request) return
    if (Array.isArray(answer)) games.value = answer
    else needsReconnect.value = true
  } catch (cause) {
    if (current !== request) return
    console.warn('[bookmarks] Loading bookmarks failed:', cause)
    error.value = cause instanceof Error ? cause.message : String(cause)
  }
}
watch([activeOnlineAccount, online], () => void load(), { immediate: true })
</script>

<template>
  <section class="card">
    <p v-if="!online" class="muted text-sm" role="status">Offline · reconnect to see bookmarks.</p>
    <UAlert
      v-else-if="needsReconnect"
      color="warning"
      variant="subtle"
      icon="i-lucide-key-round"
      :title="`Connect @${activeOnlineAccount} again`"
      :actions="[
        { label: 'Reconnect', icon: 'i-lucide-link', onClick: () => void store.connect() },
      ]"
    />
    <p v-else-if="error" class="text-error text-sm" role="alert">{{ error }}</p>
    <p v-else-if="!games" class="muted text-sm">Loading…</p>
    <UEmpty
      v-else-if="!games.length"
      variant="naked"
      icon="i-lucide-bookmark"
      :title="`No bookmarks on @${activeOnlineAccount}`"
    />
    <div v-else class="game-list no-accuracy">
      <div class="game-head" aria-hidden="true">
        <span>Result</span><span>Opponent</span><span>Mode</span><span>Rating</span
        ><span>Date</span>
      </div>
      <GameRow
        v-for="game in games"
        :key="game.id"
        :game="game"
        detailed
        hide-accuracy
        @select="openGame(game)"
      />
    </div>
  </section>
</template>
