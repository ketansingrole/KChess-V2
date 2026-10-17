<script setup lang="ts">
import { computed, ref } from 'vue'
import { useAppUpdatesStore } from '../stores/appUpdates'
import { RESTART_UPDATE_DESCRIPTION } from '../utils/appUpdate'

const store = useKChessStore()
const updates = useAppUpdatesStore()
const { status, error } = storeToRefs(updates)
const confirmRestart = ref(false)
const toast = useToast()

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
      // Without self-update the click opens the release page, so the label says download.
      if (!status.value?.canInstall)
        return status.value?.version ? `Download ${status.value.version}` : 'Update available'
      return status.value?.version ? `Update to ${status.value.version}` : 'Update available'
  }
})
const tooltip = computed(() => {
  const version = status.value?.version
  const from = status.value?.currentVersion
  switch (status.value?.phase) {
    case 'downloading':
      return `Downloading KChess ${version} (you have ${from})`
    case 'downloaded':
      return `KChess ${version} is ready. Restart now, or it installs when you quit.`
    case 'error':
      return status.value.error ?? 'The update could not finish.'
    default:
      return status.value?.canInstall
        ? `Download KChess ${version} (you have ${from})`
        : `KChess ${version} is available (you have ${from}). Download it from the release page.`
  }
})

/** The store keeps a failed action's error for Settings; say it here too, where it was clicked. */
async function run(kind: 'download' | 'install' | 'releases'): Promise<void> {
  await updates.action(kind)
  if (error.value)
    toast.add({
      title: kind === 'releases' ? 'Could not open the release page' : 'Could not update KChess',
      description: error.value,
      icon: 'i-lucide-circle-alert',
      color: 'error',
    })
}
function click(): void {
  const current = status.value
  if (!current) return
  if (current.phase === 'downloaded') confirmRestart.value = true
  else if (!current.canInstall) void run('releases')
  else void run('download')
}
/** Saves settings before the confirmed restart closes this renderer. */
async function restart(): Promise<void> {
  await store.save()
  await run('install')
}
</script>

<template>
  <template v-if="updates.attention">
    <UTooltip :text="tooltip">
      <UButton
        size="sm"
        :variant="status?.phase === 'downloaded' ? 'solid' : 'soft'"
        :color="status?.phase === 'error' ? 'error' : 'primary'"
        :icon="
          status?.phase === 'downloaded'
            ? 'i-lucide-rotate-cw'
            : status?.canInstall === false && status.phase !== 'error'
              ? 'i-lucide-external-link'
              : 'i-lucide-download'
        "
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
      :title="`Restart and install KChess ${status?.version ?? ''}?`"
      :description="RESTART_UPDATE_DESCRIPTION"
      confirm-label="Restart and install"
      @confirm="restart"
    />
  </template>
</template>
