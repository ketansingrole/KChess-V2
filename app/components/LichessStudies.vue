<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useDocumentVisibility, useIntervalFn, useOnline, useWindowFocus } from '@vueuse/core'
import { RequestScope } from '../../src/shared/requestScope'
import StudyCard from './StudyCard.vue'
import { useLichessStudiesStore } from '../stores/lichessStudies'
import { useAnalysisStore } from '../stores/analysis'
import { useStudyStore } from '../stores/studies'
import type { LichessStudyChapter } from '../../src/shared/types'

/** Studies on the player's Lichess accounts: open or keep their chapters, or send the board to one. */
const analysis = useAnalysisStore()
const library = useStudyStore()
const app = useKChessStore()

const account = ref(app.activeOnlineAccount)
const cache = useLichessStudiesStore()
const cached = computed(() => cache.entry(account.value))
const remote = computed(() => cached.value?.items ?? null)
const online = useOnline()
const focused = useWindowFocus()
const visibility = useDocumentVisibility()
const requests = new RequestScope()
onUnmounted(() => requests.invalidate())
const busy = ref(false)
const message = ref('')
const NEW_STUDY = 'new-study'
const target = ref(NEW_STUDY)
const pendingOpen = ref<(() => void) | null>(null)
function guardOpen(action: () => void): void {
  if (!analysis.study && analysis.root.children.length) pendingOpen.value = action
  else action()
}

function reconnectNote(result: unknown): boolean {
  if (result && typeof result === 'object' && 'needsReconnect' in result) {
    message.value = `Lichess needs the study permission: connect @${account.value} again in Settings → My accounts.`
    return true
  }
  return false
}
async function work(task: () => Promise<void>): Promise<void> {
  const request = requests.capture()
  busy.value = true
  message.value = ''
  try {
    await task()
  } catch (cause) {
    if (request.current()) message.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    if (request.current()) busy.value = false
  }
}
function load(force = false): Promise<void> {
  return online.value ? cache.refresh(account.value, force) : Promise.resolve()
}
watch(
  account,
  () => {
    requests.invalidate()
    target.value = NEW_STUDY
    message.value = ''
    busy.value = false
    void load()
  },
  { immediate: true },
)
watch(
  () => app.activeOnlineAccount,
  (active) => {
    account.value = active
  },
)
function revalidate(): void {
  if (focused.value && visibility.value === 'visible') void load()
}
watch([online, focused, visibility], revalidate)
useIntervalFn(revalidate, 60_000)
const status = computed(
  () =>
    message.value ||
    cached.value?.error ||
    (cached.value?.needsReconnect
      ? `Connect @${account.value} again in Settings → My accounts to allow study access.`
      : remote.value?.length === 0
        ? 'No studies on this account yet.'
        : ''),
)
function openRemote(id: string): Promise<void> {
  const saved = offlineCopy(id)
  if (saved) {
    guardOpen(() => {
      if (analysis.openStudy(saved.id)) app.selectPage('analysis')
      else message.value = 'This study could not be opened.'
    })
    return Promise.resolve()
  }
  if (!online.value) {
    message.value = 'Download this study before opening it offline.'
    return Promise.resolve()
  }
  const openingAccount = account.value
  const item = remote.value?.find((s) => s.id === id)
  if (!item) return Promise.resolve()
  const request = requests.next()
  return work(async () => {
    const result = await window.kchess.lichessStudyChapters(openingAccount, id)
    if (!request.current() || reconnectNote(result)) return
    guardOpen(() => {
      if (!request.current()) return
      try {
        const downloaded = library.offline(openingAccount, item, result as LichessStudyChapter[])
        if (!analysis.openStudy(downloaded.id)) throw new Error('This study could not be opened.')
        app.selectPage('analysis')
      } catch (cause) {
        message.value = cause instanceof Error ? cause.message : String(cause)
      }
    })
  })
}
function offlineCopy(id: string) {
  return library.items.find(
    (s) => s.cloud?.id === id && s.cloud.account.toLowerCase() === account.value.toLowerCase(),
  )
}
function download(id: string): Promise<void> {
  const downloadingAccount = account.value
  const item = remote.value?.find((s) => s.id === id)
  if (!item) return Promise.resolve()
  return work(async () => {
    const request = requests.capture()
    const result = await window.kchess.lichessStudyChapters(downloadingAccount, id)
    if (!request.current() || reconnectNote(result)) return
    analysis.flushStudy()
    const saved = library.offline(downloadingAccount, item, result as LichessStudyChapter[])
    if (analysis.studyId === saved.id) analysis.openStudy(saved.id, analysis.studyChapterId)
    if (saved.conflict)
      message.value =
        'Your offline edits were kept. The cloud version was downloaded as a separate copy.'
    else message.value = 'Study available offline.'
  })
}
function send(): Promise<void> {
  return work(async () => {
    const request = requests.capture()
    const sendingAccount = account.value
    const sendingTarget = target.value
    const chapterName = (
      analysis.study?.name ||
      analysis.root.headers?.Event ||
      'KChess analysis'
    ).slice(0, 100)
    const result = await window.kchess.exportToLichessStudy(
      sendingAccount,
      sendingTarget === NEW_STUDY ? '' : sendingTarget,
      chapterName,
      analysis.pgn(),
    )
    if (!request.current()) return
    if (reconnectNote(result)) return
    message.value =
      sendingTarget !== NEW_STUDY
        ? `Added “${chapterName}” as a chapter.`
        : `Created a private study with “${chapterName}”.`
    void cache.refresh(sendingAccount, true)
  })
}
</script>

<template>
  <div class="flex flex-col gap-4">
    <div class="flex flex-wrap items-center gap-2">
      <USelect
        v-if="app.connectedAccounts.length > 1"
        v-model="account"
        :items="app.connectedAccounts.map((a) => ({ label: `@${a.username}`, value: a.username }))"
        size="sm"
        aria-label="Account"
      />
      <UButton
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-refresh-cw"
        :loading="cached?.loading"
        :disabled="!online"
        @click="load(true)"
        >Refresh</UButton
      >
    </div>
    <p v-if="status" role="status" class="text-sm">{{ status }}</p>

    <ul v-if="remote?.length" class="remote-grid" aria-label="Lichess studies">
      <li v-for="item in remote" :key="item.id">
        <StudyCard
          :name="item.name"
          :updated-at="item.updatedAt"
          :pgn="offlineCopy(item.id)?.pgn"
          :chapter-count="offlineCopy(item.id)?.chapters.length"
          :location="offlineCopy(item.id) ? 'Offline · Lichess' : 'Lichess'"
          @open="openRemote(item.id)"
        >
          <UButton
            size="xs"
            variant="outline"
            color="neutral"
            icon="i-lucide-download"
            :loading="busy"
            :disabled="!online || busy"
            @click="download(item.id)"
            >{{ offlineCopy(item.id) ? 'Download latest' : 'Make available offline' }}</UButton
          >
        </StudyCard>
      </li>
    </ul>

    <form class="card flex flex-wrap items-center gap-2" @submit.prevent="send">
      <span class="text-sm font-semibold">Send the analysis board to</span>
      <USelect
        v-model="target"
        :items="[
          { label: 'A new private study', value: NEW_STUDY },
          ...(remote ?? []).map((s) => ({ label: s.name, value: s.id })),
        ]"
        size="sm"
        aria-label="Lichess study to add to"
        class="min-w-48"
      />
      <UButton type="submit" size="sm" icon="i-lucide-upload" :loading="busy">Send</UButton>
    </form>
    <ConfirmDialog
      :open="pendingOpen !== null"
      title="Replace the unsaved analysis?"
      description="Opening this chapter replaces the unsaved analysis board."
      confirm-label="Replace"
      color="error"
      @update:open="!$event && (pendingOpen = null)"
      @confirm="(pendingOpen?.(), (pendingOpen = null))"
    />
  </div>
</template>

<style scoped>
.remote-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
  gap: 16px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.remote-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.remote-row:hover {
  background: var(--ui-bg-elevated);
}
.chapters {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 0 0 8px 30px;
}
</style>
