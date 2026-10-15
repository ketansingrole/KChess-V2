<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type { LichessUser } from '@kchess/contracts/types'
import { PERF_LABELS, playedPerfs } from '../utils/perfs'

/**
 * A player's profile at a glance, shown in place beside the board. `full` adds a way to the
 * player's full profile on the Players page.
 */
const props = defineProps<{ username: string; full?: boolean }>()
const store = useKChessStore()

const profile = ref<LichessUser | null>(null)
const score = ref<{ mine: number; theirs: number; games: number } | null>(null)
const failed = ref(false)
const ratings = computed(() => playedPerfs(profile.value).slice(0, 4))

onMounted(async () => {
  const me = store.onlineAccount || store.activeOnlineAccount
  if (me && me.toLowerCase() !== props.username.toLowerCase())
    void window.kchess
      .crosstable(me, props.username)
      .then((table) => {
        if (table.nbGames)
          score.value = {
            mine: table.users[me.toLowerCase()] ?? 0,
            theirs: table.users[props.username.toLowerCase()] ?? 0,
            games: table.nbGames,
          }
      })
      .catch((error: unknown) => {
        console.warn('[player-card] Crosstable unavailable:', error)
        return undefined
      })
  try {
    profile.value = await window.kchess.profile(props.username)
  } catch (error) {
    console.warn('[player-card] Profile unavailable:', error)
    failed.value = true
  }
})
</script>

<template>
  <section class="player-card" :aria-label="`${username}'s profile`">
    <h3 class="player-card-name">
      <span v-if="profile?.title" class="player-card-title">{{ profile.title }}</span>
      {{ profile?.username ?? username }}
    </h3>
    <p v-if="failed" class="muted text-xs" role="status">Profile unavailable.</p>
    <p v-else-if="!profile" class="muted text-xs" role="status">Loading…</p>
    <template v-else>
      <p class="muted text-xs">
        {{ profile.count?.all ?? 0 }} games
        <template v-if="profile.createdAt">
          · joined {{ new Date(profile.createdAt).getFullYear() }}</template
        >
      </p>
      <dl v-if="ratings.length" class="player-card-ratings tabular">
        <div v-for="perf in ratings" :key="perf.key">
          <dt>{{ PERF_LABELS[perf.key] }}</dt>
          <dd>{{ perf.rating ?? '–' }}{{ perf.prov ? '?' : '' }}</dd>
        </div>
      </dl>
    </template>
    <p v-if="score" class="text-xs tabular">
      Your score <strong>{{ score.mine }} – {{ score.theirs }}</strong> in {{ score.games }} games
    </p>
    <UButton
      v-if="full"
      size="xs"
      variant="outline"
      color="neutral"
      icon="i-lucide-user"
      block
      :to="{ path: '/players', query: { name: username } }"
      >Full profile</UButton
    >
  </section>
</template>

<style scoped>
.player-card {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  width: 16rem;
  padding: 0.75rem;
}
.player-card-name {
  margin: 0;
  font-weight: 600;
}
.player-card-title {
  color: var(--ui-warning);
  margin-right: 0.25rem;
}
.player-card-ratings {
  display: grid;
  gap: 0.25rem;
  margin: 0;
  font-size: 0.75rem;
}
.player-card-ratings div {
  display: flex;
  justify-content: space-between;
  gap: 0.5rem;
}
.player-card-ratings dt {
  color: var(--ui-text-muted);
}
.player-card-ratings dd {
  margin: 0;
  font-weight: 600;
}
</style>
