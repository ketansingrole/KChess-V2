<script setup lang="ts">
const props = defineProps<{ title: string }>()
useHead(() => ({ title: `${props.title} — KChess` }))
</script>

<template>
  <header :class="$slots.default ? 'page-header' : 'sr-only'">
    <!-- The sidebar already names the page, so the title is for screen readers only. -->
    <h1 class="sr-only">{{ title }}</h1>
    <div v-if="$slots.default" class="page-actions"><slot /></div>
  </header>
</template>
