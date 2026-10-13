<script setup lang="ts">
import { computed } from 'vue'
import { useKChessStore } from '../../stores/kchess'
import { analysisSwitches, onlineSwitches, pollItems } from './options'

const props = defineProps<{ section: 'online' | 'analysis' }>()
const store = useKChessStore()
const { settings } = storeToRefs(store)
const switches = computed(() => (props.section === 'online' ? onlineSwitches : analysisSwitches))
</script>

<template>
  <section :id="`settings-${section}`" class="card settings-list">
    <h2 class="sr-only">{{ section === 'online' ? 'Online play' : 'Analysis' }}</h2>
    <div v-for="item in switches" :key="item.key" class="setting-row">
      <div class="setting-info">
        <span :id="`${item.key}-label`" class="setting-title">{{ item.title }}</span>
        <span v-if="'hint' in item" class="setting-hint">{{ item.hint }}</span>
      </div>
      <USwitch
        v-model="settings[item.key]"
        :aria-labelledby="`${item.key}-label`"
        class="setting-switch"
      />
    </div>
    <div v-if="section === 'online'" class="setting-row">
      <div class="setting-info">
        <span id="corr-label" class="setting-title">Check correspondence games</span>
        <span class="setting-hint">Only while KChess is open.</span>
      </div>
      <USelect
        v-model="settings.correspondencePoll"
        :items="pollItems"
        aria-labelledby="corr-label"
        class="setting-control"
      />
    </div>
  </section>
</template>
