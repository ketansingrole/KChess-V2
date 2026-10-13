<script setup lang="ts">
import { useKChessStore } from '../../stores/kchess'

const store = useKChessStore()
const { settings } = storeToRefs(store)
</script>

<template>
  <section id="settings-sound" class="card settings-list" aria-labelledby="sound-title">
    <h2 id="sound-title" class="sr-only">Sound</h2>
    <div class="setting-row">
      <div class="setting-info">
        <span id="sounds-label" class="setting-title">Game sounds</span>
      </div>
      <USwitch
        v-model="settings.soundEnabled"
        aria-labelledby="sounds-label"
        class="setting-switch"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="volume-label" class="setting-title">Volume</span>
        <span class="setting-hint tabular">{{ Math.round(settings.soundVolume * 100) }}%</span>
      </div>
      <USlider
        v-model="settings.soundVolume"
        class="setting-control"
        :min="0"
        :max="1"
        :step="0.05"
        :disabled="!settings.soundEnabled"
        aria-labelledby="volume-label"
      />
    </div>
  </section>
</template>
