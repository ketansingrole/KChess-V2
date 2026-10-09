<script setup lang="ts">
import { ref, watch } from 'vue'

const boundary = ref<{ clearError: () => void } | null>(null)
const router = useRouter()
// A failed page has no NuxtPage to finish navigation and update Nuxt's deferred route.
// Watch the router directly, without remounting healthy pages just to clear an error.
watch(
  () => router.currentRoute.value.path,
  () => boundary.value?.clearError(),
)
</script>

<template>
  <NuxtErrorBoundary ref="boundary">
    <slot />
    <template #error="{ clearError }">
      <div class="card">
        <h1 class="section-title" role="alert">This page couldn't load</h1>
        <UButton class="mt-3" icon="i-lucide-rotate-cw" @click="clearError">Try again</UButton>
      </div>
    </template>
  </NuxtErrorBoundary>
</template>
