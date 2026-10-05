<script setup lang="ts">
import { computed, ref } from 'vue'
import { useReviewStore } from '../stores/review'
const store = useKChessStore()
const { busy, settings, engineReady, engineInfo, engineName } = storeToRefs(store)
const { useBundledEngine, useDownloadedEngine, installEngine, deleteEngine, chooseEngine } = store
const reviewAutoItems = [
  { label: 'Off', value: 'off' },
  { label: 'Last 30 days', value: 'recent' },
  { label: 'All', value: 'all' },
]
const { status: reviewQueue } = storeToRefs(useReviewStore())
const reviewStatusText = computed(() => {
  const { current, waiting, paused } = reviewQueue.value
  if (current)
    return `Reviewing a game: ${current.done} of ${current.total} positions${
      waiting ? ` · ${waiting} more waiting` : ''
    }.`
  if (paused === 'battery') return 'Paused while on battery power.'
  if (paused === 'engine') return 'Paused while the engine is in use.'
  if (paused === 'online') return 'Paused during your online game.'
  if (paused === 'off' || settings.value?.reviewAuto === 'off')
    return 'Games are reviewed when you ask for it.'
  return waiting ? `${waiting} games waiting.` : 'All caught up.'
})

const confirmDeleteEngine = ref(false)
/** Which Stockfish is in use: the bundled one (no path), the one KChess downloaded, or a chosen file. */
const engineSource = computed<'bundled' | 'downloaded' | 'custom'>(() => {
  const path = settings.value?.enginePath
  if (!path) return 'bundled'
  return path === engineInfo.value?.managed.path ? 'downloaded' : 'custom'
})
const downloadedDescription = computed(() => {
  const managed = engineInfo.value?.managed
  if (managed?.installed) return managed.version ? `Version ${managed.version}` : 'Installed'
  return engineInfo.value?.canDownload === false
    ? 'No official download is available for this platform.'
    : 'Official native build'
})
const customDescription = computed(() =>
  engineSource.value === 'custom' ? settings.value!.enginePath : 'Stockfish executable',
)
</script>
<template>
  <div class="space-y-4">
    <section id="settings-engine" class="card settings-list" aria-labelledby="engine-title">
      <h2 id="engine-title" class="sr-only">Chess engine</h2>
      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-title">Status</span>
          <span class="setting-hint">{{
            engineReady && engineName ? engineName : 'Choose an available engine below.'
          }}</span>
        </div>
        <div>
          <UBadge
            :color="engineReady ? 'success' : 'warning'"
            variant="soft"
            :icon="engineReady ? 'i-lucide-circle-check' : 'i-lucide-triangle-alert'"
            >{{ engineReady ? 'Ready' : 'Not found' }}</UBadge
          >
        </div>
      </div>
      <div class="engine-options">
        <EngineOption
          title="Bundled Stockfish 19 lite"
          description="Included"
          :active="engineSource === 'bundled'"
        >
          <UButton
            v-if="engineSource !== 'bundled'"
            size="sm"
            variant="outline"
            color="neutral"
            :disabled="busy"
            @click="useBundledEngine"
            >Use</UButton
          >
        </EngineOption>
        <EngineOption
          title="Downloaded Stockfish"
          :description="downloadedDescription"
          :active="engineSource === 'downloaded'"
        >
          <UButton
            v-if="engineInfo?.managed.installed && engineSource !== 'downloaded'"
            size="sm"
            variant="outline"
            color="neutral"
            :disabled="busy"
            @click="useDownloadedEngine"
            >Use</UButton
          >
          <UButton
            v-if="engineInfo?.canDownload"
            size="sm"
            variant="outline"
            color="neutral"
            icon="i-lucide-download"
            :loading="busy"
            @click="installEngine"
            >{{ engineInfo.managed.installed ? 'Check for update' : 'Download' }}</UButton
          >
          <UButton
            v-if="engineInfo?.managed.installed"
            size="sm"
            variant="ghost"
            color="error"
            icon="i-lucide-trash-2"
            :disabled="busy"
            @click="confirmDeleteEngine = true"
            >Delete</UButton
          >
        </EngineOption>
        <EngineOption
          title="Your own executable"
          :description="customDescription"
          :active="engineSource === 'custom'"
        >
          <UButton
            size="sm"
            variant="outline"
            color="neutral"
            icon="i-lucide-folder-open"
            :disabled="busy"
            @click="chooseEngine"
            >Choose file…</UButton
          >
        </EngineOption>
      </div>
    </section>

    <section v-if="settings" id="settings-review" class="card" aria-labelledby="review-title">
      <div class="card-header">
        <div>
          <h2 id="review-title" class="section-title">Game review</h2>
          <p class="section-hint" aria-live="polite">{{ reviewStatusText }}</p>
        </div>
      </div>
      <div class="settings-list">
        <div class="setting-row">
          <div class="setting-info">
            <span id="review-auto-label" class="setting-title">Review my games automatically</span>
            <span class="setting-hint">Runs in the background after syncing.</span>
          </div>
          <UTabs
            v-model="settings.reviewAuto"
            :items="reviewAutoItems"
            aria-labelledby="review-auto-label"
            :content="false"
            variant="pill"
            class="setting-control"
            :ui="{ trigger: 'grow' }"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="review-battery-label" class="setting-title">Also on battery power</span>
            <span class="setting-hint">Uses more power; reviews otherwise wait for a charger.</span>
          </div>
          <USwitch
            v-model="settings.reviewOnBattery"
            aria-labelledby="review-battery-label"
            :disabled="settings.reviewAuto === 'off'"
            class="setting-switch"
          />
        </div>
      </div>
    </section>

    <ConfirmDialog
      v-model:open="confirmDeleteEngine"
      title="Delete downloaded Stockfish?"
      description="This removes the copy KChess downloaded. The bundled engine and any executable you chose yourself are not touched, and you can download it again later."
      confirm-label="Delete"
      color="error"
      @confirm="deleteEngine"
    />
  </div>
</template>
