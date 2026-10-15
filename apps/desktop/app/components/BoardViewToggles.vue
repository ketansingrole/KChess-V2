<script setup lang="ts">
/** The blindfold and zen mode toggles every game page shows at the top of its side panel. */
const store = useKChessStore()
const { settings, zenActive } = storeToRefs(store)
</script>

<template>
  <UTooltip text="Blindfold: hide the pieces">
    <UButton
      size="xs"
      :variant="settings.blindfold ? 'soft' : 'ghost'"
      :color="settings.blindfold ? 'primary' : 'neutral'"
      icon="i-lucide-eye-off"
      aria-label="Blindfold"
      :aria-pressed="settings.blindfold"
      @click="store.toggleSetting('blindfold')"
    />
  </UTooltip>
  <UTooltip text="Zen mode (Z)">
    <UButton
      size="xs"
      :variant="zenActive ? 'soft' : 'ghost'"
      :color="zenActive ? 'primary' : 'neutral'"
      icon="i-lucide-maximize"
      aria-label="Zen mode"
      :aria-pressed="settings.zenMode"
      @click="store.toggleSetting('zenMode')"
    />
  </UTooltip>
</template>
