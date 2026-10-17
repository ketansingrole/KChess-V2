<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useKChessStore } from '../../stores/kchess'
import { useUsageStore } from '../../stores/usage'
import { formatBytes, formatCount } from '../../utils/format'
import ConfirmDialog from '../ConfirmDialog.vue'

const store = useKChessStore()
const usage = useUsageStore()
const { data } = storeToRefs(store)

const exportingDiagnostics = ref(false)
const diagnosticsNote = ref('')
async function exportDiagnostics(): Promise<void> {
  exportingDiagnostics.value = true
  diagnosticsNote.value = ''
  try {
    if (await window.kchess.exportDiagnostics()) diagnosticsNote.value = 'Diagnostics exported.'
  } catch (cause) {
    console.warn('[settings] Diagnostics export failed:', cause)
    diagnosticsNote.value = cause instanceof Error ? cause.message : 'Could not export diagnostics.'
  } finally {
    exportingDiagnostics.value = false
  }
}

const confirmClearRuns = ref(false)
const clearRunsNote = ref('')
async function clearLocalRuns(): Promise<void> {
  clearRunsNote.value = ''
  try {
    await window.kchess.clearRuns()
    clearRunsNote.value = 'Local scores and progress cleared.'
  } catch (cause) {
    console.warn('[settings] Clearing local scores failed:', cause)
    clearRunsNote.value = cause instanceof Error ? cause.message : 'Could not clear local scores.'
  }
}
const confirmResetUsage = ref(false)

/** One row per account with anything to report; removed accounts stay listed so past downloads are not hidden. */
const dataRows = computed(() => {
  const known = new Map(
    data.value.accounts.map((account) => [account.username.toLowerCase(), account]),
  )
  const names = new Set([
    ...known.keys(),
    ...Object.keys(usage.report?.accounts ?? {}),
    ...Object.keys(usage.report?.storage ?? {}),
  ])
  return [...names]
    .map((name) => {
      const account = known.get(name)
      const used = usage.usageOf(name)
      const kept = usage.storageOf(name)
      return {
        name,
        label: name ? `@${account?.username ?? name}` : 'App (no account)',
        badge: account
          ? account.connected
            ? 'Connected'
            : 'Friend'
          : name
            ? 'Removed'
            : undefined,
        detail: name ? undefined : 'Sign-in and other requests not tied to one account',
        stored: kept.bytes + kept.cacheBytes,
        games: kept.games,
        downloaded: used.total.bytesIn,
        requests: used.total.requests,
      }
    })
    .filter((row) => known.has(row.name) || row.stored || row.downloaded || row.requests)
    .sort((a, b) => b.downloaded + b.stored - (a.downloaded + a.stored))
})

onMounted(() => void usage.refresh())
</script>

<template>
  <div class="contents">
    <section id="settings-data" class="card" aria-labelledby="data-title">
      <div class="card-header">
        <div>
          <h2 id="data-title" class="sr-only">Data &amp; storage</h2>
        </div>
        <UButton
          variant="outline"
          color="neutral"
          icon="i-lucide-rotate-ccw"
          :disabled="!usage.totalRequests"
          @click="confirmResetUsage = true"
          >Reset counters</UButton
        >
      </div>
      <div class="data-totals">
        <div>
          <span class="stat-label">Downloaded</span>
          <strong class="tabular">{{ formatBytes(usage.totalDownloaded) }}</strong>
          <span class="muted text-xs"
            >{{ formatCount(usage.totalRequests) }} requests{{
              usage.report?.since
                ? ` since ${new Date(usage.report.since).toLocaleDateString()}`
                : ''
            }}</span
          >
        </div>
        <div>
          <span class="stat-label">Stored games &amp; profiles</span>
          <strong class="tabular">{{ formatBytes(usage.totalStored) }}</strong>
        </div>
        <div>
          <span class="stat-label">Database file</span>
          <strong class="tabular">{{ formatBytes(usage.report?.dbBytes ?? 0) }}</strong>
        </div>
      </div>
      <div v-if="dataRows.length" class="table-scroll">
        <table class="data-table">
          <thead>
            <tr>
              <th scope="col">Account</th>
              <th scope="col" class="num">Stored here</th>
              <th scope="col" class="num">Downloaded</th>
              <th scope="col" class="num">Requests</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in dataRows" :key="row.name">
              <td>
                <strong>{{ row.label }}</strong>
                <UBadge v-if="row.badge" variant="soft" size="sm" color="neutral" class="ml-2">{{
                  row.badge
                }}</UBadge>
                <span v-if="row.detail" class="sub">{{ row.detail }}</span>
              </td>
              <td class="num tabular">
                {{ formatBytes(row.stored) }}
                <span class="sub">{{ formatCount(row.games) }} games</span>
              </td>
              <td class="num tabular">{{ formatBytes(row.downloaded) }}</td>
              <td class="num tabular">{{ formatCount(row.requests) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <UEmpty
        v-else
        variant="naked"
        size="sm"
        icon="i-lucide-database"
        title="Nothing counted yet"
      />
    </section>
    <section id="settings-puzzles" class="card" aria-labelledby="local-puzzles-title">
      <div class="card-header">
        <div>
          <h2 id="local-puzzles-title" class="section-title">Puzzles &amp; practice</h2>
        </div>
        <UButton
          variant="outline"
          color="neutral"
          icon="i-lucide-eraser"
          @click="confirmClearRuns = true"
          >Clear local scores</UButton
        >
      </div>
      <UButton
        class="mt-3"
        variant="outline"
        color="neutral"
        icon="i-lucide-download"
        @click="store.jumpToSection('offline')"
        >Offline downloads</UButton
      >
      <p v-if="clearRunsNote" role="status" class="setting-hint mt-2">{{ clearRunsNote }}</p>
    </section>
    <section class="card settings-list" aria-labelledby="diagnostics-title">
      <div class="setting-row">
        <div class="setting-info">
          <h2 id="diagnostics-title" class="setting-title">Diagnostics</h2>
          <p class="setting-hint">Exports logs without tokens, games or account databases.</p>
        </div>
        <UButton
          variant="outline"
          color="neutral"
          :loading="exportingDiagnostics"
          @click="exportDiagnostics"
          >Export diagnostics</UButton
        >
      </div>
      <p v-if="diagnosticsNote" role="status" class="setting-hint">{{ diagnosticsNote }}</p>
    </section>
    <ConfirmDialog
      v-model:open="confirmClearRuns"
      title="Clear your local scores?"
      description="This deletes the Storm, Streak, Rush and board-training scores and the endgame and lesson progress stored on this computer. Nothing on Lichess is affected."
      confirm-label="Clear"
      color="error"
      @confirm="clearLocalRuns"
    />
    <ConfirmDialog
      v-model:open="confirmResetUsage"
      title="Reset the data counters?"
      description="This only clears the download counts shown here. Your stored games and accounts are not touched."
      confirm-label="Reset"
      @confirm="usage.reset()"
    />
  </div>
</template>
