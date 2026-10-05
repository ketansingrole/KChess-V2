<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useAnalysisStore } from '../stores/analysis'
import { useStudyStore } from '../stores/studies'
import { treeFromPgn } from '../utils/analysisTree'
import type { LichessStudy, LichessStudyChapter } from '../../src/shared/types'
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
/* ── Lichess studies ───────────────────────────────────────────────── */
const app = useKChessStore()
const account = ref(app.activeOnlineAccount)
const remote = ref<LichessStudy[] | null>(null)
const remoteStudy = ref('')
const chapters = ref<LichessStudyChapter[] | null>(null)
const remoteBusy = ref(false)
const remoteMessage = ref('')
const target = ref('')
function reconnectNote(result: unknown): boolean {
  if (result && typeof result === 'object' && 'needsReconnect' in result) {
    remoteMessage.value = `Lichess needs the study permission: connect @${account.value} again in Settings → My accounts.`
    return true
  }
  return false
}
async function remoteWork(work: () => Promise<void>): Promise<void> {
  remoteBusy.value = true
  remoteMessage.value = ''
  try {
    await work()
  } catch (cause) {
    remoteMessage.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    remoteBusy.value = false
  }
}
function loadRemote(): Promise<void> {
  return remoteWork(async () => {
    const result = await window.kchess.lichessStudies(account.value)
    if (reconnectNote(result)) return
    remote.value = result as LichessStudy[]
    if (!remote.value.length) remoteMessage.value = 'No studies on this account yet.'
  })
}
function openRemote(id: string): Promise<void> {
  remoteStudy.value = id
  chapters.value = null
  return remoteWork(async () => {
    const result = await window.kchess.lichessStudyChapters(account.value, id)
    if (reconnectNote(result)) return
    chapters.value = result as LichessStudyChapter[]
  })
}
function openChapter(chapter: LichessStudyChapter): void {
  if (!analysis.loadPgn(chapter.pgn)) {
    remoteMessage.value = 'That chapter uses rules the analysis board cannot show (a variant).'
    return
  }
  selected.value = ''
  name.value = chapter.name
  feedback.value = `Opened “${chapter.name}” from Lichess. Save it to keep it here.`
}
function saveChapters(): void {
  const studyName = remote.value?.find((s) => s.id === remoteStudy.value)?.name ?? 'Lichess study'
  let saved = 0
  try {
    for (const chapter of chapters.value ?? []) {
      if (!treeFromPgn(chapter.pgn)) continue
      library.save(`${studyName} · ${chapter.name}`.slice(0, 120), chapter.pgn)
      saved++
    }
    feedback.value = `Saved ${saved} chapters to this device.`
  } catch (error) {
    feedback.value = error instanceof Error ? error.message : String(error)
  }
}
function sendToLichess(): Promise<void> {
  return remoteWork(async () => {
    const chapterName = (name.value || header('Event') || 'KChess analysis').slice(0, 100)
    const result = await window.kchess.exportToLichessStudy(
      account.value,
      target.value,
      chapterName,
      analysis.pgn(),
    )
    if (reconnectNote(result)) return
    remoteMessage.value = target.value
      ? `Added “${chapterName}” as a chapter.`
      : `Created a private study with “${chapterName}”.`
    void loadRemote()
  })
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
    <section v-if="app.connectedAccounts.length" class="panel-divider mt-4 pt-3">
      <h3 class="section-title text-sm">Lichess studies</h3>
      <p class="text-xs muted mt-1">
        Import chapters from your Lichess studies, or send this analysis to one. Chapters keep their
        variations, comments and annotations.
      </p>
      <div class="mt-2 flex flex-wrap items-center gap-2">
        <USelect
          v-if="app.connectedAccounts.length > 1"
          v-model="account"
          :items="
            app.connectedAccounts.map((a) => ({ label: `@${a.username}`, value: a.username }))
          "
          size="sm"
          aria-label="Account"
        />
        <UButton
          size="sm"
          variant="outline"
          color="neutral"
          :loading="remoteBusy"
          @click="loadRemote"
          >{{ remote ? 'Refresh studies' : 'Show my Lichess studies' }}</UButton
        >
      </div>
      <p v-if="remoteMessage" role="status" class="mt-2 text-sm">{{ remoteMessage }}</p>
      <ul v-if="remote?.length" class="mt-2 space-y-1">
        <li v-for="study in remote" :key="study.id">
          <UButton
            variant="link"
            :color="remoteStudy === study.id ? 'primary' : 'neutral'"
            @click="openRemote(study.id)"
            >{{ study.name }}</UButton
          >
          <ul v-if="remoteStudy === study.id && chapters" class="ml-4 space-y-1">
            <li v-for="chapter in chapters" :key="chapter.name + chapter.pgn.length">
              <UButton size="xs" variant="link" @click="openChapter(chapter)">{{
                chapter.name
              }}</UButton>
            </li>
            <li>
              <UButton
                size="xs"
                variant="soft"
                color="neutral"
                icon="i-lucide-download"
                @click="saveChapters"
                >Save all chapters on this device</UButton
              >
            </li>
          </ul>
        </li>
      </ul>
      <form class="mt-3 flex flex-wrap items-center gap-2" @submit.prevent="sendToLichess">
        <USelect
          v-model="target"
          :items="[
            { label: 'New private study', value: '' },
            ...(remote ?? []).map((s) => ({ label: s.name, value: s.id })),
          ]"
          size="sm"
          aria-label="Lichess study to add to"
          class="min-w-48"
        />
        <UButton type="submit" size="sm" icon="i-lucide-upload" :loading="remoteBusy"
          >Send this analysis to Lichess</UButton
        >
      </form>
    </section>
  </details>
</template>
