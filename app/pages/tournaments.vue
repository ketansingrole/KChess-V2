<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useIntervalFn, useOnline } from '@vueuse/core'
import type { TournamentSummary } from '../../src/shared/types'
import { useTournamentStore } from '../stores/tournaments'

const online = useOnline()
const connectOpen = ref(false)
const store = useKChessStore()
const tournaments = useTournamentStore()
const { connectedAccounts, activeOnlineAccount, busy } = storeToRefs(store)
const filter = ref<'playable' | 'all'>(activeOnlineAccount.value ? 'playable' : 'all')
const password = ref('')

onMounted(() => {
  if (online.value) void tournaments.refresh()
  if (online.value && tournaments.selected)
    void tournaments.open(tournaments.selected.system, tournaments.selected.id)
})
watch(online, (connected) => {
  if (connected) void tournaments.refresh()
})
// Standings move quickly while a tournament runs.
const ticker = useIntervalFn(() => {
  const selected = tournaments.selected
  if (online.value && selected && tournaments.detail?.status !== 'finished')
    void tournaments.open(selected.system, selected.id)
}, 15_000)
onUnmounted(() => ticker.pause())

const arenas = computed(() =>
  (tournaments.list?.arenas ?? []).filter((t) => filter.value === 'all' || t.playable),
)
const swiss = computed(() =>
  (tournaments.list?.swiss ?? []).filter((t) => filter.value === 'all' || t.playable),
)
function control(t: Pick<TournamentSummary, 'clock'>): string {
  const minutes = t.clock.limit / 60
  return `${Number.isInteger(minutes) ? minutes : minutes.toFixed(1)}+${t.clock.increment}`
}
function when(t: TournamentSummary): string {
  if (t.status === 'started')
    return t.finishesAt
      ? `Running · ends ${new Date(t.finishesAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
      : `Round ${t.round ?? '?'} of ${t.nbRounds ?? '?'}`
  const date = new Date(t.startsAt)
  return `Starts ${date.toLocaleDateString()} ${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
}
const detail = computed(() => tournaments.detail)
const joined = computed(() =>
  detail.value
    ? detail.value.me
      ? !detail.value.me.withdraw
      : tournaments.isJoined(detail.value.system, detail.value.id)
    : false,
)
async function join(): Promise<void> {
  if (!detail.value || !online.value) return
  if (!activeOnlineAccount.value) {
    connectOpen.value = true
    return
  }
  if (await tournaments.join(detail.value, password.value)) password.value = ''
}
</script>

<template>
  <div>
    <PageHeader title="Tournaments">
      <UTabs
        v-model="filter"
        :items="[
          { label: 'Playable here', value: 'playable' },
          { label: 'All', value: 'all' },
        ]"
        :content="false"
        size="sm"
        variant="pill"
      />
      <UButton
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-refresh-cw"
        :loading="tournaments.loading"
        :disabled="!online"
        @click="tournaments.refresh()"
        >Refresh</UButton
      >
    </PageHeader>

    <PublicOnlineNotice offline-message="Offline · standings may be out of date." />
    <div class="online-columns">
      <div class="flex flex-col gap-5">
        <UAlert
          v-if="tournaments.needsReconnect"
          color="warning"
          variant="subtle"
          icon="i-lucide-key-round"
          title="Lichess needs a new permission"
          :description="`Connect @${activeOnlineAccount} again (Settings → My accounts) to allow joining tournaments.`"
        />
        <p v-if="tournaments.error" class="text-error text-sm" role="alert">
          {{ tournaments.error }}
        </p>
        <section class="card">
          <h2 class="section-title">Arenas</h2>
          <p v-if="tournaments.list && !arenas.length" class="muted text-sm mt-3">
            No arenas to show right now.
          </p>
          <div class="list-rows mt-3">
            <button
              v-for="t in arenas"
              :key="t.id"
              type="button"
              class="list-row text-left"
              :disabled="!online"
              :class="{ 'ring-1 ring-primary': tournaments.selected?.id === t.id }"
              @click="tournaments.open('arena', t.id)"
            >
              <UIcon name="i-lucide-trophy" class="shrink-0" />
              <div class="row-main">
                <div class="row-title">{{ t.name }}</div>
                <div class="row-sub">
                  {{ control(t) }} · {{ t.variantName }} · {{ t.rated ? 'Rated' : 'Casual' }} ·
                  {{ t.minutes }} min · {{ t.nbPlayers }} players
                </div>
                <div class="row-sub">{{ when(t) }}</div>
                <div v-if="t.problem" class="row-sub text-warning">{{ t.problem }}</div>
              </div>
              <UBadge v-if="tournaments.isJoined('arena', t.id)" size="sm" variant="soft"
                >Joined</UBadge
              >
            </button>
          </div>
        </section>
        <section v-if="connectedAccounts.length" class="card">
          <h2 class="section-title">Swiss events of your teams</h2>
          <p
            v-for="problem in tournaments.list?.problems ?? []"
            :key="problem"
            class="text-xs muted"
          >
            {{ problem }}
          </p>
          <p v-if="tournaments.list && !swiss.length" class="muted text-sm mt-3">
            No upcoming Swiss events.
          </p>
          <div class="list-rows mt-3">
            <button
              v-for="t in swiss"
              :key="t.id"
              type="button"
              class="list-row text-left"
              :disabled="!online"
              :class="{ 'ring-1 ring-primary': tournaments.selected?.id === t.id }"
              @click="tournaments.open('swiss', t.id)"
            >
              <UIcon name="i-lucide-network" class="shrink-0" />
              <div class="row-main">
                <div class="row-title">{{ t.name }}</div>
                <div class="row-sub">
                  {{ t.team?.name }} · {{ control(t) }} · {{ t.variantName }} ·
                  {{ t.nbRounds }} rounds · {{ t.nbPlayers }} players
                </div>
                <div class="row-sub">{{ when(t) }}</div>
                <div v-if="t.problem" class="row-sub text-warning">{{ t.problem }}</div>
              </div>
            </button>
          </div>
        </section>
      </div>

      <section class="card" aria-live="polite">
        <template v-if="detail">
          <div class="card-header">
            <div>
              <h2 class="section-title">{{ detail.name }}</h2>
              <p class="section-hint">
                {{ control(detail) }} · {{ detail.rated ? 'Rated' : 'Casual' }} ·
                {{ detail.nbPlayers }} players ·
                {{
                  detail.status === 'finished'
                    ? 'Finished'
                    : detail.secondsToStart
                      ? `Starts in ${Math.ceil(detail.secondsToStart / 60)} min`
                      : detail.secondsToFinish
                        ? `${Math.ceil(detail.secondsToFinish / 60)} min left`
                        : detail.nextRoundIn
                          ? `Next round in ${Math.ceil(detail.nextRoundIn / 60)} min`
                          : 'Under way'
                }}
              </p>
            </div>
            <UButton
              size="xs"
              variant="ghost"
              color="neutral"
              icon="i-lucide-x"
              aria-label="Close"
              @click="tournaments.close()"
            />
          </div>
          <p v-if="detail.description" class="text-sm whitespace-pre-line mb-3">
            {{ detail.description }}
          </p>
          <ul v-if="detail.verdicts?.list.length" class="text-xs mb-3 space-y-1">
            <li v-for="entry in detail.verdicts.list" :key="entry.condition">
              <UIcon
                :name="entry.verdict === 'ok' ? 'i-lucide-check' : 'i-lucide-x'"
                :class="entry.verdict === 'ok' ? 'text-success' : 'text-error'"
              />
              {{ entry.condition }}<span v-if="entry.verdict !== 'ok'"> — {{ entry.verdict }}</span>
            </li>
          </ul>
          <UAlert
            v-if="detail.problem"
            class="mb-3"
            color="warning"
            variant="subtle"
            icon="i-lucide-triangle-alert"
            :description="detail.problem"
          />
          <div v-if="detail.status !== 'finished'" class="flex flex-wrap items-center gap-2 mb-4">
            <template v-if="!joined">
              <UInput
                v-if="activeOnlineAccount"
                v-model="password"
                type="password"
                size="sm"
                placeholder="Entry code (if any)"
                aria-label="Tournament entry code"
                autocomplete="off"
              />
              <UButton
                icon="i-lucide-log-in"
                size="sm"
                :loading="busy"
                :disabled="!online || !detail.playable || detail.verdicts?.accepted === false"
                @click="join"
                >Join</UButton
              >
            </template>
            <template v-else>
              <UBadge color="success" variant="soft">You are in</UBadge>
              <span v-if="detail.me?.rank" class="text-sm">Rank {{ detail.me.rank }}</span>
              <UButton
                v-if="detail.me?.gameId"
                size="sm"
                icon="i-lucide-swords"
                @click="store.openOngoing(activeOnlineAccount, detail.me.gameId)"
                >Go to your game</UButton
              >
              <UButton
                size="sm"
                variant="outline"
                color="neutral"
                :disabled="!online || busy"
                @click="tournaments.leave(detail)"
                >{{ detail.system === 'arena' ? 'Pause / withdraw' : 'Withdraw' }}</UButton
              >
            </template>
          </div>
          <h3 class="section-title text-sm mb-2">Standings</h3>
          <p v-if="!detail.standing.length" class="muted text-sm">No players yet.</p>
          <ol class="text-sm space-y-1">
            <li v-for="row in detail.standing" :key="row.rank + row.name" class="flex gap-2">
              <span class="muted tabular w-8 text-right">{{ row.rank }}</span>
              <span class="flex-1 truncate"
                >{{ row.title ? `${row.title} ` : '' }}{{ row.name }}
                <span v-if="row.rating" class="muted tabular">{{ row.rating }}</span></span
              >
              <span v-if="row.sheet" class="muted tabular text-xs truncate max-w-32">{{
                row.sheet
              }}</span>
              <span class="tabular font-semibold" :class="{ 'text-warning': row.fire }">{{
                row.score ?? ''
              }}</span>
            </li>
          </ol>
        </template>
        <p v-else-if="tournaments.detailError" class="text-error text-sm" role="alert">
          {{ tournaments.detailError }}
        </p>
        <UEmpty
          v-else
          variant="naked"
          icon="i-lucide-mouse-pointer-click"
          title="Pick a tournament"
        />
      </section>
    </div>
    <UModal
      v-model:open="connectOpen"
      title="Connect Lichess to join"
      description="Connect your Lichess account to join this tournament."
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
