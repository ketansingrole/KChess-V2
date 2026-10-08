<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { DropdownMenuItem } from '@nuxt/ui'
import ConfirmDialog from './ConfirmDialog.vue'
import { useAnalysisStore } from '../stores/analysis'
import { useStudyStore } from '../stores/studies'
const analysis = useAnalysisStore()
const library = useStudyStore()
const study = computed(() => analysis.study)
const chapter = computed(() => study.value?.chapters.find((c) => c.id === analysis.studyChapterId))
const editing = ref<'add' | 'rename' | null>(null)
const name = ref('')
const error = ref('')
const deleting = ref(false)
watch(
  () => [analysis.studyId, analysis.studyChapterId],
  () => {
    editing.value = null
    deleting.value = false
    error.value = ''
  },
)
function edit(mode: 'add' | 'rename'): void {
  name.value = mode === 'rename' ? (chapter.value?.name ?? '') : ''
  error.value = ''
  editing.value = mode
}
async function save(): Promise<void> {
  if (!study.value || !name.value.trim()) return
  const studyId = study.value.id
  try {
    await analysis.flushStudy()
    if (editing.value === 'add') {
      const id = await library.addChapter(studyId, name.value)
      analysis.openStudy(studyId, id)
    } else await library.renameChapter(studyId, analysis.studyChapterId, name.value)
    editing.value = null
    error.value = ''
  } catch (cause) {
    console.warn('[study-chapters] Could not save chapter:', cause)
    error.value = cause instanceof Error ? cause.message : String(cause)
  }
}
async function duplicate(): Promise<void> {
  if (!study.value) return
  const studyId = study.value.id
  try {
    await analysis.flushStudy()
    const id = await library.duplicateChapter(studyId, analysis.studyChapterId)
    analysis.openStudy(studyId, id)
  } catch (cause) {
    console.warn('[study-chapters] Could not duplicate chapter:', cause)
    error.value = cause instanceof Error ? cause.message : String(cause)
  }
}
async function remove(): Promise<void> {
  const id = study.value?.id
  const chapterId = analysis.studyChapterId
  if (!id) return
  try {
    // Detach the old board before deleting, so pending autosave cannot resurrect its chapter.
    if (!(await analysis.closeStudy())) {
      error.value = analysis.studySaveError
      return
    }
    const next = await library.removeChapter(id, chapterId)
    analysis.openStudy(id, next)
  } catch (cause) {
    analysis.openStudy(id, chapterId)
    console.warn('[study-chapters] Could not delete chapter:', cause)
    error.value = cause instanceof Error ? cause.message : String(cause)
  }
}
const menu = computed<DropdownMenuItem[][]>(() => [
  [
    {
      label: 'Add chapter',
      icon: 'i-lucide-plus',
      disabled: (study.value?.chapters.length ?? 64) >= 64,
      onSelect: () => edit('add'),
    },
    { label: 'Rename chapter', icon: 'i-lucide-pencil', onSelect: () => edit('rename') },
    {
      label: 'Duplicate chapter',
      icon: 'i-lucide-copy',
      disabled: (study.value?.chapters.length ?? 64) >= 64,
      onSelect: duplicate,
    },
  ],
  [
    {
      label: 'Delete chapter',
      icon: 'i-lucide-trash-2',
      color: 'error',
      disabled: (study.value?.chapters.length ?? 0) <= 1,
      description:
        study.value?.chapters.length === 1 ? 'A study needs at least one chapter' : undefined,
      onSelect: () => {
        deleting.value = true
      },
    },
  ],
])
</script>
<template>
  <section v-if="study" aria-label="Chapters">
    <div class="flex items-center gap-2">
      <USelectMenu
        :model-value="analysis.studyChapterId"
        value-key="value"
        :items="
          study.chapters.map((c, index) => ({ label: `${index + 1}. ${c.name}`, value: c.id }))
        "
        aria-label="Study chapter"
        placeholder="Select chapter"
        class="flex-1 min-w-0"
        size="sm"
        @update:model-value="(id: string) => analysis.openStudy(study!.id, id)"
      />
      <UDropdownMenu :items="menu">
        <UButton
          size="sm"
          variant="ghost"
          color="neutral"
          icon="i-lucide-ellipsis"
          aria-label="Chapter actions"
        />
      </UDropdownMenu>
    </div>
    <p v-if="error && !editing" class="text-sm text-error mt-2" role="alert">{{ error }}</p>
    <UModal
      :open="editing !== null"
      :title="editing === 'add' ? 'Add chapter' : 'Rename chapter'"
      @update:open="!$event && (editing = null)"
    >
      <template #body>
        <form class="flex flex-col gap-3" @submit.prevent="save">
          <UInput
            v-model="name"
            aria-label="Chapter name"
            :maxlength="120"
            autofocus
            class="w-full"
          />
          <p v-if="error" class="text-sm text-error" role="alert">{{ error }}</p>
          <div class="flex justify-end gap-2">
            <UButton color="neutral" variant="outline" @click="editing = null">Cancel</UButton>
            <UButton type="submit" :disabled="!name.trim()">{{
              editing === 'add' ? 'Add' : 'Save'
            }}</UButton>
          </div>
        </form>
      </template>
    </UModal>
    <ConfirmDialog
      v-model:open="deleting"
      :title="`Delete “${chapter?.name ?? ''}”?`"
      :description="
        study.cloud
          ? 'Removes this chapter from the offline study. The Lichess copy is unchanged.'
          : 'Removes this chapter and all of its moves, comments, and variations.'
      "
      confirm-label="Delete chapter"
      color="error"
      @confirm="remove"
    />
  </section>
</template>
