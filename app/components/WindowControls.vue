<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'

const isMaximized = ref(false)
let off: (() => void) | undefined

async function refresh(): Promise<void> {
  try {
    isMaximized.value = await window.kchess.windowIsMaximized()
  } catch {
    /* browser preview has no window controls */
  }
}

function minimize(): void {
  void window.kchess.windowMinimize().catch(() => {})
}

function toggleMaximize(): void {
  void window.kchess
    .windowToggleMaximize()
    .then((state) => {
      isMaximized.value = state.maximized
    })
    .catch(() => {})
}

function closeWindow(): void {
  void window.kchess.windowClose().catch(() => {})
}

onMounted(() => {
  void refresh()
  try {
    off = window.kchess.onWindowMaximized((state) => {
      isMaximized.value = state.maximized
    })
  } catch {
    off = undefined
  }
})

onUnmounted(() => {
  off?.()
})
</script>

<template>
  <div class="window-controls" role="group" aria-label="Window controls">
    <button
      type="button"
      class="window-control"
      title="Minimize"
      aria-label="Minimize"
      @click="minimize"
    >
      <UIcon name="i-lucide-minus" class="window-control-icon" />
    </button>
    <button
      type="button"
      class="window-control"
      :title="isMaximized ? 'Restore' : 'Maximize'"
      :aria-label="isMaximized ? 'Restore' : 'Maximize'"
      @click="toggleMaximize"
    >
      <UIcon :name="isMaximized ? 'i-lucide-copy' : 'i-lucide-square'" class="window-control-icon" />
    </button>
    <button
      type="button"
      class="window-control close"
      title="Close"
      aria-label="Close"
      @click="closeWindow"
    >
      <UIcon name="i-lucide-x" class="window-control-icon" />
    </button>
  </div>
</template>
