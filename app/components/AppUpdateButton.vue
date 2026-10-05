<script setup lang="ts">
import { computed, ref } from 'vue'
import { useAppUpdatesStore } from '../stores/appUpdates'

const store = useKChessStore()
const updates = useAppUpdatesStore()
const { status } = storeToRefs(updates)
const confirmRestart = ref(false)

const percent = computed(() =>
  Math.round(Math.max(0, Math.min(100, status.value?.progress?.percent ?? 0))),
)
const label = computed(() => {
  switch (status.value?.phase) {
    case 'downloading':
      return status.value.progress ? `Updating ${percent.value}%` : 'Updating…'
    case 'downloaded':
      return 'Restart to update'
    case 'error':
      return 'Retry update'
    default:
      return 'Update available'
  }
})
const tooltip = computed(() => {
  const version = status.value?.version
  switch (status.value?.phase) {
    case 'downloading':
      return `Downloading KChess ${version}`
    case 'downloaded':
      return `KChess ${version} is ready. Restart now, or it installs when you quit.`
    case 'error':
      return status.value.error ?? 'The update could not finish.'
    default:
      return status.value?.canInstall
        ? `Download KChess ${version}`
        : `KChess ${version} is available. Download it from the release page.`
  }
})

function click(): void {
  const current = status.value
  if (!current) return
  if (current.phase === 'downloaded') confirmRestart.value = true
  else if (!current.canInstall) void updates.action('releases')
  else void updates.action('download')
}
/** Saves settings before the confirmed restart closes this renderer. */
async function restart(): Promise<void> {
  await store.save()
  await updates.action('install')
}
</script>

<template>
  <template v-if="updates.attention">
    <UTooltip :text="tooltip">
      <UButton
        size="sm"
        :variant="status?.phase === 'downloaded' ? 'solid' : 'soft'"
        :color="status?.phase === 'error' ? 'error' : 'primary'"
        :icon="status?.phase === 'downloaded' ? 'i-lucide-rotate-cw' : 'i-lucide-download'"
        :loading="status?.phase === 'downloading'"
        :disabled="status?.phase === 'downloading'"
        :label="label"
        :aria-label="tooltip"
        @click="click"
        @dblclick.stop
      />
    </UTooltip>
    <ConfirmDialog
      v-model:open="confirmRestart"
      title="Restart and install the update?"
      description="KChess will close and reopen. Finish any game or timed puzzle run first; an unfinished local game or run will be lost, and an online game's clock will keep running."
      confirm-label="Restart and install"
      @confirm="restart"
    />
  </template>
</template>
