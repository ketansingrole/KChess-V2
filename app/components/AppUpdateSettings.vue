<script setup lang="ts">
import { computed, ref } from 'vue'
import { useAppUpdatesStore } from '../stores/appUpdates'
import { formatBytes } from '../utils/format'

const store = useKChessStore()
const { settings } = storeToRefs(store)
const updates = useAppUpdatesStore()
const { status, title, busy, error } = storeToRefs(updates)
const confirmRestart = ref(false)
const checkedAt = computed(() =>
  status.value?.checkedAt ? new Date(status.value.checkedAt).toLocaleString() : 'Not checked yet',
)
const percent = computed(() =>
  Math.round(Math.max(0, Math.min(100, status.value?.progress?.percent ?? 0))),
)

/** Flushes preference edits before a manual check can trigger a download. */
async function check(): Promise<void> {
  await store.save()
  await updates.action('check')
}
/** Saves settings before the confirmed restart closes this renderer. */
async function restart(): Promise<void> {
  await store.save()
  await updates.action('install')
}
</script>

<template>
  <section class="card settings-list" aria-labelledby="update-status-title">
    <div class="setting-row">
      <div class="setting-info">
        <h2 id="update-status-title" class="setting-title">{{ title }}</h2>
        <p class="setting-hint">Version {{ status?.currentVersion ?? '…' }}</p>
        <p class="setting-hint">Last checked: {{ checkedAt }}</p>
      </div>
      <UButton
        variant="outline"
        color="neutral"
        icon="i-lucide-refresh-cw"
        :loading="status?.phase === 'checking'"
        :disabled="!status?.canCheck || busy || status.phase === 'downloaded'"
        @click="check"
        >Check for updates</UButton
      >
    </div>
    <div v-if="status?.phase === 'downloading'" class="px-4 pb-4" role="status" aria-live="polite">
      <UProgress
        :model-value="status.progress ? percent : null"
        aria-label="Update download progress"
      />
      <p class="setting-hint mt-2">
        <template v-if="status.progress">
          {{ percent }}% · {{ formatBytes(status.progress.transferred) }} of
          {{ formatBytes(status.progress.total) }} ·
          {{ formatBytes(status.progress.bytesPerSecond) }}/s
        </template>
        <template v-else>Preparing download…</template>
      </p>
    </div>
    <div v-if="status?.phase === 'available' || status?.phase === 'downloaded'" class="setting-row">
      <div class="setting-info">
        <span class="setting-title">{{
          status.phase === 'downloaded' ? 'Ready when you are' : 'New release'
        }}</span>
        <p class="setting-hint">
          {{
            status.phase === 'downloaded'
              ? settings.updateInstallOnQuit
                ? 'Installs when you quit KChess, or you can restart now.'
                : 'Restart KChess with the button below to install.'
              : status.canInstall
                ? 'Download now or let background downloads handle it.'
                : 'Download the installer from the official release page.'
          }}
        </p>
      </div>
      <UButton
        v-if="status.phase === 'downloaded'"
        icon="i-lucide-rotate-cw"
        @click="confirmRestart = true"
      >
        Restart and install
      </UButton>
      <UButton
        v-else-if="status.canInstall"
        icon="i-lucide-download"
        @click="updates.action('download')"
      >
        Download update
      </UButton>
      <UButton v-else icon="i-lucide-external-link" @click="updates.action('releases')"
        >Download latest release</UButton
      >
    </div>
    <p v-if="status?.reason" class="setting-hint px-4 pb-4">{{ status.reason }}</p>
    <div v-if="status?.error || error" class="px-4 pb-4" role="alert">
      <p class="text-sm text-error">{{ error || status?.error }}</p>
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span class="setting-title">Release notes</span>
      </div>
      <UButton
        color="neutral"
        variant="ghost"
        icon="i-lucide-external-link"
        @click="updates.action('releases')"
        >View releases</UButton
      >
    </div>
  </section>
  <section class="card settings-list" aria-labelledby="update-preferences-title">
    <h2 id="update-preferences-title" class="sr-only">Update preferences</h2>
    <div class="setting-row">
      <div class="setting-info">
        <span id="update-check-label" class="setting-title">Automatically check for updates</span>
        <span class="setting-hint">On startup and every six hours.</span>
      </div>
      <USwitch
        v-model="settings.updateAutoCheck"
        class="setting-switch"
        aria-labelledby="update-check-label"
        :disabled="!status?.canCheck"
        @update:model-value="store.save()"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="update-download-label" class="setting-title"
          >Download updates in the background</span
        >
        <span class="setting-hint">Turning this off does not cancel an ongoing download.</span>
      </div>
      <USwitch
        v-model="settings.updateAutoDownload"
        class="setting-switch"
        aria-labelledby="update-download-label"
        :disabled="!status?.canInstall"
        @update:model-value="store.save()"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="update-install-label" class="setting-title">Install updates when I quit</span>
        <span class="setting-hint">KChess will not quit or restart automatically.</span>
      </div>
      <USwitch
        v-model="settings.updateInstallOnQuit"
        class="setting-switch"
        aria-labelledby="update-install-label"
        :disabled="!status?.canInstall"
        @update:model-value="store.save()"
      />
    </div>
  </section>
  <ConfirmDialog
    v-model:open="confirmRestart"
    title="Restart and install the update?"
    description="KChess will close and reopen. Finish any game or timed puzzle run first; an unfinished local game or run will be lost, and an online game's clock will keep running."
    confirm-label="Restart and install"
    @confirm="restart"
  />
</template>
