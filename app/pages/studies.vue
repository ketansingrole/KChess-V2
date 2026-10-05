<script setup lang="ts">
import { computed, ref } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import type { DropdownMenuItem } from '@nuxt/ui'
import { useAnalysisStore } from '../stores/analysis'
import { useStudyStore, type SavedStudy } from '../stores/studies'
import { treeFromPgn } from '../utils/analysisTree'
import { summarizeStudy } from '../utils/studies'
import { timeAgo } from '../utils/format'

/** The study library: browse, search and open saved studies, or bring them in from Lichess. */
const app = useKChessStore()
const { settings } = storeToRefs(app)
const analysis = useAnalysisStore()
const library = useStudyStore()
const toast = useToast()

const tab = useLocalStorage('kchess:studies-tab', 'device')
const tabs = computed(() => [
  {
    label: `On this device · ${library.items.length}`,
    value: 'device',
    icon: 'i-lucide-hard-drive',
  },
  { label: 'Lichess', value: 'lichess', icon: 'i-lucide-cloud' },
])

/* ── Browsing ── */
const query = ref('')
const sort = useLocalStorage<'recent' | 'name'>('kchess:studies-sort', 'recent')
const sorts = [
  { label: 'Recently edited', value: 'recent' },
  { label: 'Name', value: 'name' },
]
const cards = computed(() => {
  const words = query.value.toLowerCase().split(/\s+/).filter(Boolean)
  return library.items
    .map((study) => ({ study, summary: summarizeStudy(study.pgn) }))
    .filter(({ study, summary }) => {
      const text = `${study.name} ${summary?.players ?? ''} ${summary?.event ?? ''}`.toLowerCase()
      return words.every((word) => text.includes(word))
    })
    .sort((a, b) =>
      sort.value === 'name'
        ? a.study.name.localeCompare(b.study.name, undefined, { numeric: true })
        : b.study.updatedAt - a.study.updatedAt,
    )
})
const now = Date.now()
function details(summary: ReturnType<typeof summarizeStudy>): string {
  if (!summary) return ''
  const moves = Math.ceil(summary.plies / 2)
  const parts = [moves ? `${moves} move${moves === 1 ? '' : 's'}` : 'No moves yet']
  if (summary.variations)
    parts.push(`${summary.variations} variation${summary.variations === 1 ? '' : 's'}`)
  if (summary.comments)
    parts.push(`${summary.comments} comment${summary.comments === 1 ? '' : 's'}`)
  return parts.join(' · ')
}

/* ── Opening: never silently throw away an unsaved board ── */
const pending = ref<(() => void) | null>(null)
const unsavedBoard = computed(() => !analysis.study && analysis.root.children.length > 0)
function guard(action: () => void): void {
  if (unsavedBoard.value) pending.value = action
  else action()
}
function open(study: SavedStudy): void {
  if (analysis.studyId === study.id) return app.selectPage('analysis')
  guard(() => {
    if (analysis.openStudy(study.id)) app.selectPage('analysis')
  })
}
function create(): void {
  guard(() => {
    analysis.load()
    try {
      analysis.saveAsStudy(library.freshName())
      app.selectPage('analysis')
    } catch (cause) {
      toast.add({ title: String(cause), color: 'error' })
    }
  })
}

/* ── Card actions ── */
const renaming = ref<SavedStudy | null>(null)
const newName = ref('')
function startRename(study: SavedStudy): void {
  renaming.value = study
  newName.value = study.name
}
function rename(): void {
  if (renaming.value && newName.value.trim()) library.rename(renaming.value.id, newName.value)
  renaming.value = null
}
function remove(study: SavedStudy): void {
  const removed = library.remove(study.id)
  if (!removed) return
  toast.add({
    title: `Deleted “${removed.name}”`,
    icon: 'i-lucide-trash-2',
    actions: [{ label: 'Undo', onClick: () => library.restore(removed) }],
  })
}
function duplicate(study: SavedStudy): void {
  try {
    library.duplicate(study.id)
  } catch (cause) {
    toast.add({ title: String(cause), color: 'error' })
  }
}
async function copyPgn(study: SavedStudy): Promise<void> {
  try {
    await navigator.clipboard.writeText(study.pgn)
    toast.add({ title: 'PGN copied', icon: 'i-lucide-clipboard-check' })
  } catch {
    toast.add({ title: 'Could not copy the PGN', color: 'error' })
  }
}
// The Practice page's own keys: open its repertoire drill on this study.
const practiceTab = useLocalStorage('kchess:practice-tab', 'coordinates')
const repertoire = useLocalStorage('kchess:repertoire-study', '')
function drill(study: SavedStudy): void {
  repertoire.value = study.id
  practiceTab.value = 'openings'
  app.selectPage('practice')
}
function actions(study: SavedStudy): DropdownMenuItem[][] {
  return [
    [
      { label: 'Open', icon: 'i-lucide-square-arrow-out-up-right', onSelect: () => open(study) },
      {
        label: 'Drill as repertoire',
        icon: 'i-lucide-graduation-cap',
        onSelect: () => drill(study),
      },
    ],
    [
      { label: 'Rename…', icon: 'i-lucide-pencil', onSelect: () => startRename(study) },
      { label: 'Duplicate', icon: 'i-lucide-copy', onSelect: () => duplicate(study) },
      { label: 'Copy PGN', icon: 'i-lucide-file-text', onSelect: () => void copyPgn(study) },
    ],
    [{ label: 'Delete', icon: 'i-lucide-trash-2', color: 'error', onSelect: () => remove(study) }],
  ]
}

/* ── Import ── */
const importOpen = ref(false)
const importText = ref('')
const importName = ref('')
const importError = ref('')
function startImport(): void {
  importText.value = ''
  importName.value = ''
  importError.value = ''
  importOpen.value = true
}
function runImport(): void {
  const tree = treeFromPgn(importText.value.trim())
  if (!tree) {
    importError.value = 'Paste one PGN game with legal moves (variations and comments are kept).'
    return
  }
  const headers = tree.headers ?? {}
  const fallback = [headers.White, headers.Black].every((v) => v && v !== '?')
    ? `${headers.White} – ${headers.Black}`
    : headers.Event && headers.Event !== '?'
      ? headers.Event
      : library.freshName('Imported study')
  try {
    const id = library.save(importName.value.trim() || fallback, importText.value.trim())
    importOpen.value = false
    const study = library.items.find((item) => item.id === id)
    toast.add({
      title: 'Study imported',
      icon: 'i-lucide-book-check',
      actions: study ? [{ label: 'Open', onClick: () => open(study) }] : [],
    })
  } catch (cause) {
    importError.value = cause instanceof Error ? cause.message : String(cause)
  }
}
</script>

<template>
  <div>
    <PageHeader title="Studies">
      <template v-if="library.items.length || tab === 'lichess'">
        <UButton variant="outline" color="neutral" icon="i-lucide-import" @click="startImport"
          >Import PGN</UButton
        >
        <UButton icon="i-lucide-plus" @click="create">New study</UButton>
      </template>
    </PageHeader>

    <UTabs
      v-if="app.connectedAccounts.length"
      v-model="tab"
      :items="tabs"
      :content="false"
      variant="pill"
      size="sm"
      class="mb-5"
    />

    <LichessStudies v-if="tab === 'lichess' && app.connectedAccounts.length" />

    <template v-else>
      <div v-if="library.items.length" class="toolbar">
        <UInput
          v-model="query"
          icon="i-lucide-search"
          placeholder="Search by name, players or event"
          aria-label="Search studies"
          class="flex-1 min-w-48"
        />
        <USelect v-model="sort" :items="sorts" aria-label="Sort studies" class="w-44" />
        <span class="text-xs muted whitespace-nowrap">{{ library.items.length }} of 50</span>
      </div>
      <p v-if="library.error" role="alert" class="mb-3 text-sm text-error">{{ library.error }}</p>

      <div v-if="!library.items.length" class="card empty">
        <UIcon name="i-lucide-library-big" class="empty-icon" />
        <h2 class="text-lg font-semibold">No studies yet</h2>
        <div class="flex gap-2">
          <UButton icon="i-lucide-plus" @click="create">New study</UButton>
          <UButton variant="outline" color="neutral" icon="i-lucide-import" @click="startImport"
            >Import PGN</UButton
          >
        </div>
      </div>

      <p v-else-if="!cards.length" class="muted py-10 text-center">
        No study matches “{{ query }}”.
      </p>

      <ul v-else class="study-grid" aria-label="Saved studies">
        <li
          v-for="{ study, summary } in cards"
          :key="study.id"
          class="study-card"
          :class="{ current: analysis.studyId === study.id }"
        >
          <button
            type="button"
            class="study-open"
            :aria-label="`Open ${study.name}`"
            @click="open(study)"
          >
            <div class="thumb">
              <ChessBoard
                v-if="summary"
                :fen="summary.fen"
                :theme="settings?.boardTheme"
                coordinates="none"
                :piece-set="settings?.pieceSet"
                animation="none"
                :interactive="false"
              />
            </div>
          </button>
          <div class="study-meta">
            <div class="flex items-start gap-1">
              <button type="button" class="study-title" @click="open(study)">
                {{ study.name }}
              </button>
              <UDropdownMenu :items="actions(study)">
                <UButton
                  size="xs"
                  variant="ghost"
                  color="neutral"
                  icon="i-lucide-ellipsis-vertical"
                  :aria-label="`Actions for ${study.name}`"
                />
              </UDropdownMenu>
            </div>
            <p v-if="summary?.players || summary?.event" class="study-line">
              {{ [summary.players, summary.event].filter(Boolean).join(' · ') }}
            </p>
            <p class="study-line">{{ details(summary) }}</p>
            <p class="study-line dim">
              <UBadge v-if="analysis.studyId === study.id" size="sm" variant="soft" class="mr-1"
                >On the board</UBadge
              >Edited {{ timeAgo(study.updatedAt, now) }}
            </p>
          </div>
        </li>
      </ul>
    </template>

    <ConfirmDialog
      :open="pending !== null"
      title="Replace the unsaved analysis?"
      description="The analysis board has moves that are not saved as a study. Opening another study replaces them."
      confirm-label="Replace"
      color="error"
      @update:open="!$event && (pending = null)"
      @confirm="(pending?.(), (pending = null))"
    />

    <UModal
      :open="renaming !== null"
      title="Rename study"
      @update:open="!$event && (renaming = null)"
    >
      <template #body>
        <form class="flex gap-2" @submit.prevent="rename">
          <UInput
            v-model="newName"
            autofocus
            :maxlength="120"
            aria-label="New study name"
            class="flex-1"
          />
          <UButton type="submit" :disabled="!newName.trim()">Rename</UButton>
        </form>
      </template>
    </UModal>

    <UModal v-model:open="importOpen" title="Import a study" description="Paste one PGN game.">
      <template #body>
        <form class="flex flex-col gap-3" @submit.prevent="runImport">
          <UInput
            v-model="importName"
            placeholder="Name (optional: taken from the PGN)"
            aria-label="Study name"
            :maxlength="120"
          />
          <textarea
            v-model="importText"
            class="import-text"
            rows="8"
            spellcheck="false"
            aria-label="PGN"
            placeholder='[Event "…"]&#10;1. e4 e5 2. Nf3 { The main idea } Nc6 (2... d6) *'
          />
          <span v-if="importError" class="text-sm text-error">{{ importError }}</span>
          <div class="flex justify-end gap-2">
            <UButton color="neutral" variant="ghost" @click="importOpen = false">Cancel</UButton>
            <UButton type="submit" :disabled="!importText.trim()">Import</UButton>
          </div>
        </form>
      </template>
    </UModal>
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;
}
.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 48px 24px;
  text-align: center;
}
.empty-icon {
  color: var(--ui-text-dimmed);
  font-size: 40px;
}
.study-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
  gap: 16px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.study-card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--ui-border);
  border-radius: 12px;
  background: var(--ui-bg-elevated);
  transition:
    border-color 0.15s,
    transform 0.15s;
}
.study-card:hover {
  border-color: var(--ui-border-accented);
  transform: translateY(-2px);
}
.study-card.current {
  border-color: color-mix(in srgb, var(--ui-primary) 70%, transparent);
}
.study-open {
  display: block;
  padding: 0;
  border: 0;
  background: transparent;
  cursor: pointer;
}
.thumb {
  aspect-ratio: 1;
  pointer-events: none;
}
.study-meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px 8px 12px 12px;
}
.study-title {
  flex: 1;
  min-width: 0;
  padding: 2px 0;
  border: 0;
  background: transparent;
  color: var(--ui-text-highlighted);
  font: inherit;
  font-weight: 650;
  text-align: left;
  overflow-wrap: anywhere;
  cursor: pointer;
}
.study-title:hover {
  color: var(--ui-primary);
}
.study-line {
  overflow: hidden;
  color: var(--ui-text-muted);
  font-size: 12px;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.study-line.dim {
  color: var(--ui-text-dimmed);
}
.import-text {
  width: 100%;
  padding: 10px;
  border: 1px solid var(--ui-border);
  border-radius: 8px;
  background: var(--ui-bg);
  color: inherit;
  font:
    12px/1.5 ui-monospace,
    SFMono-Regular,
    Menlo,
    monospace;
  resize: vertical;
}
</style>
