<script setup lang="ts">
defineProps<{ title: string; description: string; active: boolean }>()
</script>

<template>
  <div class="engine-option" :aria-current="active ? 'true' : undefined">
    <div class="min-w-0">
      <div class="flex items-center gap-2">
        <strong class="truncate">{{ title }}</strong>
        <UBadge v-if="active" color="primary" variant="soft" size="sm">In use</UBadge>
      </div>
      <div class="muted text-xs break-all">{{ description }}</div>
    </div>
    <div class="toolbar-row flex-nowrap shrink-0">
      <slot />
    </div>
  </div>
</template>
