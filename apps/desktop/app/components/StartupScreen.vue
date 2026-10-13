<script setup lang="ts">
defineProps<{ error?: string; loading?: boolean }>()
defineEmits<{ retry: [] }>()
</script>

<template>
  <div class="splash">
    <img src="../assets/icon-splash.png" alt="" width="72" height="72" class="splash-logo" />
    <template v-if="error">
      <h1 class="splash-title">KChess couldn't start</h1>
      <p class="splash-text" role="alert">{{ error }}</p>
      <UButton icon="i-lucide-rotate-cw" :loading="loading" @click="$emit('retry')">
        Try again
      </UButton>
    </template>
    <template v-else>
      <UIcon name="i-lucide-loader-circle" class="animate-spin splash-spinner" />
      <p class="splash-text" role="status">Loading KChess…</p>
    </template>
  </div>
</template>
