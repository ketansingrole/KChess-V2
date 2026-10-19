<script setup lang="ts">
import type { ResultBadge } from '../utils/games'

defineProps<{ badge: ResultBadge; size?: 'sm' | 'md' }>()
</script>

<template>
  <UBadge :color="badge.color" :variant="badge.variant" :size="size" class="justify-center">{{
    badge.label
  }}</UBadge>
</template>
