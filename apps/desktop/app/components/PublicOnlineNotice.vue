<script setup lang="ts">
import { useOnline } from '@vueuse/core'
defineProps<{ offlineMessage: string }>()
const online = useOnline()
</script>

<template>
  <p v-if="!online" class="section-hint mb-4" role="status">{{ offlineMessage }}</p>
</template>
