<script setup lang="ts">
import { computed } from 'vue'
import { SETTINGS_SECTIONS } from '../stores/kchess'

const store = useKChessStore()
const { settingsSection } = storeToRefs(store)

// The sidebar lists the categories while Settings is open; the one chosen there is the page shown here.
const category = computed(
  () =>
    SETTINGS_SECTIONS.find((section) => section.id === settingsSection.value) ??
    SETTINGS_SECTIONS[0],
)
</script>

<template>
  <div>
    <PageHeader :title="category.label" />

    <div class="settings-page">
      <LazyAppUpdateSettings v-if="category.id === 'updates'" />
      <LazyOfflineDownloads v-if="category.id === 'offline'" />
      <SettingsAppearance v-else-if="category.id === 'appearance'" />
      <SettingsThemes v-else-if="category.id === 'themes'" />
      <SettingsGameplay v-else-if="category.id === 'gameplay'" />
      <SettingsGestures v-else-if="category.id === 'gestures'" />
      <SettingsOnlineAnalysis
        v-else-if="category.id === 'online' || category.id === 'analysis'"
        :section="category.id"
      />
      <SettingsSound v-else-if="category.id === 'sound'" />
      <SettingsNotifications v-else-if="category.id === 'notifications'" />
      <SettingsVoice v-else-if="category.id === 'voice'" />
      <LazyVoiceHistory v-if="category.id === 'voice'" />
      <LazyEngineSettings v-if="category.id === 'engine'" />
      <SettingsAccounts v-else-if="category.id === 'accounts'" />
      <SettingsData v-else-if="category.id === 'data'" />
    </div>
  </div>
</template>
