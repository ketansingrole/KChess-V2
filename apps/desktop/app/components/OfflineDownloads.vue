<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useIntervalFn, useOnline } from '@vueuse/core'
import type { VoiceModelStatus } from '@kchess/contracts/types'
import { formatBytes } from '../utils/format'

const app = useKChessStore()
const online = useOnline()
const voice = ref<VoiceModelStatus | null>(null)
const voiceError = ref('')
const downloading = ref(false)
let offProgress: (() => void) | undefined
let alive = true
let refreshing = false
async function refreshVoice(): Promise<void> {
  if (refreshing) return
  refreshing = true
  try {
    const status = await window.kchess.voiceModelStatus()
    if (alive) voice.value = status
  } catch (cause) {
    if (alive) {
      console.warn('[offline-downloads] Could not check the voice model:', cause)
      voiceError.value = cause instanceof Error ? cause.message : 'Could not check the voice model.'
    }
  } finally {
    refreshing = false
  }
}
onMounted(() => {
  offProgress = window.kchess.onVoiceModelProgress((progress) => {
    voice.value = { installed: false, bytes: 0, busy: true, progress }
  })
  void refreshVoice()
})
onUnmounted(() => {
  alive = false
  offProgress?.()
})
useIntervalFn(() => {
  if (voice.value?.busy) void refreshVoice()
}, 2000)
async function downloadVoice(): Promise<void> {
  if (!online.value || downloading.value || voice.value?.busy) return
  downloading.value = true
  voiceError.value = ''
  try {
    await window.kchess.ensureVoiceModel()
  } catch (cause) {
    if (alive) {
      console.warn('[offline-downloads] Could not download the voice model:', cause)
      voiceError.value =
        cause instanceof Error ? cause.message : 'Could not download the voice model.'
    }
  } finally {
    downloading.value = false
    if (alive) void refreshVoice()
  }
}
const percent = computed(() => {
  const progress = voice.value?.progress
  return progress?.total
    ? Math.min(100, Math.round((100 * progress.received) / progress.total))
    : null
})
const progressText = computed(() => {
  const progress = voice.value?.progress
  if (!progress || progress.phase === 'checking') return 'Checking the voice model…'
  if (progress.phase === 'preparing') return 'Preparing offline speech recognition…'
  return `Downloading… ${formatBytes(progress.received)}${progress.total ? ` of ${formatBytes(progress.total)}` : ''}`
})
function openSection(section: 'engine' | 'voice'): void {
  app.jumpToSection(section)
}
</script>

<template>
  <div class="space-y-4">
    <p v-if="!online" class="section-hint" role="status">
      Offline · reconnect to download content.
    </p>
    <section class="card" aria-labelledby="offline-engine-title">
      <div class="card-header">
        <h2 id="offline-engine-title" class="section-title">Chess engine</h2>
        <UBadge variant="soft" color="success">Included with KChess</UBadge>
      </div>
      <p v-if="app.engineReady && app.engineName" class="section-hint">
        Selected engine: {{ app.engineName }}.
      </p>
      <p v-if="app.engineInfo?.managed.installed" class="section-hint">
        Optional native engine installed{{
          app.engineInfo.managed.version ? ` · ${app.engineInfo.managed.version}` : ''
        }}.
      </p>
      <p v-if="!app.engineReady" class="section-hint">
        Selected engine unavailable. Choose another in Chess engine settings.
      </p>
      <UButton
        class="mt-3"
        variant="outline"
        color="neutral"
        icon="i-lucide-cpu"
        @click="openSection('engine')"
        >Chess engine settings</UButton
      >
    </section>
    <section class="card" aria-labelledby="offline-puzzles-title">
      <h2 id="offline-puzzles-title" class="section-title mb-3">Downloaded puzzles</h2>
      <PuzzleDbCard compact />
    </section>
    <section class="card" aria-labelledby="offline-voice-title">
      <div class="card-header">
        <h2 id="offline-voice-title" class="section-title">Offline voice input</h2>
        <UBadge v-if="voice?.installed" variant="soft" color="success"
          >Installed · {{ formatBytes(voice.bytes) }}</UBadge
        >
        <UBadge v-else-if="voice && !voice.busy" variant="soft" color="neutral"
          >Download needed</UBadge
        >
      </div>
      <p v-if="voice && !voice.installed" class="section-hint">
        English speech model · up to 50 MB
      </p>
      <template v-if="voice?.busy || downloading">
        <UProgress class="mt-3" :model-value="percent" />
        <p class="section-hint tabular" role="status">{{ progressText }}</p>
      </template>
      <p v-else-if="!voice && !voiceError" class="section-hint" role="status">
        Checking installed content…
      </p>
      <UButton
        v-else-if="voice && !voice.installed"
        class="mt-3"
        icon="i-lucide-download"
        :disabled="!online"
        @click="downloadVoice"
        >Download voice model</UButton
      >
      <UButton
        v-if="voice?.installed"
        class="mt-3"
        variant="outline"
        color="neutral"
        icon="i-lucide-mic"
        @click="openSection('voice')"
        >Voice input settings</UButton
      >
      <p v-if="voiceError" class="text-error text-sm mt-3" role="alert">{{ voiceError }}</p>
    </section>
    <section class="card">
      <h2 class="section-title">Saved games and studies</h2>
      <div class="toolbar-row mt-3">
        <UButton variant="outline" color="neutral" @click="app.selectPage('studies')"
          >Your studies</UButton
        >
        <UButton variant="outline" color="neutral" @click="app.selectPage('history')"
          >Game history</UButton
        >
      </div>
    </section>
  </div>
</template>
