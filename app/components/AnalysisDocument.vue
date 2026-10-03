<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useAnalysisStore } from '../stores/analysis'
import { useStudyStore } from '../stores/studies'
const analysis = useAnalysisStore()
const library = useStudyStore()
const name = ref('')
const selected = ref('')
const feedback = ref('')
watch(
  () => analysis.root,
  () => {
    selected.value = ''
    name.value = ''
  },
  { flush: 'sync' },
)
const comment = computed({
  get: () => (analysis.node.comments ?? []).join('\n\n'),
  set: (text: string) => {
    analysis.node.comments = text.trim() ? [text] : undefined
  },
})
function header(key: string): string {
  return analysis.root.headers?.[key] ?? ''
}
function setHeader(key: string, value: string): void {
  analysis.root.headers = { ...analysis.root.headers, [key]: value }
}
function save(): void {
  try {
    selected.value = library.save(name.value, analysis.pgn(), selected.value || undefined)
    feedback.value = 'Study saved on this device.'
  } catch (error) {
    feedback.value = error instanceof Error ? error.message : String(error)
  }
}
function saveAsNew(): void {
  selected.value = ''
  feedback.value = 'The next save creates a new study.'
}
function remove(id: string): void {
  library.remove(id)
  if (selected.value === id) selected.value = ''
}
function open(id: string): void {
  const study = library.items.find((item) => item.id === id)
  if (study && analysis.loadPgn(study.pgn)) {
    selected.value = id
    name.value = study.name
    feedback.value = 'Study opened.'
  }
}
</script>
<template>
  <details class="card mb-4">
    <summary class="cursor-pointer font-semibold">Study details and saved studies</summary>
    <div class="mt-3 grid gap-3 sm:grid-cols-2">
      <UFormField label="White"
        ><UInput :model-value="header('White')" @update:model-value="setHeader('White', $event)"
      /></UFormField>
      <UFormField label="Black"
        ><UInput :model-value="header('Black')" @update:model-value="setHeader('Black', $event)"
      /></UFormField>
      <UFormField label="Event"
        ><UInput :model-value="header('Event')" @update:model-value="setHeader('Event', $event)"
      /></UFormField>
      <UFormField label="Result"
        ><USelect
          :model-value="header('Result') || '*'"
          :items="['*', '1-0', '0-1', '1/2-1/2']"
          @update:model-value="setHeader('Result', $event as string)"
      /></UFormField>
      <UFormField label="Comment on this position" class="sm:col-span-2"
        ><UTextarea v-model="comment" class="w-full" :maxlength="10000"
      /></UFormField>
    </div>
    <form class="mt-3 flex flex-wrap gap-2" @submit.prevent="save">
      <UInput v-model="name" aria-label="Study name" placeholder="Study name" :maxlength="120" />
      <UButton type="submit">{{ selected ? 'Update study' : 'Save study' }}</UButton>
      <UButton v-if="selected" variant="outline" color="neutral" @click="saveAsNew"
        >Save as new</UButton
      >
    </form>
    <p role="status" class="mt-2 text-sm">{{ library.error || feedback }}</p>
    <ul class="mt-3 space-y-2">
      <li v-for="study in library.items" :key="study.id" class="flex items-center gap-2">
        <UButton variant="link" @click="open(study.id)">{{ study.name }}</UButton>
        <span class="flex-1" />
        <UButton
          size="xs"
          variant="ghost"
          color="neutral"
          :aria-label="'Remove saved study ' + study.name"
          @click="remove(study.id)"
          >Remove</UButton
        >
      </li>
    </ul>
  </details>
</template>
