<script setup lang="ts">
import { computed, ref } from 'vue'
import { useIntervalFn, useNow } from '@vueuse/core'
import { useAppUpdatesStore } from '../stores/appUpdates'
import { formatBytes, timeAgo } from '../utils/format'

const store = useKChessStore()
const { settings } = storeToRefs(store)
const updates = useAppUpdatesStore()
const { status, busy, error } = storeToRefs(updates)
const confirmRestart = ref(false)
const now = useNow({ scheduler: (cb) => useIntervalFn(cb, 30_000) })

const phase = computed(() => status.value?.phase ?? 'idle')
const percent = computed(() =>
  Math.round(Math.max(0, Math.min(100, status.value?.progress?.percent ?? 0))),
)
/** A newer release is known, so show where the update goes from and to. */
const target = computed(() => (updates.attention ? status.value?.version : undefined))
const releasedAt = computed(() =>
  status.value?.releaseDate ? new Date(status.value.releaseDate) : undefined,
)
/** "Released 2 days ago" reads faster than a date; the exact date stays in the tooltip. */
const released = computed(() => {
  const date = releasedAt.value
  if (!date || Number.isNaN(date.getTime())) return ''
  const days = (now.value.getTime() - date.getTime()) / 86_400_000
  return days >= 0 && days < 31
    ? `Released ${timeAgo(date.getTime(), now.value.getTime())}`
    : `Released ${date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}`
})
const checked = computed(() =>
  status.value?.checkedAt ? `Checked ${timeAgo(status.value.checkedAt, now.value.getTime())}` : '',
)
const badge = computed(() => {
  switch (phase.value) {
    case 'up-to-date':
      return { label: 'Up to date', color: 'success', icon: 'i-lucide-circle-check' } as const
    case 'available':
      return { label: 'Update available', color: 'primary', icon: 'i-lucide-sparkles' } as const
    case 'downloading':
      return { label: 'Downloading', color: 'primary', icon: 'i-lucide-download' } as const
    case 'downloaded':
      return { label: 'Ready to install', color: 'primary', icon: 'i-lucide-rotate-cw' } as const
    case 'error':
      return { label: 'Update failed', color: 'error', icon: 'i-lucide-circle-alert' } as const
    case 'disabled':
      return { label: 'Manual updates', color: 'neutral', icon: 'i-lucide-hand' } as const
    default:
      return undefined
  }
})
/** One line under the version: what is happening, or what the user can do next. */
const summary = computed(() => {
  const current = status.value
  switch (phase.value) {
    case 'checking':
      return 'Checking for updates…'
    case 'available':
      return current?.canInstall ? '' : (current?.reason ?? 'Download the installer to update.')
    case 'downloading':
      return current?.progress
        ? `${percent.value}% · ${formatBytes(current.progress.transferred)} of ${formatBytes(current.progress.total)} · ${formatBytes(current.progress.bytesPerSecond)}/s`
        : 'Preparing download…'
    case 'downloaded':
      return settings.value.updateInstallOnQuit
        ? 'Installs when you quit KChess, or restart now.'
        : 'Restart KChess to install.'
    case 'disabled':
      return current?.reason ?? ''
    case 'idle':
      return 'Not checked yet'
    default:
      return ''
  }
})
const primary = computed(() => {
  const current = status.value
  switch (phase.value) {
    case 'available':
      return current?.canInstall
        ? { label: 'Download update', icon: 'i-lucide-download', run: download }
        : { label: 'Get installer', icon: 'i-lucide-external-link', run: releases }
    case 'downloaded':
      return { label: 'Restart and install', icon: 'i-lucide-rotate-cw', run: askRestart }
    case 'error':
      return current?.version && current.canInstall
        ? { label: 'Retry download', icon: 'i-lucide-rotate-ccw', run: download }
        : undefined
    case 'disabled':
      return { label: 'Get latest release', icon: 'i-lucide-external-link', run: releases }
    default:
      return undefined
  }
})

/** Flushes preference edits before a manual check can trigger a download. */
async function check(): Promise<void> {
  await store.save()
  await updates.action('check')
}
function download(): void {
  void updates.action('download')
}
function releases(): void {
  void updates.action('releases')
}
function askRestart(): void {
  confirmRestart.value = true
}
/** Saves settings before the confirmed restart closes this renderer. */
async function restart(): Promise<void> {
  await store.save()
  await updates.action('install')
}
</script>

<template>
  <section class="card update-card" aria-labelledby="update-status-title">
    <div class="update-head">
      <img src="../assets/icon.png" alt="" width="52" height="52" class="update-icon" />
      <div class="update-info">
        <div class="update-title-row">
          <h2 id="update-status-title" class="update-title">
            KChess {{ status?.currentVersion ?? '…' }}
          </h2>
          <UBadge
            v-if="badge"
            :label="badge.label"
            :color="badge.color"
            :icon="badge.icon"
            variant="soft"
            size="sm"
          />
        </div>
        <div class="update-meta">
          <p class="update-summary" role="status" aria-live="polite">
            <span v-if="summary">{{ summary }}</span>
            <span v-if="summary && checked && !target" aria-hidden="true"> · </span>
            <span v-if="checked && !target">{{ checked }}</span>
          </p>
          <template v-if="!target && phase !== 'disabled'">
            <span v-if="summary || checked" aria-hidden="true">·</span>
            <UButton
              class="update-notes"
              color="neutral"
              variant="link"
              size="xs"
              trailing-icon="i-lucide-arrow-up-right"
              label="Release notes"
              @click="releases"
            />
          </template>
        </div>
      </div>
      <div class="update-actions">
        <UButton v-if="primary" :icon="primary.icon" :label="primary.label" @click="primary.run" />
        <UButton
          v-if="status?.canCheck && phase !== 'downloaded' && phase !== 'downloading'"
          :variant="primary ? 'ghost' : 'outline'"
          color="neutral"
          icon="i-lucide-refresh-cw"
          :loading="phase === 'checking'"
          :disabled="busy"
          label="Check for updates"
          @click="check"
        />
      </div>
    </div>

    <div v-if="target" class="update-target">
      <p class="update-versions">
        <span class="sr-only">Update from</span>
        <span class="update-chip">{{ status?.currentVersion }}</span>
        <UIcon name="i-lucide-arrow-right" class="update-arrow" aria-hidden="true" />
        <span class="sr-only">to</span>
        <span class="update-chip is-new">{{ target }}</span>
      </p>
      <p class="update-meta">
        <span v-if="released" :title="releasedAt?.toLocaleString()">{{ released }}</span>
        <span v-if="released" aria-hidden="true">·</span>
        <UButton
          class="update-notes"
          color="neutral"
          variant="link"
          size="xs"
          trailing-icon="i-lucide-arrow-up-right"
          label="What's new"
          @click="releases"
        />
      </p>
      <UProgress
        v-if="phase === 'downloading'"
        :model-value="status?.progress ? percent : null"
        aria-label="Update download progress"
        size="sm"
      />
    </div>

    <p v-if="status?.error || error" class="update-error" role="alert">
      <UIcon name="i-lucide-circle-alert" aria-hidden="true" />
      {{ error || status?.error }}
    </p>
  </section>

  <section
    v-if="status?.canCheck"
    class="card settings-list"
    aria-labelledby="update-preferences-title"
  >
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
        @update:model-value="store.save()"
      />
    </div>
    <template v-if="status.canInstall">
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
          @update:model-value="store.save()"
        />
      </div>
    </template>
  </section>
  <ConfirmDialog
    v-model:open="confirmRestart"
    :title="`Restart and install KChess ${status?.version ?? ''}?`"
    description="KChess will close and reopen. Finish any game or timed puzzle run first; an unfinished local game or run will be lost, and an online game's clock will keep running."
    confirm-label="Restart and install"
    @confirm="restart"
  />
</template>

<style scoped>
.update-card {
  display: grid;
  gap: 16px;
}
.update-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px;
}
.update-icon {
  flex: none;
  border-radius: 12px;
}
.update-info {
  display: grid;
  flex: 1 1 220px;
  gap: 4px;
  min-width: 0;
}
.update-title-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 10px;
}
.update-title {
  font-size: 1.125rem;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.update-summary {
  font-variant-numeric: tabular-nums;
}
.update-summary:empty {
  display: none;
}
.update-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.update-target {
  display: grid;
  gap: 8px;
  padding: 14px 16px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg);
}
.update-versions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
}
.update-chip {
  padding: 3px 10px;
  border: 1px solid var(--ui-border);
  border-radius: 999px;
  color: var(--ui-text-muted);
  font-size: 0.9375rem;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}
.update-chip.is-new {
  border-color: color-mix(in oklab, var(--ui-primary) 45%, transparent);
  background: color-mix(in oklab, var(--ui-primary) 14%, transparent);
  color: var(--ui-primary);
  font-weight: 600;
}
.update-arrow {
  color: var(--ui-text-dimmed);
}
.update-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  color: var(--ui-text-muted);
  font-size: 0.8125rem;
}
.update-info .update-meta {
  font-size: 0.875rem;
}
.update-notes {
  padding-inline: 0;
  color: var(--ui-text-muted);
  font-size: inherit;
}
.update-error {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--ui-error);
  font-size: 0.875rem;
}
</style>
