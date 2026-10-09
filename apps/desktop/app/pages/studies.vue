<script setup lang="ts">
import { computed, ref } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import type { DropdownMenuItem } from '@nuxt/ui'
import { useAnalysisStore } from '../stores/analysis'
import { useStudyStore, type SavedStudy } from '../stores/studies'
import { treeFromPgn } from '@kchess/core/domain/analysisTree'
import { summarizeStudy } from '@kchess/core/domain/studies'
import { pgnGames } from '@kchess/core/domain/pgn'
import StudyCard from '../components/StudyCard.vue'
import { useLichessStudiesStore } from '../stores/lichessStudies'

/** The study library: browse, search and open saved studies, or bring them in from Lichess. */
const app = useKChessStore()
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
const uploading = ref('')
/* ── Opening: never silently throw away an unsaved board ── */
const pending = ref<(() => void) | null>(null)
const unsavedBoard = computed(() => !analysis.study && analysis.root.children.length > 0)
function guard(action: () => void | Promise<void>): void {
  const run = (): void => void action()
  if (unsavedBoard.value) pending.value = run
  else run()
}
function open(study: SavedStudy, chapterId?: string): void {
  if (analysis.studyId === study.id && (!chapterId || analysis.studyChapterId === chapterId))
    return app.selectPage('analysis')
  guard(() => {
    if (analysis.openStudy(study.id, chapterId)) app.selectPage('analysis')
  })
}
function create(): void {
  guard(async () => {
    analysis.load()
    try {
      await analysis.saveAsStudy(library.freshName())
      app.selectPage('analysis')
    } catch (cause) {
      console.warn('[studies] Could not save study:', cause)
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
const failed = (what: string) => (cause: unknown) => {
  console.warn(`[studies] Could not ${what}:`, cause)
  toast.add({ title: cause instanceof Error ? cause.message : String(cause), color: 'error' })
}
function rename(): void {
  if (renaming.value && newName.value.trim())
    library.rename(renaming.value.id, newName.value).catch(failed('rename study'))
  renaming.value = null
}
async function remove(study: SavedStudy): Promise<void> {
  let removed
  try {
    removed = await library.remove(study.id)
  } catch (cause) {
    console.warn('[studies] Could not delete study:', cause)
    toast.add({ title: cause instanceof Error ? cause.message : String(cause), color: 'error' })
    return
  }
  if (!removed) return
  const undo = removed
  toast.add({
    title: `Deleted “${removed.name}”`,
    icon: 'i-lucide-trash-2',
    actions: [
      { label: 'Undo', onClick: () => void library.restore(undo).catch(failed('restore study')) },
    ],
  })
}
async function duplicate(study: SavedStudy): Promise<void> {
  try {
    await library.duplicate(study.id)
  } catch (cause) {
    console.warn('[studies] Could not duplicate study:', cause)
    toast.add({ title: String(cause), color: 'error' })
  }
}
async function copyPgn(study: SavedStudy): Promise<void> {
  try {
    await navigator.clipboard.writeText(library.documentPgn(study))
    toast.add({ title: 'PGN copied', icon: 'i-lucide-clipboard-check' })
  } catch (cause) {
    console.warn('[studies] Copy PGN failed:', cause)
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
      ...(study.cloud && !study.cloud.structureChanged
        ? [
            {
              label: 'Upload a new cloud copy',
              icon: 'i-lucide-cloud-upload',
              disabled: !!uploading.value,
              onSelect: () => void upload(study, true),
            },
          ]
        : []),
    ],
    [
      {
        label: study.cloud ? 'Remove offline copy' : 'Delete',
        icon: 'i-lucide-trash-2',
        color: 'error',
        onSelect: () => remove(study),
      },
    ],
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
async function runImport(): Promise<void> {
  const games = pgnGames(importText.value.trim())
  const chapters = games.map((game, index) => ({
    name: game.headers.find(([key]) => key === 'ChapterName')?.[1] ?? `Chapter ${index + 1}`,
    pgn: game.pgn,
  }))
  const tree = chapters[0] ? treeFromPgn(chapters[0].pgn) : undefined
  if (
    !tree ||
    !chapters.length ||
    chapters.length > 64 ||
    chapters.some((c) => !treeFromPgn(c.pgn))
  ) {
    importError.value = 'Paste valid PGN with up to 64 chapters.'
    return
  }
  const headers = tree.headers ?? {}
  const fallback = [headers.White, headers.Black].every((v) => v && v !== '?')
    ? `${headers.White} – ${headers.Black}`
    : headers.Event && headers.Event !== '?'
      ? headers.Event
      : library.freshName('Imported study')
  try {
    const id = await library.saveChapters(importName.value.trim() || fallback, chapters)
    importOpen.value = false
    const study = library.items.find((item) => item.id === id)
    toast.add({
      title: 'Study imported',
      icon: 'i-lucide-book-check',
      actions: study ? [{ label: 'Open', onClick: () => open(study) }] : [],
    })
  } catch (cause) {
    console.warn('[studies] Import failed:', cause)
    importError.value = cause instanceof Error ? cause.message : String(cause)
  }
}
async function upload(study: SavedStudy, copy = false): Promise<void> {
  const account = app.activeOnlineAccount
  if (!account) {
    toast.add({ title: 'Connect Lichess to upload a study', color: 'warning' })
    return
  }
  if (uploading.value) return
  if (!copy && study.cloud && study.cloud.account.toLowerCase() !== account.toLowerCase()) {
    toast.add({
      title: `Switch to @${study.cloud.account} to upload changes to this study.`,
      color: 'warning',
    })
    return
  }
  uploading.value = study.id
  try {
    await analysis.flushStudy()
    study = library.items.find((item) => item.id === study.id) ?? study
    const pgn = library.documentPgn(study)
    const cloud = study.cloud
    const linked =
      !copy && !cloud?.structureChanged && cloud?.account.toLowerCase() === account.toLowerCase()
    const result = linked
      ? await window.kchess.syncLichessStudy({
          account,
          studyId: cloud!.id,
          baseline: cloud!.downloadedPgn,
          pgn,
        })
      : await window.kchess.exportToLichessStudy(account, '', study.name.slice(0, 100), pgn)
    if ('needsReconnect' in result)
      throw new Error(`Connect @${account} again to allow study access.`)
    const chapters = Array.isArray(result)
      ? result
      : await window.kchess.lichessStudyChapters(account, result.id)
    if ('needsReconnect' in chapters)
      throw new Error('Uploaded, but study access needs reconnection.')
    await library.markCloud(
      study.id,
      account,
      linked ? cloud!.id : (result as { id: string }).id,
      chapters.map((c) => c.pgn).join('\n\n'),
    )
    void useLichessStudiesStore().refresh(account, true)
    toast.add({ title: 'Study uploaded', icon: 'i-lucide-cloud-upload' })
  } catch (cause) {
    console.warn('[studies] Study upload failed:', cause)
    toast.add({ title: cause instanceof Error ? cause.message : String(cause), color: 'error' })
  } finally {
    uploading.value = ''
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
        <li v-for="{ study } in cards" :key="study.id">
          <StudyCard
            :name="study.name"
            :updated-at="study.updatedAt"
            :pgn="study.pgn"
            :chapter-count="study.chapters.length"
            :location="study.cloud ? 'Offline · Lichess' : 'On this device'"
            @open="open(study)"
          >
            <template #menu>
              <UDropdownMenu :items="actions(study)"
                ><UButton
                  size="xs"
                  variant="ghost"
                  color="neutral"
                  icon="i-lucide-ellipsis-vertical"
                  :aria-label="`Actions for ${study.name}`"
              /></UDropdownMenu>
            </template>
            <UButton
              size="xs"
              variant="outline"
              color="neutral"
              icon="i-lucide-cloud-upload"
              :loading="uploading === study.id"
              :disabled="!!uploading"
              @click="upload(study)"
              >{{
                study.cloud?.structureChanged
                  ? 'Upload as new cloud study'
                  : study.cloud
                    ? 'Upload changes'
                    : 'Upload to Lichess'
              }}</UButton
            >
          </StudyCard>
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

    <UModal v-model:open="importOpen" title="Import a study" description="Paste PGN chapters.">
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
.import-text {
  width: 100%;
  padding: 10px;
  border: 1px solid var(--ui-border);
  border-radius: 8px;
  background: var(--ui-bg);
  color: inherit;
  font:
    12px/1.5 ui-monospace,
    monospace;
  resize: vertical;
}
</style>
