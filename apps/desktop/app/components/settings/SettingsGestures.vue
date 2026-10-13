<script setup lang="ts">
import { useKChessStore } from '../../stores/kchess'

const store = useKChessStore()
const { settings } = storeToRefs(store)
</script>

<template>
  <section id="settings-gestures" class="card settings-list" aria-labelledby="gestures-title">
    <h2 id="gestures-title" class="sr-only">Gestures</h2>
    <div class="setting-row">
      <div class="setting-info">
        <span id="swipe-nav-label" class="setting-title">Two-finger swipe</span>
        <span class="setting-hint">Go back and forward with a horizontal swipe.</span>
      </div>
      <USwitch
        v-model="settings.swipeNavigation"
        aria-labelledby="swipe-nav-label"
        class="setting-switch"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="swipe-hint-label" class="setting-title">Swipe feedback</span>
        <span class="setting-hint">Show an edge hint that follows the swipe.</span>
      </div>
      <USwitch
        v-model="settings.swipeIndicator"
        aria-labelledby="swipe-hint-label"
        class="setting-switch"
        :disabled="!settings.swipeNavigation"
      />
    </div>
  </section>
</template>
