<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { exposeToastFocusGuards, observeUiAccessibility } from '../utils/uiAccessibility'

const root = ref<HTMLElement | null>(null)
let stop: (() => void) | undefined
onMounted(() => {
  if (root.value) stop = observeUiAccessibility(root.value, exposeToastFocusGuards)
})
onUnmounted(() => stop?.())
</script>

<template>
  <!-- Keep the viewport and its sibling focus proxies inside this stable owner. -->
  <div ref="root" class="contents" data-ui-accessibility="notifications">
    <UToaster :portal="false" :ui="{ viewport: 'kchess-toasts' }" />
  </div>
</template>
