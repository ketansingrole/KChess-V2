<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import type { DropdownMenuItem } from '@nuxt/ui'
import StudyChapters from './StudyChapters.vue'
import { useAnalysisStore } from '../stores/analysis'
import { useStudyStore } from '../stores/studies'
import { timeAgo } from '../utils/format'

/**
 * One line above the board: which study the board is (saved as you go), or a way to keep it as one.
 * The library itself is its own page.
 */
const analysis = useAnalysisStore()
const library = useStudyStore()
const app = useKChessStore()
const toast = useToast()
const { study, studySaveError, root } = storeToRefs(analysis)

/* ── Name: edited in place ── */
const name = ref('')
watch(
  study,
  (open) => {
    name.value = open?.name ?? ''
  },
  { immediate: true },
)
function rename(): void {
  const open = study.value
  if (!open) return
  if (!name.value.trim()) name.value = open.name
  else library.rename(open.id, name.value)
}

/* ── Saving the board as a new study ── */
const saveOpen = ref(false)
const newName = ref('')
const saveError = ref('')
watch(saveOpen, (opened) => {
  if (!opened) return
  const white = root.value.headers?.White
  const black = root.value.headers?.Black
  newName.value =
    white && black && white !== '?' && black !== '?' ? `${white} – ${black}` : library.freshName()
  saveError.value = ''
})
function saveNew(): void {
  try {
    analysis.saveAsStudy(newName.value)
    saveOpen.value = false
    toast.add({
      title: 'Study saved',
      description: 'Changes on the board are now saved to it automatically.',
      icon: 'i-lucide-book-check',
    })
  } catch (cause) {
    console.warn('[study-bar] Could not save study:', cause)
    saveError.value = cause instanceof Error ? cause.message : String(cause)
  }
}

/* ── Game details (PGN headers) ── */
const detailsOpen = ref(false)
function header(key: string): string {
  const value = root.value.headers?.[key] ?? ''
  return value === '?' ? '' : value
}
function setHeader(key: string, value: string): void {
  root.value.headers = { ...root.value.headers, [key]: value }
}

/* ── Status ── */
const now = ref(Date.now())
const clock = setInterval(() => (now.value = Date.now()), 30_000)
onUnmounted(() => clearInterval(clock))
const status = computed(() => {
  if (studySaveError.value) return { icon: 'i-lucide-triangle-alert', text: studySaveError.value }
  const open = study.value
  if (!open) return null
  return { icon: 'i-lucide-check', text: `Saved ${timeAgo(open.updatedAt, now.value)}` }
})

function remove(): void {
  const open = study.value
  if (!open) return
  const removed = library.remove(open.id)
  if (!removed) return
  toast.add({
    title: `Deleted “${removed.name}”`,
    description: 'The board keeps its moves.',
    icon: 'i-lucide-trash-2',
    actions: [{ label: 'Undo', onClick: () => restore(removed) }],
  })
}
function restore(removed: NonNullable<typeof study.value>): void {
  library.restore(removed)
  analysis.studyId = removed.id
}
function saveCopy(): void {
  const open = study.value
  if (!open) return
  try {
    analysis.saveAsStudy(`${open.name.slice(0, 113)} (copy)`)
    toast.add({ title: 'Saved as a new study', icon: 'i-lucide-copy' })
  } catch (cause) {
    console.warn('[study-bar] Could not save study copy:', cause)
    toast.add({ title: String(cause), color: 'error' })
  }
}

const menu = computed<DropdownMenuItem[][]>(() => [
  [
    {
      label: 'Game details…',
      icon: 'i-lucide-file-pen-line',
      onSelect: () => (detailsOpen.value = true),
    },
  ],
  study.value
    ? [
        { label: 'Save a copy', icon: 'i-lucide-copy', onSelect: saveCopy },
        {
          label: 'Close study',
          icon: 'i-lucide-book-x',
          onSelect: () => analysis.closeStudy(),
        },
        {
          label: study.value?.cloud ? 'Remove offline copy' : 'Delete study',
          icon: 'i-lucide-trash-2',
          color: 'error',
          onSelect: remove,
        },
      ]
    : [],
])
</script>

<template>
  <div class="study-bar">
    <UIcon
      :name="study ? 'i-lucide-book-open' : 'i-lucide-book-dashed'"
      class="study-icon"
      aria-hidden="true"
    />
    <template v-if="study">
      <input
        v-model="name"
        class="study-name"
        aria-label="Study name"
        maxlength="120"
        @blur="rename"
        @keydown.enter="($event.target as HTMLInputElement).blur()"
        @keydown.escape="((name = study.name), ($event.target as HTMLInputElement).blur())"
      />
      <StudyChapters class="study-chapters" />
      <span
        v-if="status"
        class="study-status"
        :class="{ 'text-error': studySaveError }"
        role="status"
      >
        <UIcon :name="status.icon" />{{ status.text }}
      </span>
    </template>
    <template v-else>
      <span class="study-untitled">Unsaved analysis</span>
      <UPopover v-model:open="saveOpen">
        <UButton size="xs" variant="soft" icon="i-lucide-book-plus">Save as study</UButton>
        <template #content>
          <form class="flex w-72 flex-col gap-2 p-3" @submit.prevent="saveNew">
            <UFormField label="Study name">
              <UInput v-model="newName" autofocus :maxlength="120" class="w-full" />
            </UFormField>
            <p v-if="saveError || library.error" class="text-xs text-error">
              {{ saveError || library.error }}
            </p>
            <UButton type="submit" block :disabled="!newName.trim()">Save study</UButton>
          </form>
        </template>
      </UPopover>
    </template>

    <span class="flex-1" />
    <UButton
      size="xs"
      variant="ghost"
      color="neutral"
      icon="i-lucide-library-big"
      @click="app.selectPage('studies')"
      >All studies</UButton
    >
    <UDropdownMenu :items="menu">
      <UButton
        size="xs"
        variant="ghost"
        color="neutral"
        icon="i-lucide-ellipsis"
        aria-label="Study actions"
      />
    </UDropdownMenu>

    <UModal v-model:open="detailsOpen" title="Game details">
      <template #body>
        <div class="grid gap-3 sm:grid-cols-2">
          <UFormField label="White">
            <UInput
              :model-value="header('White')"
              class="w-full"
              @update:model-value="setHeader('White', $event)"
            />
          </UFormField>
          <UFormField label="Black">
            <UInput
              :model-value="header('Black')"
              class="w-full"
              @update:model-value="setHeader('Black', $event)"
            />
          </UFormField>
          <UFormField label="Event">
            <UInput
              :model-value="header('Event')"
              class="w-full"
              @update:model-value="setHeader('Event', $event)"
            />
          </UFormField>
          <UFormField label="Date">
            <UInput
              :model-value="header('Date')"
              placeholder="YYYY.MM.DD"
              class="w-full"
              @update:model-value="setHeader('Date', $event)"
            />
          </UFormField>
          <UFormField label="Result">
            <USelect
              :model-value="header('Result') || '*'"
              :items="['*', '1-0', '0-1', '1/2-1/2']"
              class="w-full"
              @update:model-value="setHeader('Result', $event as string)"
            />
          </UFormField>
        </div>
      </template>
    </UModal>
  </div>
</template>

<style scoped>
.study-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  min-height: 36px;
  margin-bottom: 12px;
}
.study-chapters {
  flex: 0 1 260px;
  min-width: 160px;
  max-width: 100%;
}
.study-icon {
  flex: none;
  color: var(--ui-text-muted);
  font-size: 18px;
}
.study-name {
  min-width: 6ch;
  max-width: 40ch;
  field-sizing: content;
  padding: 3px 6px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--ui-text-highlighted);
  font: inherit;
  font-size: 15px;
  font-weight: 650;
}
.study-name:hover {
  border-color: var(--ui-border);
}
.study-name:focus {
  border-color: var(--ui-primary);
  outline: none;
  background: var(--ui-bg);
}
.study-untitled {
  color: var(--ui-text-muted);
  font-size: 14px;
}
.study-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--ui-text-dimmed);
  font-size: 12px;
  white-space: nowrap;
}
</style>
