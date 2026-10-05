<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useOnline } from '@vueuse/core'
import { formatBytes, formatCount } from '../utils/format'

/** Download, see and delete the local puzzle database that Storm, Streak, Rush and offline puzzles play from. */
defineProps<{ compact?: boolean }>()

const online = useOnline()
const puzzles = usePuzzleStore()
const { db, dbProgress, dbError, dbBusy } = storeToRefs(puzzles)
onMounted(() => void puzzles.refreshDb())

const confirmDelete = ref(false)
const percent = computed(() => {
  const progress = dbProgress.value
  if (!progress?.total) return undefined
  return Math.min(100, Math.round((progress.received / progress.total) * 100))
})
const phaseText = computed(() => {
  const progress = dbProgress.value
  if (!progress) return ''
  if (progress.phase === 'importing') return 'Saving puzzles…'
  if (progress.phase === 'failed') return progress.message ?? 'The download failed.'
  return `Downloading… ${formatBytes(progress.received)}${progress.total ? ` of ${formatBytes(progress.total)}` : ''}`
})
</script>

<template>
  <div class="db-card" :class="{ compact }">
    <div v-if="!compact" class="db-head">
      <h3 class="section-title">Puzzle database</h3>
      <p class="section-hint">
        Download once to play puzzles, Storm, Streak and Rush offline. No account needed.
      </p>
    </div>

    <template v-if="dbBusy">
      <UProgress :model-value="percent ?? null" size="md" class="mt-3" />
      <p class="section-hint tabular" role="status">
        {{ phaseText }}
        <template v-if="dbProgress && dbProgress.kept">
          · {{ formatCount(dbProgress.kept) }} puzzles picked so far</template
        >
      </p>
      <UButton
        class="mt-2"
        size="sm"
        color="neutral"
        variant="outline"
        icon="i-lucide-x"
        @click="puzzles.cancelDb()"
        >Cancel</UButton
      >
    </template>
    <template v-else-if="db?.installed">
      <p class="db-line tabular">
        <UIcon name="i-lucide-circle-check" class="text-success" />
        {{ formatCount(db.count) }} puzzles · {{ formatBytes(db.bytes) }} on disk<template
          v-if="db.importedAt"
        >
          · downloaded {{ new Date(db.importedAt).toLocaleDateString() }}</template
        >
      </p>
      <div class="toolbar-row mt-2">
        <UButton
          size="sm"
          color="neutral"
          variant="outline"
          icon="i-lucide-refresh-cw"
          :disabled="!online"
          @click="puzzles.installDb()"
          >Download a fresh sample</UButton
        >
        <UButton
          size="sm"
          color="error"
          variant="soft"
          icon="i-lucide-trash-2"
          @click="confirmDelete = true"
          >Delete</UButton
        >
      </div>
    </template>
    <template v-else>
      <p class="section-hint">
        About 300 MB download; only around 100,000 puzzles (a few tens of MB) are kept.
      </p>
      <UButton
        class="mt-2"
        icon="i-lucide-download"
        :disabled="!online"
        @click="puzzles.installDb()"
        >Download puzzle database</UButton
      >
    </template>
    <p v-if="!online" class="section-hint" role="status">
      {{
        db?.installed
          ? 'Your downloaded puzzles are ready to play. Reconnect to refresh them.'
          : 'Connect to the internet once to download puzzles. You can play the computer and practice chess skills while offline.'
      }}
    </p>
    <p v-if="dbError" class="db-error" role="alert">{{ dbError }}</p>

    <ConfirmDialog
      v-model:open="confirmDelete"
      title="Delete the puzzle database?"
      description="This removes the puzzles stored on this computer. Storm, Streak, Rush and offline puzzles stop working until you download it again. Your scores are kept."
      confirm-label="Delete"
      color="error"
      @confirm="puzzles.deleteDb()"
    />
  </div>
</template>

<style scoped>
.db-card {
  padding: 16px;
  border: 1px solid var(--ui-border);
  border-radius: 12px;
  background: var(--ui-bg);
}
.db-line {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 10px 0 0;
  font-size: 13px;
}
.db-error {
  margin: 8px 0 0;
  color: var(--ui-error);
  font-size: 12px;
}
.compact {
  padding: 0;
  border: 0;
  background: transparent;
}
</style>
