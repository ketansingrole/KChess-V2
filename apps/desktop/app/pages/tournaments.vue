<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useOnline } from '@vueuse/core'
import type { TournamentSummary } from '../../../../core/src/contracts/types'
import { useTournamentStore } from '../stores/tournaments'

const online = useOnline()
const connectOpen = ref(false)
const store = useKChessStore()
const tournaments = useTournamentStore()
const { activeOnlineAccount, busy } = storeToRefs(store)
// The schedule is for browsing: show everything, marking what KChess can only watch.
const filter = ref<'playable' | 'all'>('all')
const createOpen = ref(false)
function startCreate(): void {
  if (!activeOnlineAccount.value) connectOpen.value = true
  else createOpen.value = true
}
const route = useRoute()
const router = useRouter()
/** The event being viewed (`?system=arena&id=…`); without one the page shows the schedule. */
const event = computed(() => {
  const system = route.query.system
  const id = route.query.id
  return (system === 'arena' || system === 'swiss') &&
    typeof id === 'string' &&
    /^[a-zA-Z0-9]{8}$/.test(id)
    ? { system: system as TournamentSummary['system'], id }
    : null
})
function openEvent(system: TournamentSummary['system'], id: string): void {
  void router.push({ path: '/tournaments', query: { system, id } })
}
watch(event, (now) => {
  if (!now) tournaments.close()
})

onMounted(() => {
  if (online.value) void tournaments.refresh()
})
watch(online, (connected) => {
  if (connected) void tournaments.refresh()
})

const arenas = computed(() =>
  (tournaments.list?.arenas ?? []).filter((t) => filter.value === 'all' || t.playable),
)
</script>

<template>
  <div>
    <PageHeader title="Tournaments">
      <UTabs
        v-if="!event"
        v-model="filter"
        :items="[
          { label: 'Playable here', value: 'playable' },
          { label: 'All', value: 'all' },
        ]"
        :content="false"
        variant="pill"
      />
      <UButton
        v-if="!event"
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-refresh-cw"
        :loading="tournaments.loading"
        :disabled="!online"
        @click="tournaments.refresh()"
        >Refresh</UButton
      >
      <UButton size="sm" icon="i-lucide-plus" :disabled="!online" @click="startCreate"
        >Create arena</UButton
      >
    </PageHeader>

    <PublicOnlineNotice offline-message="Offline · standings may be out of date." />
    <UAlert
      v-if="tournaments.needsReconnect"
      class="mb-4"
      color="warning"
      variant="subtle"
      icon="i-lucide-key-round"
      title="Lichess needs a new permission"
      :description="`Connect @${activeOnlineAccount} again to allow joining and creating tournaments.`"
      :actions="[{ label: 'Reconnect', icon: 'i-lucide-link', onClick: () => store.connect() }]"
    />

    <TournamentEvent
      v-if="event"
      :id="event.id"
      :system="event.system"
      @connect="connectOpen = true"
    />

    <template v-else>
      <p v-if="tournaments.error" class="text-error text-sm mb-4" role="alert">
        {{ tournaments.error }}
      </p>
      <section class="card mb-5" aria-labelledby="arena-schedule-title">
        <div class="card-header">
          <h2 id="arena-schedule-title" class="section-title">Arenas</h2>
        </div>
        <p v-if="!tournaments.list" class="muted text-sm">Loading…</p>
        <p v-else-if="!arenas.length" class="muted text-sm">No arenas to show right now.</p>
        <TournamentTimeline
          v-else
          :arenas="arenas"
          :is-joined="(id: string) => tournaments.isJoined('arena', id)"
          @select="(id: string) => openEvent('arena', id)"
        />
      </section>
    </template>

    <CreateArenaDialog v-model:open="createOpen" />
    <UModal
      v-model:open="connectOpen"
      title="Connect Lichess to join"
      description="Connect your Lichess account to join or create tournaments."
    >
      <template #body>
        <div class="flex flex-wrap gap-2">
          <UButton
            icon="i-lucide-log-in"
            :disabled="!online"
            :loading="busy"
            @click="
              store.connect().then(() => {
                if (store.activeOnlineAccount) connectOpen = false
              })
            "
            >Connect Lichess</UButton
          >
          <UButton variant="outline" color="neutral" @click="connectOpen = false"
            >Keep browsing</UButton
          >
        </div>
      </template>
    </UModal>
  </div>
</template>
