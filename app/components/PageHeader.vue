<script setup lang="ts">
defineProps<{ title: string; subtitle?: string }>()
</script>

<template>
  <header :class="subtitle || $slots.default ? 'page-header' : 'sr-only'">
    <div>
      <!-- The sidebar already names the page, so the title is for screen readers only. -->
      <h1 class="sr-only">{{ title }}</h1>
      <p v-if="subtitle" class="page-subtitle">{{ subtitle }}</p>
    </div>
    <div v-if="$slots.default" class="page-actions"><slot /></div>
  </header>
</template>
