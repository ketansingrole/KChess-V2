<script setup lang="ts">
import { useOnline } from '@vueuse/core'
defineProps<{ offlineMessage: string }>()
const online = useOnline()
</script>

<template>
  <p class="section-hint mb-4" role="status">
    <template v-if="online">Internet required · No account needed to browse.</template>
    <template v-else>{{ offlineMessage }}</template>
  </p>
</template>
